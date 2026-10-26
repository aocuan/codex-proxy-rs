import type { Ref } from 'vue'
import type { EvaluationConfig, EvaluationParameters, EvaluationPlan, EvaluationPlanSettings, EvaluationRun, EvaluationSettings } from '@/api'
import { ZNotification } from '@codex-proxy/ui'
import { ref, shallowRef } from 'vue'
import { createEvaluationPlan, submitEvaluation, updateEvaluationPlan } from '@/api'
import { useAsyncAction } from '@/composables/useAsyncAction'
import { generateRequestId } from '@/utils/operation'

export function useEvaluationEditor(onSaved: (run?: EvaluationRun) => Promise<unknown>, config: Ref<EvaluationConfig | null>, settings: Ref<EvaluationSettings | null>) {
  const open = shallowRef(false)
  const isPlan = shallowRef(false)
  const editing = shallowRef<EvaluationPlan | null>(null)
  const parameters = ref<EvaluationParameters>(emptyParameters())
  const name = shallowRef('')
  const intervalMinutes = shallowRef('60')
  const enabled = shallowRef(true)
  const action = useAsyncAction()
  let submission: { fingerprint: string, id: string } | undefined

  function show(planMode: boolean, plan: EvaluationPlan | null = null, accountIds: string[] = []) {
    isPlan.value = planMode
    editing.value = plan
    parameters.value = plan
      ? structuredClone(plan.settings.parameters)
      : emptyParameters()
    if (!plan && settings.value) {
      const { judge: _judge, ...defaults } = settings.value.defaults
      parameters.value = {
        ...parameters.value,
        ...defaults,
        clientKeyId: defaults.clientKeyId ?? '',
        model: defaults.model ?? '',
        accountIds,
      }
    }
    parameters.value.definition ??= structuredClone(config.value?.tests.find(test => test.kind === parameters.value.kind)?.definition)
    name.value = plan?.settings.name ?? ''
    intervalMinutes.value = plan
      ? String(plan.settings.intervalMinutes ?? '')
      : '60'
    enabled.value = plan?.settings.enabled ?? true
    submission = undefined
    open.value = true
  }

  async function save() {
    const params = {
      ...parameters.value,
      accountIds: [...parameters.value.accountIds],
      model: parameters.value.model.trim(),
    }
    if (!params.definition?.name.trim() || !params.definition.prompt.trim()) {
      ZNotification.warning({ message: '请填写测评名称和提示词' })
      return
    }
    if (!params.clientKeyId || !params.model || params.accountIds.length < 1 || params.accountIds.length > 20) {
      ZNotification.warning({ message: '请选择计费 Key、1 到 20 个账号，并填写模型 ID' })
      return
    }
    const grader = params.definition.grader
    if ((grader.type === 'exact' && !grader.expected.trim()) || (grader.type === 'regex' && !grader.pattern.trim())) {
      ZNotification.warning({ message: '请填写判分规则' })
      return
    }
    if (grader.type === 'model' && (!grader.config.clientKeyId || !grader.config.accountId || !grader.config.model.trim() || !grader.config.rubric.trim() || params.accountIds.includes(grader.config.accountId))) {
      ZNotification.warning({ message: '请完整填写 AI 判题配置，并选择独立的判题账号' })
      return
    }
    if (grader.type === 'typesafe' && ((!settings.value?.typesafeConfigured && (!isPlan.value || enabled.value)) || !grader.config.rubric.trim())) {
      ZNotification.warning({ message: '请配置 TypeSafe API Key 并填写评分标准' })
      return
    }
    const minutes = Number(intervalMinutes.value)
    if (isPlan.value && (!Number.isInteger(minutes) || minutes < 1 || !config.value || minutes > config.value.limits.planIntervalMinutes)) {
      ZNotification.warning({ message: '请填写有效的执行间隔，至少 1 分钟' })
      return
    }
    await action.run(async () => {
      let run: EvaluationRun | undefined
      if (isPlan.value) {
        const settings: EvaluationPlanSettings = {
          name: name.value.trim() || `${params.definition!.name.slice(0, 24)} · ${minutes} 分钟`,
          intervalMinutes: minutes,
          enabled: enabled.value,
          parameters: params,
        }
        const plan = editing.value
        if (plan)
          await updateEvaluationPlan(plan.id, plan.revision, settings)
        else
          await createEvaluationPlan(settings)
      }
      else {
        const fingerprint = JSON.stringify(params)
        if (submission?.fingerprint !== fingerprint) {
          submission = {
            fingerprint,
            id: generateRequestId(),
          }
        }
        run = await submitEvaluation({ submissionId: submission.id, parameters: params })
      }
      open.value = false
      ZNotification.success({
        message: isPlan.value ? '计划已保存' : '测评已加入队列',
      })
      await onSaved(run)
    })
  }

  return {
    open,
    isPlan,
    editing,
    parameters,
    name,
    intervalMinutes,
    enabled,
    saving: action.loading,
    show,
    save,
  }
}

function emptyParameters(): EvaluationParameters {
  return {
    kind: 'candy',
    clientKeyId: '',
    accountIds: [],
    model: '',
    reasoningEffort: 'high',
    repetitions: 1,
    timeoutSeconds: 600,
  }
}
