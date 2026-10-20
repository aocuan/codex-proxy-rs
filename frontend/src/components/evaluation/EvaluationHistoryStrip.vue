<script setup lang="ts">
import type { EvaluationHistoryItem } from '@/api'
import { ZButton, ZPopover } from '@codex-proxy/ui'
import { computed, shallowRef } from 'vue'
import { historyResult } from './presenter'

const props = defineProps<{ items: EvaluationHistoryItem[], loading: boolean, error: string }>()
const emit = defineEmits<{ inspect: [item: EvaluationHistoryItem], retry: [] }>()
const latest = computed(() => props.items[0])
const points = computed(() => [...props.items].reverse())
const active = shallowRef<EvaluationHistoryItem>()
const anchor = shallowRef<HTMLElement>()
const summary = computed(() => {
  const recent = props.items[0]
  if (!recent)
    return ''
  if (isGraded(recent, 'wrong'))
    return isGraded(props.items[1], 'wrong') ? '最近连续未通过' : '最近未通过'
  if (!isGraded(recent, 'correct'))
    return historyResult(recent).label
  const firstNonPassing = props.items.find(item => !isGraded(item, 'correct'))
  if (isGraded(firstNonPassing, 'wrong'))
    return '异常后恢复'
  return !firstNonPassing && props.items.length > 1 ? '持续通过' : '最近通过'
})
function isGraded(item: EvaluationHistoryItem | undefined, verdict: 'correct' | 'wrong') {
  return item?.status === 'succeeded' && (!item.judgmentStatus || item.judgmentStatus === 'succeeded') && item.verdict === verdict
}
function activate(item: EvaluationHistoryItem, event: Event) {
  if (event.currentTarget instanceof HTMLElement) {
    active.value = item
    anchor.value = event.currentTarget
  }
}
</script>

<template>
  <div v-if="latest || !error" class="min-w-0" :aria-busy="loading">
    <div v-if="latest" class="mb-1 text-right text-cp-xs text-cp-text-tertiary">
      {{ summary }}
    </div>
    <ZPopover class="w-full" trigger="hover" placement="top" role="tooltip" :reference-element="anchor" :disabled="!latest">
      <template #reference>
        <div
          class="flex h-7 w-full items-center gap-0.5" role="group"
          :aria-label="latest ? '最近 28 次检测，左侧较早，右侧最新；绿色通过，红色未通过，灰色未判定' : loading ? '读取测评中' : '暂无可判分测评'"
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
          {{ active.graderType === 'typesafe' ? 'TypeSafe 评审' : active.graderType === 'model' ? 'AI 评审' : '规则判分' }} · {{ active.testedAtDisplay }}
        </div>
        <div class="mt-1 text-cp-text-tertiary">
          点击查看回答 · 当前 {{ points.length }} 次记录
        </div>
      </div>
    </ZPopover>
    <ZButton v-if="error" variant="text" size="small" class="p-0!" @click="emit('retry')">
      刷新失败，重试
    </ZButton>
  </div>
  <ZButton v-else-if="error" variant="text" size="small" @click="emit('retry')">
    测评读取失败，重试
  </ZButton>
</template>
