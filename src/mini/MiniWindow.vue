<script setup>
/**
 * 语音输入小窗（T-009，F-INPUT-06、PRD 4.2.2）。
 * 布局对照 PRD 与参考截图 PixPin_2026-10-09_21-38-03.png：
 * 居中大麦克风按钮（直径 88）→ 提示文案 → 最近结果（单行）→ 底部设置按钮。
 * 关闭走原生标题栏（CloseRequested 已拦截为隐藏，PRD 3.15、T-009 验收 5）。
 * 与快捷键共用同一录音状态机：录音中按钮显示进行中，不可重复触发。
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { t } from '../i18n'
import { EVENTS } from '../api/events'
import { controlRecording } from '../api/audio'
import { getConfig } from '../api/config'
import { showSettingsWindow } from '../api/window'
import { formatCombo } from '../utils/hotkey'
import { smoothLevel } from '../float/animation'
import { resolveRecordAction, ringStyle, statusKey } from './logic'

/** 触发模式（配置 hotkey.mode；切换模式 = 点击翻转，按住模式 = 按下/松开）。 */
const mode = ref('hold')
/** 录音快捷键显示名（如"右 Alt"），空闲提示文案引用。 */
const triggerName = ref('')
/** 阶段：idle / recording / processing / feedback。 */
const phase = ref('idle')
/** 音量电平（平滑后，驱动音量环）。 */
const level = ref(0)
/** 最近识别结果（recording:result ok 文本，单行展示）。 */
const lastResult = ref('')
/** 反馈文案（失败时显示后端 message：发生了什么 + 怎么解决）。 */
const feedbackText = ref('')
/** 录音中（recording:state 驱动，互斥判定的权威状态）。 */
const recording = ref(false)

const statusText = computed(() => {
  if (phase.value === 'feedback') return feedbackText.value
  const key = statusKey(phase.value, mode.value)
  const template = key.split('.').reduce((obj, k) => obj?.[k], t) ?? key
  return typeof template === 'string' ? template.replace('{key}', triggerName.value) : key
})

const ring = computed(() => {
  const { dasharray, dashoffset } = ringStyle(level.value)
  return {
    strokeDasharray: `${dasharray}`,
    strokeDashoffset: `${dashoffset}`,
  }
})

let rawLevel = 0
let levelRaf = 0
let feedbackTimer = 0
const unlisteners = []

/** rAF 平滑音量环（事件 50 ms 节流写入，渲染侧 EMA 平滑）。 */
function pumpLevel() {
  level.value = smoothLevel(level.value, rawLevel)
  levelRaf = requestAnimationFrame(pumpLevel)
}

function send(action) {
  if (!action) return
  void controlRecording(action).catch(() => {})
}

function onMicPress(event) {
  event.currentTarget.setPointerCapture(event.pointerId)
  send(resolveRecordAction(mode.value, recording.value, 'press'))
}

function onMicRelease() {
  send(resolveRecordAction(mode.value, recording.value, 'release'))
}

function onMicClick() {
  send(resolveRecordAction(mode.value, recording.value, 'click'))
}

/** 取消按钮（T-009 范围；T-010 取消入口之一）。 */
function onCancel() {
  void controlRecording('cancel').catch(() => {})
}

function showFeedback(text, durationMs) {
  phase.value = 'feedback'
  feedbackText.value = text
  window.clearTimeout(feedbackTimer)
  feedbackTimer = window.setTimeout(() => {
    phase.value = 'idle'
    feedbackText.value = ''
  }, durationMs)
}

onMounted(async () => {
  levelRaf = requestAnimationFrame(pumpLevel)
  try {
    const config = await getConfig()
    mode.value = config.hotkey.mode
    triggerName.value = formatCombo(config.hotkey.trigger, t.hotkey.keys)
  } catch {
    /* 配置读取失败：保留默认按住模式与空键名 */
  }

  unlisteners.push(
    await listen(EVENTS.recordingState, (event) => {
      switch (event.payload.state) {
        case 'recording':
          phase.value = 'recording'
          recording.value = true
          mode.value = event.payload.mode ?? mode.value
          break
        case 'cancelled':
          recording.value = false
          showFeedback(t.mini.status.cancelled, 1200)
          break
        case 'idle':
        case 'warming':
          if (recording.value) phase.value = 'processing'
          recording.value = false
          break
        default:
          break
      }
    }),
    await listen(EVENTS.recordingLevel, (event) => {
      rawLevel = Math.min(1, Math.max(0, event.payload.level))
    }),
    await listen(EVENTS.recordingResult, (event) => {
      const payload = event.payload
      if (payload.status === 'ok') {
        lastResult.value = payload.text
        phase.value = 'idle'
      } else if (payload.status === 'failed') {
        showFeedback(payload.message, 4000)
      } else {
        // 降级：提示原因后恢复空闲。
        showFeedback(payload.reason, 2000)
      }
    }),
  )
})

onUnmounted(() => {
  cancelAnimationFrame(levelRaf)
  window.clearTimeout(feedbackTimer)
  for (const unlisten of unlisteners) unlisten()
})
</script>

<template>
  <div class="mini-root">
    <div class="mic-area" :class="{ recording: phase === 'recording' }">
      <svg class="ring" viewBox="0 0 96 96" aria-hidden="true">
        <circle class="ring-track" cx="48" cy="48" r="44" />
        <circle class="ring-value" cx="48" cy="48" r="44" :style="ring" />
      </svg>
      <button
        type="button"
        class="mic-btn"
        :class="{ active: phase === 'recording', busy: phase === 'processing' }"
        :disabled="phase === 'processing'"
        @pointerdown="onMicPress"
        @pointerup="onMicRelease"
        @click="onMicClick"
      >
        <svg viewBox="0 0 24 24" class="mic-icon" aria-hidden="true">
          <path
            d="M12 15a3.5 3.5 0 0 0 3.5-3.5v-5a3.5 3.5 0 1 0-7 0v5A3.5 3.5 0 0 0 12 15Zm6.5-3.5a6.5 6.5 0 0 1-13 0H4a8 8 0 0 0 7 7.94V22h2v-2.56A8 8 0 0 0 20 11.5h-1.5Z"
          />
        </svg>
      </button>
    </div>

    <p class="status" :class="phase">{{ statusText }}</p>

    <button v-if="recording" type="button" class="cancel-btn" @click="onCancel">
      {{ t.mini.cancel }}
    </button>

    <p v-if="lastResult" class="last-result">
      <span class="last-label">{{ t.mini.lastResult }}</span>
      <span class="last-text">{{ lastResult }}</span>
    </p>

    <div class="footer">
      <button
        type="button"
        class="square-btn"
        :aria-label="t.mini.openSettings"
        @click="showSettingsWindow"
      >
        <svg viewBox="0 0 24 24" class="gear-icon" aria-hidden="true">
          <path
            d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.488.488 0 0 0-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 0 0-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.503.503 0 0 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58ZM12 15.6a3.6 3.6 0 1 1 0-7.2 3.6 3.6 0 0 1 0 7.2Z"
          />
        </svg>
      </button>
    </div>
  </div>
</template>

<style scoped>
.mini-root {
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 100%;
  height: 100%;
  padding: 14px 16px 12px;
  box-sizing: border-box;
  background: #f7f8fa;
  color: #1f2329;
  user-select: none;
  -webkit-user-select: none;
}

/* 麦克风按钮区：88px 圆形（PRD 4.2.2），音量环环绕 */
.mic-area {
  position: relative;
  width: 96px;
  height: 96px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.ring {
  position: absolute;
  inset: 0;
  width: 96px;
  height: 96px;
  transform: rotate(-90deg);
}

.ring-track {
  fill: none;
  stroke: #e5e7eb;
  stroke-width: 3;
}

.ring-value {
  fill: none;
  stroke: #3b6ff6;
  stroke-width: 3;
  stroke-linecap: round;
  transition: stroke-dashoffset 90ms linear;
}

.mic-btn {
  width: 88px;
  height: 88px;
  border: none;
  border-radius: 50%;
  background: #eceef1;
  color: #4b5563;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background 150ms ease, color 150ms ease, transform 150ms ease;
}

.mic-btn:hover {
  background: #e2e5ea;
}

/* 录音中：主色底 + 呼吸动效（PRD 4.3） */
.mic-btn.active {
  background: #3b6ff6;
  color: #fff;
  animation: breathe 1.6s ease-in-out infinite;
}

/* 识别中：处理中动效（旋转的淡入淡出由禁用态表达） */
.mic-btn.busy {
  opacity: 0.55;
  cursor: default;
}

.mic-icon {
  width: 34px;
  height: 34px;
  fill: currentColor;
}

.status {
  margin: 10px 0 0;
  font-size: 13px;
  color: #6b7280;
  text-align: center;
  min-height: 18px;
}

.status.recording {
  color: #3b6ff6;
}

.status.feedback {
  color: #b45309;
}

.cancel-btn {
  margin-top: 8px;
  padding: 4px 14px;
  border: 1px solid #e5e7eb;
  border-radius: 14px;
  background: #fff;
  color: #dc2626;
  font-size: 12px;
  cursor: pointer;
}

.cancel-btn:hover {
  background: #fef2f2;
  border-color: #fca5a5;
}

/* 最近结果（单行，PRD 4.2.2） */
.last-result {
  display: flex;
  gap: 6px;
  align-items: baseline;
  width: 100%;
  margin: 12px 0 0;
  padding: 6px 10px;
  box-sizing: border-box;
  border-radius: 8px;
  background: #fff;
  border: 1px solid #eceef1;
  font-size: 12px;
}

.last-label {
  flex: none;
  color: #9ca3af;
}

.last-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: #374151;
}

.footer {
  display: flex;
  justify-content: flex-end;
  width: 100%;
  margin-top: auto;
}

/* 底部方形按钮（PRD 4.2.2；历史记录为 v1.0 功能，v0.1 仅设置入口） */
.square-btn {
  width: 36px;
  height: 36px;
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  background: #fff;
  color: #6b7280;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
}

.square-btn:hover {
  color: #3b6ff6;
  border-color: #c7d6fb;
}

.gear-icon {
  width: 18px;
  height: 18px;
  fill: currentColor;
}

@keyframes breathe {
  0%,
  100% {
    transform: scale(1);
  }
  50% {
    transform: scale(1.05);
  }
}
</style>
