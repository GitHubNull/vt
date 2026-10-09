<script setup>
/**
 * 通用 › 录音环境（T-013，F-SET-03、PRD 3.8.4）：
 * 麦克风选择（T-005 枚举命令）、录音时静音、麦克风常驻预热、移动麦克风占位卡片（v1.0）。
 */
import { computed, onMounted, ref } from 'vue'
import Button from 'primevue/button'
import Select from 'primevue/select'
import ToggleSwitch from 'primevue/toggleswitch'
import Message from 'primevue/message'
import SettingsRow from '../../components/SettingsRow.vue'
import { useConfigStore } from '../../stores/config'
import { listAudioDevices } from '../../api/audio'
import { toAppError } from '../../api/client'
import { t } from '../../i18n'

const store = useConfigStore()

/** 系统默认的选项值哨兵（Select 不支持 null 选项值）。 */
const DEFAULT_DEVICE_VALUE = ''

const devices = ref([])
const devicesLoaded = ref(false)

onMounted(() => {
  void store.ensureLoaded().then(refreshDevices)
})

async function refreshDevices() {
  try {
    devices.value = await listAudioDevices()
    devicesLoaded.value = true
  } catch (error) {
    store.lastError = toAppError(error)
  }
}

const micOptions = computed(() => [
  { label: t.general.recording.mic.default, value: DEFAULT_DEVICE_VALUE },
  ...devices.value.map((d) => ({ label: d.name, value: d.id })),
])

function updateAudio(patch) {
  void store.updateSection('audio', { ...store.sections.audio, ...patch })
}

/** Select 值为哨兵时映射回 null（系统默认）。 */
function updateMic(value) {
  updateAudio({ input_device_id: value === DEFAULT_DEVICE_VALUE ? null : value })
}

const micValue = computed(() => store.sections.audio.input_device_id ?? DEFAULT_DEVICE_VALUE)

const errorText = computed(() =>
  store.lastError ? `${store.lastError.message}（${t.common.errorCode} ${store.lastError.code}）` : '',
)
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.general.recording.title }}</h2>
    <p class="settings-page-desc">{{ t.general.recording.desc }}</p>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.general.recording.group }}</h3>
      <div class="settings-card">
        <SettingsRow :title="t.general.recording.mic.label" :desc="t.general.recording.mic.desc">
          <div class="dir-row">
            <Select
              :model-value="micValue"
              :options="micOptions"
              option-label="label"
              option-value="value"
              :placeholder="devicesLoaded ? t.general.recording.mic.empty : '…'"
              :aria-label="t.general.recording.mic.label"
              @update:model-value="updateMic"
            />
            <Button
              type="button"
              icon="pi pi-refresh"
              severity="secondary"
              size="small"
              :aria-label="t.general.recording.mic.refreshAria"
              @click="refreshDevices"
            />
          </div>
        </SettingsRow>
        <SettingsRow :title="t.general.recording.mute.label" :desc="t.general.recording.mute.desc">
          <ToggleSwitch
            :model-value="store.sections.audio.mute_speaker_while_recording"
            :aria-label="t.general.recording.mute.label"
            @update:model-value="(v) => updateAudio({ mute_speaker_while_recording: v })"
          />
        </SettingsRow>
        <SettingsRow :title="t.general.recording.keepWarm.label" :desc="t.general.recording.keepWarm.desc">
          <ToggleSwitch
            :model-value="store.sections.audio.keep_warm"
            :aria-label="t.general.recording.keepWarm.label"
            @update:model-value="(v) => updateAudio({ keep_warm: v })"
          />
        </SettingsRow>
        <SettingsRow :title="t.general.recording.mobileMic.label" :desc="t.general.recording.mobileMic.desc">
          <span class="mobile-mic-badge">{{ t.general.recording.mobileMic.note }}</span>
        </SettingsRow>
      </div>
    </section>

    <Message v-if="store.lastError" severity="error" class="settings-error">{{ errorText }}</Message>
  </div>
</template>

<style scoped>
.mobile-mic-badge {
  font-size: 12px;
  color: var(--vt-color-text-muted);
  background: var(--vt-color-nav-active-bg);
  border-radius: 999px;
  padding: 4px 12px;
}
</style>
