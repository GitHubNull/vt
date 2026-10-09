<script setup>
/**
 * 通用 › 模型管理（T-014，F-SET-06、PRD 3.8 节）。
 * 模型卡片：档位 / 大小 / 说明 / 状态；下载进度（model:progress 事件）、
 * 取消 / 失败重试、切换激活（确认 + 即时生效）、删除（plugin-dialog 确认，
 * 删除激活模型给出回退提示）；无模型引导与断网错误提示（错误编号）。
 */
import { computed, onMounted, ref } from 'vue'
import Badge from 'primevue/badge'
import Button from 'primevue/button'
import Message from 'primevue/message'
import ProgressBar from 'primevue/progressbar'
import { confirm } from '@tauri-apps/plugin-dialog'
import { useAsrStore } from '../../stores/asr'
import { t } from '../../i18n'

const store = useAsrStore()
/** 切换中的模型 ID（禁用重复操作）。 */
const switchingId = ref(null)

onMounted(() => {
  void store.ensureLoaded()
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
  return Math.min(100, Math.round((model.progress.downloaded_bytes / model.progress.total_bytes) * 100))
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
  const ok = await confirm(
    t.general.models.switchConfirm.replace('{name}', model.name),
    { title: t.general.models.switchTitle, kind: 'info' },
  )
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
    <Message v-if="store.notice" severity="info" class="settings-error" @close="store.notice = null">
      {{ store.notice }}
    </Message>
    <Message v-if="store.lastError" severity="error" class="settings-error" :closable="false">
      {{ errorText(t.general.models.loadFailedHint) }}
    </Message>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.general.models.group }}</h3>
      <div v-for="model in store.models" :key="model.id" class="settings-card model-card">
        <div class="model-head">
          <span class="model-name">{{ model.name }}</span>
          <Badge
            :value="model.tier === 'accuracy' ? t.general.models.tierAccuracy : t.general.models.tierSpeed"
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
          {{ model.tier === 'accuracy' ? t.general.models.tierAccuracyDesc : t.general.models.tierSpeedDesc }}
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
          <Button
            type="button"
            severity="secondary"
            :label="t.general.models.cancelDownload"
            @click="onCancel(model)"
          />
        </div>
        <div v-else class="model-actions">
          <template v-if="store.statusOf(model) === 'missing'">
            <Button type="button" icon="pi pi-download" :label="t.general.models.download" @click="onDownload(model)" />
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

.models-no-model-desc {
  margin: 4px 0 0;
}
</style>
