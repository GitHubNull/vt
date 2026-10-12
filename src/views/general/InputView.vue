<script setup>
/**
 * 通用 › 输入与录音（T-013，F-INPUT-01/02 配置面）：
 * 录音快捷键（键位录制）、触发模式、取消快捷键、最长录音时长、悬浮球与提示音。
 * 快捷键保存即写 hotkey 配置，T-004 消费 config:changed 热注册，冲突报 E-HOTKEY-001。
 * 快捷键冲突检测（T-543，F-SYS-03）：录入即时反馈——应用内重复（录音键 / 取消键 /
 * 将来模板键）直接拒绝保存并指出冲突项（PRD 3.1.1 规则 7）；OS 常用组合（Ctrl+C、
 * Alt+F4、Win+组合等三平台清单）仅警告、允许保存；保存时的权威校验在后端
 * set_config 接入点（E-HOTKEY-003）。
 */
import { computed, onMounted, ref } from 'vue'
import Select from 'primevue/select'
import ToggleSwitch from 'primevue/toggleswitch'
import Message from 'primevue/message'
import HotkeyInput from '../../components/HotkeyInput.vue'
import SettingsRow from '../../components/SettingsRow.vue'
import { useConfigStore } from '../../stores/config'
import { t } from '../../i18n'
import {
  canonicalDisplayNames,
  findDuplicateGroups,
  hotkeyConflictFeedback,
  hotkeyPresetFeedback,
  validateHotkeys,
} from '../../api/hotkey'
import { formatCombo } from '../../utils/hotkey'

const store = useConfigStore()

onMounted(() => {
  void store.ensureLoaded()
})

function updateHotkey(patch) {
  return store.updateSection('hotkey', { ...store.sections.hotkey, ...patch })
}

/** 录入即时反馈（T-543）：error=应用内冲突（保存被拒绝）；warning=OS 预警（已保存）。 */
const hotkeyFeedback = ref(null)
/** 录入流水号：异步校验返回时丢弃过期结果（连续快速录入只保留最新一次）。 */
let hotkeyChangeSeq = 0

/** t.hotkey.conflict 文案（集成 i18n 条目前以可选链兜底，避免界面崩溃）。 */
const conflictLabels = computed(() => t.hotkey.conflict ?? {})

const presetLabels = computed(() => ({
  osPresetWarning: conflictLabels.value.osPresetWarning,
  osPresets: conflictLabels.value.osPresets,
  usageSep: conflictLabels.value.usageSep,
}))

/** 归一化组合显示名：规范修饰键名复用现有键位文案（Ctrl / Alt / Shift / Meta）。 */
const comboLabel = computed(() => {
  const names = canonicalDisplayNames(t.hotkey.keys)
  return (spec) => formatCombo(spec, names)
})

/** 当前两个可配置快捷键槽位（field 位置用待保存值替换）。 */
function currentSlots(field, value) {
  const hotkey = store.sections.hotkey
  return [
    { id: 'trigger', spec: field === 'trigger' ? value : hotkey.trigger },
    { id: 'cancel_trigger', spec: field === 'cancel_trigger' ? value : hotkey.cancel_trigger },
  ]
}

/**
 * 快捷键录入统一入口（录音键 / 取消键）：
 * 1. 本地同步判重（与后端同一归一化口径）→ 冲突立即反馈并拒绝保存；
 * 2. 后端权威校验（OS 常用组合预警表）→ 预警不拦截，保存后提示。
 */
async function onHotkeyChange(field, value) {
  const seq = ++hotkeyChangeSeq
  const slots = currentSlots(field, value)

  const conflict = hotkeyConflictFeedback(
    findDuplicateGroups(slots),
    conflictLabels.value,
    comboLabel.value,
  )
  if (conflict) {
    hotkeyFeedback.value = conflict
    return
  }

  // 后端权威校验（含三平台 OS 预警清单）；后端不可用时不阻断保存。
  let report = null
  try {
    report = await validateHotkeys(slots)
  } catch {
    report = null
  }
  if (seq !== hotkeyChangeSeq) return // 已有更新的录入，过期结果丢弃

  if (report && report.conflicts.length > 0) {
    hotkeyFeedback.value = hotkeyConflictFeedback(
      report.conflicts,
      conflictLabels.value,
      comboLabel.value,
    )
    return
  }

  const saved = await updateHotkey({ [field]: value })
  if (seq !== hotkeyChangeSeq) return
  if (!saved) return // 写盘失败：store.lastError 已提示
  hotkeyFeedback.value = report ? hotkeyPresetFeedback(report.warnings, presetLabels.value) : null
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
            @update:model-value="(v) => onHotkeyChange('trigger', v)"
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
            @update:model-value="(v) => onHotkeyChange('cancel_trigger', v)"
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
      <!-- 录入即时反馈（T-543）：冲突拒绝保存（error）/ OS 组合预警（warning）。 -->
      <Message
        v-if="hotkeyFeedback"
        :severity="hotkeyFeedback.level"
        :closable="true"
        class="settings-error"
        @close="hotkeyFeedback = null"
      >{{ hotkeyFeedback.text }}</Message>
    </section>

    <Message v-if="store.lastError" severity="error" class="settings-error">{{ errorText }}</Message>
  </div>
</template>
