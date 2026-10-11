<script setup>
/**
 * 词典 › 替换词典（T-011，F-DICT-01、PRD 3.4.1）。
 * 对照原型 PixPin_2026-10-09_21-32-16.png：说明文案、计数、添加 / 保存、
 * 错误词 → 正确词列表、行内启停与删除；搜索框为原型新增项。
 * 删除确认走 @tauri-apps/plugin-dialog（tech/03 3.3）；
 * 导入 / 导出文件路径由系统原生对话框选定（plugin-dialog save / open）。
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
import { confirm, open, save } from '@tauri-apps/plugin-dialog'
import { useDictStore, MAX_RULES } from '../../stores/dict'
import { useConfigStore } from '../../stores/config'
import { toAppError } from '../../api/client'
import { t } from '../../i18n'

const store = useDictStore()
const configStore = useConfigStore()

const search = ref('')
/** 编辑对话框状态（null = 关闭）。 */
const editor = ref(null)
/** 对话框可见性（v-model:visible 需要可写 computed）。 */
const editorVisible = computed({
  get: () => editor.value !== null,
  set: (visible) => {
    if (!visible) editor.value = null
  },
})

onMounted(() => {
  void store.ensureLoaded()
  void configStore.ensureLoaded()
})

const filteredRules = computed(() => {
  const query = search.value.trim().toLowerCase()
  if (!query) return store.rules
  return store.rules.filter(
    (rule) => rule.wrong.toLowerCase().includes(query) || rule.right.toLowerCase().includes(query),
  )
})

const counterText = computed(() => `${store.rules.length} / ${MAX_RULES}`)

/** 导出 / 导入状态与提示（{ ok, text }）。 */
const exporting = ref(false)
const importing = ref(false)
const ioNotice = ref(null)

/** 导出：系统原生保存对话框选定路径 → 后端写文件；取消对话框则静默返回。 */
async function onExport() {
  exporting.value = true
  ioNotice.value = null
  try {
    const path = await save({
      title: t.dict.replace.exportTitle,
      defaultPath: 'vibetype-dict.json',
      filters: [{ name: 'JSON', extensions: ['json'] }],
    })
    if (!path) return
    const count = await store.exportRulesTo(path)
    if (count === null) {
      ioNotice.value = { ok: false, text: ioErrorText('export') }
      return
    }
    ioNotice.value = {
      ok: true,
      text: t.dict.replace.exportDone
        .replace('{count}', String(count))
        .replace('{path}', path),
    }
  } catch (error) {
    ioNotice.value = {
      ok: false,
      text: `${t.dict.replace.exportFailed}（${t.common.errorCode} ${toAppError(error).code}）`,
    }
  } finally {
    exporting.value = false
  }
}

/** 导入：系统原生打开对话框选定 JSON → 后端校验并导入；取消对话框则静默返回。 */
async function onImport() {
  importing.value = true
  ioNotice.value = null
  try {
    const path = await open({
      title: t.dict.replace.importTitle,
      filters: [{ name: 'JSON', extensions: ['json'] }],
    })
    if (!path) return
    const count = await store.importRulesFrom(path)
    if (count === null) {
      // JSON 结构非法等失败：展示后端返回的错误信息。
      ioNotice.value = { ok: false, text: ioErrorText('import') }
      return
    }
    ioNotice.value = { ok: true, text: t.dict.replace.importDone.replace('{count}', String(count)) }
  } catch (error) {
    ioNotice.value = {
      ok: false,
      text: `${t.dict.replace.importFailed}（${t.common.errorCode} ${toAppError(error).code}）`,
    }
  } finally {
    importing.value = false
  }
}

/** 失败提示：优先取 store 记录的后端错误信息（含结构非法），无则给通用文案。 */
function ioErrorText(kind) {
  const generic = kind === 'export' ? t.dict.replace.exportFailed : t.dict.replace.importFailed
  const err = store.lastError
  return err ? `${generic}：${err.message}（${t.common.errorCode} ${err.code}）` : generic
}

const errorText = computed(() =>
  store.lastError
    ? `${store.lastError.message}（${t.common.errorCode} ${store.lastError.code}）`
    : '',
)

function openAdd() {
  editor.value = { id: null, wrong: '', right: '', enabled: true }
}

function openEdit(rule) {
  editor.value = { ...rule }
}

async function saveEditor() {
  const current = editor.value
  if (!current) return
  const ok =
    current.id === null
      ? await store.addRule(current.wrong.trim(), current.right)
      : await store.updateRule({
          id: current.id,
          wrong: current.wrong.trim(),
          right: current.right,
          enabled: current.enabled,
        })
  if (ok) editor.value = null
}

async function onToggle(rule, enabled) {
  await store.toggleRule(rule.id, enabled)
}

async function onDelete(rule) {
  const ok = await confirm(
    t.dict.replace.deleteConfirm.replace('{wrong}', rule.wrong || t.dict.replace.emptyWrong),
    { title: t.dict.replace.deleteTitle, kind: 'warning' },
  )
  if (ok) await store.removeRule(rule.id)
}

function updateDictEnabled(enabled) {
  void configStore.updateSection('dict', { ...configStore.sections.dict, enabled })
}
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.dict.replace.title }}</h2>
    <p class="settings-page-desc">{{ t.dict.replace.desc }}</p>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.dict.replace.group }}</h3>
      <div class="settings-card">
        <div class="dict-toolbar">
          <span class="dict-counter">{{ counterText }}</span>
          <InputText
            v-model="search"
            class="dict-search"
            :placeholder="t.dict.replace.searchPlaceholder"
            :aria-label="t.dict.replace.searchPlaceholder"
          />
          <Button
            type="button"
            icon="pi pi-plus"
            :label="t.dict.replace.add"
            :disabled="store.isFull"
            @click="openAdd"
          />
          <Button
            type="button"
            icon="pi pi-upload"
            :label="exporting ? t.dict.replace.exporting : t.dict.replace.export"
            :loading="exporting"
            severity="secondary"
            @click="onExport"
          />
          <Button
            type="button"
            icon="pi pi-download"
            :label="importing ? t.dict.replace.importing : t.dict.replace.import"
            :loading="importing"
            severity="secondary"
            @click="onImport"
          />
          <ToggleSwitch
            :model-value="configStore.sections.dict.enabled"
            :aria-label="t.dict.replace.masterToggle"
            @update:model-value="updateDictEnabled"
          />
          <span class="dict-master-label">{{ t.dict.replace.masterToggle }}</span>
        </div>
        <p v-if="store.isFull" class="dict-hint">{{ t.dict.replace.limitHint }}</p>
        <p v-if="!configStore.sections.dict.enabled" class="dict-hint">
          {{ t.dict.replace.disabledHint }}
        </p>
        <p v-if="ioNotice" class="dict-hint" :class="{ 'dict-hint-error': !ioNotice.ok }">
          {{ ioNotice.text }}
        </p>

        <DataTable
          :value="filteredRules"
          data-key="id"
          class="dict-table"
          :empty-message="t.dict.replace.empty"
        >
          <Column field="wrong" :header="t.dict.replace.colWrong" class="dict-col-wrong">
            <template #body="{ data }">
              <span>{{ data.wrong || t.dict.replace.emptyWrong }}</span>
            </template>
          </Column>
          <Column :header="t.dict.replace.colArrow" class="dict-col-arrow">
            <template #body>
              <i class="pi pi-arrow-right dict-arrow" aria-hidden="true" />
            </template>
          </Column>
          <Column field="right" :header="t.dict.replace.colRight" class="dict-col-right">
            <template #body="{ data }">
              <span v-if="data.right">{{ data.right }}</span>
              <em v-else class="dict-delete-mark">（{{ t.dict.replace.deleteMark }}）</em>
            </template>
          </Column>
          <Column :header="t.dict.replace.colEnabled" class="dict-col-enabled">
            <template #body="{ data }">
              <ToggleSwitch
                :model-value="data.enabled"
                :aria-label="`${t.dict.replace.colEnabled}: ${data.wrong}`"
                @update:model-value="(v) => onToggle(data, v)"
              />
            </template>
          </Column>
          <Column :header="t.dict.replace.colActions" class="dict-col-actions">
            <template #body="{ data }">
              <Button
                type="button"
                icon="pi pi-pencil"
                severity="secondary"
                size="small"
                text
                :aria-label="t.dict.replace.edit"
                @click="openEdit(data)"
              />
              <Button
                type="button"
                icon="pi pi-times"
                severity="danger"
                size="small"
                text
                :aria-label="t.dict.replace.delete"
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

    <Dialog
      v-model:visible="editorVisible"
      :header="editor?.id === null ? t.dict.replace.addTitle : t.dict.replace.editTitle"
      modal
      class="dict-editor-dialog"
    >
      <div v-if="editor" class="dict-editor">
        <label class="dict-editor-field">
          <span>{{ t.dict.replace.colWrong }}</span>
          <InputText v-model="editor.wrong" :placeholder="t.dict.replace.wrongPlaceholder" />
        </label>
        <label class="dict-editor-field">
          <span>{{ t.dict.replace.colRight }}</span>
          <Textarea
            v-model="editor.right"
            rows="4"
            :placeholder="t.dict.replace.rightPlaceholder"
          />
        </label>
        <p class="dict-editor-hint">{{ t.dict.replace.emptyRightHint }}</p>
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
          :disabled="!editor?.wrong.trim()"
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
  min-width: 52px;
}

.dict-search {
  flex: 1;
  min-width: 160px;
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

.dict-arrow {
  color: var(--vt-color-text-muted);
}

.dict-delete-mark {
  color: var(--vt-color-text-muted);
  font-style: normal;
}

.dict-col-arrow {
  width: 48px;
  text-align: center;
}

.dict-col-enabled {
  width: 72px;
}

.dict-col-actions {
  width: 96px;
}

.dict-editor {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-md);
  min-width: 360px;
}

.dict-editor-field {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-xs);
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.dict-editor-hint {
  margin: 0;
  font-size: 12px;
  color: var(--vt-color-text-muted);
}
</style>
