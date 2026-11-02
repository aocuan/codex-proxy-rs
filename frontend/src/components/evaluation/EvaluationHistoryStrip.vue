<script setup lang="ts">
import type { EvaluationAccountMonitoring, EvaluationHistoryItem } from '@/api'
import { ZButton, ZPopover } from '@codex-proxy/ui'
import { computed, shallowRef, watch } from 'vue'
import { durationText, historyPending, historyResult } from './presenter'

const props = defineProps<{ items: EvaluationHistoryItem[], loading: boolean, error: string, monitoring?: EvaluationAccountMonitoring }>()
const emit = defineEmits<{ inspect: [item: EvaluationHistoryItem], retry: [] }>()
const latest = computed(() => props.items[0])
const condition = computed(() => latest.value
  ? `${latest.value.model} · ${latest.value.reasoningEffort ?? '跟随账号配置'}`
  : '')
const current = computed(() => props.items.find(item => !historyPending(item)) ?? latest.value)
const pending = computed(() => latest.value && historyPending(latest.value)
  ? latest.value
  : undefined)
const monitorLabel = computed(() => {
  const state = props.monitoring
  if (!state)
    return ''
  if (!state.monitored)
    return '不在监测范围'
  if (state.freshness === 'expired')
    return '结果已过期 · 待复测'
  if (state.freshness === 'environment_changed')
    return '环境变化 · 待复测'
  if (state.signal === 'accepted')
    return '票据稳定'
  if (state.signal === 'refreshed')
    return '票据变化'
  return state.latestAttempt
    ? historyResult(state.latestAttempt).label
    : '待首次检测'
})
const monitorAttempt = computed(() => {
  const state = props.monitoring
  const attempt = state?.latestAttempt
  if (!attempt || !state.signal)
    return ''
  if (attempt.status !== 'succeeded' || attempt.turnState === 'unknown')
    return historyResult(attempt).label
  return ''
})
const monitorClass = computed(() => {
  const state = props.monitoring
  if (state?.freshness !== 'fresh')
    return 'text-z-text-tertiary'
  return state.signal === 'accepted'
    ? 'text-z-success-text'
    : 'text-z-warning-text'
})
const points = computed(() => [...props.items].reverse())
const active = shallowRef<EvaluationHistoryItem>()
const anchor = shallowRef<HTMLElement>()
watch(() => props.items, (items) => {
  active.value = items.find(item => item.id === active.value?.id)
  if (!active.value)
    anchor.value = undefined
})
function activate(item: EvaluationHistoryItem, event: Event) {
  if (event.currentTarget instanceof HTMLElement) {
    active.value = item
    anchor.value = event.currentTarget
  }
}
</script>

<template>
  <div v-if="latest || !error" class="min-w-0" :aria-busy="loading">
    <div class="flex min-h-4 items-center justify-end gap-1 text-z-xs">
      <span v-if="latest" class="mr-auto min-w-0 truncate text-z-text-tertiary" :title="condition">{{ condition }}</span>
      <span v-if="monitoring" class="shrink-0 whitespace-nowrap" :class="monitorClass" :title="monitoring.freshness === 'environment_changed' ? '账号环境或正式方案已变化，需要重新检测' : undefined">{{ monitorAttempt ? '上次' : '' }}{{ monitorLabel }}</span>
      <span v-else-if="current" :class="historyResult(current).text">
        {{ pending && current.id !== pending.id ? '上次 ' : '' }}{{ historyResult(current).label }}
      </span>
      <span v-else class="text-z-text-tertiary">{{ loading ? '读取中' : '暂无记录' }}</span>
      <span v-if="monitorAttempt" class="shrink-0 whitespace-nowrap" :class="monitoring?.latestAttempt?.status === 'failed' ? 'text-z-error-text' : 'text-z-text-secondary'">· {{ monitorAttempt }}</span>
      <span v-else-if="!monitoring && pending && current?.id !== pending.id" class="text-z-primary-text">· {{ historyResult(pending).label }}</span>
    </div>
    <ZPopover class="w-full" trigger="hover" placement="top" role="tooltip" :reference-element="anchor" :disabled="!latest">
      <template #reference>
        <div
          class="flex h-7 w-full items-center gap-0.5" role="group"
          :aria-label="latest ? latest.kind === 'turnstate' ? '最近 28 次测票，左侧较早，右侧最新；绿色票据稳定，黄色票据变化，红色检测失败，灰色未判定' : '最近 28 次检测，左侧较早，右侧最新；绿色通过，红色未通过，灰色未判定' : loading ? '读取历史记录中' : '当前测评条件暂无记录'"
        >
          <span v-for="index in Math.max(0, 28 - points.length)" :key="`empty-${index}`" class="h-6 min-w-0 flex-1 rounded-xs bg-z-fill-secondary ring-1 ring-z-border ring-inset" aria-hidden="true" />
          <ZButton
            v-for="point in points" :key="point.id" variant="text"
            class="group h-7! min-w-0 flex-1 bg-transparent! p-0! [&>span]:w-full"
            :aria-label="`${point.name} · ${point.testedAtDisplay} · ${historyResult(point).label}，查看结果`"
            @mouseenter="activate(point, $event)" @focus="activate(point, $event)" @click="emit('inspect', point)"
          >
            <span class="block h-6 w-full rounded-xs transition-[filter] group-hover:brightness-110 motion-reduce:transition-none" :class="historyResult(point).color" />
          </ZButton>
        </div>
      </template>
      <div v-if="active" class="max-w-72 p-3 text-z-xs">
        <div class="font-emphasis">
          第 {{ points.findIndex(point => point.id === active?.id) + 1 }} 次 · {{ active.name }}{{ historyResult(active).label }}
        </div>
        <div class="mt-1 text-z-text-secondary">
          {{ active.model }} · {{ active.reasoningEffort ?? '跟随账号配置' }}
        </div>
        <div class="mt-1 text-z-text-secondary">
          {{ active.kind === 'turnstate' ? '票据稳定性' : active.graderType === 'typesafe' ? 'TypeSafe 评审' : active.graderType === 'model' ? 'AI 评审' : '规则判分' }} · {{ active.testedAtDisplay }}
        </div>
        <div v-if="active.durationMs !== null" class="mt-1 text-z-text-secondary">
          耗时 {{ durationText(active.durationMs) }}
        </div>
        <div class="mt-1 text-z-text-tertiary">
          {{ active.kind === 'turnstate' ? '点击查看测票详情' : '点击查看回答' }} · 当前 {{ points.length }} 次记录
        </div>
        <div v-if="active.kind === 'turnstate'" class="mt-1 text-z-text-tertiary">
          票据变化需要能力评测复核
        </div>
      </div>
    </ZPopover>
    <div v-if="monitoring?.signalAtDisplay" class="flex justify-end text-z-xs text-z-text-tertiary" :title="monitoring.expiresAtDisplay ? `有效至 ${monitoring.expiresAtDisplay}` : undefined">
      {{ monitoring.signalAtDisplay }}
    </div>
    <div v-else-if="current && !historyPending(current)" class="flex min-w-0 justify-end gap-1 text-z-xs text-z-text-tertiary">
      <span class="truncate" :title="current.testedAtDisplay">{{ current.testedAtDisplay }}</span>
      <span v-if="current.durationMs !== null" class="shrink-0">· {{ durationText(current.durationMs) }}</span>
    </div>
    <ZButton v-if="error" variant="text" size="small" class="p-0!" @click="emit('retry')">
      刷新失败，重试
    </ZButton>
  </div>
  <ZButton v-else-if="error" variant="text" size="small" @click="emit('retry')">
    测评读取失败，重试
  </ZButton>
</template>
