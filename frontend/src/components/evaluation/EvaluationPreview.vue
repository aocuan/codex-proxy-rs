<script setup lang="ts">
import { ZButton } from '@codex-proxy/ui'
import { useFullscreen } from '@vueuse/core'
import { shallowRef, useTemplateRef, watch } from 'vue'

const props = defineProps<{ html: string }>()
const active = shallowRef(true)
const container = useTemplateRef<HTMLElement>('container')
const { isFullscreen, isSupported, toggle } = useFullscreen(container)
watch(() => props.html, () => {
  active.value = true
})
</script>

<template>
  <div ref="container" class="flex min-w-0 flex-col gap-2 bg-cp-bg-container" :class="isFullscreen ? 'h-screen p-3' : ''">
    <div class="flex shrink-0 items-center justify-end gap-2">
      <ZButton v-if="isSupported && (active || isFullscreen)" size="small" @click="toggle()">
        {{ isFullscreen ? '退出全屏' : '全屏预览' }}
      </ZButton>
      <ZButton size="small" @click="active = !active">
        {{ active ? '卸载预览' : '加载预览' }}
      </ZButton>
    </div>
    <iframe
      v-if="active"
      :srcdoc="props.html" title="HTML 作品预览"
      class="w-full rounded-cp border-0 bg-white"
      :class="isFullscreen ? 'min-h-0 flex-1' : 'h-[70vh] min-h-96 resize-y'"
    />
  </div>
</template>
