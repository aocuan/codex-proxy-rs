<script setup lang="ts">
import type { OperationsError } from '@/api'
import { ZButton } from '@codex-proxy/ui'
import { computed, shallowRef, watch } from 'vue'
import { getOperationsErrors } from '@/api'
import { useCopyText } from '@/composables/useCopyText'
import { useRequestState } from '@/composables/useRequestState'

const props = withDefaults(defineProps<{
  active: boolean
  requestId: string | null
  completedAt: string | null
  message: string | null
  errorCode?: string | null
  title?: string
}>(), { errorCode: null, title: '请求错误' })
const detail = shallowRef<OperationsError | null>(null)
const request = useRequestState()
const { loading, error } = request
const copyText = useCopyText()
const message = computed(() => detail.value?.message || props.message || '请求失败')
const metadata = computed(() => [
  detail.value?.upstreamStatusCode ? `上游 HTTP ${detail.value.upstreamStatusCode}` : null,
  detail.value?.providerErrorCode || props.errorCode,
].filter(Boolean).join(' · '))
const errorText = computed(() => [
  props.requestId ? `请求 ID：${props.requestId}` : null,
  metadata.value,
  message.value,
  detail.value?.errorDetails,
].filter(Boolean).join('\n\n'))

async function load() {
  if (!props.active || !props.requestId)
    return
  const id = request.start()
  detail.value = null
  try {
    // 以测评完成时间查询，避免旧测评落在默认的最近七天之外
    const completedAt = props.completedAt ? Date.parse(props.completedAt) : Number.NaN
    const result = await getOperationsErrors({
      requestId: props.requestId,
      period: '7d',
      asOf: Number.isFinite(completedAt) ? completedAt + 1 : Date.now(),
      currentPage: 1,
      pageSize: 100,
    }, { signal: request.signal, silent: true })
    // 同一次请求可能包含重试事件，只展示最终请求的错误，避免误用中途失败
    if (request.isCurrent(id))
      detail.value = result.items.find(item => item.id === props.requestId && item.metadata.source === 'model_request') ?? null
  }
  catch (error) {
    request.fail(id, error)
  }
  finally {
    request.finish(id)
  }
}

watch([() => props.active, () => props.requestId, () => props.completedAt], () => {
  request.invalidate()
  error.value = ''
  detail.value = null
  void load()
}, { immediate: true })
</script>

<template>
  <section class="mb-5 rounded-cp bg-cp-fill-quaternary p-4" :aria-label="title">
    <div class="flex flex-wrap items-center gap-2">
      <h3 class="m-0 text-cp-sm font-emphasis">
        {{ title }}
      </h3>
      <span v-if="metadata" class="text-cp-xs text-cp-text-secondary wrap-anywhere">{{ metadata }}</span>
      <ZButton class="ml-auto" size="small" variant="text" :disabled="loading" @click="copyText(errorText, { successText: '错误信息已复制' })">
        复制错误
      </ZButton>
    </div>
    <p class="mt-2 mb-0 text-cp-sm text-cp-error-text whitespace-pre-wrap wrap-anywhere">
      {{ message }}
    </p>
    <p v-if="loading" class="mt-3 mb-0 text-cp-sm text-cp-text-secondary" role="status">
      正在读取原始报错
    </p>
    <template v-else-if="error">
      <p class="mt-3 mb-0 text-cp-sm text-cp-warning-text">
        原始报错读取失败：{{ error }}
      </p>
      <ZButton size="small" variant="text" @click="load">
        重试
      </ZButton>
    </template>
    <pre v-else-if="detail?.errorDetails" class="mt-3 mb-0 max-h-80 overflow-auto font-mono text-cp-xs whitespace-pre-wrap wrap-anywhere">{{ detail.errorDetails }}</pre>
    <div v-else-if="requestId" class="mt-3 flex flex-wrap items-center gap-2 text-cp-xs text-cp-text-secondary">
      <span>未找到原始报错，可能尚未写入、未记录或已清理</span>
      <ZButton size="small" variant="text" @click="load">
        重新读取
      </ZButton>
    </div>
  </section>
</template>
