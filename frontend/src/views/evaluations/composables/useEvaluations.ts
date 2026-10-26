import type { EvaluationConfig, EvaluationDetail, EvaluationPlan, EvaluationRun } from '@/api'
import { useDocumentVisibility, useIntervalFn } from '@vueuse/core'
import { onMounted, shallowRef, watch } from 'vue'
import { deleteEvaluationPlan, getEvaluationConfig, getEvaluationDetail, getEvaluationPlans, getEvaluationRuns, stopEvaluation, updateEvaluationPlan } from '@/api'
import { useAsyncAction } from '@/composables/useAsyncAction'
import { useRequestState } from '@/composables/useRequestState'

export function useEvaluations() {
  const config = shallowRef<EvaluationConfig | null>(null)
  const runs = shallowRef<EvaluationRun[]>([])
  const plans = shallowRef<EvaluationPlan[]>([])
  const detail = shallowRef<EvaluationDetail | null>(null)
  const selectedId = shallowRef('')
  const showDetail = shallowRef(false)
  const page = shallowRef(1)
  const hasNext = shallowRef(false)
  const pageSize = 20
  const tab = shallowRef('history')
  const runRequest = useRequestState()
  const planRequest = useRequestState()
  const detailRequest = useRequestState()
  const configRequest = useRequestState()
  const stoppingId = shallowRef('')
  const changingPlanId = shallowRef('')
  const stopAction = useAsyncAction()
  const planAction = useAsyncAction()
  const visibility = useDocumentVisibility()
  let polling = false

  async function loadRuns(silent = false, targetPage = page.value) {
    const id = runRequest.start(silent)
    try {
      const result = await getEvaluationRuns({ limit: pageSize + 1, offset: (targetPage - 1) * pageSize }, { silent, signal: runRequest.signal })
      if (!runRequest.isCurrent(id))
        return
      runs.value = result.slice(0, pageSize)
      hasNext.value = result.length > pageSize
      page.value = targetPage
    }
    catch (error) {
      runRequest.fail(id, error, silent)
    }
    finally {
      runRequest.finish(id)
    }
  }

  async function loadPlans(silent = false) {
    const id = planRequest.start(silent)
    try {
      const result = await getEvaluationPlans({ silent, signal: planRequest.signal })
      if (planRequest.isCurrent(id))
        plans.value = result
    }
    catch (error) {
      planRequest.fail(id, error, silent)
    }
    finally {
      planRequest.finish(id)
    }
  }

  async function loadDetail(silent = false) {
    if (!selectedId.value || !showDetail.value)
      return
    const id = detailRequest.start(silent)
    try {
      const result = await getEvaluationDetail(selectedId.value, { silent, signal: detailRequest.signal })
      if (detailRequest.isCurrent(id))
        detail.value = result
    }
    catch (error) {
      detailRequest.fail(id, error, silent)
    }
    finally {
      detailRequest.finish(id)
    }
  }

  function inspect(run: EvaluationRun) {
    selectedId.value = run.id
    detail.value = null
    showDetail.value = true
    void loadDetail()
  }

  async function stop(run: EvaluationRun) {
    await stopAction.run(async () => {
      stoppingId.value = run.id
      try {
        const result = await stopEvaluation(run.id)
        if (selectedId.value === run.id && showDetail.value)
          detail.value = result
        await loadRuns(true)
      }
      finally {
        stoppingId.value = ''
      }
    })
  }

  async function changePlan(plan: EvaluationPlan, remove = false) {
    await planAction.run(async () => {
      changingPlanId.value = plan.id
      try {
        if (remove) {
          await deleteEvaluationPlan(plan.id, plan.revision)
        }
        else {
          const { name, parameters, intervalMinutes, cronExpression } = plan.settings
          await updateEvaluationPlan(plan.id, plan.revision, {
            name,
            parameters,
            intervalMinutes,
            cronExpression,
            enabled: !plan.settings.enabled,
          })
        }
        await loadPlans(true)
      }
      finally {
        changingPlanId.value = ''
      }
    })
  }

  async function loadConfig() {
    const id = configRequest.start()
    try {
      const result = await getEvaluationConfig({ signal: configRequest.signal })
      if (configRequest.isCurrent(id))
        config.value = result
    }
    catch (error) {
      configRequest.fail(id, error)
    }
    finally {
      configRequest.finish(id)
    }
  }

  async function refresh() {
    await Promise.allSettled([
      loadRuns(),
      loadPlans(),
      config.value
        ? Promise.resolve()
        : loadConfig(),
    ])
  }

  onMounted(() => void refresh())
  watch(showDetail, (open) => {
    if (!open)
      detailRequest.invalidate()
  })
  useIntervalFn(async () => {
    if (visibility.value !== 'visible' || polling)
      return
    polling = true
    try {
      await Promise.allSettled([
        tab.value === 'history' && !runRequest.loading.value
          ? loadRuns(true)
          : Promise.resolve(),
        tab.value === 'plans' && !planRequest.loading.value
          ? loadPlans(true)
          : Promise.resolve(),
        showDetail.value && detail.value?.run.activeItems && !detailRequest.loading.value
          ? loadDetail(true)
          : Promise.resolve(),
      ])
    }
    finally {
      polling = false
    }
  }, 5000)

  return {
    config,
    runs,
    plans,
    detail,
    showDetail,
    page,
    hasNext,
    tab,
    loadingRuns: runRequest.loading,
    loadingPlans: planRequest.loading,
    loadingDetail: detailRequest.loading,
    historyError: runRequest.error,
    plansError: planRequest.error,
    detailError: detailRequest.error,
    stoppingId,
    changingPlanId,
    stopping: stopAction.loading,
    changingPlan: planAction.loading,
    inspect,
    stop,
    changePlan,
    loadRuns,
    loadPlans,
    loadDetail,
    refresh,
  }
}
