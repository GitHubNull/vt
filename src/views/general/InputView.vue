<script setup>
/**
 * 通用 › 输入与录音（T-013，F-INPUT-01/02 配置面）：
 * 录音快捷键（键位录制）、触发模式、取消快捷键、最长录音时长、悬浮球与提示音。
 * 快捷键保存即写 hotkey 配置，T-004 消费 config:changed 热注册，冲突报 E-HOTKEY-001。
 */
import { computed, onMounted } from 'vue'
import Select from 'primevue/select'
import ToggleSwitch from 'primevue/toggleswitch'
import Message from 'primevue/message'
import HotkeyInput from '../../components/HotkeyInput.vue'
import SettingsRow from '../../components/SettingsRow.vue'
import { useConfigStore } from '../../stores/config'
import { t } from '../../i18n'

const store = useConfigStore()

onMounted(() => {
  void store.ensureLoaded()
})

function updateHotkey(patch) {
  void store.updateSection('hotkey', { ...store.sections.hotkey, ...patch })
}

const modeOptions = computed(() => [
  { label: t.general.input.recordMode.options.hold, value: 'hold' },
  { label: t.general.input.recordMode.options.toggle, value: 'toggle' },
])

const durationOptions = computed(() =>
  Array.from({ length: 10 }, (_, i) => {
    const minutes = i + 1
    return { label: `${minutes} ${t.general.input.maxDuration.unit}`, value: minutes }
  }),
)

const floatBallPositionOptions = computed(() => [
  { label: t.general.input.floatBallPosition.options.cursor, value: 'cursor' },
  { label: t.general.input.floatBallPosition.options.bottomCenter, value: 'bottom_center' },
  { label: t.general.input.floatBallPosition.options.fixed, value: 'fixed' },
])

const errorText = computed(() =>
  store.lastError ? `${store.lastError.message}（${t.common.errorCode} ${store.lastError.code}）` : '',
)
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.general.input.title }}</h2>
    <p class="settings-page-desc">{{ t.general.input.desc }}</p>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.general.input.group }}</h3>
      <div class="settings-card">
        <SettingsRow
          :title="t.general.input.recordHotkey.label"
          :desc="t.general.input.recordHotkey.desc"
        >
          <HotkeyInput
            :model-value="store.sections.hotkey.trigger"
            :input-aria-label="t.general.input.recordHotkey.label"
            @update:model-value="(v) => updateHotkey({ trigger: v })"
          />
        </SettingsRow>
        <SettingsRow :title="t.general.input.recordMode.label" :desc="t.general.input.recordMode.desc">
          <Select
            :model-value="store.sections.hotkey.mode"
            :options="modeOptions"
            option-label="label"
            option-value="value"
            :aria-label="t.general.input.recordMode.label"
            @update:model-value="(v) => updateHotkey({ mode: v })"
          />
        </SettingsRow>
        <SettingsRow
          :title="t.general.input.cancelHotkey.label"
          :desc="t.general.input.cancelHotkey.desc"
        >
          <HotkeyInput
            :model-value="store.sections.hotkey.cancel_trigger"
            :capture-escape="true"
            :input-aria-label="t.general.input.cancelHotkey.label"
            @update:model-value="(v) => updateHotkey({ cancel_trigger: v })"
          />
        </SettingsRow>
        <SettingsRow
          :title="t.general.input.maxDuration.label"
          :desc="t.general.input.maxDuration.desc"
        >
          <Select
            :model-value="store.sections.hotkey.max_recording_minutes"
            :options="durationOptions"
            option-label="label"
            option-value="value"
            :aria-label="t.general.input.maxDuration.label"
            @update:model-value="(v) => updateHotkey({ max_recording_minutes: v })"
          />
        </SettingsRow>
        <SettingsRow :title="t.general.input.floatBall.label" :desc="t.general.input.floatBall.desc">
          <ToggleSwitch
            :model-value="store.sections.hotkey.show_float_ball"
            :aria-label="t.general.input.floatBall.label"
            @update:model-value="(v) => updateHotkey({ show_float_ball: v })"
          />
        </SettingsRow>
        <SettingsRow
          :title="t.general.input.floatBallPosition.label"
          :desc="t.general.input.floatBallPosition.desc"
        >
          <Select
            :model-value="store.sections.hotkey.float_ball_position"
            :options="floatBallPositionOptions"
            option-label="label"
            option-value="value"
            :aria-label="t.general.input.floatBallPosition.label"
            @update:model-value="(v) => updateHotkey({ float_ball_position: v })"
          />
        </SettingsRow>
        <SettingsRow
          :title="t.general.input.soundFeedback.label"
          :desc="t.general.input.soundFeedback.desc"
        >
          <ToggleSwitch
            :model-value="store.sections.hotkey.sound_hint"
            :aria-label="t.general.input.soundFeedback.label"
            @update:model-value="(v) => updateHotkey({ sound_hint: v })"
          />
        </SettingsRow>
      </div>
    </section>

    <Message v-if="store.lastError" severity="error" class="settings-error">{{ errorText }}</Message>
  </div>
</template>
