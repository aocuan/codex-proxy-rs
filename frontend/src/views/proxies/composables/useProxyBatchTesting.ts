import type { Ref } from 'vue'
import type { ProxyTestExecutor } from './useProxyBatchTest'
import type { OutboundProxyRecord } from '@/api'
import { computed, onScopeDispose, shallowRef, watch } from 'vue'
import { getProxies } from '@/api'
import { useRequestState } from '@/composables/useRequestState'
import { needsProxyBatchRetry } from '../utils/batch'
import { useProxyBatchTest } from './useProxyBatchTest'

export interface ProxyBatchTestingOptions {
  scope: 'selected' | 'filtered'
  search: string
  selectedIds: string[]
  test: ProxyTestExecutor
  reload: () => Promise<unknown>
}

export function useProxyBatchTesting(open: Ref<boolean>, options: ProxyBatchTestingOptions) {
  const targets = shallowRef<OutboundProxyRecord[]>([])
  const warning = shallowRef('')
  const request = useRequestState()
  const { loading, error } = request
  const tests = useProxyBatchTest((proxy, requestOptions, detectLocation) => options.test(proxy, requestOptions, detectLocation))
  const { items, running, stopping } = tests
  const busy = computed(() => loading.value || running.value)
  const canRetry = computed(() => items.value.some(needsProxyBatchRetry))
  let disposed = false

  async function loadTargets(ids?: string[]) {
    const requestId = request.start()
    warning.value = ''
    targets.value = []
    const wanted = ids ? new Set(ids) : undefined
    const search = wanted ? undefined : options.search.trim() || undefined
    const collected = new Map<string, OutboundProxyRecord>()
    try {
      let totalPages = 1
      for (let page = 1; page <= totalPages; page += 1) {
        const result = await getProxies({ page, pageSize: 200, search }, { silent: true, signal: request.signal })
        if (!request.isCurrent(requestId))
          return
        // 只读取首次分页给出的范围，完成后固定实际名单，不追随新增记录无限加载
        if (page === 1)
          totalPages = result.page.totalPages
        for (const proxy of result.items) {
          if (!wanted || wanted.has(proxy.id))
            collected.set(proxy.id, proxy)
        }
        if (wanted && collected.size === wanted.size)
          break
      }
      if (!request.isCurrent(requestId))
        return
      targets.value = [...collected.values()]
      if (wanted && collected.size < wanted.size)
        warning.value = `${wanted.size - collected.size} 个选中代理未找到，未加入本次测试`
      return true
    }
    catch (cause) {
      request.fail(requestId, cause)
    }
    finally {
      request.finish(requestId)
    }
  }

  async function start() {
    if (disposed || !open.value || busy.value || !targets.value.length || error.value)
      return
    await tests.run(targets.value)
    if (!disposed)
      await options.reload()
  }

  async function retry() {
    if (busy.value)
      return
    const ids = items.value.filter(needsProxyBatchRetry).map(item => item.key)
    if (await loadTargets(ids))
      await start()
  }

  watch(open, (value) => {
    if (!value) {
      request.invalidate()
      tests.stop()
      return
    }
    tests.reset()
    void loadTargets(options.scope === 'selected' ? options.selectedIds : undefined)
  })

  onScopeDispose(() => {
    disposed = true
  })

  return { targets, warning, loading, error, items, running, stopping, busy, canRetry, start, retry, stop: tests.stop }
}
