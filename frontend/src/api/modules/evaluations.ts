import type { RequestOptions } from '../request'
import request from '../request'

export type EvaluationKind = 'candy' | 'pelican'
export type EvaluationStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled' | 'interrupted' | 'skipped'
export type EvaluationVerdict = 'correct' | 'wrong' | 'ungraded' | 'not_applicable'

export interface EvaluationAccountAccess {
  accountId: string
  allowed: boolean
  reason: string | null
}

export function checkEvaluationAccountAccess(clientKeyId: string, accountIds: string[], options: RequestOptions = {}) {
  return request<EvaluationAccountAccess[]>({
    url: '/api/admin/evaluations/accounts/access',
    method: 'POST',
    data: { clientKeyId, accountIds },
    ...options,
  })
}

export interface EvaluationJudgeConfig {
  clientKeyId: string
  accountId: string
  model: string
  reasoningEffort: string | null
  timeoutSeconds: number
  rubric: string
  referenceAnswer: string | null
}

export interface TypeSafeJudgeConfig {
  model: string
  decisionThreshold: number
  timeoutSeconds: number
  rubric: string
  referenceAnswer: string | null
}

export type EvaluationJudgeDefaults
  = ({ type: 'model' } & Omit<EvaluationJudgeConfig, 'rubric' | 'referenceAnswer'>)
    | ({ type: 'typesafe' } & Omit<TypeSafeJudgeConfig, 'rubric' | 'referenceAnswer'>)

export interface EvaluationDefaults {
  clientKeyId: string | null
  model: string | null
  reasoningEffort: string | null
  repetitions: number
  timeoutSeconds: number
  judge: EvaluationJudgeDefaults | null
}

export interface EvaluationSettings {
  revision: number
  defaults: EvaluationDefaults
  typesafeConfigured: boolean
}

export type EvaluationGrader
  = { type: 'none' }
    | { type: 'exact', source: 'full_text' | 'final_integer', expected: string }
    | { type: 'regex', pattern: string }
    | { type: 'model', config: EvaluationJudgeConfig }
    | { type: 'typesafe', config: TypeSafeJudgeConfig }

export interface EvaluationDefinition {
  name: string
  prompt: string
  outputFormat: 'text' | 'html'
  grader: EvaluationGrader
}

export interface EvaluationParameters {
  kind: EvaluationKind
  definition?: EvaluationDefinition
  clientKeyId: string
  accountIds: string[]
  model: string
  reasoningEffort: string | null
  repetitions: number
  timeoutSeconds: number
}

export interface EvaluationProfile {
  definition?: EvaluationDefinition
  prompt: string
  promptVersion: string
  requestProfileVersion: string
  graderVersion: string
  gatewayVersion: string
}

export interface EvaluationRun {
  id: string
  parameters: EvaluationParameters
  profile: EvaluationProfile
  planId: string | null
  scheduledAt: string | null
  stopRequested: boolean
  createdAt: string
  createdAtDisplay: string
  totalItems: number
  retainedItems: number
  activeItems: number
  succeededItems: number
  correctItems: number
}

export interface EvaluationObservation {
  requestHistoryAvailable: boolean
  provider: string | null
  accountId: string | null
  upstreamModel: string | null
  responseModel: string | null
  reasoningEffort: string | null
  reasoningPreset: string | null
  serviceTier: string | null
  inputTokens: number | null
  outputTokens: number | null
  cachedInputTokens: number | null
  reasoningOutputTokens: number | null
  costAmount: string | null
  costCurrency: string | null
  costSource: string | null
}

export interface EvaluationItem {
  judgment: EvaluationJudgment | null
  id: string
  runId: string
  accountId: string
  repetition: number
  status: EvaluationStatus
  requestId: string | null
  verdict: EvaluationVerdict
  finalAnswer: number | null
  errorCode: string | null
  message: string | null
  ttftMs: number | null
  durationMs: number | null
  startedAt: string | null
  startedAtDisplay: string | null
  completedAt: string | null
  completedAtDisplay: string | null
  hasOutput: boolean
  hasArtifact: boolean
  observation: EvaluationObservation
}

export interface EvaluationJudgment {
  id: string
  itemId: string
  accountId: string | null
  attempt: number
  status: EvaluationStatus
  requestId: string | null
  verdict: EvaluationVerdict
  reason: string | null
  observation: EvaluationObservation
  createdAt: string
  completedAt: string | null
}

export interface EvaluationDetail {
  run: EvaluationRun
  items: EvaluationItem[]
}

export interface EvaluationHistoryItem {
  name: string
  profileVersion: string
  graderType: string
  judgmentStatus: EvaluationStatus | null
  id: string
  runId: string
  accountId: string
  kind: EvaluationKind
  model: string
  reasoningEffort: string | null
  status: EvaluationStatus
  verdict: EvaluationVerdict
  finalAnswer: number | null
  createdAt: string
  startedAt: string | null
  completedAt: string | null
  testedAtDisplay: string
}

export interface EvaluationOutput {
  responseText: string
  artifactHtml: string | null
}

export type EvaluationAccountSelection = { mode: 'fixed' } | { mode: 'all' } | { mode: 'groups', groupIds: string[] }

export interface EvaluationPlanSettings {
  accountSelection?: EvaluationAccountSelection
  name: string
  parameters: EvaluationParameters
  enabled: boolean
  intervalMinutes?: number
  cronExpression?: string
}

export interface EvaluationPlan {
  availableAccounts: number | null
  id: string
  revision: number
  settings: EvaluationPlanSettings & { timezone: string }
  nextRunAt: string | null
  nextRunAtDisplay: string | null
  createdAt: string
  updatedAt: string
}

export interface EvaluationConfig {
  tests: { kind: EvaluationKind, label: string, prompt: string, promptVersion: string, definition: EvaluationDefinition }[]
  timezone: string
  requestProfileVersion: string
  graderVersion: string
  limits: {
    accounts: number
    repetitions: number
    timeoutSeconds: number
    queueItems: number
    parallelRequests: number
    responseBytes: number
    artifactBytes: number
    retentionDays: number
    planIntervalMinutes: number
  }
}

const root = '/api/admin/evaluations'

export function getEvaluationConfig(options: RequestOptions = {}) {
  return request<EvaluationConfig>({ url: `${root}/config`, ...options })
}

export function getEvaluationRuns(params: { limit: number, offset: number }, options: RequestOptions = {}) {
  return request<EvaluationRun[]>({ url: `${root}/runs`, params, ...options })
}

export function getAccountEvaluationHistory(accountIds: string[], options: RequestOptions = {}) {
  return request<EvaluationHistoryItem[]>({
    url: `${root}/accounts/history`,
    params: {
      accountIds: accountIds.join(','),
      limit: 28,
    },
    ...options,
  })
}

export function submitEvaluation(data: { submissionId: string, parameters: EvaluationParameters }) {
  // 同一草稿重试使用同一提交标识，网络丢失响应不应再消耗一次额度
  return request<EvaluationRun>({
    url: `${root}/runs/create`,
    method: 'POST',
    data,
    retry: true,
  })
}

export function getEvaluationDetail(id: string, options: RequestOptions = {}) {
  return request<EvaluationDetail>({
    url: `${root}/runs/detail`,
    params: { id },
    ...options,
  })
}

export function stopEvaluation(id: string) {
  return request<EvaluationDetail>({
    url: `${root}/runs/stop`,
    method: 'POST',
    data: { id },
  })
}

export function getEvaluationOutput(id: string, options: RequestOptions = {}) {
  return request<EvaluationOutput>({
    url: `${root}/items/output`,
    params: { id },
    ...options,
  })
}

export function getEvaluationPlans(options: RequestOptions = {}) {
  return request<EvaluationPlan[]>({ url: `${root}/plans`, ...options })
}

export function createEvaluationPlan(data: EvaluationPlanSettings) {
  return request<EvaluationPlan>({ url: `${root}/plans/create`, method: 'POST', data })
}

export function updateEvaluationPlan(id: string, revision: number, settings: EvaluationPlanSettings) {
  return request<EvaluationPlan>({
    url: `${root}/plans/update`,
    method: 'POST',
    data: { id, revision, settings },
  })
}

export function deleteEvaluationPlan(id: string, revision: number) {
  return request<null>({
    url: `${root}/plans/delete`,
    method: 'POST',
    data: { id, revision },
  })
}

export function retryEvaluationJudgment(id: string) {
  return request<null>({
    url: `${root}/items/retry-judgment`,
    method: 'POST',
    data: { id },
  })
}

export function getEvaluationSettings(options: RequestOptions = {}) {
  return request<EvaluationSettings>({ url: `${root}/settings`, ...options })
}

export function updateEvaluationSettings(data: { revision: number, defaults: EvaluationDefaults, typesafeApiKey?: string }) {
  return request<EvaluationSettings>({ url: `${root}/settings/update`, method: 'POST', data })
}
