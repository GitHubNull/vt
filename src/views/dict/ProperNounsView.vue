<script setup>
/**
 * 词典 › 专有名词（T-522，F-DICT-02、PRD 3.4.2；T-523 导入导出）。
 * 标准写法 + 读音别名（多值，逐行编辑）+ 备注；行内启停与删除；
 * 搜索框带清除按钮（沿用词典页约定）；删除确认走 plugin-dialog；
 * 导出经系统保存对话框写统一格式（v2）文件；导入走导入向导（T-523）。
 */
import { computed, onMounted, ref } from 'vue'
import Button from 'primevue/button'
import Column from 'primevue/column'
import DataTable from 'primevue/datatable'
import Dialog from 'primevue/dialog'
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import Textarea from 'primevue/textarea'
import ToggleSwitch from 'primevue/toggleswitch'
import { confirm, save } from '@tauri-apps/plugin-dialog'
import { properNounExportUnified } from '../../api/properNouns'
import { useDictStore, MAX_NOUNS } from '../../stores/dict'
import { useConfigStore } from '../../stores/config'
import { toAppError } from '../../api/client'
import { t } from '../../i18n'
import { i18nText } from './importWizard'
import ImportWizard from './ImportWizard.vue'

const store = useDictStore()
const configStore = useConfigStore()

/**
 * 本页导出 / 导入文案：i18n key 与取值列入 T-523 报告片段；
 * 合并前命中 `getMessage` 兜底（见 importWizard.ts 说明）。
 */
const nounsIo = {
  export: i18nText('dict.properNouns.export', '导出'),
  import: i18nText('dict.properNouns.import', '导入'),
  exporting: i18nText('dict.properNouns.exporting', '导出中…'),
  exportTitle: i18nText('dict.properNouns.exportTitle', '导出专有名词'),
  exportDone: i18nText('dict.properNouns.exportDone', '已导出 {count} 条专有名词到 {path}。'),
  exportFailed: i18nText('dict.properNouns.exportFailed', '导出失败'),
}

const search = ref('')
/** 编辑对话框状态（null = 关闭；aliasesText 为逐行文本形态）。 */
const editor = ref(null)
const editorVisible = computed({
  get: () => editor.value !== null,
  set: (visible) => {
    if (!visible) editor.value = null
  },
})

/** 导出状态与提示（{ ok, text }）；导入向导可见性。 */
const exporting = ref(false)
const ioNotice = ref(null)
const wizardVisible = ref(false)

onMounted(() => {
  void store.ensureNounsLoaded()
  void configStore.ensureLoaded()
})

/** 导出：系统原生保存对话框选定路径 → 后端写统一格式（v2）文件；取消则静默返回。 */
async function onExport() {
  exporting.value = true
  ioNotice.value = null
  try {
    const path = await save({
      title: nounsIo.exportTitle,
      defaultPath: 'vibetype-proper-nouns.json',
      filters: [{ name: 'JSON', extensions: ['json'] }],
    })
    if (!path) return
    const count = await properNounExportUnified(path)
    ioNotice.value = {
      ok: true,
      text: nounsIo.exportDone.replace('{count}', String(count)).replace('{path}', path),
    }
  } catch (error) {
    const err = toAppError(error)
    ioNotice.value = {
      ok: false,
      text: `${nounsIo.exportFailed}：${err.message}（${t.common.errorCode} ${err.code}）`,
    }
  } finally {
    exporting.value = false
  }
}

const filteredNouns = computed(() => {
  const query = search.value.trim().toLowerCase()
  if (!query) return store.nouns
  return store.nouns.filter(
    (noun) =>
      noun.term.toLowerCase().includes(query) ||
      noun.aliases.some((alias) => alias.toLowerCase().includes(query)) ||
      noun.note.toLowerCase().includes(query),
  )
})

const counterText = computed(() => `${store.nouns.length} / ${MAX_NOUNS}`)

const errorText = computed(() =>
  store.lastError
    ? `${store.lastError.message}（${t.common.errorCode} ${store.lastError.code}）`
    : '',
)

function openAdd() {
  editor.value = { id: null, term: '', aliasesText: '', note: '', enabled: true }
}

function openEdit(noun) {
  editor.value = {
    id: noun.id,
    term: noun.term,
    aliasesText: noun.aliases.join('\n'),
    note: noun.note,
    enabled: noun.enabled,
  }
}

/** 别名文本 → 数组：逐行去空白、去空行、去重（保持顺序）。 */
function parseAliases(text) {
  const seen = new Set()
  const result = []
  for (const line of String(text).split('\n')) {
    const alias = line.trim()
    if (!alias || seen.has(alias)) continue
    seen.add(alias)
    result.push(alias)
  }
  return result
}

async function saveEditor() {
  const current = editor.value
  if (!current) return
  const aliases = parseAliases(current.aliasesText)
  const ok =
    current.id === null
      ? await store.addNoun(current.term.trim(), aliases, current.note.trim())
      : await store.updateNoun({
          id: current.id,
          term: current.term.trim(),
          aliases,
          note: current.note.trim(),
          enabled: current.enabled,
        })
  if (ok) editor.value = null
}

async function onToggle(noun, enabled) {
  await store.toggleNoun(noun.id, enabled)
}

async function onDelete(noun) {
  const ok = await confirm(
    t.dict.properNouns.deleteConfirm.replace('{term}', noun.term),
    { title: t.dict.properNouns.deleteTitle, kind: 'warning' },
  )
  if (ok) await store.removeNoun(noun.id)
}

function updateProperNounsEnabled(enabled) {
  void configStore.updateSection('dict', {
    ...configStore.sections.dict,
    proper_nouns: { enabled },
  })
}
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.dict.properNouns.title }}</h2>
    <p class="settings-page-desc">{{ t.dict.properNouns.desc }}</p>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.dict.properNouns.group }}</h3>
      <div class="settings-card">
        <div class="dict-toolbar">
          <span class="dict-counter">{{ counterText }}</span>
          <div class="dict-search-wrap">
            <InputText
              v-model="search"
              class="dict-search"
              :placeholder="t.dict.properNouns.searchPlaceholder"
              :aria-label="t.dict.properNouns.searchPlaceholder"
            />
            <button
              v-if="search"
              type="button"
              class="dict-search-clear"
              :aria-label="t.common.clear"
              :title="t.common.clear"
              @click="search = ''"
            >
              <i class="pi pi-times" aria-hidden="true" />
            </button>
          </div>
          <Button
            type="button"
            icon="pi pi-plus"
            :label="t.dict.properNouns.add"
            :disabled="store.nounsFull"
            @click="openAdd"
          />
          <Button
            type="button"
            icon="pi pi-download"
            :label="exporting ? nounsIo.exporting : nounsIo.export"
            :loading="exporting"
            severity="secondary"
            @click="onExport"
          />
          <Button
            type="button"
            icon="pi pi-upload"
            :label="nounsIo.import"
            severity="secondary"
            @click="wizardVisible = true"
          />
          <ToggleSwitch
            :model-value="configStore.sections.dict.proper_nouns.enabled"
            :aria-label="t.dict.properNouns.masterToggle"
            @update:model-value="updateProperNounsEnabled"
          />
          <span class="dict-master-label">{{ t.dict.properNouns.masterToggle }}</span>
        </div>
        <p v-if="store.nounsFull" class="dict-hint">{{ t.dict.properNouns.limitHint }}</p>
        <p v-if="!configStore.sections.dict.proper_nouns.enabled" class="dict-hint">
          {{ t.dict.properNouns.disabledHint }}
        </p>
        <p v-if="ioNotice" class="dict-hint" :class="{ 'dict-hint-error': !ioNotice.ok }">
          {{ ioNotice.text }}
        </p>

        <DataTable
          :value="filteredNouns"
          data-key="id"
          class="dict-table"
          :empty-message="t.dict.properNouns.empty"
        >
          <Column field="term" :header="t.dict.properNouns.colTerm" class="pn-col-term">
            <template #body="{ data }">
              <span>{{ data.term }}</span>
            </template>
          </Column>
          <Column :header="t.dict.properNouns.colAliases" class="pn-col-aliases">
            <template #body="{ data }">
              <span v-if="data.aliases.length" class="pn-aliases">
                {{ data.aliases.join(t.dict.properNouns.aliasSeparator) }}
              </span>
              <em v-else class="dict-delete-mark">{{ t.dict.properNouns.emptyAliases }}</em>
            </template>
          </Column>
          <Column field="note" :header="t.dict.properNouns.colNote" class="pn-col-note">
            <template #body="{ data }">
              <span v-if="data.note">{{ data.note }}</span>
              <em v-else class="dict-delete-mark">—</em>
            </template>
          </Column>
          <Column :header="t.dict.properNouns.colEnabled" class="dict-col-enabled">
            <template #body="{ data }">
              <ToggleSwitch
                :model-value="data.enabled"
                :aria-label="`${t.dict.properNouns.colEnabled}: ${data.term}`"
                @update:model-value="(v) => onToggle(data, v)"
              />
            </template>
          </Column>
          <Column :header="t.dict.properNouns.colActions" class="dict-col-actions">
            <template #body="{ data }">
              <Button
                type="button"
                icon="pi pi-pencil"
                severity="secondary"
                size="small"
                text
                :aria-label="t.dict.properNouns.edit"
                @click="openEdit(data)"
              />
              <Button
                type="button"
                icon="pi pi-times"
                severity="danger"
                size="small"
                text
                :aria-label="t.dict.properNouns.delete"
                @click="onDelete(data)"
              />
            </template>
          </Column>
        </DataTable>
      </div>
    </section>

    <Message v-if="store.lastError" severity="error" class="settings-error">{{
      errorText
    }}</Message>

    <ImportWizard v-model:visible="wizardVisible" kind="proper_nouns" />

    <Dialog
      v-model:visible="editorVisible"
      :header="editor?.id === null ? t.dict.properNouns.addTitle : t.dict.properNouns.editTitle"
      modal
      class="pn-editor-dialog"
    >
      <div v-if="editor" class="pn-editor">
        <label class="pn-editor-field">
          <span>{{ t.dict.properNouns.colTerm }}</span>
          <InputText v-model="editor.term" :placeholder="t.dict.properNouns.termPlaceholder" />
        </label>
        <label class="pn-editor-field">
          <span>{{ t.dict.properNouns.aliasesLabel }}</span>
          <Textarea
            v-model="editor.aliasesText"
            rows="5"
            :placeholder="t.dict.properNouns.aliasesPlaceholder"
          />
        </label>
        <label class="pn-editor-field">
          <span>{{ t.dict.properNouns.colNote }}</span>
          <InputText v-model="editor.note" :placeholder="t.dict.properNouns.notePlaceholder" />
        </label>
        <p class="pn-editor-hint">{{ t.dict.properNouns.aliasesHint }}</p>
      </div>
      <template #footer>
        <Button
          type="button"
          severity="secondary"
          :label="t.common.cancel"
          @click="editor = null"
        />
        <Button
          type="button"
          :label="t.common.confirm"
          :disabled="!editor?.term.trim()"
          @click="saveEditor"
        />
      </template>
    </Dialog>
  </div>
</template>

<style scoped>
.dict-toolbar {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
  padding: var(--vt-spacing-md);
  border-bottom: 1px solid var(--vt-color-border);
  flex-wrap: wrap;
}

.dict-counter {
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
  min-width: 72px;
}

.dict-search-wrap {
  position: relative;
  flex: 1;
  min-width: 160px;
}

.dict-search {
  width: 100%;
  padding-right: 30px;
}

.dict-search-clear {
  position: absolute;
  top: 50%;
  right: 8px;
  transform: translateY(-50%);
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--vt-color-text-muted);
  font-size: 12px;
  cursor: pointer;
}

.dict-master-label {
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.dict-hint {
  margin: 0;
  padding: 8px var(--vt-spacing-md);
  font-size: 12.5px;
  color: var(--vt-color-warning);
  border-bottom: 1px solid var(--vt-color-border);
}

.dict-hint-error {
  color: var(--vt-color-error);
}

.dict-delete-mark {
  color: var(--vt-color-text-muted);
  font-style: normal;
}

.pn-aliases {
  color: var(--vt-color-text);
}

.pn-col-term {
  min-width: 140px;
}

.pn-col-aliases {
  min-width: 160px;
}

.pn-col-note {
  min-width: 120px;
}

.dict-col-enabled {
  width: 72px;
}

.dict-col-actions {
  width: 96px;
}

.pn-editor {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-md);
  min-width: 380px;
}

.pn-editor-field {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-xs);
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.pn-editor-hint {
  margin: 0;
  font-size: 12px;
  color: var(--vt-color-text-muted);
  white-space: pre-line;
}
</style>
