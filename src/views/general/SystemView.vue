<script setup>
/**
 * 通用 › 系统与数据（T-013，F-SET-05、PRD 3.8.6）：
 * 开机自启、保存语音数据集（+ 打开数据集目录）、数据目录展示（plugin-opener 打开）、诊断模式。
 * 打开外部目录一律走 @tauri-apps/plugin-opener（tech/03 3.3）。
 */
import { computed, onMounted, ref } from 'vue'
import Button from 'primevue/button'
import InputText from 'primevue/inputtext'
import ToggleSwitch from 'primevue/toggleswitch'
import Message from 'primevue/message'
import { openPath } from '@tauri-apps/plugin-opener'
import SettingsRow from '../../components/SettingsRow.vue'
import { useConfigStore } from '../../stores/config'
import { getAppInfo } from '../../api/about'
import { toAppError } from '../../api/client'
import { t } from '../../i18n'

const store = useConfigStore()

const appInfo = ref(null)
const infoError = ref('')

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

function updateGeneral(patch) {
  void store.updateSection('general', { ...store.sections.general, ...patch })
}

function openDir(path) {
  if (!path) return
  openPath(path).catch((error) => {
    store.lastError = toAppError(error)
  })
}

const datasetDir = computed(() => (appInfo.value ? `${appInfo.value.data_dir}/dataset` : undefined))

const dirs = computed(() =>
  appInfo.value
    ? [
        { label: t.general.system.dirs.config, path: appInfo.value.config_dir },
        { label: t.general.system.dirs.data, path: appInfo.value.data_dir },
        { label: t.general.system.dirs.logs, path: appInfo.value.log_dir },
      ]
    : [],
)

const errorText = computed(() =>
  store.lastError ? `${store.lastError.message}（${t.common.errorCode} ${store.lastError.code}）` : '',
)
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.general.system.title }}</h2>
    <p class="settings-page-desc">{{ t.general.system.desc }}</p>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.general.system.group }}</h3>
      <div class="settings-card">
        <SettingsRow
          :title="t.general.system.launchAtLogin.label"
          :desc="t.general.system.launchAtLogin.desc"
        >
          <ToggleSwitch
            :model-value="store.sections.general.launch_at_login"
            :aria-label="t.general.system.launchAtLogin.label"
            @update:model-value="(v) => updateGeneral({ launch_at_login: v })"
          />
        </SettingsRow>
        <SettingsRow
          :title="t.general.system.saveDataset.label"
          :desc="t.general.system.saveDataset.desc"
        >
          <div class="dir-row">
            <Button
              type="button"
              icon="pi pi-folder-open"
              severity="secondary"
              size="small"
              :label="t.general.system.saveDataset.openDir"
              :disabled="!datasetDir"
              @click="openDir(datasetDir)"
            />
            <ToggleSwitch
              :model-value="store.sections.general.save_voice_dataset"
              :aria-label="t.general.system.saveDataset.label"
              @update:model-value="(v) => updateGeneral({ save_voice_dataset: v })"
            />
          </div>
        </SettingsRow>
        <p v-if="store.sections.general.save_voice_dataset" class="dataset-hint">
          {{ t.general.system.saveDataset.hint }}
        </p>
        <SettingsRow :title="t.general.system.dirs.label" :desc="t.general.system.dirs.desc">
          <div class="dir-list">
            <div v-for="d in dirs" :key="d.label" class="dir-list-item">
              <span class="dir-list-label">{{ d.label }}</span>
              <InputText readonly class="dir-path" :model-value="d.path" :aria-label="d.label" />
              <Button
                type="button"
                severity="secondary"
                size="small"
                :label="t.general.system.dirs.open"
                @click="openDir(d.path)"
              />
            </div>
            <p v-if="infoError" class="dir-list-error">{{ infoError }}</p>
          </div>
        </SettingsRow>
        <SettingsRow
          :title="t.general.system.diagnostic.label"
          :desc="t.general.system.diagnostic.desc"
        >
          <ToggleSwitch
            :model-value="store.sections.general.diagnostic_mode"
            :aria-label="t.general.system.diagnostic.label"
            @update:model-value="(v) => updateGeneral({ diagnostic_mode: v })"
          />
        </SettingsRow>
      </div>
    </section>

    <Message v-if="store.lastError" severity="error" class="settings-error">{{ errorText }}</Message>
  </div>
</template>

<style scoped>
.dataset-hint {
  margin: 0;
  padding: 10px 20px;
  font-size: 12.5px;
  color: var(--vt-color-warning);
  border-top: 1px solid var(--vt-color-border);
}

.dir-list {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-sm);
}

.dir-list-item {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
}

.dir-list-label {
  width: 64px;
  flex-shrink: 0;
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.dir-list-error {
  margin: 0;
  font-size: 12.5px;
  color: var(--vt-color-error);
}
</style>
