import type { EvaluationHistoryItem, EvaluationKind, EvaluationStatus, EvaluationVerdict } from '@/api'
import { formatDuration } from '@/utils/format'

export const kindLabels: Record<EvaluationKind, string> = { candy: '糖果题', pelican: '画鹈鹕', turnstate: '快速测票' }
export const turnStateLabels = { accepted: '票据稳定', refreshed: '票据变化', unknown: '未判定' }
export const statusLabels: Record<EvaluationStatus, string> = {
  queued: '等待中',
  running: '执行中',
  succeeded: '已完成',
  failed: '请求失败',
  cancelled: '已取消',
  interrupted: '执行中断',
  skipped: '已跳过',
}
export const verdictLabels: Record<EvaluationVerdict, string> = {
  correct: '通过',
  wrong: '未通过',
  ungraded: '未判定',
  not_applicable: '人工查看',
}

export function durationText(ms: number | null) {
  return ms === null
    ? '未观测'
    : formatDuration(ms)
}

export function historyResult(item: Pick<EvaluationHistoryItem, 'status' | 'verdict' | 'judgmentStatus'>) {
  const status = item.status === 'succeeded'
    ? item.judgmentStatus ?? item.status
    : item.status
  if (status === 'queued' || status === 'running') {
    return {
      label: item.judgmentStatus ? '等待评审' : statusLabels[status],
      color: 'bg-cp-primary',
      text: 'text-cp-primary-text',
    }
  }
  if (status === 'failed' || status === 'interrupted') {
    return {
      label: item.judgmentStatus ? '判题异常' : statusLabels[status],
      color: 'bg-cp-warning',
      text: 'text-cp-warning-text',
    }
  }
  if (status !== 'succeeded')
    return { label: statusLabels[status], color: 'bg-cp-fill-secondary', text: 'text-cp-text-tertiary' }
  if (item.verdict === 'correct')
    return { label: '通过', color: 'bg-cp-success', text: 'text-cp-success-text' }
  if (item.verdict === 'wrong')
    return { label: '未通过', color: 'bg-cp-error', text: 'text-cp-error-text' }
  return { label: '未判定', color: 'bg-cp-fill-secondary', text: 'text-cp-text-tertiary' }
}
