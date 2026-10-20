<script setup lang="ts">
import type { SelectOption } from '@codex-proxy/ui'
import type { EvaluationJudgeDefaults } from '@/api'
import { ZFormItem, ZInput, ZInputNumber, ZSelect } from '@codex-proxy/ui'
import { computed } from 'vue'
import { effortOptions } from './defaults'

defineProps<{
  accountOptions: SelectOption[]
  keyOptions: SelectOption[]
  disabled?: boolean
  loadingAccounts: boolean
  loadingKeys: boolean
}>()
const emit = defineEmits<{ searchAccounts: [query: string], searchKeys: [query: string] }>()
const judge = defineModel<EvaluationJudgeDefaults | null>({ required: true })
const kind = computed({
  get: () => judge.value?.type ?? 'none',
  set: (type: string | undefined) => {
    judge.value = type === 'typesafe'
      ? { type: 'typesafe', model: 'jev-1.13.0', decisionThreshold: 80, timeoutSeconds: 60 }
      : type === 'model'
        ? { type: 'model', model: '', accountId: '', clientKeyId: '', reasoningEffort: null, timeoutSeconds: 180 }
        : null
  },
})
const effort = computed({
  get: () => judge.value?.type === 'model' ? judge.value.reasoningEffort ?? 'default' : 'default',
  set: (value: string | undefined) => {
    if (judge.value?.type === 'model')
      judge.value.reasoningEffort = value && value !== 'default' ? value : null
  },
})
</script>

<template>
  <ZFormItem class="sm:col-span-2" label="默认判题器" description="仅用于选择 AI 判题的题目">
    <ZSelect v-model="kind" :disabled="disabled" :options="[{ label: '暂不配置', value: 'none' }, { label: '网关模型', value: 'model' }, { label: 'TypeSafe Jev', value: 'typesafe' }]" />
  </ZFormItem>
  <template v-if="judge">
    <ZFormItem label="判题模型" required>
      <ZInput v-model="judge.model" :disabled="disabled" :placeholder="judge.type === 'typesafe' ? '例如 jev-1.13.0' : '网关公开模型 ID'" />
    </ZFormItem>
    <ZFormItem label="判题超时">
      <ZInputNumber v-model="judge.timeoutSeconds" :disabled="disabled" :min="1" :max="judge.type === 'typesafe' ? 120 : 600" :precision="0" unit="秒" aria-label="判题超时" />
    </ZFormItem>
    <template v-if="judge.type === 'model'">
      <ZFormItem label="判题 Key" required>
        <ZSelect :model-value="judge.clientKeyId || undefined" :disabled="disabled" :options="keyOptions" remote filterable :loading="loadingKeys" placeholder="搜索 Key 名称" @update:model-value="judge.clientKeyId = $event ?? ''" @query-change="emit('searchKeys', $event)" />
      </ZFormItem>
      <ZFormItem label="判题账号" required description="使用独立账号，不能与被测账号相同">
        <ZSelect :model-value="judge.accountId || undefined" :disabled="disabled" :options="accountOptions" remote filterable :loading="loadingAccounts" placeholder="搜索判题账号" @update:model-value="judge.accountId = $event ?? ''" @query-change="emit('searchAccounts', $event)" />
      </ZFormItem>
      <ZFormItem label="判题推理强度">
        <ZSelect v-model="effort" :disabled="disabled" :options="effortOptions" />
      </ZFormItem>
    </template>
    <ZFormItem v-else label="判定阈值" :description="`≥ ${judge.decisionThreshold}% 通过、≤ ${100 - judge.decisionThreshold}% 未通过，中间保留未判定`">
      <ZInputNumber v-model="judge.decisionThreshold" :disabled="disabled" :min="51" :max="100" :precision="0" unit="%" aria-label="判定阈值" />
    </ZFormItem>
  </template>
</template>
