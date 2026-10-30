import type { TableSort } from '@codex-proxy/ui'
import type { Ref } from 'vue'
import type { AccountEvaluationSnapshot } from './useAccountEvaluations'
import type { Account, AccountEvaluationStatus } from '@/api'

import { useDocumentVisibility, useTimeoutPoll } from '@vueuse/core'
import { computed, onMounted, shallowRef, watch } from 'vue'
import { getAccounts } from '@/api'
import { usePagedQuery } from '@/composables/usePagedQuery'
import { useTableSort } from '@/composables/useTableSort'
import { accountColumns } from '../constants'

export function useAccountsQuery(options: { includeEvaluationHistory: Ref<boolean> }) {
  const searchQuery = shallowRef('')
  const providerQuery = shallowRef('')
  const statusQuery = shallowRef('')
  const evaluationStatusQuery = shallowRef<AccountEvaluationStatus | ''>('')
  const groupQuery = shallowRef('')
  const sort = useTableSort(accountColumns, 'accounts')
  const refreshInterval = shallowRef('0')
  const refreshing = shallowRef(false)
  const evaluationSnapshot = shallowRef<AccountEvaluationSnapshot>()
  const accountSummary = shallowRef({
    total: 0,
    normal: 0,
    quotaExhausted: 0,
    rateLimited: 0,
    disabled: 0,
    error: 0,
  })

  const query = usePagedQuery({
    initialPageSize: 20,
    load: ({ page, pageSize }, requestOptions) =>
      getAccounts({
        page,
        pageSize,
        search: searchQuery.value,
        provider: providerQuery.value || undefined,
        status: statusQuery.value || undefined,
        evaluationStatus: evaluationStatusQuery.value || undefined,
        groupId: groupQuery.value || undefined,
        sortBy: sort.value?.key,
        sortDirection: sort.value?.direction,
        includeEvaluationHistory: options.includeEvaluationHistory.value || undefined,
      }, requestOptions),
    onSuccess: (result) => {
      accountSummary.value = result.summary
      evaluationSnapshot.value = {
        accountIds: result.items.filter(account => account.capabilities.turnStateProbe).map(account => account.id),
        items: result.evaluationHistory,
      }
    },
  })

  const accountPagination = computed(() => ({
    currentPage: query.page.value,
    pageSize: query.pageSize.value,
    total: query.total.value,
  }))

  function handlePageChange(page: number) {
    void query.execute({ page })
  }

  function handlePageSizeChange(pageSize: number) {
    void query.execute({ page: 1, pageSize })
  }

  function handleSortChange(nextSort: TableSort | undefined) {
    sort.value = nextSort
    void query.reloadFromStart()
  }

  async function replaceAccount(updated: Account) {
    // 先取消旧查询并应用接口返回的账号，避免旧响应覆盖最新行数据。
    query.invalidate()
    query.items.value = query.items.value.map(account => account.id === updated.id
      ? updated
      : account)

    // 筛选、排序、概览和末页回退仍由回读校准，但不触发整表加载。
    if (!await query.execute({ background: true }))
      return true // 回读失败或被新查询取代时，不依据旧页面取消选择。
    return query.items.value.some(account => account.id === updated.id)
  }

  async function refreshAccounts(silent = false) {
    if (query.loading.value || refreshing.value)
      return
    refreshing.value = true
    try {
      await query.execute({ background: true, silent })
    }
    finally {
      refreshing.value = false
    }
  }

  const visibility = useDocumentVisibility()
  const poll = useTimeoutPoll(
    () => refreshAccounts(true),
    computed(() => Math.max(1, Number(refreshInterval.value)) * 1000),
    { immediate: false },
  )
  watch([visibility, refreshInterval], ([visible, interval]) => {
    poll.pause()
    if (visible === 'visible' && interval !== '0')
      poll.resume()
  }, { immediate: true })

  watch(searchQuery, (_value, _previous, onCleanup) => {
    const timer = setTimeout(() => {
      void query.reloadFromStart()
    }, 250)
    onCleanup(() => clearTimeout(timer))
  })

  watch([
    providerQuery,
    statusQuery,
    groupQuery,
    evaluationStatusQuery,
  ], () => {
    void query.reloadFromStart()
  })

  onMounted(() => {
    void query.execute()
  })

  return {
    page: query.page,
    pageSize: query.pageSize,
    totalAccounts: query.total,
    loading: query.loading,
    refreshing,
    refreshInterval,
    refreshAccounts,
    accounts: query.items,
    evaluationSnapshot,
    loadAccounts: query.execute,
    refreshAccountsSilently: () => query.execute({ silent: true }),
    searchQuery,
    providerQuery,
    statusQuery,
    evaluationStatusQuery,
    groupQuery,
    sort,
    accountSummary,
    accountPagination,
    replaceAccount,
    handlePageChange,
    handlePageSizeChange,
    handleSortChange,
  }
}
