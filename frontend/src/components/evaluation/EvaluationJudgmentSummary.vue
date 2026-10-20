<script setup lang="ts">
import type { EvaluationJudgeConfig, EvaluationJudgment, TypeSafeJudgeConfig } from '@/api'
import { ZTag } from '@codex-proxy/ui'
import { computed } from 'vue'
import { verdictLabels } from './presenter'

const props = defineProps<{ judgment: EvaluationJudgment, config: EvaluationJudgeConfig | TypeSafeJudgeConfig | null, accountName: (id: string) => string }>()
const status = computed(() => {
  switch (props.judgment.status) {
    case 'queued': return '等待判题'
    case 'running': return '正在判题'
    case 'succeeded': return verdictLabels[props.judgment.verdict]
    case 'cancelled': return '判题已取消'
    case 'interrupted': return '判题已中断'
    default: return '判题异常'
  }
})
</script>

<template>
  <section class="mb-5 rounded-cp bg-cp-fill-quaternary p-4" aria-label="AI 判题结果">
    <div class="flex flex-wrap items-center gap-2">
      <span class="font-emphasis">AI 评审</span>
      <ZTag :type="judgment.verdict === 'correct' ? 'success' : judgment.verdict === 'wrong' ? 'danger' : 'default'">
        {{ status }}
      </ZTag>
      <span v-if="config?.model" class="text-cp-xs text-cp-text-secondary">{{ config.model }}</span>
      <span v-if="judgment.attempt > 1" class="text-cp-xs text-cp-text-secondary">第 {{ judgment.attempt }} 次判题</span>
    </div>
    <p v-if="judgment.reason" class="mb-0 text-cp-sm whitespace-pre-wrap wrap-anywhere">
      {{ judgment.reason }}
    </p>
    <div v-if="config?.rubric" class="mt-3 text-cp-sm">
      <h3 class="m-0 text-cp-xs font-emphasis text-cp-text-secondary">
        评分标准
      </h3>
      <p class="mt-1 mb-0 max-h-48 overflow-auto whitespace-pre-wrap wrap-anywhere">
        {{ config.rubric }}
      </p>
    </div>
    <dl class="mt-3 mb-0 flex flex-wrap gap-x-5 gap-y-2 text-cp-xs text-cp-text-secondary">
      <div class="flex min-w-0 gap-2">
        <dt class="shrink-0">
          {{ judgment.accountId ? '判题账号' : '判题服务' }}
        </dt><dd class="m-0 wrap-anywhere">
          {{ judgment.accountId ? accountName(judgment.accountId) : 'TypeSafe · 独立计费' }}
        </dd>
      </div>
      <div v-if="judgment.observation.costAmount !== null" class="flex gap-2">
        <dt>费用</dt><dd class="m-0">
          {{ judgment.observation.costAmount }} {{ judgment.observation.costCurrency ?? '' }}
        </dd>
      </div>
      <div v-if="judgment.observation.inputTokens !== null || judgment.observation.outputTokens !== null" class="flex gap-2">
        <dt>输入 / 输出 Token</dt><dd class="m-0">
          {{ judgment.observation.inputTokens ?? '—' }} / {{ judgment.observation.outputTokens ?? '—' }}
        </dd>
      </div>
    </dl>
  </section>
</template>
