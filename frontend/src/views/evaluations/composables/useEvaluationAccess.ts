import type { SelectOption } from '@codex-proxy/ui'
import type { EvaluationAccountAccess } from '@/api'
import { computed, shallowRef, watch } from 'vue'
import { checkEvaluationAccountAccess } from '@/api'
import { useRequestState } from '@/composables/useRequestState'

export function useEvaluationAccess(active: () => boolean, key: () => string, options: () => SelectOption[], selected: () => string[]) {
  const request = useRequestState()
  const access = shallowRef(new Map<string, EvaluationAccountAccess>())
  const ids = computed(() => [
    ...new Set([
      ...options().map(item => item.value),
      ...selected(),
    ]),
  ])
  const checked = shallowRef('')
  const fingerprint = () => JSON.stringify([
    key(),
    ids.value,
  ])

  async function refresh() {
    request.invalidate()
    request.error.value = ''
    access.value = new Map()
    checked.value = ''
    if (!active() || !key() || !ids.value.length)
      return
    const id = request.start()
    const target = fingerprint()
    try {
      const result = await checkEvaluationAccountAccess(key(), ids.value, { signal: request.signal, silent: true })
      if (!request.isCurrent(id))
        return
      checked.value = target
      access.value = new Map(result.map(item => [item.accountId, item]))
    }
    catch (error) {
      request.fail(id, error)
    }
    finally {
      request.finish(id)
    }
  }

  const ready = computed(() => !request.loading.value && checked.value === fingerprint())
  const message = computed(() => {
    if (!key())
      return '请先选择 Key'
    if (request.error.value)
      return '账号授权检查失败，请重试'
    if (!ready.value) {
      return selected().length
        ? '正在检查账号授权'
        : ''
    }
    const disabled = selected().find(id => options().find(item => item.value === id)?.disabled)
    if (disabled)
      return `${options().find(item => item.value === disabled)?.label ?? disabled}：账号已停用`
    const rejected = selected().filter(id => !access.value.get(id)?.allowed)
    if (!rejected.length)
      return ''
    const names = rejected.map(id => options().find(item => item.value === id)?.label ?? id).join('、')
    return `${names}：${access.value.get(rejected[0]!)?.reason ?? '账号授权信息不可用'}`
  })
  const accountOptions = computed(() => options().map((item) => {
    const result = ready.value
      ? access.value.get(item.value)
      : undefined
    return {
      ...item,
      // 已选项保留移除入口，取消后立即禁止重新选入
      disabled: (item.disabled || !result?.allowed) && !selected().includes(item.value),
      description: [
        item.description,
        result && !result.allowed
          ? 'Key 未授权'
          : null,
        item.disabled ? '账号已停用' : null,
      ].filter(Boolean).join(' · '),
    }
  }))
  const valid = computed(() => !!key() && ready.value && selected().every(id => access.value.get(id)?.allowed && !options().find(item => item.value === id)?.disabled))
  watch([active, key, ids], refresh, { immediate: true })
  return {
    accountOptions,
    message,
    valid,
    loading: request.loading,
    error: request.error,
    refresh,
  }
}
