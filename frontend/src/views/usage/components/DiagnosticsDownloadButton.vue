<script setup lang="ts">
import { ZButton } from '@codex-proxy/ui'
import { Download } from '@lucide/vue'
import { getDiagnosticsDownloadUrl } from '@/api'

const props = defineProps<{ requestId?: string | null, errorId?: string | null, active: boolean }>()

function download() {
  const id = props.requestId || props.errorId
  if (!id || !props.active)
    return
  const target = props.requestId ? { requestId: props.requestId } : { errorId: id }
  window.open(getDiagnosticsDownloadUrl(target), '_blank', 'noopener,noreferrer')
}
</script>

<template>
  <ZButton type="primary" variant="plain" :disabled="!active || (!requestId && !errorId)" @click="download">
    <template #icon>
      <Download :size="16" />
    </template>
    下载诊断包
  </ZButton>
</template>
