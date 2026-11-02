import type { Ref } from 'vue'
import type { Account, EvaluationHistoryItem, EvaluationItem, EvaluationMonitoringSnapshot, EvaluationRun } from '@/api'
import { isEqual } from 'es-toolkit'
import { computed, shallowRef, watch } from 'vue'
import { getEvaluationDetail } from '@/api'
import { useRequestState } from '@/composables/useRequestState'

export interface AccountEvaluationSnapshot {
  accountIds: string[]
  items: EvaluationHistoryItem[] | undefined
  monitoring?: EvaluationMonitoringSnapshot
}

export function useAccountEvaluations(accounts: Ref<Account[]>, options: {
  enabled: Ref<boolean>
  prefetched: Ref<AccountEvaluationSnapshot | undefined>
  loading: Ref<boolean>
  error: Ref<string>
  reload: () => Promise<unknown>
}) {
  const resultRequest = useRequestState()
  const lastMonitoring = shallowRef<EvaluationMonitoringSnapshot>()
  const history = shallowRef(new Map<string, EvaluationHistoryItem[]>())
  const ids = computed(() => accounts.value.map(account => account.id).sort().join(','))
  const open = shallowRef(false)
  const item = shallowRef<EvaluationItem | null>(null)
  const run = shallowRef<EvaluationRun | null>(null)

  function updateHistory(items: EvaluationHistoryItem[], accountIds: string[]) {
    const grouped = new Map(accountIds.map(id => [id, [] as EvaluationHistoryItem[]]))
    for (const point of items)
      grouped.get(point.accountId)?.push(point)
    for (const [id, points] of grouped) {
      const previous = history.value.get(id)
      if (previous && isEqual(previous, points))
        grouped.set(id, previous)
    }
    if (grouped.size !== history.value.size || [...grouped].some(([id, points]) => history.value.get(id) !== points))
      history.value = grouped
  }

  async function inspect(point: EvaluationHistoryItem) {
    const id = resultRequest.start()
    open.value = false
    try {
      const detail = await getEvaluationDetail(point.runId, { signal: resultRequest.signal })
      if (!resultRequest.isCurrent(id))
        return
      run.value = detail.run
      item.value = detail.items.find(item => item.id === point.id) ?? null
      open.value = item.value !== null
    }
    catch (error) {
      resultRequest.fail(id, error)
    }
    finally {
      resultRequest.finish(id)
    }
  }

  watch([ids, options.prefetched], ([accountIds, prefetched], previous) => {
    if (accountIds !== previous?.[0]) {
      resultRequest.invalidate()
      open.value = false
    }
    if (prefetched?.monitoring !== undefined)
      lastMonitoring.value = prefetched.monitoring
    if (prefetched?.items !== undefined && [...prefetched.accountIds].sort().join(',') === accountIds)
      updateHistory(prefetched.items, prefetched.accountIds)
    else if (!accountIds)
      history.value = new Map()
  }, { immediate: true })
  const monitoring = computed(() => lastMonitoring.value)
  const states = computed(() => new Map(monitoring.value?.accounts.map(account => [account.accountId, account]) ?? []))
  return {
    history,
    monitoring,
    states,
    loading: options.loading,
    error: computed(() => options.error.value || (options.enabled.value && options.prefetched.value?.items === undefined && options.prefetched.value
      ? '监测暂不可用，保留上次结果'
      : '')),
    refresh: options.reload,
    inspect,
    open,
    item,
    run,
  }
}
