<script setup lang="ts">
import type { ProxyBatchInput } from '../utils/batch'
import { ZForm, ZFormItem, ZInput, ZSelect, ZTextarea } from '@codex-proxy/ui'
import { computed } from 'vue'
import { proxyProtocolOptions } from '../utils/batch'

const props = defineProps<{ rows: ProxyBatchInput[], error: string, disabled: boolean }>()
const text = defineModel<string>({ required: true })
const prefix = defineModel<string>('prefix', { required: true })
const protocol = defineModel<string>('protocol', { required: true })
const invalid = computed(() => props.rows.filter(row => row.error))
</script>

<template>
  <ZForm class="grid gap-5">
    <div class="grid gap-4 sm:grid-cols-2">
      <ZFormItem label="名称前缀" description="按输入顺序生成名称">
        <ZInput v-model="prefix" maxlength="96" :disabled="disabled" aria-label="批量代理名称前缀" placeholder="代理" />
      </ZFormItem>
      <ZFormItem label="无协议行使用" description="完整 URL 使用各自协议">
        <ZSelect v-model="protocol" :options="proxyProtocolOptions" :disabled="disabled" aria-label="无协议代理的协议" />
      </ZFormItem>
    </div>
    <ZFormItem label="代理列表" required :error="error || undefined" description="每行一条，最多 100 条，简写支持 host:port:用户名:密码">
      <ZTextarea v-model="text" :rows="8" :disabled="disabled" aria-label="批量代理列表" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="http://user:password@proxy.example.com:8080&#10;proxy.example.net:1080:user:password" />
    </ZFormItem>
    <p v-if="rows.length && !error" class="m-0 text-cp-xs text-cp-text-secondary" role="status">
      共 {{ rows.length }} 条<span v-if="invalid.length">，{{ invalid.length }} 条格式错误或重复，其余条目可继续添加</span>
    </p>
    <ul v-if="invalid.length && !error" class="m-0 list-none space-y-1 p-0 text-cp-xs text-cp-error-text" role="alert">
      <li v-for="row in invalid.slice(0, 5)" :key="row.line">
        第 {{ row.line }} 行：{{ row.error }}
      </li>
      <li v-if="invalid.length > 5">
        另有 {{ invalid.length - 5 }} 条需处理
      </li>
    </ul>
  </ZForm>
</template>
