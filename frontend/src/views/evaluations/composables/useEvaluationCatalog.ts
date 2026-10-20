import type { SelectOption } from '@codex-proxy/ui'
import { useDebounceFn } from '@vueuse/core'
import { computed, shallowRef } from 'vue'
import { getAccounts, getApiKeys } from '@/api'
import { useRequestState } from '@/composables/useRequestState'

export function useEvaluationCatalog() {
  const accounts = shallowRef<SelectOption[]>([])
  const keys = shallowRef<SelectOption[]>([])
  const knownAccounts = shallowRef(new Map<string, SelectOption>())
  const knownKeys = shallowRef(new Map<string, SelectOption>())
  const accountRequest = useRequestState()
  const keyRequest = useRequestState()

  async function loadAccounts(search = '') {
    const id = accountRequest.start()
    try {
      const page = await getAccounts({ page: 1, pageSize: 100, search }, { signal: accountRequest.signal })
      if (!accountRequest.isCurrent(id))
        return
      accounts.value = page.items.map(item => ({
        value: item.id,
        label: item.name || item.id,
        description: `${item.provider} · ${item.id}`,
        disabled: !item.enabled,
      }))
      knownAccounts.value = new Map([...knownAccounts.value, ...accounts.value.map(item => [item.value, item] as const)])
    }
    catch (error) {
      accountRequest.fail(id, error)
    }
    finally {
      accountRequest.finish(id)
    }
  }

  async function loadKeys(search = '') {
    const id = keyRequest.start()
    try {
      const page = await getApiKeys({ limit: 100, search }, { signal: keyRequest.signal })
      if (!keyRequest.isCurrent(id))
        return
      keys.value = page.items.map(item => ({
        value: item.id,
        label: item.name || item.id,
        description: item.groups.length ? item.groups.map(group => group.name).join('、') : '全部账号',
        disabled: !item.enabled,
      }))
      knownKeys.value = new Map([...knownKeys.value, ...keys.value.map(item => [item.value, item] as const)])
    }
    catch (error) {
      keyRequest.fail(id, error)
    }
    finally {
      keyRequest.finish(id)
    }
  }

  function accountOptions(selected: string[]) {
    return mergeSelected(accounts.value, selected, knownAccounts.value)
  }
  function keyOptions(selected: string) {
    return mergeSelected(keys.value, selected ? [selected] : [], knownKeys.value)
  }
  return {
    accountOptions,
    keyOptions,
    accountName: (id: string) => knownAccounts.value.get(id)?.label ?? id,
    loadingAccounts: accountRequest.loading,
    loadingKeys: keyRequest.loading,
    ready: computed(() => !accountRequest.loading.value && !keyRequest.loading.value),
    load: () => Promise.allSettled([loadAccounts(), loadKeys()]),
    searchAccounts: useDebounceFn(loadAccounts, 250),
    searchKeys: useDebounceFn(loadKeys, 250),
  }
}

function mergeSelected(options: SelectOption[], selected: string[], known: Map<string, SelectOption>) {
  const existing = new Set(options.map(item => item.value))
  return [...options, ...selected.filter(id => !existing.has(id)).map(id => known.get(id) ?? { value: id, label: id })]
}
