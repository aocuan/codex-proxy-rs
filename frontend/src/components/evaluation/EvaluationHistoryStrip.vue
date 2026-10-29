<script setup lang="ts">
import type { EvaluationHistoryItem } from '@/api'
import { ZButton, ZPopover } from '@codex-proxy/ui'
import { computed, shallowRef, watch } from 'vue'
import { durationText, historyPending, historyResult } from './presenter'

const props = defineProps<{ items: EvaluationHistoryItem[], loading: boolean, error: string }>()
const emit = defineEmits<{ inspect: [item: EvaluationHistoryItem], retry: [] }>()
const latest = computed(() => props.items[0])
const current = computed(() => props.items.find(item => !historyPending(item)) ?? latest.value)
const pending = computed(() => latest.value && historyPending(latest.value)
  ? latest.value
  : undefined)
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
    <div class="flex min-h-4 items-center justify-end gap-1 text-cp-xs">
      <span v-if="current" :class="historyResult(current).text">
        {{ pending && current.id !== pending.id ? '上次 ' : '' }}{{ historyResult(current).label }}
      </span>
      <span v-else class="text-cp-text-tertiary">{{ loading ? '读取中' : '未检测' }}</span>
      <span v-if="pending && current?.id !== pending.id" class="text-cp-primary-text">· {{ historyResult(pending).label }}</span>
    </div>
    <ZPopover class="w-full" trigger="hover" placement="top" role="tooltip" :reference-element="anchor" :disabled="!latest">
      <template #reference>
        <div
          class="flex h-7 w-full items-center gap-0.5" role="group"
          :aria-label="latest ? latest.kind === 'turnstate' ? '最近 28 次测票，左侧较早，右侧最新；绿色票据稳定，黄色票据变化或请求异常，灰色未判定' : '最近 28 次检测，左侧较早，右侧最新；绿色通过，红色未通过，灰色未判定' : loading ? '读取检测中' : '未检测'"
        >
          <span v-for="index in Math.max(0, 28 - points.length)" :key="`empty-${index}`" class="h-6 min-w-0 flex-1 rounded-xs bg-cp-fill-secondary ring-1 ring-cp-border ring-inset" aria-hidden="true" />
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
      <div v-if="active" class="max-w-72 p-3 text-cp-xs">
        <div class="font-emphasis">
          第 {{ points.findIndex(point => point.id === active?.id) + 1 }} 次 · {{ active.name }}{{ historyResult(active).label }}
        </div>
        <div class="mt-1 text-cp-text-secondary">
          {{ active.model }} · {{ active.reasoningEffort ?? '跟随账号配置' }}
        </div>
        <div class="mt-1 text-cp-text-secondary">
          {{ active.kind === 'turnstate' ? '票据稳定性' : active.graderType === 'typesafe' ? 'TypeSafe 评审' : active.graderType === 'model' ? 'AI 评审' : '规则判分' }} · {{ active.testedAtDisplay }}
        </div>
        <div v-if="active.durationMs !== null" class="mt-1 text-cp-text-secondary">
          耗时 {{ durationText(active.durationMs) }}
        </div>
        <div class="mt-1 text-cp-text-tertiary">
          {{ active.kind === 'turnstate' ? '点击查看测票详情' : '点击查看回答' }} · 当前 {{ points.length }} 次记录
        </div>
        <div v-if="active.kind === 'turnstate'" class="mt-1 text-cp-text-tertiary">
          票据变化需要能力评测复核
        </div>
      </div>
    </ZPopover>
    <div v-if="current && !historyPending(current)" class="flex min-w-0 justify-end gap-1 text-cp-xs text-cp-text-tertiary">
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
