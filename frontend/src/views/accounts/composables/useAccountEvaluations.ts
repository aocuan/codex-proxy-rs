import type { Ref } from 'vue'
import type { Account, EvaluationHistoryItem, EvaluationItem, EvaluationRun } from '@/api'
import { useDocumentVisibility, useIntervalFn } from '@vueuse/core'
import { computed, shallowRef, watch } from 'vue'
import { getAccountEvaluationHistory, getEvaluationDetail } from '@/api'
import { useRequestState } from '@/composables/useRequestState'

export function useAccountEvaluations(accounts: Ref<Account[]>) {
  const visibility = useDocumentVisibility()
  const request = useRequestState()
  const resultRequest = useRequestState()
  const history = shallowRef(new Map<string, EvaluationHistoryItem[]>())
  const ids = computed(() => accounts.value.map(account => account.id).join(','))
  const open = shallowRef(false)
  const item = shallowRef<EvaluationItem | null>(null)
  const run = shallowRef<EvaluationRun | null>(null)

  async function refresh() {
    if (!ids.value || visibility.value !== 'visible')
      return
    const id = request.start()
    try {
      const result = await getAccountEvaluationHistory(ids.value.split(','), { signal: request.signal, silent: true })
      if (!request.isCurrent(id))
        return
      const grouped = new Map<string, EvaluationHistoryItem[]>()
      for (const point of result) {
        const points = grouped.get(point.accountId) ?? []
        points.push(point)
        grouped.set(point.accountId, points)
      }
      history.value = grouped
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

  watch([ids, visibility], ([accountIds], previous) => {
    request.invalidate()
    if (accountIds !== previous?.[0]) {
      history.value = new Map()
      resultRequest.invalidate()
      open.value = false
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
