import type { Ref } from 'vue'
import type { Account, EvaluationHistoryItem, EvaluationItem, EvaluationRun } from '@/api'
import { useDocumentVisibility, useIntervalFn } from '@vueuse/core'
import { isEqual } from 'es-toolkit'
import { computed, shallowRef, watch } from 'vue'
import { getAccountEvaluationHistory, getEvaluationDetail } from '@/api'
import { useRequestState } from '@/composables/useRequestState'

export interface AccountEvaluationSnapshot {
  accountIds: string[]
  items: EvaluationHistoryItem[] | undefined
}

export function useAccountEvaluations(accounts: Ref<Account[]>, options: {
  enabled: Ref<boolean>
  prefetched: Ref<AccountEvaluationSnapshot | undefined>
}) {
  const visibility = useDocumentVisibility()
  const request = useRequestState()
  const resultRequest = useRequestState()
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

  async function refresh() {
    if (!options.enabled.value || !ids.value || visibility.value !== 'visible')
      return
    const accountIds = ids.value.split(',')
    const id = request.start()
    try {
      const result = await getAccountEvaluationHistory(accountIds, { signal: request.signal, silent: true })
      if (!request.isCurrent(id))
        return
      updateHistory(result, accountIds)
    }
    catch (error) {
      request.fail(id, error)
    }
    finally {
      request.finish(id)
    }
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

  watch([
    ids,
    visibility,
    options.enabled,
    options.prefetched,
  ], ([accountIds, , enabled, prefetched], previous) => {
    request.invalidate()
    if (accountIds !== previous?.[0]) {
      resultRequest.invalidate()
      open.value = false
    }
    if (!enabled)
      return
    if (prefetched !== previous?.[3] && prefetched?.items !== undefined
      && [...prefetched.accountIds].sort().join(',') === accountIds) {
      const id = request.start()
      updateHistory(prefetched.items, prefetched.accountIds)
      request.finish(id)
      return
    }
    if (accountIds && visibility.value === 'visible')
      void refresh()
  }, { immediate: true })
  useIntervalFn(() => {
    if (!request.loading.value)
      void refresh()
  }, 15000)
  return {
    history,
    loading: request.loading,
    error: request.error,
    refresh,
    inspect,
    open,
    item,
    run,
  }
}
