<script setup lang="ts">
import { ZButton, ZEmpty } from '@codex-proxy/ui'
import { computed, shallowRef, watch } from 'vue'
import { createEvaluationPreview } from './preview'

const props = defineProps<{ html: string }>()
const active = shallowRef(false)
watch(() => props.html, () => {
  active.value = false
})
const preview = computed(() => {
  if (!active.value)
    return null
  try {
    return { document: createEvaluationPreview(props.html), error: '' }
  }
  catch (error) {
    return { document: null, error: error instanceof Error ? error.message : '作品无法预览' }
  }
})
</script>

<template>
  <div class="min-w-0">
    <ZEmpty v-if="!active" title="作品尚未加载" description="仅预览内联 SVG 与 CSS 动画">
      <template #action>
        <ZButton @click="active = true">
          加载安全预览
        </ZButton>
      </template>
    </ZEmpty>
    <ZEmpty v-else-if="preview?.error" title="无法预览" :description="preview.error" />
    <template v-else-if="preview?.document">
      <iframe
        :srcdoc="preview.document.srcdoc" sandbox="" referrerpolicy="no-referrer" title="鹈鹕作品安全预览"
        class="h-96 w-full rounded-cp border-0 bg-cp-bg-container"
      />
      <div class="mt-2 flex items-center justify-between gap-3">
        <span class="text-cp-xs text-cp-text-tertiary">已禁用脚本、外部资源和跳转，原始回答保持不变</span>
        <ZButton size="small" @click="active = false">
          卸载预览
        </ZButton>
      </div>
    </template>
  </div>
</template>
