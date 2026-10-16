<script setup lang="ts">
import type { ProxyBatchItem } from '../utils/batch'
import { ZPagination, ZScrollbar } from '@codex-proxy/ui'
import { computed, shallowRef, watch } from 'vue'
import { proxyBatchStates } from '../utils/batch'
import ProxyRegion from './ProxyRegion.vue'

const props = defineProps<{ items: ProxyBatchItem[], tests?: ProxyBatchItem[], testing?: boolean }>()
const page = shallowRef(1)
const pageSize = 20
const testItems = computed(() => props.testing ? props.items : props.tests ?? [])
const rows = computed(() => {
  const tests = new Map(testItems.value.map(item => [item.key, item]))
  return props.items.map((item) => {
    const test = props.testing ? item : item.record ? tests.get(item.record.id) : undefined
    const detection = test?.record?.lastTest?.location
    return {
      item,
      test,
      result: test ?? item,
      message: test && test.status !== 'succeeded'
        ? test.message
        : detection?.status === 'failed'
          ? detection.message
          : item.status !== 'succeeded' ? item.message : '',
    }
  })
})
const visible = computed(() => props.testing ? rows.value.slice((page.value - 1) * pageSize, page.value * pageSize) : rows.value)
const completed = computed(() => props.items.filter(item => !['pending', 'running', 'skipped'].includes(item.status)).length)
const created = computed(() => props.items.filter(item => item.status === 'succeeded').length)
const invalid = computed(() => props.items.filter(item => item.status === 'failed').length)
const succeeded = computed(() => testItems.value.filter(item => item.status === 'succeeded').length)
const failed = computed(() => testItems.value.filter(item => item.status === 'failed').length)
const allOperations = computed(() => props.testing ? props.items : [...props.items, ...testItems.value])
const attention = computed(() => allOperations.value.filter(item => ['error', 'conflict', 'busy', 'unknown'].includes(item.status)).length)
const skipped = computed(() => allOperations.value.filter(item => item.status === 'skipped').length)
const locationPending = computed(() => testItems.value.filter(item => item.status === 'succeeded' && item.record?.lastTest?.location.status !== 'detected').length)
watch(() => props.items, () => {
  page.value = 1
})
</script>

<template>
  <section class="min-w-0 space-y-3" :aria-label="testing ? '批量测试结果' : '批量添加结果'">
    <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-cp-xs text-cp-text-secondary" role="status" aria-live="polite">
      <span>已处理 <strong class="font-mono font-medium text-cp-text">{{ completed }} / {{ items.length }}</strong></span>
      <template v-if="!testing">
        <span>已添加 {{ created }}</span>
        <span v-if="invalid">格式错误 {{ invalid }}</span>
      </template>
      <template v-if="testItems.length">
        <span>连接正常 {{ succeeded }}</span>
        <span>连接失败 {{ failed }}</span>
        <span v-if="locationPending">地区待处理 {{ locationPending }}</span>
      </template>
      <span v-if="attention">需处理 {{ attention }}</span>
      <span v-if="skipped">未执行 {{ skipped }}</span>
    </div>
    <ZScrollbar max-height="min(26rem, 50dvh)" always-visible role="region" aria-label="代理结果滚动列表">
      <ol class="m-0 list-none space-y-1 p-0">
        <li v-for="row in visible" :key="row.item.key" class="grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-1 rounded-cp-sm px-3 py-2.5 odd:bg-cp-fill-quaternary">
          <span class="min-w-0 truncate font-mono text-cp-xs text-cp-text" :title="row.item.endpoint">{{ row.item.endpoint || `第 ${row.item.key} 行` }}</span>
          <span class="flex flex-wrap justify-end gap-x-2 text-cp-xs" :class="proxyBatchStates[row.result.status].tone">
            <span v-if="!testing && row.test" class="text-cp-text-secondary">已添加</span>
            <span>{{ row.result.status === 'succeeded' ? row.test ? '连接正常' : '已添加' : proxyBatchStates[row.result.status].label }}</span>
          </span>
          <div class="col-span-2 flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-cp-xs text-cp-text-secondary">
            <span class="max-w-full truncate" :title="row.item.name">{{ row.item.name }}</span>
            <ProxyRegion v-if="row.test?.record" :proxy="row.test.record" class="max-w-full" />
            <span v-if="row.test?.record?.lastTest" class="min-w-0 break-all font-mono">
              {{ row.test.record.lastTest.latencyMs }} ms<span v-if="row.test.record.lastTest.exitIpv4"> · IPv4 {{ row.test.record.lastTest.exitIpv4 }}</span><span v-if="row.test.record.lastTest.exitIpv6"> · IPv6 {{ row.test.record.lastTest.exitIpv6 }}</span>
            </span>
          </div>
          <span v-if="row.message" class="col-span-2 text-cp-xs leading-relaxed wrap-anywhere" :class="row.result.status === 'succeeded' ? 'text-cp-warning-text' : proxyBatchStates[row.result.status].tone">{{ row.message }}</span>
        </li>
      </ol>
    </ZScrollbar>
    <ZPagination v-if="testing && items.length > pageSize" :current-page="page" :page-size="pageSize" :total="items.length" :page-sizes="[pageSize]" @current-change="page = $event" />
  </section>
</template>
