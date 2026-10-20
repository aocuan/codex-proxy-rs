<script setup lang="ts">
import type { EvaluationComparison } from '../composables/useEvaluationComparison'
import { ZButton, ZDialog, ZEmpty } from '@codex-proxy/ui'
import { computed } from 'vue'
import EvaluationPreview from '@/components/evaluation/EvaluationPreview.vue'
import { durationText } from '../presenter'

const props = defineProps<{ entries: EvaluationComparison[], accountName: (id: string) => string }>()
const emit = defineEmits<{ remove: [id: string] }>()
const open = defineModel<boolean>({ required: true })
const sameConditions = computed(() => {
  const conditions = props.entries.map(({ run }) => JSON.stringify({
    profile: run.profile,
    model: run.parameters.model,
    clientKeyId: run.parameters.clientKeyId,
    effort: run.parameters.reasoningEffort,
    timeout: run.parameters.timeoutSeconds,
  }))
  return new Set(conditions).size <= 1
})
</script>

<template>
  <ZDialog v-model="open" title="作品对比" width="82rem">
    <p v-if="!sameConditions" class="mt-0 mb-4 text-cp-sm text-cp-warning-text">
      两个作品的模型或请求条件不同，请结合参数判断差异
    </p>
    <ZEmpty v-if="!entries.length" title="还没有选择作品" description="在测评详情中将鹈鹕作品加入对比" />
    <div v-else class="grid gap-5 lg:grid-cols-2">
      <section v-for="entry in entries" :key="entry.item.id" class="min-w-0 rounded-cp-card bg-cp-fill-quaternary p-4">
        <div class="mb-3 flex items-start justify-between gap-3">
          <div class="min-w-0">
            <h3 class="m-0 truncate text-cp-base font-emphasis" :title="entry.item.accountId">
              {{ accountName(entry.item.accountId) }}
            </h3>
            <p class="mt-1 mb-0 text-cp-xs text-cp-text-secondary">
              {{ entry.run.parameters.model }} · {{ entry.run.parameters.reasoningEffort ?? '跟随账号配置' }} · {{ entry.run.createdAtDisplay }}
            </p>
          </div>
          <ZButton size="small" @click="emit('remove', entry.item.id)">
            移除
          </ZButton>
        </div>
        <EvaluationPreview v-if="open" :html="entry.html" />
        <p class="mt-3 mb-0 text-cp-xs text-cp-text-secondary">
          耗时 {{ durationText(entry.item.durationMs) }} · 输出 {{ entry.item.observation.outputTokens ?? '未观测' }} Token · {{ entry.run.profile.promptVersion }}
        </p>
      </section>
    </div>
    <p class="mt-4 mb-0 text-cp-xs text-cp-text-tertiary">
      作品质量由人工比较，单次表现和返回模型名称不能证明模型身份
    </p>
  </ZDialog>
</template>
