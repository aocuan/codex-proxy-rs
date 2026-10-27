<script setup lang="ts">
import type { EvaluationItem, EvaluationPlan, EvaluationRun } from '@/api'
import { ZButton, ZCard, ZConfirmDialog, ZPageHeader, ZSegmented, ZTable, ZTag } from '@codex-proxy/ui'
import { CalendarClock, Columns2, Plus, RefreshCw, Settings2 } from '@lucide/vue'
import { onMounted, shallowRef, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { retryEvaluationJudgment } from '@/api'
import EvaluationOutputModal from '@/components/evaluation/EvaluationOutputModal.vue'
import { useAsyncAction } from '@/composables/useAsyncAction'
import EvaluationComparisonModal from './components/EvaluationComparisonModal.vue'
import EvaluationDetailModal from './components/EvaluationDetailModal.vue'
import EvaluationFormModal from './components/EvaluationFormModal.vue'
import EvaluationSettingsModal from './components/EvaluationSettingsModal.vue'
import { useEvaluationCatalog } from './composables/useEvaluationCatalog'
import { useEvaluationComparison } from './composables/useEvaluationComparison'
import { useEvaluationEditor } from './composables/useEvaluationEditor'
import { useEvaluations } from './composables/useEvaluations'
import { useEvaluationSettings } from './composables/useEvaluationSettings'
import { kindLabels, planColumns, runColumns, runStatus } from './presenter'

const evaluation = useEvaluations()
const {
  config,
  runs,
  plans,
  detail,
  showDetail,
  page,
  hasNext,
  tab,
  loadingRuns,
  loadingPlans,
  loadingDetail,
  historyError,
  plansError,
  detailError,
  stoppingId,
  changingPlanId,
  changingPlan,
  stopping,
} = evaluation
const catalog = useEvaluationCatalog()
const settings = useEvaluationSettings()
const { current: settingsCurrent, lastSavedRevision } = settings
onMounted(() => {
  void catalog.load()
  void settings.load()
})
const comparison = useEvaluationComparison()
const { entries, open: showComparison, loading: loadingComparison } = comparison
const editor = useEvaluationEditor(async (run) => {
  await Promise.allSettled([
    evaluation.loadRuns(false, 1),
    evaluation.loadPlans(),
  ])
  if (run) {
    tab.value = 'history'
    evaluation.inspect(run)
  }
  else {
    tab.value = 'plans'
  }
}, config, settingsCurrent)
const route = useRoute()
const router = useRouter()
watch([
  () => route.query.accounts,
  config,
  settingsCurrent,
], ([accounts, available, defaults]) => {
  if (typeof accounts !== 'string' || !available || !defaults)
    return
  const ids = [
    ...new Set(accounts.split(',').filter(Boolean)),
  ].slice(0, 20)
  if (ids.length)
    editor.show(false, null, ids)
  const { accounts: _accounts, ...query } = route.query
  void router.replace({ query })
}, { immediate: true })
const retryAction = useAsyncAction()
const retryingItemId = shallowRef<string | null>(null)
async function retryJudgment(item: EvaluationItem) {
  retryingItemId.value = item.id
  try {
    await retryAction.run(async () => {
      await retryEvaluationJudgment(item.id)
      await evaluation.refresh()
    })
  }
  finally { retryingItemId.value = null }
}
const showOutput = shallowRef(false)
const outputItem = shallowRef<EvaluationItem | null>(null)
const outputRun = shallowRef<EvaluationRun | null>(null)
const showDelete = shallowRef(false)
const deletingPlan = shallowRef<EvaluationPlan | null>(null)

function inspectOutput(item: EvaluationItem, run: EvaluationRun) {
  outputItem.value = item
  outputRun.value = run
  showOutput.value = true
}
function requestDelete(plan: EvaluationPlan) {
  deletingPlan.value = plan
  showDelete.value = true
}
async function confirmDelete() {
  if (!deletingPlan.value)
    return
  await evaluation.changePlan(deletingPlan.value, true)
  if (!plans.value.some(plan => plan.id === deletingPlan.value?.id))
    showDelete.value = false
}
</script>

<template>
  <div class="flex min-h-0 w-full flex-col">
    <ZPageHeader title="模型测评" description="自定义题目和判分规则，观察同条件下的表现变化" />
    <ZCard class="mt-5">
      <template #header>
        <div class="flex w-full flex-wrap items-center gap-3">
          <ZSegmented v-model="tab" :options="[{ label: '测评历史', value: 'history' }, { label: '定时计划', value: 'plans' }]" aria-label="测评视图" />
          <div class="ml-auto flex flex-wrap justify-end gap-2">
            <ZButton v-if="entries.length" :loading="loadingComparison" @click="showComparison = true">
              <template #icon>
                <Columns2 class="size-4" />
              </template>
              作品对比 {{ entries.length }}/2
            </ZButton>
            <ZButton @click="settings.show">
              <template #icon>
                <Settings2 class="size-4" />
              </template>
              测评设置
            </ZButton>
            <ZButton :loading="loadingRuns || loadingPlans" @click="evaluation.refresh">
              <template #icon>
                <RefreshCw class="size-4" />
              </template>
              刷新
            </ZButton>
            <ZButton :disabled="!config || !settingsCurrent" @click="editor.show(true)">
              <template #icon>
                <CalendarClock class="size-4" />
              </template>
              新建计划
            </ZButton>
            <ZButton type="primary" :disabled="!config || !settingsCurrent" @click="editor.show(false)">
              <template #icon>
                <Plus class="size-4" />
              </template>
              新建测评
            </ZButton>
          </div>
        </div>
      </template>
      <template #body>
        <div v-if="tab === 'history'">
          <p v-if="historyError" class="px-5 text-cp-sm text-cp-error-text" role="alert">
            {{ historyError }}
          </p>
          <ZTable :columns="runColumns" :data="runs" :loading="loadingRuns" row-key="id" empty-text="还没有测评记录">
            <template #test="{ row }">
              <span class="block font-emphasis">{{ row.profile.definition?.name ?? kindLabels[row.parameters.kind] }}</span>
              <span class="text-cp-xs text-cp-text-tertiary">{{ row.parameters.model }} · {{ row.planId ? '定时' : '手动' }}</span>
            </template>
            <template #status="{ row }">
              <ZTag :type="row.activeItems ? 'info' : 'default'">
                {{ runStatus(row) }}
              </ZTag>
            </template>
            <template #progress="{ row }">
              {{ row.succeededItems }} / {{ row.totalItems }}
            </template>
            <template #result="{ row }">
              <span v-if="row.profile.definition ? row.profile.definition.grader.type !== 'none' : row.parameters.kind === 'candy'">{{ row.correctItems }} 次通过</span>
              <span v-else>查看作品</span>
              <span v-if="row.activeItems" class="mt-1 block text-cp-xs text-cp-text-tertiary">{{ row.activeItems }} 次未结束</span>
            </template>
            <template #actions="{ row }">
              <div class="flex gap-2">
                <ZButton size="small" @click="evaluation.inspect(row)">
                  详情
                </ZButton>
                <ZButton v-if="row.activeItems" size="small" type="danger" :loading="stoppingId === row.id" :disabled="row.stopRequested || stopping" @click="evaluation.stop(row)">
                  停止
                </ZButton>
              </div>
            </template>
          </ZTable>
          <div class="flex items-center justify-end gap-3 px-5 py-4">
            <span class="text-cp-sm text-cp-text-secondary">第 {{ page }} 页</span>
            <ZButton size="small" :disabled="page <= 1 || loadingRuns" @click="evaluation.loadRuns(false, page - 1)">
              上一页
            </ZButton>
            <ZButton size="small" :disabled="!hasNext || loadingRuns" @click="evaluation.loadRuns(false, page + 1)">
              下一页
            </ZButton>
          </div>
        </div>
        <div v-else>
          <p v-if="plansError" class="px-5 text-cp-sm text-cp-error-text" role="alert">
            {{ plansError }}
          </p>
          <ZTable :columns="planColumns" :data="plans" :loading="loadingPlans" row-key="id" empty-text="还没有定时计划">
            <template #name="{ row }">
              {{ row.settings.name }}
              <span class="mt-1 block text-cp-xs text-cp-text-tertiary">{{ row.settings.accountSelection?.mode === 'all' ? '自动跟随全部账号' : row.settings.accountSelection?.mode === 'groups' ? '自动跟随指定分组' : `${row.settings.parameters.accountIds.length} 个固定账号` }} · 每账号 {{ row.settings.parameters.repetitions }} 次</span>
            </template>
            <template #test="{ row }">
              {{ row.settings.parameters.definition?.name ?? kindLabels[row.settings.parameters.kind] }}
              <span class="mt-1 block text-cp-xs text-cp-text-tertiary">{{ row.settings.parameters.model }}</span>
            </template>
            <template #schedule="{ row }">
              {{ row.settings.intervalMinutes ? `每隔 ${row.settings.intervalMinutes} 分钟` : '按原定时间' }}
            </template>
            <template #status="{ row }">
              <ZTag :type="row.settings.enabled ? 'success' : 'default'">
                {{ !row.settings.enabled ? '已暂停' : row.availableAccounts === 0 ? '等待可用账号' : (row.availableAccounts ?? 0) > 20 ? '账号数超过 20，等待缩小范围' : '已启用' }}
              </ZTag>
            </template>
            <template #actions="{ row }">
              <div class="flex gap-2">
                <ZButton size="small" :disabled="changingPlan" @click="editor.show(true, row)">
                  编辑
                </ZButton>
                <ZButton size="small" :loading="changingPlanId === row.id" :disabled="changingPlan" @click="evaluation.changePlan(row)">
                  {{ row.settings.enabled ? '暂停' : '启用' }}
                </ZButton>
                <ZButton size="small" type="danger" :disabled="changingPlan" @click="requestDelete(row)">
                  删除
                </ZButton>
              </div>
            </template>
          </ZTable>
        </div>
      </template>
    </ZCard>
    <EvaluationFormModal :editor="editor" :catalog="catalog" :config="config" :settings="settingsCurrent" :settings-saved-revision="lastSavedRevision" @configure="settings.show" />
    <EvaluationSettingsModal :settings="settings" :catalog="catalog" />
    <EvaluationDetailModal
      v-model="showDetail" :detail="detail" :loading="loadingDetail" :error="detailError"
      :stopping="stopping" :retrying-item-id="retryingItemId" :account-name="catalog.accountName"
      @retry-judgment="retryJudgment" @refresh="evaluation.loadDetail()" @stop="evaluation.stop" @output="inspectOutput" @compare="comparison.add"
    />
    <EvaluationOutputModal v-model="showOutput" :item="outputItem" :run="outputRun" :account-name="catalog.accountName" />
    <EvaluationComparisonModal v-model="showComparison" :entries="entries" :account-name="catalog.accountName" @remove="comparison.remove" />
    <ZConfirmDialog
      v-model="showDelete" title="删除测评计划" :description="`删除「${deletingPlan?.settings.name ?? ''}」后不再定时执行，已创建的测评记录会保留`"
      destructive :loading="changingPlan" confirm-text="删除计划" @confirm="confirmDelete"
    />
  </div>
</template>
