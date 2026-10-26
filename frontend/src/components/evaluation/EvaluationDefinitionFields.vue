<script setup lang="ts">
import type { EvaluationDefinition, EvaluationGrader, EvaluationJudgeDefaults } from '@/api'
import { ZButton, ZFormItem, ZInput, ZSegmented, ZSelect, ZTextarea } from '@codex-proxy/ui'
import { computed } from 'vue'
import { graderLabel, judgeGrader } from './defaults'

const props = defineProps<{ defaultJudge: EvaluationJudgeDefaults | null, disabled?: boolean }>()
const emit = defineEmits<{ configureJudge: [] }>()
const definition = defineModel<EvaluationDefinition>({ required: true })
const graderType = computed({
  get: () => definition.value.grader.type === 'typesafe'
    ? 'model'
    : definition.value.grader.type,
  set: (value: string | undefined) => {
    let grader: EvaluationGrader = { type: 'none' }
    if (value === 'exact') {
      grader = { type: 'exact', source: 'full_text', expected: '' }
    }
    else if (value === 'regex') {
      grader = { type: 'regex', pattern: '' }
    }
    else if (value === 'model') {
      grader = judgeGrader(props.defaultJudge ?? {
        type: 'model',
        clientKeyId: '',
        accountId: '',
        model: '',
        reasoningEffort: null,
        timeoutSeconds: 180,
      })
    }
    definition.value = { ...definition.value, grader }
  },
})
const aiConfig = computed(() => {
  const grader = definition.value.grader
  return grader.type === 'model' || grader.type === 'typesafe'
    ? grader.config
    : null
})
function applyDefaultJudge() {
  if (props.defaultJudge) {
    definition.value = {
      ...definition.value,
      grader: judgeGrader(props.defaultJudge, aiConfig.value?.rubric, aiConfig.value?.referenceAnswer),
    }
  }
}
</script>

<template>
  <div class="grid min-w-0 gap-5 sm:grid-cols-2">
    <ZFormItem label="测评名称" required>
      <ZInput v-model="definition.name" :disabled="disabled" maxlength="128" placeholder="例如 糖果推理题" />
    </ZFormItem>
    <ZFormItem label="输出形式">
      <ZSegmented v-model="definition.outputFormat" :disabled="disabled" :options="[{ label: '文本', value: 'text' }, { label: 'HTML', value: 'html' }]" aria-label="输出形式" />
    </ZFormItem>
    <ZFormItem class="sm:col-span-2" label="提示词" required>
      <ZTextarea v-model="definition.prompt" :disabled="disabled" :rows="7" maxlength="32768" placeholder="填写本次测评要求模型完成的任务" aria-label="测评提示词" />
    </ZFormItem>
    <ZFormItem class="sm:col-span-2" label="判分方式" :description="graderType === 'none' ? '仅保留回答供查看，不显示通过或未通过' : undefined">
      <ZSelect v-model="graderType" :disabled="disabled" :options="[{ label: '不自动判分', value: 'none' }, { label: '精确答案', value: 'exact' }, { label: '正则匹配', value: 'regex' }, { label: 'AI 判题', value: 'model' }]" />
    </ZFormItem>
    <template v-if="definition.grader.type === 'exact'">
      <ZFormItem label="匹配内容">
        <ZSelect v-model="definition.grader.source" :disabled="disabled" :options="[{ label: '完整回答', value: 'full_text' }, { label: '最终整数答案', value: 'final_integer' }]" />
      </ZFormItem>
      <ZFormItem label="预期答案" required :description="definition.grader.source === 'final_integer' ? '末行须为“最终答案：N”，且只有一个结论' : undefined">
        <ZInput v-model="definition.grader.expected" :disabled="disabled" maxlength="4096" placeholder="填写正确答案" />
      </ZFormItem>
    </template>
    <ZFormItem v-else-if="definition.grader.type === 'regex'" class="sm:col-span-2" label="正则表达式" required>
      <ZInput v-model="definition.grader.pattern" :disabled="disabled" maxlength="4096" placeholder="\A正确答案\z" class="font-mono" />
    </ZFormItem>
    <template v-else-if="aiConfig">
      <div class="flex flex-wrap items-center gap-2 text-cp-sm sm:col-span-2">
        <span>{{ graderLabel(definition.grader) }}</span>
        <ZButton v-if="defaultJudge" size="small" :disabled="disabled" @click="applyDefaultJudge">
          使用默认判题器
        </ZButton>
        <ZButton size="small" :disabled="disabled" @click="emit('configureJudge')">
          配置判题器
        </ZButton>
      </div>
      <ZFormItem class="sm:col-span-2" label="评分标准" required>
        <ZTextarea v-model="aiConfig.rubric" :disabled="disabled" :rows="4" maxlength="16384" aria-label="评分标准" placeholder="明确回答应满足的条件" />
      </ZFormItem>
      <ZFormItem class="sm:col-span-2" label="参考答案">
        <ZTextarea :model-value="aiConfig.referenceAnswer ?? ''" :disabled="disabled" :rows="3" maxlength="32768" aria-label="参考答案" placeholder="可选" @update:model-value="aiConfig.referenceAnswer = $event || null" />
      </ZFormItem>
      <p v-if="definition.outputFormat === 'html'" class="m-0 text-cp-sm text-cp-text-secondary sm:col-span-2">
        AI 判题读取 HTML 源码，不评判实际画面或动画效果
      </p>
    </template>
  </div>
</template>
