<script setup lang="ts">
import type { OutboundProxyRecord } from '@/api'
import { Globe } from '@lucide/vue'
import { computed } from 'vue'
import { countryFlagUrl, countryName, proxyExitLocation } from '../utils/location'

const props = defineProps<{ proxy: OutboundProxyRecord }>()
const location = computed(() => proxyExitLocation(props.proxy))
const flag = computed(() => location.value ? countryFlagUrl(location.value.country) : undefined)
const label = computed(() => {
  if (location.value)
    return `${countryName(location.value.country)} · ${location.value.city}`
  const detection = props.proxy.lastTest?.location
  if (detection?.status === 'conflict')
    return '地区冲突'
  if (detection?.status === 'failed')
    return '解析失败'
  return '未识别'
})
const description = computed(() => {
  const detection = props.proxy.lastTest?.location
  if (detection?.status === 'failed')
    return detection.message
  if (detection?.status === 'conflict')
    return 'IPv4 与 IPv6 出口时区不一致'
  return location.value ? `${label.value} · ${location.value.timezone}` : '测试代理时解析出口地区'
})
</script>

<template>
  <span class="flex min-w-0 items-center gap-2" :class="location ? 'text-cp-text' : 'text-cp-text-tertiary'" :title="description">
    <img v-if="flag" :src="flag" alt="" aria-hidden="true" class="size-4.5 shrink-0" loading="lazy" decoding="async">
    <Globe v-else class="size-4 shrink-0 text-cp-text-tertiary" aria-hidden="true" />
    <span class="truncate">{{ label }}</span>
  </span>
</template>
