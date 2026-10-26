import type { EvaluationDefaults, EvaluationSettings } from '@/api'
import { ZNotification } from '@codex-proxy/ui'
import { ref, shallowRef, toRaw } from 'vue'
import { getEvaluationSettings, updateEvaluationSettings } from '@/api'
import { useAsyncAction } from '@/composables/useAsyncAction'
import { useRequestState } from '@/composables/useRequestState'

export function useEvaluationSettings() {
  const current = shallowRef<EvaluationSettings | null>(null)
  const lastSavedRevision = shallowRef<number | null>(null)
  const open = shallowRef(false)
  const draft = ref<EvaluationDefaults | null>(null)
  const apiKey = shallowRef('')
  const clearKey = shallowRef(false)
  const request = useRequestState()
  const action = useAsyncAction()
  let editingRevision = 0

  async function load() {
    const id = request.start()
    try {
      const settings = await getEvaluationSettings({ signal: request.signal })
      if (request.isCurrent(id))
        current.value = settings
    }
    catch (error) {
      request.fail(id, error)
    }
    finally { request.finish(id) }
  }

  async function show() {
    await load()
    if (!current.value || request.error.value) {
      ZNotification.error({ message: request.error.value || '无法读取测评设置，请重试' })
      return
    }
    draft.value = structuredClone(toRaw(current.value.defaults))
    editingRevision = current.value.revision
    apiKey.value = ''
    clearKey.value = false
    open.value = true
  }

  async function save() {
    if (!draft.value)
      return
    if (draft.value.judge?.type === 'typesafe' && !apiKey.value && (!current.value?.typesafeConfigured || clearKey.value)) {
      ZNotification.warning({ message: '请填写 TypeSafe API Key' })
      return
    }
    await action.run(async () => {
      current.value = await updateEvaluationSettings({
        revision: editingRevision,
        defaults: {
          ...structuredClone(toRaw(draft.value!)),
          model: draft.value!.model?.trim() || null,
        },
        ...(apiKey.value
          ? {
              typesafeApiKey: apiKey.value.trim(),
            }
          : clearKey.value
            ? { typesafeApiKey: '' }
            : {}),
      })
      lastSavedRevision.value = current.value.revision
      apiKey.value = ''
      open.value = false
      ZNotification.success({ message: '测评设置已保存，新建测评将自动带入' })
    })
  }

  return {
    current,
    lastSavedRevision,
    open,
    draft,
    apiKey,
    clearKey,
    loading: request.loading,
    saving: action.loading,
    load,
    show,
    save,
  }
}
