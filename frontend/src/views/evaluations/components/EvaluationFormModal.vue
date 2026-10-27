<script setup lang="ts">
import type { useEvaluationCatalog } from '../composables/useEvaluationCatalog'
import type { useEvaluationEditor } from '../composables/useEvaluationEditor'
import type { EvaluationConfig, EvaluationDefinition, EvaluationSettings } from '@/api'
import { ZButton, ZCheckbox, ZDialog, ZForm, ZFormItem, ZInput, ZInputNumber, ZNotification, ZSelect } from '@codex-proxy/ui'
import { isEqual } from 'es-toolkit'
import { computed, ref, shallowRef, toRaw, watch } from 'vue'
import { effortOptions, graderLabel, judgeGrader } from '@/components/evaluation/defaults'
import EvaluationDefinitionFields from '@/components/evaluation/EvaluationDefinitionFields.vue'
import { useEvaluationAccess } from '../composables/useEvaluationAccess'

const props = defineProps<{
  editor: ReturnType<typeof useEvaluationEditor>
  catalog: ReturnType<typeof useEvaluationCatalog>
  config: EvaluationConfig | null
  settings: EvaluationSettings | null
  settingsSavedRevision: number | null
}>()
const emit = defineEmits<{ configure: [] }>()
const { open, isPlan, editing, parameters, name, intervalMinutes, enabled, accountSelection, saving } = props.editor
const { loadingAccounts, loadingKeys, groups, loadingGroups } = props.catalog
const selectionMode = computed({
  get: () => accountSelection.value.mode,
  set: (mode: 'fixed' | 'all' | 'groups') => {
    accountSelection.value = mode === 'groups'
      ? { mode, groupIds: [] }
      : { mode }
  },
})
const automatic = computed(() => isPlan.value && selectionMode.value !== 'fixed')
const panel = shallowRef<'main' | 'definition' | 'execution'>('main')
const definitionDraft = ref<EvaluationDefinition | null>(null)
const access = useEvaluationAccess(() => open.value, () => parameters.value.clientKeyId, () => props.catalog.accountOptions(parameters.value.accountIds), () => parameters.value.accountIds)
const { accountOptions, message: accessMessage, valid: accessValid, loading: checkingAccess, error: accessError } = access
const keyOptions = computed(() => props.catalog.keyOptions(parameters.value.clientKeyId))
const mainTitle = computed(() => isPlan.value
  ? editing.value ? '编辑测评计划' : '新建测评计划'
  : '新建测评')
const title = computed(() => panel.value === 'definition'
  ? '编辑题目'
  : panel.value === 'execution'
    ? '运行设置'
    : mainTitle.value)
const grader = computed(() => parameters.value.definition?.grader)
const effort = computed({
  get: () => parameters.value.reasoningEffort ?? 'default',
  set: (value: string | undefined) => {
    parameters.value.reasoningEffort = value && value !== 'default'
      ? value
      : null
  },
})
const preset = computed({
  get: () => {
    const match = props.config?.tests.findIndex(item => isEqual(item.definition, parameters.value.definition)) ?? -1
    return match >= 0
      ? String(match)
      : 'custom'
  },
  set: (value: string | undefined) => {
    if (value === 'custom') {
      editDefinition(true)
      return
    }
    const selected = value === undefined
      ? undefined
      : props.config?.tests[Number(value)]
    if (selected) {
      parameters.value.kind = selected.kind
      parameters.value.definition = structuredClone(toRaw(selected.definition))
    }
  },
})
const presetOptions = computed(() => [
  ...(props.config?.tests ?? []).map((item, index) => ({
    label: item.label,
    value: String(index),
  })),
  {
    label: preset.value === 'custom' && parameters.value.definition?.name
      ? `自定义 · ${parameters.value.definition.name}`
      : '自定义题目',
    value: 'custom',
  },
])
let applyDefaultsOnSave = false
function configure() {
  applyDefaultsOnSave = true
  emit('configure')
}
function editDefinition(blank = false) {
  definitionDraft.value = blank
    ? {
        name: '',
        prompt: '',
        outputFormat: 'text',
        grader: { type: 'none' },
      }
    : structuredClone(toRaw(parameters.value.definition!))
  panel.value = 'definition'
}
function saveDefinition() {
  if (!definitionDraft.value?.name.trim() || !definitionDraft.value.prompt.trim()) {
    ZNotification.warning({ message: '请填写测评名称和提示词' })
    return
  }
  parameters.value.definition = structuredClone(toRaw(definitionDraft.value))
  panel.value = 'main'
}
watch(() => props.settingsSavedRevision, () => {
  if (!applyDefaultsOnSave || !open.value || !props.settings)
    return
  applyDefaultsOnSave = false
  const { judge, ...defaults } = props.settings.defaults
  if (panel.value === 'definition' && definitionDraft.value) {
    const grading = definitionDraft.value.grader
    if (judge && (grading.type === 'model' || grading.type === 'typesafe'))
      definitionDraft.value.grader = judgeGrader(judge, grading.config.rubric, grading.config.referenceAnswer)
  }
  else {
    Object.assign(parameters.value, defaults, { clientKeyId: defaults.clientKeyId ?? '', model: defaults.model ?? parameters.value.model })
  }
})
watch(open, (value) => {
  panel.value = 'main'
  definitionDraft.value = null
  applyDefaultsOnSave = false
  if (value)
    void props.catalog.load()
})
</script>

<template>
  <ZDialog v-model="open" :title="title" :width="panel === 'definition' ? '48rem' : '38rem'" :show-close="!saving" :close-on-click-modal="!saving" :close-on-press-escape="!saving">
    <ZForm v-if="panel === 'main'" class="grid gap-5">
      <ZFormItem v-if="isPlan" label="账号范围" required>
        <ZSelect v-model="selectionMode" :disabled="saving" :options="[{ label: '固定账号', value: 'fixed' }, { label: '自动跟随全部账号', value: 'all' }, { label: '自动跟随指定分组', value: 'groups' }]" aria-label="账号范围" />
        <p v-if="automatic" class="mt-2 mb-0 text-cp-sm text-cp-text-secondary">
          每轮自动选择已启用且获 Key 授权的账号，新增账号下轮加入，独立判题账号不参与答题
        </p>
      </ZFormItem>
      <ZFormItem v-if="isPlan && accountSelection.mode === 'groups'" label="跟随分组" required>
        <ZSelect v-model="accountSelection.groupIds" :options="groups" multiple filterable :loading="loadingGroups" :disabled="saving" placeholder="选择账号分组" />
      </ZFormItem>
      <ZFormItem v-if="!automatic" label="被测账号" required>
        <ZSelect v-model="parameters.accountIds" :options="accountOptions" :disabled="saving" multiple remote filterable :loading="loadingAccounts || checkingAccess" collapse-tags :max-collapse-tags="3" placeholder="选择要检测的账号" @query-change="catalog.searchAccounts">
          <template #option="{ option }">
            <span class="min-w-0 flex-1 truncate" :title="option.label">{{ option.label }}</span>
            <span class="max-w-1/2 shrink-0 truncate text-cp-xs font-normal" :class="option.disabled ? 'text-cp-text-disabled' : 'text-cp-text-secondary'" :title="option.description">{{ option.description }}</span>
          </template>
        </ZSelect>
        <p v-if="accessMessage" class="mt-2 mb-0 text-cp-sm text-cp-warning-text wrap-anywhere" role="status">
          {{ accessMessage }}
        </p>
        <ZButton v-if="accessError" size="small" variant="text" @click="access.refresh">
          重试授权检查
        </ZButton>
      </ZFormItem>
      <ZFormItem label="测评内容" required>
        <div class="flex items-center gap-2">
          <ZSelect v-model="preset" class="min-w-0 flex-1" :disabled="saving" :options="presetOptions" aria-label="选择测评预设" />
          <ZButton :disabled="saving || !parameters.definition" @click="editDefinition()">
            编辑题目
          </ZButton>
        </div>
        <p v-if="grader" class="mt-2 mb-0 text-cp-sm text-cp-text-secondary">
          {{ graderLabel(grader) }}
        </p>
      </ZFormItem>
      <ZFormItem label="被测模型" required>
        <ZInput v-model="parameters.model" :disabled="saving" placeholder="填写网关模型 ID" />
      </ZFormItem>
      <ZFormItem v-if="isPlan" label="执行间隔" required :description="editing?.settings.cronExpression ? '填写分钟数，保存后替换原来的执行时间' : undefined">
        <ZInput v-model="intervalMinutes" type="number" inputmode="numeric" :disabled="saving" :min="1" :max="config?.limits.planIntervalMinutes" :step="1" placeholder="例如 30" aria-label="执行间隔">
          <template #suffix>
            分钟
          </template>
        </ZInput>
      </ZFormItem>
      <div class="flex flex-wrap items-center justify-between gap-2 text-cp-sm">
        <span :class="parameters.clientKeyId ? 'text-cp-text-secondary' : 'text-cp-warning-text'">
          {{ parameters.clientKeyId ? `计费 Key：${catalog.keyOptions(parameters.clientKeyId).find(item => item.value === parameters.clientKeyId)?.label ?? parameters.clientKeyId}` : '尚未配置计费 Key' }}
        </span>
        <div class="flex gap-2">
          <ZButton size="small" :disabled="saving" @click="panel = 'execution'">
            运行设置
          </ZButton>
          <ZButton v-if="!parameters.clientKeyId" size="small" :disabled="saving" @click="configure">
            配置默认值
          </ZButton>
        </div>
      </div>
      <p class="m-0 text-cp-sm text-cp-text-secondary">
        {{ automatic ? `按每轮可用账号数调用，每账号 ${parameters.repetitions} 次，无可用账号时等待` : `${isPlan ? '每轮' : '本次'}调用 ${parameters.accountIds.length * parameters.repetitions} 次被测模型` }}{{ grader?.type === 'model' ? '，并增加独立模型判题调用' : grader?.type === 'typesafe' ? '，另由 TypeSafe 判题并计费' : '' }}{{ isPlan && !enabled ? ' · 计划保持暂停' : '' }}
      </p>
    </ZForm>
    <EvaluationDefinitionFields v-else-if="panel === 'definition' && definitionDraft" v-model="definitionDraft" :disabled="saving" :default-judge="settings?.defaults.judge ?? null" @configure-judge="configure" />
    <ZForm v-else-if="panel === 'execution'" class="grid gap-5 sm:grid-cols-2">
      <ZFormItem class="sm:col-span-2" label="计费 Key" required>
        <ZSelect :model-value="parameters.clientKeyId || undefined" :options="keyOptions" :disabled="saving" remote filterable :loading="loadingKeys" placeholder="搜索 Key 名称" @update:model-value="parameters.clientKeyId = $event ?? ''" @query-change="catalog.searchKeys" />
      </ZFormItem>
      <ZFormItem label="推理强度">
        <ZSelect v-model="effort" :disabled="saving" :options="effortOptions" />
      </ZFormItem>
      <ZFormItem label="每账号次数">
        <ZInputNumber v-model="parameters.repetitions" aria-label="每账号次数" :disabled="saving" :min="1" :max="5" :precision="0" />
      </ZFormItem>
      <ZFormItem label="单次超时">
        <ZInputNumber v-model="parameters.timeoutSeconds" aria-label="单次超时" :disabled="saving" :min="1" :max="600" :precision="0" unit="秒" />
      </ZFormItem>
      <template v-if="isPlan">
        <ZFormItem label="计划名称" class="sm:col-span-2">
          <ZInput v-model="name" :disabled="saving" maxlength="128" placeholder="留空按题目和间隔自动生成" />
        </ZFormItem>
        <ZCheckbox v-model="enabled" class="sm:col-span-2" :disabled="saving" label="启用定时测评" />
      </template>
    </ZForm>
    <template #footer>
      <template v-if="panel === 'definition'">
        <ZButton @click="panel = 'main'">
          取消编辑
        </ZButton>
        <ZButton type="primary" @click="saveDefinition">
          使用此题目
        </ZButton>
      </template>
      <ZButton v-else-if="panel === 'execution'" type="primary" @click="panel = 'main'">
        完成
      </ZButton>
      <template v-else>
        <ZButton :disabled="saving" @click="open = false">
          取消
        </ZButton>
        <ZButton type="primary" :loading="saving" :disabled="!config || !parameters.clientKeyId || (!automatic && (!isPlan || enabled) && (!accessValid || checkingAccess))" @click="editor.save">
          {{ isPlan ? '保存计划' : '开始测评' }}
        </ZButton>
      </template>
    </template>
  </ZDialog>
</template>
