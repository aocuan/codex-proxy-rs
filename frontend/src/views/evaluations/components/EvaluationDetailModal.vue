<script setup lang="ts">
import type { EvaluationDetail, EvaluationItem, EvaluationRun } from '@/api'
import { ZButton, ZDialog, ZEmpty, ZTable, ZTag } from '@codex-proxy/ui'
import { turnStateLabels } from '@/components/evaluation/presenter'
import { durationText, itemColumns, kindLabels, runStatus, statusLabels, verdictLabels } from '../presenter'

defineProps<{ detail: EvaluationDetail | null, loading: boolean, error: string, stopping: boolean, retryingItemId: string | null, accountName: (id: string) => string }>()
const emit = defineEmits<{
  retryJudgment: [item: EvaluationItem]
  refresh: []
  stop: [run: EvaluationRun]
  output: [item: EvaluationItem, run: EvaluationRun]
  compare: [item: EvaluationItem, run: EvaluationRun]
}>()
const open = defineModel<boolean>({ required: true })
</script>

<template>
  <ZDialog v-model="open" title="测评详情" width="74rem">
    <div v-if="loading" class="py-10 text-center text-cp-text-secondary" role="status">
      正在读取测评
    </div>
    <ZEmpty v-else-if="!detail" :title="error ? '读取失败' : '暂无测评数据'" :description="error">
      <template #action>
        <ZButton @click="emit('refresh')">
          重试
        </ZButton>
      </template>
    </ZEmpty>
    <template v-else>
      <div class="mb-5 flex flex-wrap items-center gap-3">
        <span class="font-emphasis">{{ detail.run.profile.definition?.name ?? kindLabels[detail.run.parameters.kind] }} · {{ detail.run.parameters.model }}</span>
        <ZTag :type="detail.run.activeItems ? 'info' : 'default'">
          {{ runStatus(detail.run) }}
        </ZTag>
        <span class="text-cp-sm text-cp-text-secondary">{{ detail.run.createdAtDisplay }}</span>
        <div class="ml-auto flex gap-2">
          <ZButton size="small" @click="emit('refresh')">
            刷新
          </ZButton>
          <ZButton v-if="detail.run.activeItems" size="small" type="danger" :loading="stopping" :disabled="detail.run.stopRequested" @click="emit('stop', detail.run)">
            停止测评
          </ZButton>
        </div>
      </div>
      <p v-if="detail.run.retainedItems < detail.run.totalItems" class="text-cp-sm text-cp-warning-text">
        部分历史已按保留策略清理，当前显示 {{ detail.run.retainedItems }} / {{ detail.run.totalItems }} 条
      </p>
      <ZTable :columns="itemColumns" :data="detail.items" row-key="id" empty-text="任务记录已清理">
        <template #account="{ row }">
          <div class="truncate" :title="row.accountId">
            {{ accountName(row.accountId) }}
          </div>
          <span class="text-cp-xs text-cp-text-tertiary">第 {{ row.repetition }} 次</span>
        </template>
        <template #status="{ row }">
          <ZTag :type="row.status === 'failed' ? 'danger' : row.status === 'running' ? 'info' : 'default'">
            {{ statusLabels[row.status] }}
          </ZTag>
          <span v-if="row.message" class="mt-1 block truncate text-cp-xs text-cp-text-tertiary" :title="row.message">{{ row.message }}</span>
        </template>
        <template #verdict="{ row }">
          <ZTag :type="row.verdict === 'correct' ? 'success' : row.verdict === 'wrong' ? 'danger' : 'default'">
            {{ row.observation.turnState ? turnStateLabels[row.observation.turnState.outcome] : row.judgment?.status === 'queued' ? '等待判题' : row.judgment?.status === 'running' ? '正在判题' : verdictLabels[row.verdict] }}
          </ZTag>
          <span v-if="row.judgment" class="mt-1 block text-cp-xs text-cp-text-tertiary">AI 评审</span>
        </template>
        <template #duration="{ row }">
          {{ durationText(row.durationMs) }}
          <span class="mt-1 block text-cp-xs text-cp-text-tertiary">首字 {{ durationText(row.ttftMs) }}</span>
        </template>
        <template #actions="{ row }">
          <div class="flex gap-2">
            <ZButton size="small" :disabled="!row.hasOutput && row.status !== 'failed'" @click="emit('output', row, detail.run)">
              {{ row.status === 'failed' ? '查看错误' : detail.run.parameters.kind === 'turnstate' ? '测票详情' : '查看回答' }}
            </ZButton>
            <ZButton
              v-if="row.judgment && ['failed', 'interrupted', 'cancelled'].includes(row.judgment.status) && row.judgment.attempt < 5 && !detail.run.stopRequested"
              size="small" :loading="retryingItemId === row.id" :disabled="retryingItemId !== null" @click="emit('retryJudgment', row)"
            >
              重试判题
            </ZButton>
            <ZButton v-if="row.hasArtifact" size="small" @click="emit('compare', row, detail.run)">
              加入对比
            </ZButton>
          </div>
        </template>
      </ZTable>
      <section class="mt-5 text-cp-sm" aria-label="测评题目">
        <div class="flex flex-wrap items-center gap-x-4 gap-y-1">
          <h3 class="m-0 text-cp-sm font-emphasis">
            题目
          </h3>
          <span class="text-cp-xs text-cp-text-tertiary">
            推理强度 {{ detail.run.parameters.reasoningEffort ?? '跟随账号配置' }} · 超时 {{ detail.run.parameters.timeoutSeconds }} 秒
          </span>
        </div>
        <pre class="mt-3 mb-0 max-h-64 overflow-auto rounded-cp bg-cp-fill-quaternary p-3 font-sans whitespace-pre-wrap wrap-anywhere">{{ detail.run.profile.prompt }}</pre>
      </section>
    </template>
  </ZDialog>
</template>
