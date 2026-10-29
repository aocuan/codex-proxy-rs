<script setup lang="ts">
import type { EvaluationHistorySource } from '@/api'
import { ZButton, ZFormItem, ZPopover, ZSelect } from '@codex-proxy/ui'
import { Activity } from '@lucide/vue'
import { computed } from 'vue'

const props = defineProps<{ sources: EvaluationHistorySource[], loading: boolean, error: string }>()
const emit = defineEmits<{ retry: [] }>()
const selection = defineModel<string>({ required: true })
const selected = computed(() => props.sources.find(source => source.id === selection.value))
const options = computed(() => {
  const labels = props.sources.map(source => `${source.name} · ${source.model} · ${source.reasoningEffort ?? '跟随账号配置'}`)
  const values = props.sources.map((source, index) => ({
    value: source.id,
    label: labels.filter(label => label === labels[index]).length > 1
      ? `${labels[index]} · 配置 ${source.id.slice(0, 6)}`
      : labels[index]!,
  }))
  if (!selected.value) {
    values.unshift({
      value: selection.value,
      label: selection.value ? '已选来源暂无记录' : '快速测票 · 暂无记录',
    })
  }
  return values
})
</script>

<template>
  <ZPopover placement="bottom-end">
    <template #reference="{ open }">
      <ZButton :aria-expanded="open" aria-label="检测状态设置">
        <Activity class="size-4" />
        检测状态
      </ZButton>
    </template>
    <div class="w-80 max-w-[calc(100vw-2rem)] p-4">
      <ZFormItem label="状态来源">
        <ZSelect v-model="selection" class="w-full" :options="options" :loading="loading" aria-label="账号检测状态来源" />
      </ZFormItem>
      <div v-if="selected" class="mt-2 text-cp-xs text-cp-text-secondary">
        {{ selected.kind === 'turnstate' ? '票据稳定性' : '自动判分' }} · {{ selected.model }} · {{ selected.reasoningEffort ?? '跟随账号配置' }}
      </div>
      <div class="mt-2 text-cp-xs text-cp-text-tertiary">
        只列出已有记录且可自动判定的评测，选择保存在当前浏览器
      </div>
      <ZButton v-if="error" variant="text" size="small" class="mt-2" @click="emit('retry')">
        读取失败，重试
      </ZButton>
    </div>
  </ZPopover>
</template>
