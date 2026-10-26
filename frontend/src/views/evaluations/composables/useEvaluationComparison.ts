import type { EvaluationItem, EvaluationRun } from '@/api'
import { ZNotification } from '@codex-proxy/ui'
import { shallowRef } from 'vue'
import { getEvaluationOutput } from '@/api'
import { useAsyncAction } from '@/composables/useAsyncAction'
import { useRequestState } from '@/composables/useRequestState'

export interface EvaluationComparison {
  item: EvaluationItem
  run: EvaluationRun
  html: string
}

export function useEvaluationComparison() {
  const entries = shallowRef<EvaluationComparison[]>([])
  const open = shallowRef(false)
  const action = useAsyncAction()
  const request = useRequestState()

  async function add(item: EvaluationItem, run: EvaluationRun) {
    if (entries.value.some(entry => entry.item.id === item.id)) {
      open.value = true
      return
    }
    if (entries.value.length >= 2) {
      ZNotification.warning({ message: '最多对比两个作品，请先移除一个' })
      open.value = true
      return
    }
    await action.run(async () => {
      const id = request.start()
      try {
        const output = await getEvaluationOutput(item.id, { signal: request.signal })
        if (!request.isCurrent(id))
          return
        if (!output.artifactHtml)
          throw new Error('此回答没有可对比的作品')
        entries.value = [
          ...entries.value,
          { item, run, html: output.artifactHtml },
        ]
        ZNotification.success({ message: `已加入作品对比（${entries.value.length}/2）` })
      }
      finally {
        request.finish(id)
      }
    })
  }
  function remove(id: string) {
    entries.value = entries.value.filter(entry => entry.item.id !== id)
  }
  return {
    entries,
    open,
    loading: action.loading,
    add,
    remove,
  }
}
