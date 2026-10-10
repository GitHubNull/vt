<script setup>
/**
 * 关于页（T-016，F-ABOUT-01/02/03、PRD 3.12）：
 * 版本信息 + 检查更新（只发版本号 / OS / 架构，NFR-PR-06；签名校验失败拒绝安装，NFR-S-06）、
 * 导出脱敏日志（plugin-dialog 选位置，NFR-S-02）、法律信息（LICENSE / 第三方声明 / 免责声明 / 隐私说明）。
 * 外部链接与目录一律走 @tauri-apps/plugin-opener（tech/03 3.3）。
 */
import { computed, onMounted, ref } from 'vue'
import Button from 'primevue/button'
import Message from 'primevue/message'
import ToggleSwitch from 'primevue/toggleswitch'
import { openUrl } from '@tauri-apps/plugin-opener'
import { save } from '@tauri-apps/plugin-dialog'
import SettingsRow from '../../components/SettingsRow.vue'
import LegalTextDialog from '../../components/LegalTextDialog.vue'
import { useConfigStore } from '../../stores/config'
import {
  checkUpdate,
  applyUpdate,
  exportLogs,
  getAppInfo,
  getLegalText,
} from '../../api/about'
import { AppError, toAppError } from '../../api/client'
import { t } from '../../i18n'
import { LINKS, SKIPPED_VERSION_KEY } from '../../constants'

const store = useConfigStore()

const appInfo = ref(null)
const infoError = ref('')

/** 更新检查状态机：idle / checking / up_to_date / available / applying / skipped / error。 */
const updateState = ref('idle')
const updateResult = ref(null)
const updateError = ref('')

/** 导出日志状态。 */
const exporting = ref(false)
const exportMessage = ref(null) // { ok, text }

/** 法律文本对话框。 */
const legalDialog = ref({ visible: false, title: '', text: '', markdown: false })
const legalLoading = ref(false)

onMounted(() => {
  void store.ensureLoaded()
  void loadAppInfo()
})

async function loadAppInfo() {
  try {
    appInfo.value = await getAppInfo()
  } catch (error) {
    infoError.value = `${t.common.loadFailed}（${t.common.errorCode} ${toAppError(error).code}）`
  }
}

function openExternal(url) {
  openUrl(url).catch((error) => {
    store.lastError = toAppError(error)
  })
}

function skippedVersion() {
  try {
    return window.localStorage.getItem(SKIPPED_VERSION_KEY)
  } catch {
    return null
  }
}

async function onCheckUpdate() {
  updateState.value = 'checking'
  updateError.value = ''
  updateResult.value = null
  try {
    const result = await checkUpdate()
    if (result.status === 'available' && result.version === skippedVersion()) {
      updateState.value = 'skipped'
    } else if (result.status === 'available') {
      updateResult.value = result
      updateState.value = 'available'
    } else if (result.status === 'up_to_date') {
      updateState.value = 'up_to_date'
    } else {
      // unavailable：更新服务器未就绪（v0.1 正常路径）
      updateResult.value = result
      updateState.value = 'error'
    }
  } catch (error) {
    updateError.value = `${t.about.update.failedPrefix}（${t.common.errorCode} ${toAppError(error).code}）`
    updateState.value = 'error'
  }
}

async function onApplyUpdate() {
  updateState.value = 'applying'
  try {
    await applyUpdate()
  } catch (error) {
    updateError.value = `${t.about.update.applyFailedPrefix}（${t.common.errorCode} ${toAppError(error).code}）`
    updateState.value = 'error'
  }
}

function onSkipVersion() {
  const version = updateResult.value?.version
  if (version) {
    try {
      window.localStorage.setItem(SKIPPED_VERSION_KEY, version)
    } catch {
      // 本地存储不可用：仅本次会话生效
    }
  }
  updateState.value = 'skipped'
}

function onLater() {
  updateState.value = 'idle'
}

async function onExportLogs() {
  exporting.value = true
  exportMessage.value = null
  try {
    const path = await save({
      title: t.about.exportLogs.saveTitle,
      defaultPath: 'vibetype-logs.txt',
      filters: [{ name: 'Text', extensions: ['txt'] }],
    })
    if (!path) return // 用户取消
    const result = await exportLogs(path)
    exportMessage.value = { ok: true, text: `${t.about.exportLogs.donePrefix}${result.path}` }
  } catch (error) {
    exportMessage.value = {
      ok: false,
      text: `${t.about.exportLogs.failedPrefix}（${t.common.errorCode} ${toAppError(error).code}）`,
    }
  } finally {
    exporting.value = false
  }
}

async function openLegal(document, title, markdown = false) {
  legalLoading.value = true
  legalDialog.value = { visible: true, title, text: '', markdown }
  try {
    legalDialog.value.text = await getLegalText(document)
  } catch (error) {
    legalDialog.value.visible = false
    const appError = toAppError(error)
    store.lastError = new AppError(appError.code, `${t.about.legal.loadFailed}：${appError.message}`)
  } finally {
    legalLoading.value = false
  }
}

const legalItems = computed(() => [
  { key: 'license', label: t.about.legal.license },
  { key: 'third_party', label: t.about.legal.thirdParty },
  { key: 'disclaimer', label: t.about.legal.disclaimer, markdown: true },
  { key: 'privacy', label: t.about.legal.privacy, markdown: true },
])

const versionText = computed(() =>
  appInfo.value ? `v${appInfo.value.version}` : '',
)

const errorText = computed(() =>
  store.lastError ? `${store.lastError.message}（${t.common.errorCode} ${store.lastError.code}）` : '',
)
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.about.title }}</h2>
    <p class="settings-page-desc">{{ t.app.subtitle }}</p>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.app.title }}</h3>
      <div class="settings-card">
        <SettingsRow :title="`${t.app.title} ${versionText}`" :desc="t.app.subtitle">
          <div class="about-actions">
            <Button
              type="button"
              :label="t.about.versionCard.official"
              severity="link"
              @click="openExternal(LINKS.official)"
            />
            <Button
              type="button"
              :label="t.about.versionCard.openSource"
              severity="link"
              @click="openExternal(LINKS.repository)"
            />
            <Button
              type="button"
              icon="pi pi-download"
              :label="updateState === 'checking' ? t.about.versionCard.checking : t.about.versionCard.checkUpdate"
              :loading="updateState === 'checking' || updateState === 'applying'"
              @click="onCheckUpdate"
            />
          </div>
        </SettingsRow>
        <SettingsRow
          :title="t.about.versionCard.autoCheck.label"
          :desc="t.about.versionCard.autoCheck.desc"
        >
          <ToggleSwitch
            :model-value="store.sections.about.auto_check_update"
            :aria-label="t.about.versionCard.autoCheck.label"
            @update:model-value="(v) => store.updateSection('about', { ...store.sections.about, auto_check_update: v })"
          />
        </SettingsRow>
      </div>

      <div class="update-result">
        <Message v-if="updateState === 'up_to_date'" severity="success">
          {{ t.about.update.upToDate }}
        </Message>
        <Message v-else-if="updateState === 'skipped'" severity="info">
          {{ t.about.update.skipped }}
        </Message>
        <Message v-else-if="updateState === 'applying'" severity="info">
          {{ t.about.update.applying }}
        </Message>
        <Message v-else-if="updateState === 'error'" severity="error">
          {{ updateError || t.about.update.unavailable }}
        </Message>
        <div v-else-if="updateState === 'available' && updateResult" class="update-available">
          <Message severity="info">
            <div class="update-available-body">
              <p class="update-version">{{ t.about.update.newVersionPrefix }}{{ updateResult.version }}</p>
              <p v-if="updateResult.notes" class="update-notes">{{ updateResult.notes }}</p>
            </div>
          </Message>
          <div class="update-available-actions">
            <Button type="button" :label="t.about.update.applyNow" @click="onApplyUpdate" />
            <Button type="button" :label="t.about.update.later" severity="secondary" @click="onLater" />
            <Button type="button" :label="t.about.update.skip" severity="secondary" @click="onSkipVersion" />
          </div>
        </div>
      </div>
      <p v-if="infoError" class="about-error">{{ infoError }}</p>
    </section>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.about.changelog.title }}</h3>
      <div class="settings-card">
        <SettingsRow :title="t.about.changelog.title" :desc="t.about.changelog.desc">
          <Button
            type="button"
            icon="pi pi-history"
            :label="t.about.changelog.view"
            severity="secondary"
            @click="openExternal(LINKS.releases)"
          />
        </SettingsRow>
      </div>
    </section>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.about.exportLogs.title }}</h3>
      <div class="settings-card">
        <SettingsRow :title="t.about.exportLogs.title" :desc="t.about.exportLogs.desc">
          <Button
            type="button"
            icon="pi pi-file-export"
            :label="exporting ? t.about.exportLogs.exporting : t.about.exportLogs.button"
            :loading="exporting"
            severity="secondary"
            @click="onExportLogs"
          />
        </SettingsRow>
      </div>
      <div class="update-result">
        <Message v-if="exportMessage" :severity="exportMessage.ok ? 'success' : 'error'">
          {{ exportMessage.text }}
        </Message>
      </div>
    </section>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.about.legal.title }}</h3>
      <div class="settings-card">
        <SettingsRow v-for="item in legalItems" :key="item.key" :title="item.label">
          <Button
            type="button"
            :label="t.common.open"
            severity="link"
            :loading="legalLoading && legalDialog.title === item.label"
            @click="openLegal(item.key, item.label, item.markdown)"
          />
        </SettingsRow>
      </div>
    </section>

    <LegalTextDialog
      v-model:visible="legalDialog.visible"
      :title="legalDialog.title"
      :text="legalDialog.text"
      :markdown="legalDialog.markdown"
    />

    <Message v-if="store.lastError" severity="error" class="settings-error">{{ errorText }}</Message>
  </div>
</template>

<style scoped>
.about-actions {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
}

.update-result {
  margin-top: var(--vt-spacing-md);
}

.update-available-body {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-xs);
}

.update-version {
  margin: 0;
  font-weight: 600;
}

.update-notes {
  margin: 0;
  white-space: pre-wrap;
}

.update-available-actions {
  display: flex;
  gap: var(--vt-spacing-sm);
  margin-top: var(--vt-spacing-sm);
}

.about-error {
  margin: var(--vt-spacing-sm) 0 0;
  font-size: 12.5px;
  color: var(--vt-color-error);
}
</style>
