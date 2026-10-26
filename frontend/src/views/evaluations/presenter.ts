import type { EvaluationItem, EvaluationPlan, EvaluationRun } from '@/api'
import { defineTableColumns } from '@codex-proxy/ui'

export { durationText, kindLabels, statusLabels, verdictLabels } from '@/components/evaluation/presenter'

export function runStatus(run: EvaluationRun) {
  if (run.activeItems)
    return run.stopRequested ? '正在停止' : '执行中'
  if (run.stopRequested)
    return '已停止'
  return '已结束'
}

export const runColumns = defineTableColumns<EvaluationRun>([
  {
    key: 'test',
    label: '测评',
    kind: 'identity',
    size: 'xl',
  },
  { key: 'status', label: '状态', kind: 'status' },
  {
    key: 'progress',
    label: '成功请求',
    kind: 'numeric',
    size: 'md',
  },
  {
    key: 'result',
    label: '题目结果',
    kind: 'text',
    size: 'lg',
  },
  { key: 'createdAtDisplay', label: '开始时间', kind: 'datetime' },
  {
    key: 'actions',
    label: '操作',
    kind: 'actions',
    size: 'lg',
  },
])

export const itemColumns = defineTableColumns<EvaluationItem>([
  {
    key: 'account',
    label: '账号 / 轮次',
    kind: 'identity',
    size: 'xl',
  },
  { key: 'status', label: '执行状态', kind: 'status' },
  { key: 'verdict', label: '题目结果', kind: 'status' },
  {
    key: 'duration',
    label: '耗时 / 首字',
    kind: 'text',
    size: 'lg',
  },
  {
    key: 'actions',
    label: '操作',
    kind: 'actions',
    size: 'xl',
  },
])

export const planColumns = defineTableColumns<EvaluationPlan>([
  {
    key: 'name',
    label: '计划',
    kind: 'identity',
    size: 'xl',
  },
  {
    key: 'test',
    label: '测评',
    kind: 'text',
    size: 'lg',
  },
  {
    key: 'schedule',
    label: '执行频率',
    kind: 'text',
    size: 'lg',
  },
  { key: 'nextRunAtDisplay', label: '下次执行', kind: 'datetime' },
  { key: 'status', label: '状态', kind: 'status' },
  {
    key: 'actions',
    label: '操作',
    kind: 'actions',
    size: 'xl',
  },
])
