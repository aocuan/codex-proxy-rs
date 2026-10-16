<script setup lang="ts">
import type { ProxyBatchTestingOptions } from '../composables/useProxyBatchTesting'
import { ZButton, ZDialog } from '@codex-proxy/ui'
import { Square, Wifi } from '@lucide/vue'
import { useProxyBatchTesting } from '../composables/useProxyBatchTesting'
import ProxyBatchResults from './ProxyBatchResults.vue'

const props = defineProps<ProxyBatchTestingOptions>()
const open = defineModel<boolean>({ required: true })
const { targets, warning, loading, error, items, running, stopping, busy, canRetry, start, retry, stop } = useProxyBatchTesting(open, props)
</script>

<template>
  <ZDialog v-model="open" title="批量测试代理" width="48rem" :show-close="!running" :close-on-click-modal="!running" :close-on-press-escape="!running">
    <div class="grid min-w-0 gap-4">
      <p class="m-0 text-cp-sm text-cp-text-secondary">
        {{ scope === 'selected' ? '测试选中的代理' : search.trim() ? `测试名称包含“${search.trim()}”的代理` : '测试全部代理' }}<span v-if="!loading">，本次 {{ targets.length }} 条</span>
      </p>
      <p v-if="loading" class="m-0 text-cp-sm text-cp-text-secondary" role="status">
        正在读取代理名单
      </p>
      <p v-if="error" class="m-0 text-cp-sm text-cp-error-text" role="alert">
        {{ error }}
      </p>
      <p v-if="warning" class="m-0 text-cp-sm text-cp-warning-text" role="status">
        {{ warning }}
      </p>
      <p v-if="!items.length && !loading && !error" class="m-0 text-cp-xs text-cp-text-secondary">
        同时测试 2 条，停止后等待已开始的测试结束
      </p>
      <ProxyBatchResults v-if="items.length" :items="items" testing />
    </div>
    <template #footer>
      <ZButton v-if="running" :disabled="stopping" @click="stop">
        <Square class="size-3.5" />
        {{ stopping ? '等待当前测试结束' : '停止后续测试' }}
      </ZButton>
      <ZButton v-else :disabled="loading" @click="open = false">
        关闭
      </ZButton>
      <ZButton v-if="!items.length" type="primary" :loading="busy" :disabled="!targets.length || Boolean(error)" @click="start">
        <Wifi class="size-4" />
        开始测试
      </ZButton>
      <ZButton v-else-if="!running && canRetry" type="primary" :loading="loading" @click="retry">
        重试未成功项
      </ZButton>
    </template>
  </ZDialog>
</template>
