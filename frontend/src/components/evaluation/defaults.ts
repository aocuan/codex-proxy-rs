import type { EvaluationGrader, EvaluationJudgeDefaults } from '@/api'

export const effortOptions = [
  { label: '跟随账号配置', value: 'default' },
  { label: 'Low', value: 'low' },
  { label: 'Medium', value: 'medium' },
  { label: 'High', value: 'high' },
  { label: 'Xhigh', value: 'xhigh' },
]

export function judgeGrader(judge: EvaluationJudgeDefaults, rubric = '', referenceAnswer: string | null = null): EvaluationGrader {
  if (judge.type === 'typesafe') {
    const { type, ...connection } = judge
    return { type, config: { ...connection, rubric, referenceAnswer } }
  }
  const { type, ...connection } = judge
  return { type, config: { ...connection, rubric, referenceAnswer } }
}

export function graderLabel(grader: EvaluationGrader) {
  switch (grader.type) {
    case 'none': return '仅查看回答或作品'
    case 'exact': return '自动核对答案'
    case 'regex': return '按规则判分'
    case 'model': return `AI 判题 · ${grader.config.model || '待配置'}`
    case 'typesafe': return `TypeSafe 判题 · ${grader.config.model}`
  }
}
