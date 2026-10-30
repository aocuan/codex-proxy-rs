import type { SelectOption } from '@codex-proxy/ui'
import type { EvaluationKind } from '@/api'
import { useDebounceFn } from '@vueuse/core'
import { computed, shallowRef } from 'vue'
import { getAccountGroupOptions, getAccounts, getApiKeys } from '@/api'
import { useRequestState } from '@/composables/useRequestState'
import { formatProviderLabel } from '@/utils/providers'
import { accountIdentityLabel } from '@/views/accounts/utils/accountIdentity'

interface EvaluationAccountOption extends SelectOption {
  turnStateProbe: boolean
}

export function useEvaluationCatalog() {
  const accounts = shallowRef<EvaluationAccountOption[]>([])
  const groups = shallowRef<SelectOption[]>([])
  const preferenceGroups = shallowRef<SelectOption[]>([])
  const knownGroups = shallowRef(new Map<string, SelectOption>())
  const groupRequest = useRequestState()
  const keys = shallowRef<SelectOption[]>([])
  const knownAccounts = shallowRef(new Map<string, EvaluationAccountOption>())
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
        label: accountIdentityLabel(item),
        description: [
          formatProviderLabel(item.provider),
          item.authenticationKind === 'api_key'
            ? 'API'
            : item.planTypeDisplay,
          ...item.groups.map(group => group.name),
        ].filter(Boolean).join(' · '),
        disabled: !item.enabled,
        turnStateProbe: item.capabilities.turnStateProbe,
      }))
      knownAccounts.value = new Map([
        ...knownAccounts.value,
        ...accounts.value.map(item => [item.value, item] as const),
      ])
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
        description: item.groups.length
          ? item.groups.map(group => group.name).join('、')
          : '全部账号',
        disabled: !item.enabled,
      }))
      knownKeys.value = new Map([
        ...knownKeys.value,
        ...keys.value.map(item => [item.value, item] as const),
      ])
    }
    catch (error) {
      keyRequest.fail(id, error)
    }
    finally {
      keyRequest.finish(id)
    }
  }

  async function loadGroups(search = '') {
    const id = groupRequest.start()
    try {
      const result = await getAccountGroupOptions({ page: 1, pageSize: 100, search: search || undefined }, { signal: groupRequest.signal })
      if (groupRequest.isCurrent(id)) {
        if (!search)
          groups.value = result.items.map(item => ({ value: item.id, label: item.name }))
        preferenceGroups.value = result.items.map(item => ({ value: item.id, label: item.name, disabled: !item.enabled }))
        knownGroups.value = new Map([
          ...knownGroups.value,
          ...preferenceGroups.value.map(item => [item.value, item] as const),
        ])
      }
    }
    catch (error) { groupRequest.fail(id, error) }
    finally { groupRequest.finish(id) }
  }

  function supportedAccountIds(selected: string[], kind?: EvaluationKind) {
    return kind === 'turnstate'
      ? selected.filter(id => knownAccounts.value.get(id)?.turnStateProbe !== false)
      : selected
  }
  function accountOptions(selected: string[], kind?: EvaluationKind) {
    const options = kind === 'turnstate'
      ? accounts.value.filter(account => account.turnStateProbe)
      : accounts.value
    return mergeSelected(options, supportedAccountIds(selected, kind), knownAccounts.value)
  }
  function keyOptions(selected: string) {
    return mergeSelected(keys.value, selected
      ? [selected]
      : [], knownKeys.value)
  }
  return {
    accountOptions,
    supportedAccountIds,
    keyOptions,
    groups,
    preferenceGroupOptions: (selected: string) => mergeSelected(preferenceGroups.value, selected
      ? [selected]
      : [], knownGroups.value),
    groupName: (id: string) => knownGroups.value.get(id)?.label ?? id,
    loadingGroups: groupRequest.loading,
    accountName: (id: string) => knownAccounts.value.get(id)?.label ?? id,
    loadingAccounts: accountRequest.loading,
    loadingKeys: keyRequest.loading,
    ready: computed(() => !accountRequest.loading.value && !keyRequest.loading.value),
    load: () => Promise.allSettled([
      loadAccounts(),
      loadKeys(),
      loadGroups(),
    ]),
    searchAccounts: useDebounceFn(loadAccounts, 250),
    searchKeys: useDebounceFn(loadKeys, 250),
    searchGroups: useDebounceFn(loadGroups, 250),
  }
}

function mergeSelected(options: SelectOption[], selected: string[], known: ReadonlyMap<string, SelectOption>) {
  const existing = new Set(options.map(item => item.value))
  return [
    ...options,
    ...selected.filter(id => !existing.has(id)).map(id => known.get(id) ?? { value: id, label: id }),
  ]
}
