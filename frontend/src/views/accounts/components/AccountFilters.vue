<script setup lang="ts">
import type { AccountGroupRef } from '@/api'
import { ZButton, ZInput, ZMenuItem, ZPopover, ZSelect } from '@codex-proxy/ui'

import { ChevronDown, Download, ListTodo, Pencil, Search, Trash2, Upload } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import ProviderFilter from '@/components/ProviderFilter.vue'
import { accountStatusFilterOptions } from '../constants'

const props = defineProps<{
  hasImportTasks: boolean
  activeImportCount: number
  selectedCount: number
  batchDeleting: boolean
  exportingAccounts: boolean
  exportDisabledReason: string
  groups: AccountGroupRef[]
  groupsLoading: boolean
}>()

const emit = defineEmits<{
  deleteSelected: []
  exportSelected: []
  importTasks: []
  create: []
  editSelected: []
}>()

const { t } = useI18n({ useScope: 'global' })

const search = defineModel<string>('search', { required: true })
const status = defineModel<string>('status', { required: true })
const provider = defineModel<string>('provider', { required: true })
const group = defineModel<string>('group', { required: true })
const groupOptions = computed(() => [
  { label: t('accounts.filters.allGroups'), value: '' },
  { label: t('accounts.filters.ungroupedAccounts'), value: 'ungrouped' },
  ...props.groups.map(group => ({
    label: group.enabled ? group.name : t('accounts.filters.disabled', { name: group.name }),
    value: group.id,
  })),
])
</script>

<template>
  <div
    class="flex w-full flex-col gap-3 xl:flex-row xl:flex-wrap xl:items-center"
    role="group"
    :aria-label="t('accounts.filters.label')"
  >
    <div
      class="grid w-full min-w-0 grid-cols-2 items-center gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_7.75rem] xl:flex xl:w-auto xl:flex-1 xl:flex-wrap xl:gap-3"
    >
      <ZInput
        v-model="search"
        :placeholder="t('accounts.filters.searchLabel')"
        class="col-span-2 min-w-0 sm:col-span-3 xl:w-48 xl:min-w-40 xl:max-w-80 xl:flex-1"
      >
        <template #prefix>
          <Search class="size-4.5 text-cp-text-tertiary" />
        </template>
      </ZInput>

      <ZSelect
        v-model="status"
        :options="accountStatusFilterOptions"
        :aria-label="t('accounts.filters.statusLabel')"
        class="w-full min-w-0 xl:w-40 xl:shrink-0"
      />

      <ZSelect
        v-model="group"
        :options="groupOptions"
        :disabled="groupsLoading"
        :aria-label="t('accounts.filters.groupLabel')"
        class="w-full min-w-0 xl:w-40 xl:shrink-0"
      />

      <ProviderFilter
        v-model="provider"
        class="col-span-2 w-full sm:col-span-1 xl:w-auto xl:shrink-0"
      />
    </div>

    <div
      class="grid w-full grid-cols-2 gap-2 xl:flex xl:w-auto xl:flex-wrap xl:shrink-0 xl:self-end xl:items-center xl:justify-end xl:ml-auto"
    >
      <ZPopover v-if="selectedCount > 0" placement="bottom-end" class="col-span-2 justify-self-end">
        <template #reference="{ open }">
          <ZButton class="whitespace-nowrap" :aria-expanded="open">
            {{ t('accounts.filters.batchActions', { count: selectedCount }) }}
            <ChevronDown class="size-3.5" />
          </ZButton>
        </template>
        <template #default="{ close }">
          <div class="w-44 p-1.5">
            <ZMenuItem @click="(close(), emit('editSelected'))">
              <template #icon>
                <Pencil class="size-3.5 text-cp-link" />
              </template>
              {{ t('accounts.batchEdit.title') }}
            </ZMenuItem>
            <ZMenuItem
              :loading="exportingAccounts"
              :disabled="Boolean(exportDisabledReason) || exportingAccounts"
              :title="exportDisabledReason || undefined"
              :aria-description="exportDisabledReason || undefined"
              @click="(close(), emit('exportSelected'))"
            >
              <template #icon>
                <Download class="size-3.5" />
              </template>
              {{ t('accounts.filters.exportSelected', { count: selectedCount }) }}
            </ZMenuItem>
            <ZMenuItem type="danger" :disabled="batchDeleting" @click="(close(), emit('deleteSelected'))">
              <template #icon>
                <Trash2 class="size-3.5" />
              </template>
              {{ t('accounts.filters.deleteSelected', { count: selectedCount }) }}
            </ZMenuItem>
          </div>
        </template>
      </ZPopover>
      <div
        class="col-span-2 flex min-w-0 flex-wrap items-center justify-end gap-2"
        :class="selectedCount > 0 ? 'w-full xl:w-auto' : 'justify-self-end'"
      >
        <ZButton v-if="hasImportTasks" class="whitespace-nowrap" @click="emit('importTasks')">
          <ListTodo class="size-4" />
          {{ t('accounts.filters.importTasks') }} <span v-if="activeImportCount" class="font-mono text-cp-link">{{ activeImportCount }}</span>
        </ZButton>
        <slot name="actions" />
        <ZButton
          type="primary"
          class="whitespace-nowrap"
          :class="selectedCount > 0 ? 'flex-1 xl:flex-none' : undefined"
          @click="emit('create')"
        >
          <Upload class="size-4" />
          {{ t('accounts.filters.importAccount') }}
        </ZButton>
      </div>
    </div>
  </div>
</template>
