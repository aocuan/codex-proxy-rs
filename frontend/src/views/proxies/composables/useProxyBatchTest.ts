import type { ProxyBatchItem } from '../utils/batch'
import type { OutboundProxyRecord } from '@/api'
import type { RequestOptions } from '@/api/request'
import { onScopeDispose, ref, shallowRef } from 'vue'
import { proxyRequestFailure } from '../utils/batch'

export type ProxyTestExecutor = (proxy: OutboundProxyRecord, options: RequestOptions, detectLocation: boolean) => Promise<OutboundProxyRecord>

export function useProxyBatchTest(test: ProxyTestExecutor) {
  const items = ref<ProxyBatchItem[]>([])
  const running = shallowRef(false)
  const stopping = shallowRef(false)
  let disposed = false

  function stop() {
    stopping.value = true
  }

  function reset() {
    if (running.value)
      return
    items.value = []
    stopping.value = false
  }

  async function run(proxies: OutboundProxyRecord[]) {
    if (running.value || disposed)
      return
    const targets = [...new Map(proxies.map(proxy => [proxy.id, proxy])).values()]
    items.value = targets.map(proxy => ({ key: proxy.id, name: proxy.name, endpoint: proxy.endpoint, status: 'pending', message: '' }))
    running.value = true
    stopping.value = false
    let cursor = 0
    async function worker() {
      while (cursor < targets.length) {
        if (stopping.value || disposed)
          break
        const index = cursor++
        const target = targets[index]!
        const item = items.value[index]!
        item.status = 'running'
        try {
          const record = await test(target, { silent: true, retry: false }, true)
          item.record = record
          if (record.lastTest) {
            item.status = record.lastTest.success ? 'succeeded' : 'failed'
            item.message = record.lastTest.message || (record.lastTest.success ? '连接正常' : '连接失败')
          }
          else {
            item.status = 'unknown'
            item.message = '未取得连通性结果，请刷新列表核对'
            stop()
          }
        }
        catch (error) {
          const failure = proxyRequestFailure(error)
          item.status = failure.status
          item.message = failure.message
          if (failure.stop)
            stop()
        }
      }
    }
    try {
      // 后端最多并行测试四条，这里使用两个 worker，保留单条操作的容量
      await Promise.all([worker(), worker()])
    }
    finally {
      for (const item of items.value) {
        if (item.status === 'pending') {
          item.status = 'skipped'
          item.message = '已停止后续测试'
        }
      }
      running.value = false
    }
  }

  onScopeDispose(() => {
    disposed = true
    stop()
  })

  return { items, running, stopping, run, stop, reset }
}
