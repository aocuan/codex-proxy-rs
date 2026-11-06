<script setup lang="ts">
import type { EvaluationConfig, EvaluationSettings } from '@/api'
import { ZButton, ZCheckbox, ZDialog, ZForm, ZFormItem, ZInput, ZInputNumber, ZSelect } from '@codex-proxy/ui'
import { computed, shallowRef, watch } from 'vue'
import { createAccountGroup, getEvaluationConfig, getEvaluationPlans, getEvaluationSettings } from '@/api'
import { useAsyncAction } from '@/composables/useAsyncAction'
import { useRequestState } from '@/composables/useRequestState'
import { useEvaluationCatalog } from '../composables/useEvaluationCatalog'
import { useEvaluationEditor } from '../composables/useEvaluationEditor'

const emit = defineEmits<{ saved: [] }>()
const open = defineModel<boolean>({ required: true })
const request = useRequestState()
const { loading, error } = request
const config = shallowRef<EvaluationConfig | null>(null)
const settings = shallowRef<EvaluationSettings | null>(null)
const catalog = useEvaluationCatalog()
const editor = useEvaluationEditor(async () => {
  open.value = false
  emit('saved')
}, config, settings)
const { parameters, accountSelection, enabled, intervalMinutes, monitoringPolicy, editing } = editor
const groupAction = useAsyncAction()
const saving = computed(() => editor.saving.value || groupAction.loading.value)
const advanced = shallowRef(false)
const target = shallowRef('create')
const groupName = shallowRef('优选账号')
const selectionMode = computed({
  get: () => accountSelection.value.mode,
  set: (mode: 'all' | 'groups' | 'fixed') => {
    accountSelection.value = mode === 'groups'
      ? { mode, groupIds: [] }
      : { mode }
  },
})
const accountOptions = computed(() => catalog.accountOptions(parameters.value.accountIds, 'turnstate'))
const targetOptions = computed(() => [
  { value: 'create', label: '新建优选分组' },
  { value: 'none', label: '仅监测，不调整分组' },
  ...catalog.preferenceGroupOptions(target.value === 'create' || target.value === 'none'
    ? ''
    : target.value),
])
const sourceConflict = computed(() => accountSelection.value.mode === 'groups' && accountSelection.value.groupIds.includes(target.value))
const timingError = computed(() => {
  const interval = Number(intervalMinutes.value)
  if (!Number.isInteger(interval) || interval < 1 || interval > 525600)
    return '检测间隔须为 1 到 525600 分钟'
  if (target.value !== 'none' && monitoringPolicy.value.validMinutes <= interval * (Math.max(monitoringPolicy.value.stableCount, monitoringPolicy.value.changedCount) - 1))
    return '结果有效期须长于连续确认所需的时间'
  return ''
})
const invalid = computed(() => !parameters.value.model.trim() || sourceConflict.value || !!timingError.value
  || (target.value === 'create' && !groupName.value.trim())
  || (accountSelection.value.mode === 'fixed' && (!parameters.value.accountIds.length || parameters.value.accountIds.length > 20
    || parameters.value.accountIds.some(id => accountOptions.value.find(option => option.value === id)?.disabled)))
  || (accountSelection.value.mode === 'groups' && !accountSelection.value.groupIds.length))

async function load() {
  const id = request.start()
  try {
    const [plans, loadedConfig, loadedSettings] = await Promise.all([
      getEvaluationPlans({ signal: request.signal }),
      getEvaluationConfig({ signal: request.signal }),
      getEvaluationSettings({ signal: request.signal }),
      catalog.load({ keys: false }),
    ])
    if (!request.isCurrent(id))
      return
    config.value = loadedConfig
    settings.value = loadedSettings
    const ticketPlans = plans.filter(plan => plan.settings.parameters.kind === 'turnstate')
    const plan = plans.find(plan => plan.settings.monitoring) ?? (ticketPlans.length === 1
      ? ticketPlans[0]!
      : null)
    editor.show(true, plan)
    parameters.value.kind = 'turnstate'
    parameters.value.definition = undefined
    parameters.value.clientKeyId = ''
    parameters.value.reasoningEffort = 'low'
    parameters.value.repetitions = 1
    editor.monitoringEnabled.value = true
    if (!plan) {
      editor.name.value = '账号优选'
      parameters.value.timeoutSeconds = 60
      intervalMinutes.value = '10'
      accountSelection.value = { mode: 'all' }
    }
    target.value = plan?.settings.accountPreference?.groupId ?? (plan?.settings.monitoring ? 'none' : 'create')
    groupName.value = '优选账号'
    advanced.value = !parameters.value.model
  }
  catch (cause) { request.fail(id, cause) }
  finally { request.finish(id) }
}

async function save() {
  if (invalid.value || saving.value)
    return
  await groupAction.run(async () => {
    if (target.value === 'create') {
      const group = await createAccountGroup({
        name: groupName.value.trim(),
        description: null,
        color: '#3B82F6FF',
      })
      // 分组创建与计划保存分别提交，后续失败时重试复用已创建的组
      target.value = group.id
      await catalog.load({ keys: false })
    }
    editor.preferenceEnabled.value = target.value !== 'none'
    editor.preferenceGroupId.value = target.value === 'none'
      ? ''
      : target.value
    await editor.save()
  })
}

watch(open, (value) => {
  if (value)
    void load()
  else
    request.invalidate()
}, { immediate: true })
</script>

<template>
  <ZDialog v-model="open" title="账号优选" width="36rem" :show-close="!saving" :close-on-click-modal="!saving" :close-on-press-escape="!saving">
    <p v-if="loading" class="text-z-sm text-z-text-secondary" role="status">
      正在读取优选设置
    </p>
    <div v-else-if="error" class="flex items-center gap-3" role="alert">
      <span class="text-z-sm text-z-error-text">{{ error }}</span>
      <ZButton @click="load">
        重新加载
      </ZButton>
    </div>
    <ZForm v-else class="grid gap-5">
      <ZFormItem label="检测范围" required>
        <ZSelect v-model="selectionMode" :disabled="saving" :options="[{ value: 'all', label: '全部启用的 OAuth 账号' }, { value: 'groups', label: '指定来源分组' }, { value: 'fixed', label: '指定账号' }]" aria-label="检测范围" />
        <p class="mt-2 mb-0 text-z-sm text-z-text-secondary">
          自动排除 API Key 账号，登录与代理沿用账号配置
        </p>
      </ZFormItem>
      <ZFormItem v-if="accountSelection.mode === 'groups'" label="来源分组" required>
        <ZSelect v-model="accountSelection.groupIds" :options="catalog.groups.value" :disabled="saving" :loading="catalog.loadingGroups.value" multiple filterable placeholder="选择要持续检测的来源分组" aria-label="来源分组" />
      </ZFormItem>
      <ZFormItem v-if="accountSelection.mode === 'fixed'" label="检测账号" required>
        <ZSelect v-model="parameters.accountIds" :options="accountOptions" :disabled="saving" :loading="catalog.loadingAccounts.value" multiple remote filterable collapse-tags placeholder="最多选择 20 个账号" aria-label="检测账号" @query-change="catalog.searchAccounts" />
      </ZFormItem>
      <ZFormItem label="优选结果分组" required>
        <ZSelect v-model="target" :options="targetOptions" :disabled="saving" :loading="catalog.loadingGroups.value" remote filterable aria-label="优选结果分组" @query-change="catalog.searchGroups" />
        <ZInput v-if="target === 'create'" v-model="groupName" class="mt-2" :disabled="saving" maxlength="128" placeholder="优选分组名称" aria-label="新分组名称" />
        <p v-if="sourceConflict" class="mt-2 mb-0 text-z-sm text-z-warning-text" role="alert">
          来源分组不能包含优选结果组，否则移出的账号无法继续复检
        </p>
        <p v-else-if="target !== 'none'" class="mt-2 mb-0 text-z-sm text-z-text-secondary">
          连续 {{ monitoringPolicy.stableCount }} 次票据稳定后加入，连续 {{ monitoringPolicy.changedCount }} 次变化后移出，仅调整这个分组
        </p>
        <p v-else class="mt-2 mb-0 text-z-sm text-z-text-secondary">
          只更新账号列表的票据状态，现有分组成员保持不变
        </p>
      </ZFormItem>
      <ZCheckbox v-model="enabled" :disabled="saving" label="启用定时检测" />
      <div class="flex flex-wrap items-center justify-between gap-2 text-z-sm text-z-text-secondary">
        <span>{{ parameters.model || '待设置模型' }} · 每 {{ intervalMinutes }} 分钟</span>
        <ZButton size="small" variant="text" :aria-expanded="advanced" @click="advanced = !advanced">
          {{ advanced ? '收起高级设置' : '高级设置' }}
        </ZButton>
      </div>
      <template v-if="advanced">
        <ZFormItem label="测票模型" required>
          <ZInput v-model="parameters.model" :disabled="saving" placeholder="填写上游模型 ID" aria-label="测票模型" />
        </ZFormItem>
        <div class="grid gap-4 sm:grid-cols-2">
          <ZFormItem label="检测间隔">
            <ZInput v-model="intervalMinutes" type="number" :min="1" :max="525600" :disabled="saving" aria-label="检测间隔">
              <template #suffix>
                分钟
              </template>
            </ZInput>
          </ZFormItem>
          <ZFormItem label="每账号超时">
            <ZInputNumber v-model="parameters.timeoutSeconds" :min="1" :max="600" :disabled="saving" unit="秒" aria-label="每账号超时" />
          </ZFormItem>
          <ZFormItem label="连续稳定后加入">
            <ZInputNumber v-model="monitoringPolicy.stableCount" :min="1" :max="10" :disabled="saving" unit="次" aria-label="连续稳定次数" />
          </ZFormItem>
          <ZFormItem label="连续变化后移出">
            <ZInputNumber v-model="monitoringPolicy.changedCount" :min="1" :max="10" :disabled="saving" unit="次" aria-label="连续变化次数" />
          </ZFormItem>
          <ZFormItem label="结果有效期">
            <ZInputNumber v-model="monitoringPolicy.validMinutes" :min="1" :max="10080" :disabled="saving" unit="分钟" aria-label="结果有效期" />
          </ZFormItem>
        </div>
        <ZCheckbox v-if="target !== 'none'" v-model="monitoringPolicy.removeExpired" :disabled="saving" label="结果过期或账号环境变化后移出优选组" />
      </template>
      <p v-if="timingError" class="m-0 text-z-sm text-z-warning-text" role="alert">
        {{ timingError }}
      </p>
      <p class="m-0 text-z-sm text-z-text-secondary">
        {{ enabled ? '保存后按间隔检测，也可从账号列表立即复测' : '暂停后停止检测与自动改组，现有成员保持不变' }}
        <span v-if="target !== 'none'" class="block">停用、授权失效、额度耗尽或模型不可用时移出，过载和超时保留成员</span>
        <span v-if="editing?.settings.cronExpression" class="block">保存后使用分钟间隔执行</span>
      </p>
    </ZForm>
    <template #footer>
      <ZButton :disabled="saving" @click="open = false">
        取消
      </ZButton>
      <ZButton type="primary" :loading="saving" :disabled="loading || !!error || invalid" @click="save">
        {{ enabled ? '保存并启用' : '保存为暂停' }}
      </ZButton>
    </template>
  </ZDialog>
</template>
