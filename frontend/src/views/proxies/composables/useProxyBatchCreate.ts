import type { ProxyBatchItem } from '../utils/batch'
import type { ProxyTestExecutor } from './useProxyBatchTest'
import { computed, onScopeDispose, ref, shallowRef } from 'vue'
import { createProxy } from '@/api'
import { MAX_BATCH_INPUT_BYTES, parseProxyBatch, proxyBatchInputError, proxyRequestFailure } from '../utils/batch'
import { useProxyBatchTest } from './useProxyBatchTest'

interface CreateItem extends ProxyBatchItem {
  input: string
  proxyUrl: string
}

export function useProxyBatchCreate(options: { test: ProxyTestExecutor, reload: () => Promise<unknown> }) {
  const text = shallowRef('')
  const prefix = shallowRef('代理')
  const protocol = shallowRef('http')
  const items = ref<CreateItem[]>([])
  const busy = shallowRef(false)
  const stopRequested = shallowRef(false)
  const testingRequested = shallowRef(false)
  const tests = useProxyBatchTest(options.test)
  const stopping = computed(() => stopRequested.value || tests.stopping.value)
  const parsed = computed(() => text.value.length > MAX_BATCH_INPUT_BYTES ? [] : parseProxyBatch(text.value, prefix.value, protocol.value))
  const inputError = computed(() => proxyBatchInputError(text.value, parsed.value.length))
  const created = computed(() => items.value.flatMap(item => item.record ? [item.record] : []))
  const canEditFailures = computed(() => items.value.some(item => !item.record && item.status !== 'unknown' && Boolean(item.input)))
  let disposed = false

  function stop() {
    stopRequested.value = true
    tests.stop()
  }

  function clear() {
    if (busy.value)
      return
    text.value = ''
    prefix.value = '代理'
    protocol.value = 'http'
    items.value = []
    testingRequested.value = false
    stopRequested.value = false
    tests.reset()
  }

  function editFailures() {
    if (busy.value)
      return
    text.value = items.value.filter(item => !item.record && item.status !== 'unknown').map(item => item.input).filter(Boolean).join('\n')
    items.value = []
    testingRequested.value = false
    stopRequested.value = false
    tests.reset()
  }

  async function submit(testAfterCreate: boolean) {
    if (busy.value || disposed || inputError.value || !parsed.value.length)
      return
    items.value = parsed.value.map(row => ({
      key: String(row.line),
      name: row.name,
      endpoint: row.endpoint,
      input: row.raw,
      proxyUrl: row.proxyUrl,
      status: row.error ? 'failed' : 'pending',
      message: row.error,
    }))
    // 开始后只保留逐行输入，成功项立即清除认证，避免原始文本保留另一份明文
    text.value = ''
    busy.value = true
    stopRequested.value = false
    testingRequested.value = testAfterCreate
    try {
      for (const item of items.value) {
        if (stopRequested.value || disposed)
          break
        if (item.status !== 'pending')
          continue
        item.status = 'running'
        try {
          const result = await createProxy({ name: item.name, proxyUrl: item.proxyUrl, autoLocation: false, location: null }, { silent: true, retry: false })
          item.record = result.record
          item.endpoint = result.record.endpoint
          item.status = 'succeeded'
          item.message = '已添加'
          item.input = ''
          item.proxyUrl = ''
        }
        catch (error) {
          const failure = proxyRequestFailure(error)
          item.status = failure.status
          item.message = failure.status === 'conflict' ? '代理已存在或配置冲突，未覆盖已有记录' : failure.message
          if (failure.stop)
            stop()
        }
      }
      if (testAfterCreate && !stopRequested.value && !disposed && created.value.length)
        await tests.run(created.value)
    }
    finally {
      for (const item of items.value) {
        if (item.status === 'pending') {
          item.status = 'skipped'
          item.message = '已停止后续添加'
        }
      }
      busy.value = false
      if (!disposed)
        await options.reload()
    }
  }

  async function testCreated() {
    if (busy.value || disposed || !created.value.length)
      return
    busy.value = true
    stopRequested.value = false
    testingRequested.value = true
    try {
      await tests.run(created.value)
    }
    finally {
      busy.value = false
      if (!disposed)
        await options.reload()
    }
  }

  onScopeDispose(() => {
    disposed = true
    stop()
    text.value = ''
    for (const item of items.value) {
      item.input = ''
      item.proxyUrl = ''
    }
  })

  return { text, prefix, protocol, items, busy, stopping, testingRequested, tests, parsed, inputError, created, canEditFailures, submit, testCreated, stop, clear, editFailures }
}
