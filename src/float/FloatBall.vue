<script setup>
/**
 * 录音悬浮球（T-008，F-INPUT-05、PRD 4.2.3 / 4.3）。
 * 胶囊形：波形动效（Canvas，RMS 驱动）+ 时长 + 模式标签 + 取消按钮。
 * 零打扰（产品原则 2）：页面无任何可聚焦控件（div/span 不在 Tab 序列），
 * 窗口声明 focusable:false（tech/09 9.2.4），不出现、不夺取输入焦点。
 * 交互：拖动移动（位置记忆）；拖出阈值或点 ✕ 取消（T-010 取消入口之一）。
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { t } from '../i18n'
import { EVENTS } from '../api/events'
import { controlRecording } from '../api/audio'
import { getConfig } from '../api/config'
import { hideFloat, moveFloat } from '../api/window'
import {
  WAVE_BAR_COUNT,
  WAVE_BAR_GAP,
  breathScale,
  dotOpacity,
  feedbackDurationMs,
  formatElapsed,
  isCancelDrag,
  isSignificantDrag,
  remainingSeconds,
  smoothLevel,
  waveHeight,
} from './animation'

/** 阶段：boot（等待首个事件）→ recording → processing → feedback。 */
const phase = ref('boot')
/** 反馈类型：success / degraded / failed / cancelled。 */
const feedbackKind = ref('success')
/** 触发模式（recording:state 事件携带）：hold / toggle。 */
const mode = ref(null)
/** 平滑后的音量电平（0~1），驱动波形与呼吸。 */
const level = ref(0)
/** 录音时长（ms，rAF 计时）。 */
const elapsedMs = ref(0)
/** 最长录音时长（分钟，用于即将超时倒计时，PRD 4.3）。 */
const maxMinutes = ref(5)

const modeLabel = computed(() => {
  if (mode.value === 'toggle') return t.float.modeToggle
  if (mode.value === 'hold') return t.float.modeHold
  return ''
})

const durationText = computed(() => formatElapsed(elapsedMs.value))

/** 倒计时秒数（临近上限时橙色显示；充足时为 null）。 */
const countdown = computed(() => remainingSeconds(elapsedMs.value, maxMinutes.value))

const feedbackText = computed(() => {
  switch (feedbackKind.value) {
    case 'success':
      return t.float.feedback.success
    case 'degraded':
      return t.float.feedback.degraded
    case 'failed':
      return t.float.feedback.failed
    case 'cancelled':
      return t.float.feedback.cancelled
    default:
      return ''
  }
})

// ---- 波形 Canvas（录音中渲染，rAF 驱动） ----
const waveCanvas = ref(null)
let rafId = 0
let startedAt = 0
/** 最近一次原始电平（事件驱动写入，rAF 读取做 EMA 平滑）。 */
let rawLevel = 0

function drawFrame(now) {
  rafId = requestAnimationFrame(drawFrame)
  elapsedMs.value = phase.value === 'recording' ? now - startedAt : elapsedMs.value
  level.value = smoothLevel(level.value, rawLevel)

  const canvas = waveCanvas.value
  if (!canvas || phase.value === 'boot') return
  const ctx = canvas.getContext('2d')
  const dpr = window.devicePixelRatio || 1
  const cssWidth = canvas.clientWidth
  const cssHeight = canvas.clientHeight
  if (canvas.width !== cssWidth * dpr) canvas.width = cssWidth * dpr
  if (canvas.height !== cssHeight * dpr) canvas.height = cssHeight * dpr
  ctx.clearRect(0, 0, canvas.width, canvas.height)

  // 呼吸底环：静音时提示"正在听"（PRD 4.5.4）。
  const scale = breathScale(now)
  ctx.beginPath()
  ctx.fillStyle = 'rgba(255, 255, 255, 0.18)'
  ctx.arc(
    (cssWidth / 2) * dpr,
    (cssHeight / 2) * dpr,
    (Math.min(cssWidth, cssHeight) / 2 - 2) * scale * dpr,
    0,
    Math.PI * 2,
  )
  ctx.fill()

  // 波形条：音量驱动 + 逐条相位（animation.ts 纯函数）。
  const barWidth = Math.floor((cssWidth - WAVE_BAR_GAP * (WAVE_BAR_COUNT - 1)) / WAVE_BAR_COUNT)
  ctx.fillStyle = 'rgba(255, 255, 255, 0.92)'
  for (let i = 0; i < WAVE_BAR_COUNT; i += 1) {
    const h = waveHeight(level.value, i, now) * (cssHeight - 6)
    const x = i * (barWidth + WAVE_BAR_GAP)
    const y = (cssHeight - h) / 2
    ctx.fillRect(x * dpr, y * dpr, barWidth * dpr, h * dpr)
  }

  // 识别中：三点加载（PRD 4.3）。
  if (phase.value === 'processing') {
    ctx.clearRect(0, 0, canvas.width, canvas.height)
    for (let i = 0; i < 3; i += 1) {
      ctx.beginPath()
      ctx.fillStyle = `rgba(255, 255, 255, ${dotOpacity(i, now).toFixed(3)})`
      ctx.arc(
        (cssWidth / 2 + (i - 1) * 10) * dpr,
        (cssHeight / 2) * dpr,
        2.5 * dpr,
        0,
        Math.PI * 2,
      )
      ctx.fill()
    }
  }
}

// ---- 反馈与隐藏 ----
let feedbackTimer = 0

/** 反馈停留结束后隐藏窗口（等效销毁，T-008）。 */
function scheduleHide() {
  window.clearTimeout(feedbackTimer)
  feedbackTimer = window.setTimeout(() => {
    void hideFloat().catch(() => {})
    phase.value = 'boot'
  }, feedbackDurationMs(feedbackKind.value))
}

function enterFeedback(kind) {
  feedbackKind.value = kind
  phase.value = 'feedback'
  scheduleHide()
}

// ---- 拖动（移动 + 取消区） ----
const dragOffset = ref({ x: 0, y: 0 })
const dragCancelArmed = ref(false)
let dragStart = null
let dragBase = { x: 0, y: 0 }

async function onPointerDown(event) {
  if (phase.value !== 'recording') return
  dragStart = { x: event.clientX, y: event.clientY }
  dragCancelArmed.value = false
  dragOffset.value = { x: 0, y: 0 }
  try {
    dragBase = await getCurrentWindow().outerPosition()
  } catch {
    dragBase = { x: 0, y: 0 }
  }
  event.currentTarget.setPointerCapture(event.pointerId)
}

function onPointerMove(event) {
  if (!dragStart) return
  const dx = event.clientX - dragStart.x
  const dy = event.clientY - dragStart.y
  dragOffset.value = { x: dx, y: dy }
  dragCancelArmed.value = isCancelDrag(dx, dy)
}

function onPointerUp() {
  if (!dragStart) return
  const { x: dx, y: dy } = dragOffset.value
  dragStart = null
  const cancel = dragCancelArmed.value
  dragCancelArmed.value = false
  dragOffset.value = { x: 0, y: 0 }
  if (cancel) {
    // 拖动到取消区：放弃本次输入（T-010，音频即弃）。
    void controlRecording('cancel').catch(() => {})
  } else if (isSignificantDrag(dx, dy)) {
    // 有效拖动：移动窗口并记忆位置（T-008 验收：位置下次保持）。
    void moveFloat(dragBase.x + dx, dragBase.y + dy).catch(() => {})
  }
}

/** 点击 ✕ 取消（不可聚焦控件，仅指针交互）。 */
function onCancelClick() {
  void controlRecording('cancel').catch(() => {})
}

// ---- 事件接入 ----
const unlisteners = []
let watchdog = 0

onMounted(async () => {
  rafId = requestAnimationFrame(drawFrame)

  // 最长录音时长：用于即将超时倒计时。
  try {
    const config = await getConfig()
    maxMinutes.value = config.hotkey.max_recording_minutes
  } catch {
    /* 配置读取失败：保留默认 5 分钟 */
  }

  // 兜底：窗口被显示却未收到任何录音事件（异常路径）时自行隐藏，避免挡交互。
  watchdog = window.setTimeout(() => {
    if (phase.value === 'boot') void hideFloat().catch(() => {})
  }, 5000)

  unlisteners.push(
    await listen(EVENTS.recordingState, (event) => {
      const payload = event.payload
      switch (payload.state) {
        case 'recording':
          window.clearTimeout(watchdog)
          phase.value = 'recording'
          mode.value = payload.mode ?? null
          startedAt = performance.now()
          elapsedMs.value = 0
          rawLevel = 0
          level.value = 0
          break
        case 'cancelled':
          // T-010：已取消反馈（0.5 s 后淡出，PRD 4.3）。
          window.clearTimeout(watchdog)
          if (phase.value !== 'boot') enterFeedback('cancelled')
          break
        case 'idle':
        case 'warming':
          // 录音结束进入识别：show 三点加载，等待 recording:result。
          if (phase.value === 'recording') phase.value = 'processing'
          break
        default:
          break
      }
    }),
    await listen(EVENTS.recordingLevel, (event) => {
      rawLevel = Math.min(1, Math.max(0, event.payload.level))
    }),
    await listen(EVENTS.recordingResult, (event) => {
      window.clearTimeout(watchdog)
      const payload = event.payload
      if (payload.status === 'ok') enterFeedback('success')
      else if (payload.status === 'degraded') enterFeedback('degraded')
      else enterFeedback('failed')
    }),
  )
})

onUnmounted(() => {
  cancelAnimationFrame(rafId)
  window.clearTimeout(feedbackTimer)
  window.clearTimeout(watchdog)
  for (const unlisten of unlisteners) unlisten()
})
</script>

<template>
  <!-- tabindex 全缺省：页面无任何可聚焦控件，Tab 无法聚焦（T-008 验收）。 -->
  <div class="float-stage">
    <div
      class="capsule"
      :class="[`is-${phase}`, `fb-${feedbackKind}`, { 'drag-cancel': dragCancelArmed }]"
      :style="
        dragOffset.x || dragOffset.y
          ? { transform: `translate(${dragOffset.x}px, ${dragOffset.y}px)` }
          : undefined
      "
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
    >
      <canvas ref="waveCanvas" class="wave"></canvas>

      <span v-if="phase === 'recording'" class="duration" :class="{ countdown }">
        {{ countdown !== null ? countdown : durationText }}
      </span>
      <span v-if="phase === 'processing'" class="processing-text">{{ t.float.processing }}</span>
      <span v-if="phase === 'feedback'" class="fb-icon" aria-hidden="true">
        <template v-if="feedbackKind === 'success'">✓</template>
        <template v-else-if="feedbackKind === 'cancelled'">×</template>
        <template v-else>!</template>
      </span>
      <span v-if="phase === 'feedback' && feedbackText" class="fb-text">{{ feedbackText }}</span>

      <span v-if="modeLabel && phase === 'recording'" class="mode-tag">{{ modeLabel }}</span>

      <div
        v-if="phase === 'recording'"
        class="cancel-btn"
        role="presentation"
        @pointerdown.stop
        @click="onCancelClick"
      >
        ✕
      </div>
    </div>
    <div v-if="dragCancelArmed" class="drag-hint">{{ t.float.dragCancelHint }}</div>
  </div>
</template>

<style scoped>
.float-stage {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  width: 100%;
  height: 100%;
  user-select: none;
  -webkit-user-select: none;
}

/* 胶囊（PRD 4.2.3：高约 40，宽度随内容自适应 120–280） */
.capsule {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 40px;
  min-width: 120px;
  max-width: 280px;
  padding: 0 10px;
  border-radius: 20px;
  background: rgba(59, 111, 246, 0.94);
  color: #fff;
  font-size: 13px;
  box-shadow: 0 2px 10px rgba(30, 64, 175, 0.35);
  cursor: grab;
  touch-action: none;
  animation: fade-in 150ms ease-out;
  transition: background 120ms ease;
}

.capsule:active {
  cursor: grabbing;
}

/* 拖动到取消区：整球变红提示"松开取消" */
.capsule.drag-cancel {
  background: rgba(220, 38, 38, 0.95);
}

.wave {
  width: 40px;
  height: 24px;
  flex: none;
}

.duration {
  font-variant-numeric: tabular-nums;
  min-width: 34px;
  text-align: center;
}

/* 即将超时：倒计时橙色（PRD 4.3） */
.duration.countdown {
  color: #fbbf24;
  font-weight: 600;
}

.processing-text {
  font-size: 12px;
  opacity: 0.85;
}

.fb-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  font-size: 12px;
  font-weight: 700;
  background: rgba(255, 255, 255, 0.25);
}

.fb-success {
  background: rgba(22, 163, 74, 0.95);
}

.fb-degraded {
  background: rgba(217, 119, 6, 0.95);
}

.fb-failed {
  background: rgba(220, 38, 38, 0.95);
}

.fb-cancelled {
  background: rgba(107, 114, 128, 0.95);
}

.fb-text {
  font-size: 12px;
  white-space: nowrap;
}

.mode-tag {
  padding: 1px 6px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.22);
  font-size: 11px;
  white-space: nowrap;
}

/* 取消按钮：非聚焦控件，仅指针交互 */
.cancel-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  flex: none;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.18);
  font-size: 11px;
  line-height: 1;
  cursor: pointer;
}

.cancel-btn:hover {
  background: rgba(255, 255, 255, 0.38);
}

.drag-hint {
  padding: 2px 10px;
  border-radius: 10px;
  background: rgba(220, 38, 38, 0.9);
  color: #fff;
  font-size: 11px;
}

@keyframes fade-in {
  from {
    opacity: 0;
    transform: scale(0.92);
  }
  to {
    opacity: 1;
    transform: scale(1);
  }
}
</style>
