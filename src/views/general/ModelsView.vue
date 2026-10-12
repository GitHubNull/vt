<script setup>
/**
 * 通用 › 模型管理（T-014，F-SET-06、PRD 3.8 节；下载体验增强 T-017）。
 * 模型卡片：档位 / 大小 / 说明 / 状态；下载进度 + 实时速度与预计剩余时间
 * （model:progress 事件）、取消 / 失败重试、切换激活（确认 + 即时生效）、
 * 删除（plugin-dialog 确认，删除激活模型给出回退提示）；
 * 标点恢复模型卡片（T-545）：下载 / 进度 / 取消 / 删除 / 校验失败提示；
 * 下载设置：下载源（镜像默认 / 官方）、下载代理、模型存储位置（plugin-dialog 选择）。
 */
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import Badge from 'primevue/badge'
import Button from 'primevue/button'
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import ProgressBar from 'primevue/progressbar'
import Select from 'primevue/select'
import { confirm, open } from '@tauri-apps/plugin-dialog'
import { listen } from '@tauri-apps/api/event'
import SettingsRow from '../../components/SettingsRow.vue'
import { useAsrStore } from '../../stores/asr'
import { useConfigStore } from '../../stores/config'
import {
  cancelPunctModel,
  deletePunctModel,
  downloadPunctModel,
  PUNCT_MODEL_ID,
  punctModelStatus,
} from '../../api/asr'
import { EVENTS } from '../../api/events'
import { toAppError } from '../../api/client'
import { getMessage, t } from '../../i18n'

const store = useAsrStore()
const configStore = useConfigStore()
/** 切换中的模型 ID（禁用重复操作）。 */
const switchingId = ref(null)

// ---- 标点恢复模型卡片（T-545）：独立于识别模型清单，进度按 model_id 过滤 ----
const punct = ref(null)
/** 标点进度事件监听取消句。 */
let unlistenPunct = null

async function loadPunct() {
  try {
    punct.value = await punctModelStatus()
  } catch (error) {
    store.lastError = toAppError(error)
  }
}

/** 标点卡片状态（派生自 PunctModelInfo + 下载进度事件）。 */
function punctStatus() {
  const info = punct.value
  if (!info) return 'missing'
  if (info.progress) return 'downloading'
  if (info.invalid) return 'invalid'
  if (info.downloaded) return 'downloaded'
  return 'missing'
}

const punctStatusText = computed(() => ({
  downloading: t.general.models.statusDownloading,
  downloaded: t.general.models.statusDownloaded,
  invalid: t.general.models.statusInvalid,
  missing: t.general.models.statusMissing,
}))

async function startPunctListener() {
  try {
    unlistenPunct = await listen(EVENTS.modelProgress, (event) => {
      const payload = event.payload
      if (payload.model_id !== PUNCT_MODEL_ID) return
      if (payload.status === 'downloading' && punct.value) {
        punct.value = {
          ...punct.value,
          progress: {
            downloaded_bytes: payload.downloaded_bytes,
            total_bytes: payload.total_bytes,
            bytes_per_sec: payload.bytes_per_sec,
          },
        }
      } else if (payload.status !== 'downloading') {
        // 完成 / 失败 / 取消以服务端状态为准。
        void loadPunct()
      }
    })
  } catch {
    // Webview 环境外（测试）不支持 listen：忽略
  }
}

async function onPunctDownload() {
  try {
    await downloadPunctModel()
  } catch (error) {
    store.lastError = toAppError(error)
  }
  void loadPunct()
}

async function onPunctCancel() {
  try {
    await cancelPunctModel()
  } catch (error) {
    store.lastError = toAppError(error)
  }
  void loadPunct()
}

async function onPunctDelete() {
  const ok = await confirm(getMessage('general.punct.deleteConfirm'), {
    title: getMessage('general.punct.deleteTitle'),
    kind: 'warning',
  })
  if (!ok) return
  try {
    await deletePunctModel()
    await loadPunct()
  } catch (error) {
    store.lastError = toAppError(error)
  }
}

onMounted(() => {
  void store.ensureLoaded()
  void configStore.ensureLoaded()
  void loadPunct()
  void startPunctListener()
})

onUnmounted(() => {
  if (unlistenPunct) unlistenPunct()
})

const statusText = computed(() => ({
  active: t.general.models.statusActive,
  downloading: t.general.models.statusDownloading,
  downloaded: t.general.models.statusDownloaded,
  invalid: t.general.models.statusInvalid,
  missing: t.general.models.statusMissing,
}))

function formatBytes(bytes) {
  if (bytes >= 1 << 30) return `${(bytes / (1 << 30)).toFixed(2)} GB`
  return `${Math.max(1, Math.round(bytes / (1 << 20)))} MB`
}

function progressPercent(model) {
  if (!model.progress || !model.progress.total_bytes) return 0
  return Math.min(
    100,
    Math.round((model.progress.downloaded_bytes / model.progress.total_bytes) * 100),
  )
}

/** 实时速度文本（12 MB/s 量级）；无进展时为 "—"。 */
function speedText(model) {
  const speed = model.progress?.bytes_per_sec ?? 0
  if (!speed) return '—'
  return t.general.models.speedPerSec.replace('{speed}', formatBytes(speed))
}

/** 预计剩余时间文本（秒 → 分钟）；速度为 0 或缺总量时不显示。 */
function etaText(model) {
  const progress = model.progress
  if (!progress || !progress.bytes_per_sec || !progress.total_bytes) return ''
  const remaining = progress.total_bytes - progress.downloaded_bytes
  if (remaining <= 0) return ''
  const seconds = Math.ceil(remaining / progress.bytes_per_sec)
  const text = seconds >= 60 ? `${Math.ceil(seconds / 60)} min` : `${seconds}s`
  return t.general.models.etaRemaining.replace('{time}', text)
}

/** 生效的模型存储目录（来自 list_models；空清单时为空串）。 */
const effectiveModelsDir = computed(() => store.models[0]?.models_dir ?? '')

const sourceOptions = computed(() => [
  { label: t.general.models.sourceMirror, value: 'mirror' },
  { label: t.general.models.sourceOfficial, value: 'official' },
])

function updateAsr(patch) {
  void configStore.updateSection('asr', { ...configStore.sections.asr, ...patch }).then((ok) => {
    if (ok) void store.refresh()
  })
}

// ---- 下载代理：本地草稿 + 失焦提交（验收 4.6.8） ----
// 根因：后端只接受空串或 http(s):// 前缀的代理地址（config_store.rs 校验），
// 逐键写配置时中间值（如 "h"、"ht"）被后端拒绝 → 乐观值回滚 → 输入框永远打不了字。
// 改为编辑本地草稿、失焦才写盘；被拒绝时回显已生效值并展示后端错误。
const proxyDraft = ref('')
/** 编辑中标记：聚焦期间不回写草稿，避免覆盖正在输入的内容。 */
let proxyEditing = false

watch(
  () => configStore.sections.asr.download_proxy,
  (value) => {
    if (!proxyEditing) proxyDraft.value = value ?? ''
  },
  { immediate: true },
)

function onProxyFocus() {
  proxyEditing = true
}

async function onProxyBlur() {
  proxyEditing = false
  const value = proxyDraft.value.trim()
  const current = configStore.sections.asr.download_proxy ?? ''
  if (value === current) return
  const ok = await configStore.updateSection('asr', {
    ...configStore.sections.asr,
    download_proxy: value || null,
  })
  if (ok) {
    void store.refresh()
  } else {
    // 后端校验拒绝：回显已生效值（错误信息经 configStore.lastError 展示）。
    proxyDraft.value = configStore.sections.asr.download_proxy ?? ''
  }
}

async function chooseModelsDir() {
  try {
    const dir = await open({ directory: true, title: t.general.models.dirChooseTitle })
    if (typeof dir === 'string' && dir) updateAsr({ models_dir: dir })
  } catch (error) {
    store.lastError = toAppError(error)
  }
}

function errorText(prefix) {
  return store.lastError
    ? `${prefix}：${store.lastError.message}（${t.common.errorCode} ${store.lastError.code}）`
    : ''
}

async function onDownload(model) {
  await store.download(model.id)
  void store.refresh()
}

async function onCancel(model) {
  await store.cancel(model.id)
}

async function onActivate(model) {
  const ok = await confirm(t.general.models.switchConfirm.replace('{name}', model.name), {
    title: t.general.models.switchTitle,
    kind: 'info',
  })
  if (!ok) return
  switchingId.value = model.id
  try {
    await store.activate(model.id)
    void store.refresh()
  } finally {
    switchingId.value = null
  }
}

async function onDelete(model) {
  const template = model.active
    ? t.general.models.deleteActiveConfirm
    : t.general.models.deleteConfirm
  const ok = await confirm(template.replace('{name}', model.name), {
    title: t.general.models.deleteTitle,
    kind: 'warning',
  })
  if (!ok) return
  await store.remove(model.id)
}
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.general.models.title }}</h2>
    <p class="settings-page-desc">{{ t.general.models.desc }}</p>

    <Message v-if="!store.hasUsableModel" severity="info" class="settings-error" :closable="false">
      <strong>{{ t.general.models.noModelTitle }}</strong>
      <p class="models-no-model-desc">{{ t.general.models.noModelDesc }}</p>
    </Message>
    <Message
      v-if="store.notice"
      severity="info"
      class="settings-error"
      @close="store.notice = null"
    >
      {{ store.notice }}
    </Message>
    <Message v-if="store.lastError" severity="error" class="settings-error" :closable="false">
      {{ errorText(t.general.models.loadFailedHint) }}
    </Message>
    <Message v-if="configStore.lastError" severity="error" class="settings-error" :closable="false">
      {{ configStore.lastError.message }}（{{ t.common.errorCode }}
      {{ configStore.lastError.code }}）
    </Message>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.general.models.group }}</h3>
      <div v-for="model in store.models" :key="model.id" class="settings-card model-card">
        <div class="model-head">
          <span class="model-name">{{ model.name }}</span>
          <Badge
            :value="
              model.tier === 'accuracy' ? t.general.models.tierAccuracy : t.general.models.tierSpeed
            "
            :severity="model.tier === 'accuracy' ? 'info' : 'secondary'"
          />
          <Badge
            :value="statusText[store.statusOf(model)]"
            :severity="
              store.statusOf(model) === 'active'
                ? 'success'
                : store.statusOf(model) === 'invalid'
                  ? 'danger'
                  : 'secondary'
            "
            class="model-status"
          />
        </div>
        <p class="model-desc">
          {{
            model.tier === 'accuracy'
              ? t.general.models.tierAccuracyDesc
              : t.general.models.tierSpeedDesc
          }}
        </p>
        <p class="model-meta">
          <span>{{ t.general.models.languagesLabel }}：{{ model.languages }}</span>
          <span>{{ t.general.models.sizeLabel }}：{{ formatBytes(model.size_bytes) }}</span>
        </p>

        <p v-if="store.statusOf(model) === 'invalid'" class="model-invalid-hint">
          {{ t.general.models.invalidHint }}
        </p>

        <div v-if="store.statusOf(model) === 'downloading'" class="model-actions">
          <ProgressBar :value="progressPercent(model)" class="model-progress" />
          <span class="model-progress-text">{{ progressPercent(model) }}%</span>
          <span class="model-speed"
            >{{ speedText(model)
            }}<template v-if="etaText(model)"> · {{ etaText(model) }}</template></span
          >
          <Button
            type="button"
            severity="secondary"
            :label="t.general.models.cancelDownload"
            @click="onCancel(model)"
          />
        </div>
        <div v-else class="model-actions">
          <template v-if="store.statusOf(model) === 'missing'">
            <Button
              type="button"
              icon="pi pi-download"
              :label="t.general.models.download"
              @click="onDownload(model)"
            />
          </template>
          <template v-if="store.statusOf(model) === 'downloaded'">
            <Button
              type="button"
              :label="t.general.models.activate"
              :loading="switchingId === model.id"
              :disabled="switchingId !== null"
              @click="onActivate(model)"
            />
          </template>
          <template v-if="store.statusOf(model) === 'active'">
            <Button
              type="button"
              severity="secondary"
              :label="t.general.models.activate"
              :loading="switchingId === model.id"
              :disabled="switchingId !== null"
              @click="onActivate(model)"
            />
          </template>
          <Button
            v-if="store.statusOf(model) !== 'missing'"
            type="button"
            severity="danger"
            :label="t.general.models.delete"
            :disabled="switchingId !== null"
            @click="onDelete(model)"
          />
        </div>
      </div>

      <!-- 标点恢复模型（T-545）：单卡片，状态 / 进度 / 下载 / 删除。 -->
      <div v-if="punct" class="settings-card model-card">
        <div class="model-head">
          <span class="model-name">{{ getMessage('general.punct.cardTitle') }}</span>
          <Badge
            :value="punctStatusText[punctStatus()]"
            :severity="punctStatus() === 'invalid' ? 'danger' : 'secondary'"
            class="model-status"
          />
        </div>
        <p class="model-desc">{{ getMessage('general.punct.cardDesc') }}</p>
        <p class="model-meta">
          <span>{{ t.general.models.sizeLabel }}：{{ formatBytes(punct.size_bytes) }}</span>
        </p>

        <p v-if="punctStatus() === 'invalid'" class="model-invalid-hint">
          {{ t.general.models.invalidHint }}
        </p>

        <div v-if="punctStatus() === 'downloading'" class="model-actions">
          <ProgressBar :value="progressPercent(punct)" class="model-progress" />
          <span class="model-progress-text">{{ progressPercent(punct) }}%</span>
          <span class="model-speed"
            >{{ speedText(punct)
            }}<template v-if="etaText(punct)"> · {{ etaText(punct) }}</template></span
          >
          <Button
            type="button"
            severity="secondary"
            :label="t.general.models.cancelDownload"
            @click="onPunctCancel"
          />
        </div>
        <div v-else class="model-actions">
          <Button
            v-if="punctStatus() === 'missing'"
            type="button"
            icon="pi pi-download"
            :label="t.general.models.download"
            @click="onPunctDownload"
          />
          <Button
            v-if="punctStatus() !== 'missing'"
            type="button"
            severity="danger"
            :label="t.general.models.delete"
            @click="onPunctDelete"
          />
        </div>
      </div>
    </section>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.general.models.downloadSettings }}</h3>
      <div class="settings-card">
        <SettingsRow :title="t.general.models.sourceLabel" :desc="t.general.models.sourceDesc">
          <Select
            class="model-source-select"
            :model-value="configStore.sections.asr.model_source"
            :options="sourceOptions"
            option-label="label"
            option-value="value"
            :aria-label="t.general.models.sourceLabel"
            @update:model-value="(v) => updateAsr({ model_source: v })"
          />
        </SettingsRow>
        <SettingsRow :title="t.general.models.proxyLabel" :desc="t.general.models.proxyDesc">
          <InputText
            v-model="proxyDraft"
            class="model-proxy-input"
            :placeholder="t.general.models.proxyPlaceholder"
            :aria-label="t.general.models.proxyLabel"
            @focus="onProxyFocus"
            @blur="onProxyBlur"
          />
        </SettingsRow>
        <SettingsRow :title="t.general.models.dirLabel" :desc="t.general.models.dirDesc">
          <div class="dir-row">
            <InputText
              readonly
              class="dir-path"
              :model-value="effectiveModelsDir"
              :aria-label="t.general.models.dirLabel"
            />
            <Button
              type="button"
              severity="secondary"
              size="small"
              icon="pi pi-folder-open"
              :label="t.general.models.dirChoose"
              @click="chooseModelsDir"
            />
            <Button
              v-if="configStore.sections.asr.models_dir"
              type="button"
              severity="secondary"
              size="small"
              :label="t.general.models.dirReset"
              @click="updateAsr({ models_dir: null })"
            />
          </div>
        </SettingsRow>
      </div>
    </section>
  </div>
</template>

<style scoped>
.model-card {
  padding: var(--vt-spacing-md);
  margin-bottom: var(--vt-spacing-md);
}

.model-head {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
}

.model-name {
  font-size: 15px;
  font-weight: 600;
}

.model-status {
  margin-inline-start: auto;
}

.model-desc {
  margin: var(--vt-spacing-sm) 0 0;
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.model-meta {
  display: flex;
  gap: var(--vt-spacing-lg);
  margin: var(--vt-spacing-sm) 0 0;
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.model-invalid-hint {
  margin: var(--vt-spacing-sm) 0 0;
  font-size: 12.5px;
  color: var(--vt-color-error);
}

.model-actions {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
  margin-top: var(--vt-spacing-md);
}

.model-progress {
  flex: 1;
  max-width: 360px;
}

.model-progress-text {
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
  min-width: 40px;
}

.model-speed {
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
  white-space: nowrap;
}

.model-source-select {
  width: 220px;
}

.model-proxy-input {
  width: 280px;
}

.dir-row {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
}

.dir-path {
  flex: 1;
  min-width: 260px;
}

.models-no-model-desc {
  margin: 4px 0 0;
}
</style>
