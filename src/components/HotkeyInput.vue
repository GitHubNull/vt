<script setup>
/**
 * 快捷键录制输入框（T-013，PRD 4.4“快捷键显示框”）：
 * 聚焦后捕获下一次按键 / 鼠标侧键，转为规范组合字符串（如 ControlLeft+ShiftLeft+Space）。
 * 单独的修饰键（如右 Alt）按下并松开即作为热键（默认录音键 AltRight）。
 * 保存即写配置，冲突等错误由后端 set_config 返回（E-HOTKEY-001 等）。
 */
import { computed, ref } from 'vue'
import Button from 'primevue/button'
import InputText from 'primevue/inputtext'
import { t } from '../i18n'
import {
  comboFromKeyEvent,
  comboFromModifierKeyUp,
  comboFromMouseButton,
  formatCombo,
  isModifierCode,
} from '../utils/hotkey'

const props = defineProps({
  modelValue: { type: String, required: true },
  /** 是否把 Esc 捕获为有效组合（取消录音快捷键用）；默认 false = Esc 退出录制。 */
  captureEscape: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
  inputAriaLabel: { type: String, default: '' },
})

const emit = defineEmits(['update:modelValue'])

const recording = ref(false)
/** 录制期间是否已按下过主键（区分“修饰键单独成键”与“修饰键组合等待中”）。 */
let mainKeyPressed = false

function startRecording() {
  mainKeyPressed = false
  recording.value = true
}

function stopRecording() {
  recording.value = false
}

const isMac =
  typeof navigator !== 'undefined' && /mac/i.test(navigator.platform || navigator.userAgent || '')

const keyNames = computed(() => {
  const names = { ...t.hotkey.keys }
  if (isMac) {
    names.AltRight = t.hotkey.keys.AltRightMac
    names.MetaLeft = t.hotkey.keys.MetaLeftMac
  }
  return names
})

const display = computed(() =>
  props.modelValue ? formatCombo(props.modelValue, keyNames.value) : '',
)

function onKeydown(event) {
  if (!recording.value || props.disabled) return
  event.preventDefault()
  event.stopPropagation()
  if (event.code === 'Escape' && !props.captureEscape) {
    stopRecording()
    return
  }
  if (isModifierCode(event.code)) return // 修饰键：等待主键，或在 keyup 时单独成键
  mainKeyPressed = true
  const combo = comboFromKeyEvent(event)
  if (!combo) return
  recording.value = false
  emit('update:modelValue', combo)
}

/** 修饰键单独按下并松开（未组合其他键）→ 以该修饰键自身为热键（如右 Alt）。 */
function onKeyup(event) {
  if (!recording.value || props.disabled) return
  if (mainKeyPressed) return
  const combo = comboFromModifierKeyUp(event)
  if (!combo) return
  event.preventDefault()
  recording.value = false
  emit('update:modelValue', combo)
}

function onMouseDown(event) {
  if (!recording.value || props.disabled) return
  const combo = comboFromMouseButton(event.button)
  if (!combo) return
  event.preventDefault()
  recording.value = false
  emit('update:modelValue', combo)
}

function toggleRecording() {
  if (props.disabled) return
  if (recording.value) {
    stopRecording()
  } else {
    startRecording()
  }
}
</script>

<template>
  <div class="hotkey-input" @keydown="onKeydown" @keyup="onKeyup" @mousedown="onMouseDown">
    <InputText
      readonly
      class="hotkey-input-field"
      :model-value="recording ? t.general.input.hotkeyRecording : display"
      :placeholder="t.general.input.hotkeyPlaceholder"
      :disabled="disabled"
      :aria-label="inputAriaLabel"
      @focus="!disabled && startRecording()"
    />
    <Button
      type="button"
      size="small"
      :severity="recording ? 'danger' : 'secondary'"
      :label="recording ? t.common.cancel : t.general.input.hotkeyChange"
      @click="toggleRecording"
    />
  </div>
</template>

<style scoped>
.hotkey-input {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
}

.hotkey-input-field {
  width: 220px;
  text-align: center;
  cursor: pointer;
  font-family: ui-monospace, 'Cascadia Mono', Menlo, Consolas, monospace;
}
</style>
