<script setup lang="ts">
import type { EvaluationItem, EvaluationOutput, EvaluationRun } from '@/api'
import { ZButton, ZDialog, ZEmpty, ZSegmented } from '@codex-proxy/ui'
import { useDocumentVisibility, useIntervalFn } from '@vueuse/core'
import { computed, shallowRef, watch } from 'vue'
import { getEvaluationDetail, getEvaluationOutput } from '@/api'
import { useRequestState } from '@/composables/useRequestState'
import EvaluationErrorDetails from './EvaluationErrorDetails.vue'
import EvaluationJudgmentSummary from './EvaluationJudgmentSummary.vue'
import EvaluationPreview from './EvaluationPreview.vue'
import { durationText, turnStateLabels, verdictLabels } from './presenter'

const props = defineProps<{ item: EvaluationItem | null, run: EvaluationRun | null, accountName: (id: string) => string }>()
const open = defineModel<boolean>({ required: true })
const item = shallowRef(props.item)
const visibility = useDocumentVisibility()
const statusRequest = useRequestState()
const { error: statusError } = statusRequest
const active = computed(() => ['queued', 'running'].includes(item.value?.status ?? '')
  || ['queued', 'running'].includes(item.value?.judgment?.status ?? ''))
const output = shallowRef<EvaluationOutput | null>(null)
const tab = shallowRef('text')
const request = useRequestState()
const { loading, error } = request
const observation = computed(() => item.value?.observation)
const judgeConfig = computed(() => {
  const grader = props.run?.profile.definition?.grader
  return grader?.type === 'model' || grader?.type === 'typesafe'
    ? grader.config
    : null
})
const tabs = computed(() => [
  { label: '原始回答', value: 'text' },
  ...(output.value?.artifactHtml
    ? [
        { label: '作品预览', value: 'preview' },
      ]
    : []),
])
async function load() {
  if (!item.value || !open.value)
    return
  const id = request.start()
  output.value = null
  if (!item.value.hasOutput) {
    request.finish(id)
    return
  }
  try {
    const result = await getEvaluationOutput(item.value.id, { signal: request.signal })
    if (request.isCurrent(id))
      output.value = result
  }
  catch (error) {
    request.fail(id, error)
  }
  finally {
    request.finish(id)
  }
}
async function refreshStatus() {
  if (!props.run || !open.value || visibility.value !== 'visible' || statusRequest.loading.value)
    return
  const id = statusRequest.start()
  try {
    const detail = await getEvaluationDetail(props.run.id, { signal: statusRequest.signal, silent: true })
    if (statusRequest.isCurrent(id))
      item.value = detail.items.find(candidate => candidate.id === props.item?.id) ?? item.value
  }
  catch (error) {
    statusRequest.fail(id, error)
  }
  finally {
    statusRequest.finish(id)
  }
}
watch([
  open,
  () => props.item,
], () => {
  statusRequest.invalidate()
  item.value = props.item
})
watch([
  open,
  () => item.value?.id,
  () => item.value?.hasOutput,
], () => {
  tab.value = 'text'
  if (open.value) {
    void load()
  }
  else {
    request.invalidate()
    output.value = null
  }
})
useIntervalFn(() => {
  if (active.value)
    void refreshStatus()
}, 5000)
</script>

<template>
  <ZDialog v-model="open" :title="item?.status === 'failed' ? '测评错误' : run?.parameters.kind === 'turnstate' ? '测票详情' : '测评回答'" width="64rem">
    <template v-if="item">
      <dl class="mb-5 grid grid-cols-2 gap-x-5 gap-y-3 text-cp-sm sm:grid-cols-3">
        <div>
          <dt class="text-cp-text-tertiary">
            账号
          </dt><dd class="m-0 break-all" :title="item.accountId">
            {{ accountName(item.accountId) }}
          </dd>
        </div>
        <div>
          <dt class="text-cp-text-tertiary">
            {{ observation?.turnState ? '测票结果' : '题目结果' }}
          </dt><dd class="m-0">
            {{ observation?.turnState ? turnStateLabels[observation.turnState.outcome] : verdictLabels[item.verdict] }}{{ item.finalAnswer === null ? '' : ` · ${item.finalAnswer}` }}
          </dd>
        </div>
        <div>
          <dt class="text-cp-text-tertiary">
            耗时 / 首字
          </dt><dd class="m-0">
            {{ durationText(item.durationMs) }} / {{ durationText(item.ttftMs) }}
          </dd>
        </div>
        <div>
          <dt class="text-cp-text-tertiary">
            请求模型
          </dt><dd class="m-0">
            {{ run?.parameters.model }}
          </dd>
        </div>
        <div>
          <dt class="text-cp-text-tertiary">
            上游 / 返回模型
          </dt><dd class="m-0">
            {{ observation?.upstreamModel ?? '未观测' }} / {{ observation?.responseModel ?? '未观测' }}
          </dd>
        </div>
        <div>
          <dt class="text-cp-text-tertiary">
            推理强度
          </dt><dd class="m-0">
            {{ observation?.reasoningEffort ?? '未观测' }}
          </dd>
        </div>
        <div>
          <dt class="text-cp-text-tertiary">
            输入 / 输出 Token
          </dt><dd class="m-0">
            {{ observation?.inputTokens ?? '未观测' }} / {{ observation?.outputTokens ?? '未观测' }}
          </dd>
        </div>
        <div>
          <dt class="text-cp-text-tertiary">
            费用
          </dt><dd class="m-0">
            {{ observation?.costAmount ?? '未观测' }} {{ observation?.costCurrency ?? '' }}
          </dd>
        </div>
        <div>
          <dt class="text-cp-text-tertiary">
            请求 ID
          </dt><dd class="m-0 break-all font-mono text-cp-xs">
            {{ item.requestId ?? (observation?.turnState ? '账号诊断，不进入模型账本' : '未开始') }}
          </dd>
        </div>
      </dl>
      <EvaluationErrorDetails
        v-if="item.status === 'failed'"
        :active="open" :request-id="item.requestId" :completed-at="item.completedAt"
        :message="item.message" :error-code="item.errorCode"
      />
      <EvaluationJudgmentSummary v-if="item.judgment" :judgment="item.judgment" :config="judgeConfig" :account-name="accountName" />
      <EvaluationErrorDetails
        v-if="item.judgment?.status === 'failed' && item.judgment.requestId"
        :active="open" :request-id="item.judgment.requestId" :completed-at="item.judgment.completedAt"
        :message="item.judgment.reason" title="判题请求错误"
      />
      <p v-if="!item.judgment && item.status !== 'failed' && item.message" class="mb-4 text-cp-sm text-cp-warning-text">
        {{ item.message }}
      </p>
      <ZButton v-if="statusError" variant="text" size="small" @click="refreshStatus">
        状态刷新失败，重试
      </ZButton>
    </template>
    <div v-if="observation?.turnState" class="mb-4 grid gap-2 text-cp-sm" aria-label="测票请求记录">
      <p class="m-0 text-cp-text-secondary">
        两次短请求只检测路由票据是否稳定，票据变化需要能力题复核
      </p>
      <div v-for="(step, index) in observation.turnState.steps" :key="index" class="rounded-cp bg-cp-fill-quaternary p-3">
        第 {{ index + 1 }} 次 · HTTP {{ step.status }} · {{ durationText(step.durationMs) }} · 票据 {{ step.ticketLength }} 字符
        <span v-if="step.ticketFingerprint" class="ml-2 font-mono text-cp-xs">{{ step.ticketFingerprint.slice(0, 8) }}</span>
        <span v-if="step.routeChanged" class="ml-2 text-cp-warning-text">路由已变化</span>
      </div>
    </div>
    <div v-if="loading" class="py-10 text-center text-cp-text-secondary" role="status">
      正在读取回答
    </div>
    <ZEmpty v-else-if="error" title="回答读取失败" :description="error">
      <template #action>
        <ZButton @click="load">
          重试
        </ZButton>
      </template>
    </ZEmpty>
    <template v-else-if="output && (output.responseText || output.artifactHtml || item?.status !== 'failed')">
      <ZSegmented v-model="tab" :options="tabs" aria-label="回答显示方式" class="mb-4" />
      <EvaluationPreview v-if="open && tab === 'preview' && output.artifactHtml" :html="output.artifactHtml" />
      <pre v-else class="m-0 max-h-[50dvh] overflow-auto rounded-cp bg-cp-fill-quaternary p-4 font-mono text-cp-sm whitespace-pre-wrap wrap-anywhere">{{ output.responseText || '没有文本输出' }}</pre>
    </template>
    <ZEmpty v-else-if="item?.status !== 'failed'" title="没有保存的回答" description="可查看上方执行状态和原因" />
  </ZDialog>
</template>
