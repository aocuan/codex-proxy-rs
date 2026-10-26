<script setup lang="ts">
import type { useEvaluationCatalog } from '../composables/useEvaluationCatalog'
import type { useEvaluationSettings } from '../composables/useEvaluationSettings'
import { ZButton, ZCheckbox, ZDialog, ZForm, ZFormItem, ZInput, ZInputNumber, ZSelect } from '@codex-proxy/ui'
import { computed, watch } from 'vue'
import { effortOptions } from '@/components/evaluation/defaults'
import EvaluationJudgeConnectionFields from '@/components/evaluation/EvaluationJudgeConnectionFields.vue'
import { useEvaluationAccess } from '../composables/useEvaluationAccess'

const props = defineProps<{ settings: ReturnType<typeof useEvaluationSettings>, catalog: ReturnType<typeof useEvaluationCatalog> }>()
const { open, draft, current, apiKey, clearKey, saving } = props.settings
const { loadingAccounts, loadingKeys } = props.catalog
const effort = computed({
  get: () => draft.value?.reasoningEffort ?? 'default',
  set: (value: string | undefined) => {
    if (draft.value) {
      draft.value.reasoningEffort = value && value !== 'default'
        ? value
        : null
    }
  },
})
function judgeIds() {
  return draft.value?.judge?.type === 'model' && draft.value.judge.accountId
    ? [draft.value.judge.accountId]
    : []
}
const judgeAccess = useEvaluationAccess(() => open.value && draft.value?.judge?.type === 'model', () => draft.value?.judge?.type === 'model'
  ? draft.value.judge.clientKeyId
  : '', () => props.catalog.accountOptions(judgeIds()), judgeIds)
const { accountOptions: judgeAccountOptions, message: judgeAccessMessage, valid: judgeAccessValid, error: judgeAccessError } = judgeAccess
const judgeKeyOptions = computed(() => props.catalog.keyOptions(draft.value?.judge?.type === 'model'
  ? draft.value.judge.clientKeyId
  : ''))
watch(() => draft.value?.judge?.type, (type) => {
  if (type === 'typesafe')
    clearKey.value = false
  else
    apiKey.value = ''
})
watch(open, (value) => {
  if (value)
    void props.catalog.load()
  else
    apiKey.value = ''
})
</script>

<template>
  <ZDialog v-model="open" title="测评设置" width="44rem" :show-close="!saving" :close-on-click-modal="!saving" :close-on-press-escape="!saving">
    <p class="mt-0 mb-5 text-cp-sm text-cp-text-secondary">
      保存后自动用于新建测评，已有计划保持各自的配置
    </p>
    <ZForm v-if="draft" class="grid gap-5 sm:grid-cols-2">
      <ZFormItem label="默认计费 Key" class="sm:col-span-2">
        <ZSelect :model-value="draft.clientKeyId ?? undefined" :disabled="saving" :options="catalog.keyOptions(draft.clientKeyId ?? '')" clearable remote filterable :loading="loadingKeys" placeholder="选择测评使用的 Key" @update:model-value="draft.clientKeyId = $event ?? null" @query-change="catalog.searchKeys" />
      </ZFormItem>
      <ZFormItem label="默认被测模型">
        <ZInput :model-value="draft.model ?? ''" :disabled="saving" placeholder="网关公开模型 ID" @update:model-value="draft.model = $event || null" />
      </ZFormItem>
      <ZFormItem label="推理强度">
        <ZSelect v-model="effort" :disabled="saving" :options="effortOptions" />
      </ZFormItem>
      <ZFormItem label="每账号次数">
        <ZInputNumber v-model="draft.repetitions" :disabled="saving" :min="1" :max="5" :precision="0" aria-label="默认每账号次数" />
      </ZFormItem>
      <ZFormItem label="单次超时">
        <ZInputNumber v-model="draft.timeoutSeconds" :disabled="saving" :min="1" :max="600" :precision="0" unit="秒" aria-label="默认单次超时" />
      </ZFormItem>
      <EvaluationJudgeConnectionFields v-model="draft.judge" :disabled="saving" :account-options="judgeAccountOptions" :key-options="judgeKeyOptions" :loading-accounts="loadingAccounts" :loading-keys="loadingKeys" @search-accounts="catalog.searchAccounts" @search-keys="catalog.searchKeys" />
      <p v-if="draft.judge?.type === 'model' && judgeAccessMessage" class="m-0 text-cp-sm text-cp-warning-text wrap-anywhere sm:col-span-2" role="status">
        {{ judgeAccessMessage }}
      </p>
      <ZButton v-if="draft.judge?.type === 'model' && judgeAccessError" size="small" variant="text" @click="judgeAccess.refresh">
        重试授权检查
      </ZButton>
      <template v-if="draft.judge?.type === 'typesafe'">
        <ZFormItem class="sm:col-span-2" label="TypeSafe API Key" :required="!current?.typesafeConfigured" :description="current?.typesafeConfigured ? '已配置，留空保留原密钥' : '仅保存在服务端，费用由 TypeSafe 计收'">
          <ZInput v-model="apiKey" type="password" :disabled="saving" autocomplete="new-password" placeholder="填写 TypeSafe API Key" aria-label="TypeSafe API Key" />
        </ZFormItem>
        <p class="m-0 text-cp-sm text-cp-text-secondary sm:col-span-2">
          Jev 返回概率，不提供文字推理，HTML 仅评审源码，糖果题默认用精确答案判分
        </p>
      </template>
      <ZCheckbox v-else-if="current?.typesafeConfigured" v-model="clearKey" :disabled="saving" class="sm:col-span-2" label="清除已保存的 TypeSafe 密钥" />
    </ZForm>
    <template #footer>
      <ZButton :disabled="saving" @click="open = false">
        取消
      </ZButton>
      <ZButton type="primary" :loading="saving" :disabled="draft?.judge?.type === 'model' && !judgeAccessValid" @click="settings.save">
        保存设置
      </ZButton>
    </template>
  </ZDialog>
</template>
