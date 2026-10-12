<script setup>
/**
 * 编程 › 场景规则（T-502，F-CODE-01、PRD 3.2.1）。
 * 场景卡片列表（样式对齐 AI 供应商卡片：可折叠、右上"添加"）：展开编辑名称、
 * 图标颜色、匹配应用 + 标题关键字、输出规则；内置场景可编辑不可删除、可恢复默认；
 * 列表顶部"当前识别到：×× → ××场景" + 锁定开关（PRD 3.2.1 业务规则 3）；
 * 拖动卡片头部调整顺序（排序即同级规则的匹配优先级）。
 *
 * 文案：coding.scenes.*（i18n 文件由集成方合并；tx() 在键缺失时回退键路径，不崩溃）。
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import Button from 'primevue/button'
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import Select from 'primevue/select'
import ToggleSwitch from 'primevue/toggleswitch'
import { confirm } from '@tauri-apps/plugin-dialog'
import { listen } from '@tauri-apps/api/event'
import {
  onSceneChanged,
  sceneAddEntry,
  sceneClearOverride,
  sceneDeleteEntry,
  sceneGetCurrent,
  sceneListEntries,
  sceneReorderEntries,
  sceneRestoreEntryDefault,
  sceneSetOverride,
  sceneUpdateEntry,
} from '../../api/scene'
import { toAppError } from '../../api/client'
import { EVENTS } from '../../api/events'
import { getMessage, t } from '../../i18n'
import {
  DEFAULT_ICON_COLOR,
  MAX_APP_LEN,
  MAX_ENTRY_APPS,
  MAX_ENTRY_NAME_LEN,
  MAX_SCENE_ENTRIES,
  MAX_TITLE_KEYWORD_LEN,
  SCENE_KIND_OPTIONS,
  addApp,
  isBuiltinEntry,
  moveEntry,
  newEntryDraft,
  normalizeEntry,
  removeApp,
  validateEntryDraft,
} from './scenes'

/** 场景页文案：`coding.scenes.*`；集成前键缺失时回退键路径本身（不崩溃）。 */
function tx(key) {
  return getMessage(`coding.scenes.${key}`)
}

/** 场景展示名（归属场景 / 当前识别到）。 */
function sceneName(kind) {
  return tx(`sceneNames.${kind}`)
}

/** 新建草稿在卡片列表中的合成 ID（不与后端条目冲突）。 */
const NEW_ENTRY_ID = '__new__'

const entries = ref([])
const status = ref(null)
const loading = ref(false)
const lastError = ref(null)
/** 表单前置校验提示（后端错误走 lastError）。 */
const formError = ref('')

/** 展开编辑中的条目 ID（null = 全部折叠；NEW_ENTRY_ID = 正在新建）。 */
const expandedId = ref(null)
/** 编辑草稿（保存时整体提交；取消即丢弃）。 */
const draft = ref(null)

const creating = computed(() => expandedId.value === NEW_ENTRY_ID)
const locked = computed(() => status.value?.source === 'override')
const atLimit = computed(() => entries.value.length >= MAX_SCENE_ENTRIES)

/** 卡片渲染序列：已有条目 +（新建时）草稿合成卡片。 */
const cards = computed(() => {
  if (creating.value && draft.value) {
    return [...entries.value, { ...draft.value, id: NEW_ENTRY_ID }]
  }
  return entries.value
})

const draftBuiltin = computed(() => (draft.value ? isBuiltinEntry(draft.value) : false))

/** 锁定目标（默认跟随当前识别场景；用户手动选择后不再自动同步）。 */
const lockTarget = ref('ide')
const lockTargetPicked = ref(false)

let unlistenScene = null
let unlistenConfig = null

onMounted(async () => {
  await Promise.all([loadEntries(), loadStatus()])
  try {
    unlistenScene = await onSceneChanged(() => {
      void loadStatus()
    })
    unlistenConfig = await listen(EVENTS.configChanged, () => {
      void loadEntries()
      void loadStatus()
    })
  } catch {
    // Webview 环境外（测试）不支持 listen：忽略
  }
})

onUnmounted(() => {
  if (unlistenScene) unlistenScene()
  if (unlistenConfig) unlistenConfig()
})

async function loadEntries() {
  loading.value = true
  try {
    entries.value = await sceneListEntries()
    lastError.value = null
  } catch (error) {
    lastError.value = toAppError(error)
  } finally {
    loading.value = false
  }
}

async function loadStatus() {
  try {
    status.value = await sceneGetCurrent()
    if (!locked.value && !lockTargetPicked.value && status.value.scene !== 'unknown') {
      lockTarget.value = status.value.scene
    }
  } catch {
    // 状态查询失败不阻塞页面；场景 / 配置事件到达时重试。
    status.value = null
  }
}

const sceneKindOptions = computed(() =>
  SCENE_KIND_OPTIONS.map((kind) => ({ label: sceneName(kind), value: kind })),
)

const punctOptions = computed(() => [
  { label: tx('punctOptions.follow'), value: 'follow' },
  { label: tx('punctOptions.halfWidth'), value: 'half_width' },
  { label: tx('punctOptions.fullWidth'), value: 'full_width' },
])

const trailingOptions = computed(() => [
  { label: tx('trailingOptions.follow'), value: 'follow' },
  { label: tx('trailingOptions.keep'), value: 'keep' },
  { label: tx('trailingOptions.remove'), value: 'remove' },
])

const cjkOptions = computed(() => [
  { label: tx('cjkOptions.follow'), value: 'follow' },
  { label: tx('cjkOptions.auto'), value: 'auto' },
  { label: tx('cjkOptions.none'), value: 'none' },
])

/** 顶部"当前识别到"主文案（锁定时为锁定提示）。 */
const statusText = computed(() => {
  if (locked.value && status.value) {
    return tx('statusLocked').replace('{scene}', sceneName(status.value.scene))
  }
  if (!status.value) return tx('statusLoading')
  const app = status.value.app || tx('statusUnknownApp')
  const entry = status.value.entry_name || sceneName(status.value.scene)
  return `${tx('statusPrefix')}${app} → ${entry}`
})

/** 卡片副标题：场景名 · 应用数量 / 自动识别 · 标题关键字。 */
function metaText(entry) {
  const parts = [sceneName(entry.scene)]
  if (entry.apps.length > 0) {
    parts.push(tx('appsCount').replace('{count}', String(entry.apps.length)))
  } else if (!entry.title_keyword) {
    parts.push(tx('autoMatch'))
  }
  if (entry.title_keyword) {
    parts.push(tx('keywordMeta').replace('{keyword}', entry.title_keyword))
  }
  return parts.join(' · ')
}

function isActiveEntry(entry) {
  return status.value?.entry_id === entry.id
}

function isLockedEntry(entry) {
  return locked.value && status.value?.scene === entry.scene
}

function errorText() {
  return lastError.value
    ? `${lastError.value.message}（${t.common.errorCode} ${lastError.value.code}）`
    : ''
}

function dbError(error) {
  lastError.value = toAppError(error)
}

function toggleExpand(entry) {
  if (expandedId.value === entry.id) {
    collapse()
    return
  }
  formError.value = ''
  lastError.value = null
  expandedId.value = entry.id
  draft.value = {
    ...entry,
    apps: [...entry.apps],
    icon_color: entry.icon_color || DEFAULT_ICON_COLOR,
  }
}

function collapse() {
  expandedId.value = null
  draft.value = null
  formError.value = ''
  appInput.value = ''
}

function onAdd() {
  if (atLimit.value) return
  formError.value = ''
  lastError.value = null
  expandedId.value = NEW_ENTRY_ID
  draft.value = newEntryDraft()
}

function onColorInput(event) {
  const target = event.target
  if (draft.value && target instanceof HTMLInputElement) {
    draft.value.icon_color = target.value
  }
}

/** 匹配应用输入框。 */
const appInput = ref('')

function onAddApp() {
  if (!draft.value) return
  const next = addApp(draft.value.apps, appInput.value)
  if (next !== draft.value.apps && next.length > MAX_ENTRY_APPS) {
    formError.value = tx('validation.appsTooMany')
    return
  }
  draft.value.apps = next
  appInput.value = ''
}

function onRemoveApp(app) {
  if (!draft.value) return
  draft.value.apps = removeApp(draft.value.apps, app)
}

/** 自动回车开关：终端场景允许开启但需二次确认（PRD 3.2.5 规则 1）。 */
async function onAutoEnterToggle(entry, value) {
  if (!entry) return
  if (value && entry.scene === 'terminal') {
    const ok = await confirm(tx('autoEnterConfirm'), {
      title: tx('autoEnterConfirmTitle'),
      kind: 'warning',
    })
    if (!ok) return
  }
  entry.auto_enter = value
}

async function onSave() {
  const current = draft.value
  if (!current) return
  const normalized = normalizeEntry(current)
  const code = validateEntryDraft(normalized)
  if (code) {
    formError.value = tx(`validation.${code}`)
    return
  }
  formError.value = ''
  try {
    if (creating.value) {
      await sceneAddEntry(normalized)
    } else {
      await sceneUpdateEntry(normalized)
    }
    collapse()
    await Promise.all([loadEntries(), loadStatus()])
  } catch (error) {
    dbError(error)
  }
}

async function onDelete(entry) {
  const ok = await confirm(tx('deleteConfirm').replace('{name}', entry.name), {
    title: tx('deleteTitle'),
    kind: 'warning',
  })
  if (!ok) return
  try {
    await sceneDeleteEntry(entry.id)
    if (expandedId.value === entry.id) collapse()
    await Promise.all([loadEntries(), loadStatus()])
  } catch (error) {
    dbError(error)
  }
}

async function onRestoreDefault(entry) {
  const ok = await confirm(tx('restoreConfirm').replace('{name}', entry.name), {
    title: tx('restoreDefault'),
    kind: 'warning',
  })
  if (!ok) return
  try {
    await sceneRestoreEntryDefault(entry.id)
    if (expandedId.value === entry.id) collapse()
    await Promise.all([loadEntries(), loadStatus()])
  } catch (error) {
    dbError(error)
  }
}

/** 锁定：把选择的目标场景写入 override（PRD 3.2.1 规则 3 临时锁定）。 */
async function onLock() {
  try {
    await sceneSetOverride(lockTarget.value)
    await loadStatus()
  } catch (error) {
    dbError(error)
  }
}

/** 解除锁定，恢复自动识别。 */
async function onUnlock() {
  try {
    await sceneClearOverride()
    lockTargetPicked.value = false
    await loadStatus()
  } catch (error) {
    dbError(error)
  }
}

// ---------- 拖动排序 ----------

const dragIndex = ref(-1)

function onDragStart(index) {
  dragIndex.value = index
}

function onDragEnd() {
  dragIndex.value = -1
}

function onDrop(index) {
  const from = dragIndex.value
  dragIndex.value = -1
  if (from < 0) return
  const next = moveEntry(entries.value, from, index)
  if (next === entries.value) return
  entries.value = next
  void persistOrder(next)
}

async function persistOrder(list) {
  try {
    await sceneReorderEntries(list.map((entry) => entry.id))
  } catch (error) {
    dbError(error)
    await loadEntries()
  }
}
</script>

<template>
  <div class="settings-page">
    <div class="scene-page-head">
      <div>
        <h2 class="settings-page-title">{{ tx('title') }}</h2>
        <p class="settings-page-desc">{{ tx('desc') }}</p>
      </div>
      <Button
        type="button"
        icon="pi pi-plus"
        :label="tx('add')"
        :disabled="atLimit"
        @click="onAdd"
      />
    </div>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ tx('statusGroup') }}</h3>
      <div class="settings-card scene-status-card">
        <div class="scene-status-main">
          <span v-if="locked" class="scene-badge scene-badge-lock">
            <i class="pi pi-lock" aria-hidden="true" />
            {{ tx('lockedBadge') }}
          </span>
          <span class="scene-status-text">{{ statusText }}</span>
        </div>
        <div class="scene-lock-actions">
          <template v-if="locked">
            <Button
              type="button"
              icon="pi pi-lock-open"
              :label="tx('unlock')"
              severity="secondary"
              @click="onUnlock"
            />
          </template>
          <template v-else>
            <Select
              v-model="lockTarget"
              :options="sceneKindOptions"
              option-label="label"
              option-value="value"
              :aria-label="tx('lockTarget')"
              @update:model-value="lockTargetPicked = true"
            />
            <Button
              type="button"
              icon="pi pi-lock"
              :label="tx('lock')"
              :disabled="!status"
              @click="onLock"
            />
          </template>
        </div>
      </div>
      <p class="scene-status-hint">{{ tx('lockHint') }}</p>
    </section>

    <section class="settings-group">
      <h3 class="settings-group-title">
        {{ tx('group') }}
        <span class="scene-counter">{{ entries.length }} / {{ MAX_SCENE_ENTRIES }}</span>
      </h3>
      <p class="settings-page-desc scene-list-hint">{{ tx('hint') }}</p>

      <div class="scene-list">
        <div
          v-for="(card, index) in cards"
          :key="card.id"
          class="scene-card"
          :class="{ 'scene-card-active': card.id === NEW_ENTRY_ID || isActiveEntry(card) }"
          @dragover.prevent
          @drop="onDrop(index)"
        >
          <div
            class="scene-card-head"
            :draggable="card.id !== NEW_ENTRY_ID"
            @dragstart="onDragStart(index)"
            @dragend="onDragEnd"
            @click="toggleExpand(card)"
          >
            <i
              class="pi scene-expander"
              :class="expandedId === card.id ? 'pi-chevron-down' : 'pi-chevron-right'"
              aria-hidden="true"
            />
            <span
              class="scene-dot"
              :style="{ background: card.icon_color || DEFAULT_ICON_COLOR }"
              aria-hidden="true"
            />
            <div class="scene-head-text">
              <div class="scene-head-line">
                <span class="scene-name">{{ card.name || tx('newEntry') }}</span>
                <span v-if="card.builtin" class="scene-badge">{{ tx('builtinBadge') }}</span>
                <span v-if="isLockedEntry(card)" class="scene-badge scene-badge-lock">
                  <i class="pi pi-lock" aria-hidden="true" />
                  {{ tx('lockedBadge') }}
                </span>
                <span v-if="isActiveEntry(card) && !locked" class="scene-badge scene-badge-active">
                  {{ tx('activeBadge') }}
                </span>
              </div>
              <div class="scene-head-meta">
                {{ card.id === NEW_ENTRY_ID ? tx('newEntryHint') : metaText(card) }}
              </div>
            </div>
            <div v-if="card.id !== NEW_ENTRY_ID" class="scene-head-actions" @click.stop>
              <Button
                v-if="card.builtin"
                type="button"
                icon="pi pi-refresh"
                size="small"
                text
                severity="secondary"
                :aria-label="tx('restoreDefault')"
                :title="tx('restoreDefault')"
                @click="onRestoreDefault(card)"
              />
              <Button
                v-else
                type="button"
                icon="pi pi-trash"
                size="small"
                text
                severity="danger"
                :aria-label="tx('delete')"
                :title="tx('delete')"
                @click="onDelete(card)"
              />
              <i
                class="pi pi-bars scene-drag-handle"
                :title="tx('dragHint')"
                :aria-label="tx('dragHint')"
                role="button"
                @click.stop
              />
            </div>
          </div>

          <div v-if="expandedId === card.id && draft" class="scene-card-body">
            <div class="scene-form-grid">
              <label class="scene-field">
                <span class="scene-field-label">{{ tx('name') }}</span>
                <InputText
                  v-model="draft.name"
                  :maxlength="MAX_ENTRY_NAME_LEN"
                  :placeholder="tx('namePlaceholder')"
                />
              </label>
              <label class="scene-field">
                <span class="scene-field-label">{{ tx('sceneKind') }}</span>
                <Select
                  v-model="draft.scene"
                  :options="sceneKindOptions"
                  option-label="label"
                  option-value="value"
                  :disabled="draftBuiltin"
                  :aria-label="tx('sceneKind')"
                />
              </label>
              <div class="scene-field">
                <span class="scene-field-label">{{ tx('iconColor') }}</span>
                <div class="scene-color-row">
                  <input
                    type="color"
                    class="scene-color-picker"
                    :value="draft.icon_color || DEFAULT_ICON_COLOR"
                    :aria-label="tx('iconColor')"
                    @input="onColorInput"
                  />
                  <span class="scene-color-text">{{ draft.icon_color || DEFAULT_ICON_COLOR }}</span>
                </div>
              </div>
            </div>

            <div class="scene-field scene-field-wide">
              <span class="scene-field-label">{{ tx('apps') }}</span>
              <div class="scene-apps">
                <span v-for="app in draft.apps" :key="app" class="scene-app-chip">
                  {{ app }}
                  <button
                    type="button"
                    class="scene-app-remove"
                    :aria-label="tx('appRemoveAria').replace('{app}', app)"
                    @click="onRemoveApp(app)"
                  >
                    <i class="pi pi-times" aria-hidden="true" />
                  </button>
                </span>
                <span v-if="!draft.apps.length" class="scene-apps-empty">{{ tx('appsEmpty') }}</span>
              </div>
              <div class="scene-app-add">
                <InputText
                  v-model="appInput"
                  :maxlength="MAX_APP_LEN"
                  :placeholder="tx('appsPlaceholder')"
                  :aria-label="tx('appsPlaceholder')"
                  @keyup.enter="onAddApp"
                />
                <Button
                  type="button"
                  icon="pi pi-plus"
                  size="small"
                  :label="tx('appAdd')"
                  :disabled="!appInput.trim() || draft.apps.length >= MAX_ENTRY_APPS"
                  @click="onAddApp"
                />
              </div>
              <p class="scene-field-hint">{{ tx('appsHint') }}</p>
            </div>

            <label class="scene-field scene-field-wide">
              <span class="scene-field-label">{{ tx('titleKeyword') }}</span>
              <InputText
                v-model="draft.title_keyword"
                :maxlength="MAX_TITLE_KEYWORD_LEN"
                :placeholder="tx('titleKeywordPlaceholder')"
              />
              <span class="scene-field-hint">{{ tx('titleKeywordHint') }}</span>
            </label>

            <h4 class="scene-rules-title">{{ tx('outputGroup') }}</h4>
            <div class="scene-form-grid">
              <label class="scene-field">
                <span class="scene-field-label">{{ tx('punctStyle') }}</span>
                <Select
                  v-model="draft.punct_style"
                  :options="punctOptions"
                  option-label="label"
                  option-value="value"
                  :aria-label="tx('punctStyle')"
                />
              </label>
              <label class="scene-field">
                <span class="scene-field-label">{{ tx('trailingPunct') }}</span>
                <Select
                  v-model="draft.trailing_punct"
                  :options="trailingOptions"
                  option-label="label"
                  option-value="value"
                  :aria-label="tx('trailingPunct')"
                />
              </label>
              <label class="scene-field">
                <span class="scene-field-label">{{ tx('cjkSpace') }}</span>
                <Select
                  v-model="draft.cjk_space"
                  :options="cjkOptions"
                  option-label="label"
                  option-value="value"
                  :aria-label="tx('cjkSpace')"
                />
              </label>
              <div class="scene-field scene-field-toggle">
                <span class="scene-field-label">{{ tx('commands') }}</span>
                <ToggleSwitch v-model="draft.commands" :aria-label="tx('commands')" />
                <span class="scene-field-hint">{{ tx('commandsHint') }}</span>
              </div>
              <div class="scene-field scene-field-toggle">
                <span class="scene-field-label">{{ tx('autoEnter') }}</span>
                <ToggleSwitch
                  :model-value="draft.auto_enter"
                  :aria-label="tx('autoEnter')"
                  @update:model-value="(v) => onAutoEnterToggle(draft, v)"
                />
                <span class="scene-field-hint">{{ tx('autoEnterHint') }}</span>
              </div>
              <div class="scene-field">
                <span class="scene-field-label">{{ tx('aiTemplate') }}</span>
                <Select
                  :model-value="draft.ai_template"
                  :options="[{ label: tx('aiTemplateNone'), value: null }]"
                  option-label="label"
                  option-value="value"
                  disabled
                  :aria-label="tx('aiTemplate')"
                />
                <span class="scene-field-hint">{{ tx('aiTemplateHint') }}</span>
              </div>
            </div>

            <p v-if="formError" class="scene-form-error">{{ formError }}</p>

            <div class="scene-form-footer">
              <Button
                type="button"
                severity="secondary"
                :label="t.common.cancel"
                @click="collapse"
              />
              <Button type="button" :label="tx('save')" @click="onSave" />
            </div>
          </div>
        </div>
      </div>
    </section>

    <Message v-if="lastError" severity="error" class="settings-error">{{
      errorText()
    }}</Message>
  </div>
</template>

<style scoped>
.scene-page-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--vt-spacing-md);
}

.scene-status-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--vt-spacing-md);
  padding: var(--vt-spacing-md);
  flex-wrap: wrap;
}

.scene-status-main {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
  min-width: 0;
}

.scene-status-text {
  font-size: 13.5px;
  color: var(--vt-color-text);
  overflow-wrap: anywhere;
}

.scene-lock-actions {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
}

.scene-status-hint {
  margin: var(--vt-spacing-xs) 0 0;
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.scene-counter {
  margin-left: var(--vt-spacing-sm);
  font-size: 12.5px;
  font-weight: 400;
  color: var(--vt-color-text-muted);
}

.scene-list-hint {
  margin-bottom: var(--vt-spacing-sm);
}

.scene-list {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-sm);
}

.scene-card {
  border: 1px solid var(--vt-color-border);
  border-radius: var(--vt-radius-card);
  background: var(--vt-color-surface);
  overflow: hidden;
}

.scene-card-active {
  border-color: var(--vt-color-primary);
}

.scene-card-head {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
  padding: 10px var(--vt-spacing-md);
  cursor: pointer;
  user-select: none;
}

.scene-expander {
  font-size: 12px;
  color: var(--vt-color-text-muted);
}

.scene-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  flex: none;
}

.scene-head-text {
  flex: 1;
  min-width: 0;
}

.scene-head-line {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
  flex-wrap: wrap;
}

.scene-name {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--vt-color-text);
}

.scene-head-meta {
  margin-top: 2px;
  font-size: 12px;
  color: var(--vt-color-text-muted);
  overflow-wrap: anywhere;
}

.scene-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 7px;
  border-radius: 9px;
  font-size: 11.5px;
  background: var(--vt-color-nav-active-bg);
  color: var(--vt-color-text-muted);
}

.scene-badge-lock {
  background: rgba(217, 119, 6, 0.12);
  color: var(--vt-color-warning);
}

.scene-badge-active {
  background: rgba(59, 130, 246, 0.12);
  color: var(--vt-color-primary);
}

.scene-head-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  flex: none;
}

.scene-drag-handle {
  padding: 6px;
  font-size: 13px;
  color: var(--vt-color-text-muted);
  cursor: grab;
}

.scene-card-body {
  padding: var(--vt-spacing-md);
  border-top: 1px solid var(--vt-color-border);
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-md);
}

.scene-form-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: var(--vt-spacing-md);
}

.scene-field {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-xs);
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
  min-width: 0;
}

.scene-field-wide {
  width: 100%;
}

.scene-field-toggle {
  flex-direction: row;
  align-items: center;
  gap: var(--vt-spacing-sm);
  flex-wrap: wrap;
}

.scene-field-toggle .scene-field-hint {
  flex-basis: 100%;
}

.scene-field-label {
  font-size: 12.5px;
}

.scene-field-hint {
  font-size: 12px;
  color: var(--vt-color-text-muted);
}

.scene-rules-title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--vt-color-text);
}

.scene-color-row {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
}

.scene-color-picker {
  width: 36px;
  height: 28px;
  padding: 2px;
  border: 1px solid var(--vt-color-border);
  border-radius: var(--vt-radius-control);
  background: var(--vt-color-surface);
  cursor: pointer;
}

.scene-color-text {
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.scene-apps {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-xs);
  flex-wrap: wrap;
}

.scene-app-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 4px 2px 9px;
  border: 1px solid var(--vt-color-border);
  border-radius: 12px;
  font-size: 12px;
  color: var(--vt-color-text);
  background: var(--vt-color-bg);
}

.scene-app-remove {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--vt-color-text-muted);
  font-size: 10px;
  cursor: pointer;
}

.scene-app-remove:hover {
  background: var(--vt-color-border);
  color: var(--vt-color-text);
}

.scene-apps-empty {
  font-size: 12px;
  color: var(--vt-color-text-muted);
}

.scene-app-add {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
}

.scene-form-error {
  margin: 0;
  font-size: 12.5px;
  color: var(--vt-color-error);
}

.scene-form-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--vt-spacing-sm);
}
</style>
