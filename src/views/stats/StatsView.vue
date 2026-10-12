<script setup>
/**
 * 统计（T-541，F-STAT-01、PRD 3.10）。
 * 概览卡片（字数 / 录音时长 / 估算节省时间 / 平均语速 / AI 次数）+ 按天趋势 +
 * 场景 / 口令 / 模板 / 词典命中分布；清空需二次确认（plugin-dialog）。
 * 隐私：统计只含计数与时长，页面不展示任何输入内容（PRD 3.10 业务规则 2）。
 */
import { computed, onMounted, ref } from 'vue'
import Button from 'primevue/button'
import InputNumber from 'primevue/inputnumber'
import Message from 'primevue/message'
import SelectButton from 'primevue/selectbutton'
import ToggleSwitch from 'primevue/toggleswitch'
import { confirm } from '@tauri-apps/plugin-dialog'
import { statsClear, statsSummary } from '../../api/stats'
import { toAppError } from '../../api/client'
import { useConfigStore } from '../../stores/config'
import { t } from '../../i18n'

const configStore = useConfigStore()

const RANGE_OPTIONS = [
  { value: 'today', label: t.stats.range.today },
  { value: 'week', label: t.stats.range.week },
  { value: 'month', label: t.stats.range.month },
  { value: 'all', label: t.stats.range.all },
]

const range = ref('today')
const summary = ref(null)
const loading = ref(false)
const lastError = ref(null)
const clearNotice = ref(null)
/** 估算节省时间用打字速度（PRD 3.10：默认 60 字/分钟，可在页面上修改）。 */
const typingSpeed = ref(60)

async function load() {
  loading.value = true
  lastError.value = null
  try {
    summary.value = await statsSummary(range.value)
  } catch (error) {
    lastError.value = toAppError(error)
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  void load()
  void configStore.ensureLoaded()
})

function onRangeChange(value) {
  if (value) {
    range.value = value
    void load()
  }
}

const totals = computed(() => summary.value?.totals ?? null)

/** 录音分钟数（一位小数；0 显示为 0）。 */
const recordingMinutes = computed(() => {
  if (!totals.value) return 0
  return Math.round((totals.value.recording_ms / 60000) * 10) / 10
})

/** 估算节省时间（分钟）= 字数 ÷ 打字速度 − 录音时长，负值按 0 计。 */
const savedMinutes = computed(() => {
  if (!totals.value || typingSpeed.value <= 0) return 0
  const byTyping = totals.value.chars / typingSpeed.value
  return Math.max(0, Math.round((byTyping - recordingMinutes.value) * 10) / 10)
})

/** 平均语速（字 / 分钟）= 字数 ÷ 录音分钟数。 */
const speechRate = computed(() => {
  if (!totals.value || recordingMinutes.value <= 0) return 0
  return Math.round(totals.value.chars / recordingMinutes.value)
})

function formatMinutes(minutes) {
  return t.stats.minutesValue.replace('{count}', String(minutes))
}

/** 词典命中条形宽度（按当前范围内最大命中数归一化）。 */
function dictHitStyle(hit) {
  const hits = summary.value?.dict_hits ?? []
  const max = Math.max(1, ...hits.map((item) => item.count))
  return { width: `${Math.round((hit.count / max) * 100)}%` }
}

function formatSpeed(count) {
  return t.stats.speedValue.replace('{count}', String(count))
}

/** 趋势柱高度百分比（按最大值归一化；空数据时不渲染）。 */
const trendBars = computed(() => {
  const days = summary.value?.days ?? []
  const max = Math.max(1, ...days.map((day) => day.chars))
  return days.map((day) => ({
    ...day,
    heightPercent: Math.round((day.chars / max) * 100),
  }))
})

/** 场景分布（含百分比与展示名）。 */
const sceneBars = computed(() => {
  const scenes = summary.value?.scenes ?? []
  const total = scenes.reduce((sum, item) => sum + item.count, 0)
  return scenes.map((item) => ({
    ...item,
    label: t.stats.sceneNames[item.name] ?? item.name,
    percent: total > 0 ? Math.round((item.count / total) * 100) : 0,
  }))
})

async function onClear() {
  const ok = await confirm(t.stats.clearConfirm, {
    title: t.stats.clearTitle,
    kind: 'warning',
  })
  if (!ok) return
  clearNotice.value = null
  try {
    await statsClear()
    clearNotice.value = { ok: true, text: t.stats.clearDone }
    await load()
  } catch (error) {
    const appError = toAppError(error)
    clearNotice.value = {
      ok: false,
      text: `${t.stats.clearFailed}（${t.common.errorCode} ${appError.code}）`,
    }
  }
}

function updateAnonymousUpload(enabled) {
  void configStore.updateSection('stats', {
    ...configStore.sections.stats,
    anonymous_upload: enabled,
  })
}
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.stats.title }}</h2>
    <p class="settings-page-desc">{{ t.stats.desc }}</p>

    <div class="stats-toolbar">
      <SelectButton
        :model-value="range"
        :options="RANGE_OPTIONS"
        option-value="value"
        option-label="label"
        :allow-empty="false"
        :aria-label="t.stats.title"
        @update:model-value="onRangeChange"
      />
      <div class="stats-speed">
        <label class="stats-speed-label" for="stats-typing-speed">
          {{ t.stats.typingSpeedLabel }}
        </label>
        <InputNumber
          id="stats-typing-speed"
          v-model="typingSpeed"
          :min="10"
          :max="300"
          :suffix="` ${t.stats.typingSpeedUnit}`"
          :disabled="!totals"
        />
      </div>
    </div>

    <Message v-if="lastError" severity="error" class="settings-error">
      {{ lastError.message }}（{{ t.common.errorCode }} {{ lastError.code }}）
    </Message>

    <section class="settings-group">
      <div class="settings-card stats-cards">
        <div class="stats-card">
          <span class="stats-card-value">{{ totals ? totals.chars : '—' }}</span>
          <span class="stats-card-label">{{ t.stats.cards.chars }}</span>
        </div>
        <div class="stats-card">
          <span class="stats-card-value">
            {{ totals ? formatMinutes(recordingMinutes) : '—' }}
          </span>
          <span class="stats-card-label">{{ t.stats.cards.recordingTime }}</span>
        </div>
        <div class="stats-card">
          <span class="stats-card-value">
            {{ totals ? formatMinutes(savedMinutes) : '—' }}
          </span>
          <span class="stats-card-label">{{ t.stats.cards.savedTime }}</span>
        </div>
        <div class="stats-card">
          <span class="stats-card-value">{{ totals ? formatSpeed(speechRate) : '—' }}</span>
          <span class="stats-card-label">{{ t.stats.cards.speed }}</span>
        </div>
        <div class="stats-card">
          <span class="stats-card-value">{{ totals ? totals.ai_uses : '—' }}</span>
          <span class="stats-card-label">{{ t.stats.cards.aiUses }}</span>
        </div>
      </div>
      <p class="stats-formula">{{ t.stats.savedFormula }}</p>
    </section>

    <section v-if="trendBars.length" class="settings-group">
      <h3 class="settings-group-title">{{ t.stats.trend }}</h3>
      <div class="settings-card stats-trend">
        <div
          v-for="day in trendBars"
          :key="day.date"
          class="stats-trend-col"
          :title="`${day.date} · ${day.chars}`"
        >
          <span class="stats-trend-value">{{ day.chars }}</span>
          <div class="stats-trend-bar-track">
            <div class="stats-trend-bar" :style="{ height: `${day.heightPercent}%` }" />
          </div>
          <span class="stats-trend-date">{{ day.date.slice(5) }}</span>
        </div>
      </div>
    </section>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.stats.scenes }}</h3>
      <div class="settings-card">
        <p v-if="!sceneBars.length" class="stats-empty">{{ t.stats.empty }}</p>
        <div v-for="scene in sceneBars" :key="scene.name" class="stats-dist-row">
          <span class="stats-dist-name">{{ scene.label }}</span>
          <div class="stats-dist-track">
            <div class="stats-dist-bar" :style="{ width: `${scene.percent}%` }" />
          </div>
          <span class="stats-dist-count">{{ scene.count }}（{{ scene.percent }}%）</span>
        </div>
      </div>
    </section>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.stats.dictHits }}</h3>
      <div class="settings-card">
        <p v-if="!(summary?.dict_hits ?? []).length" class="stats-empty">{{ t.stats.empty }}</p>
        <div
          v-for="hit in summary?.dict_hits ?? []"
          :key="hit.name"
          class="stats-dist-row"
        >
          <span class="stats-dist-name">{{ hit.name }}</span>
          <div class="stats-dist-track">
            <div class="stats-dist-bar stats-dist-bar-secondary" :style="dictHitStyle(hit)" />
          </div>
          <span class="stats-dist-count">{{ hit.count }}</span>
        </div>
      </div>
    </section>

    <div class="stats-two-col">
      <section class="settings-group stats-col">
        <h3 class="settings-group-title">{{ t.stats.commands }}</h3>
        <div class="settings-card">
          <p v-if="!(summary?.commands ?? []).length" class="stats-empty">{{ t.stats.empty }}</p>
          <ol class="stats-top-list">
            <li v-for="item in summary?.commands ?? []" :key="item.name">
              <code class="stats-top-name">{{ item.name }}</code>
              <span class="stats-top-count">{{ item.count }}</span>
            </li>
          </ol>
        </div>
      </section>
      <section class="settings-group stats-col">
        <h3 class="settings-group-title">{{ t.stats.templates }}</h3>
        <div class="settings-card">
          <p v-if="!(summary?.templates ?? []).length" class="stats-empty">{{ t.stats.empty }}</p>
          <ol class="stats-top-list">
            <li v-for="item in summary?.templates ?? []" :key="item.name">
              <span class="stats-top-name">{{ item.name }}</span>
              <span class="stats-top-count">{{ item.count }}</span>
            </li>
          </ol>
        </div>
      </section>
    </div>

    <section class="settings-group">
      <div class="settings-card stats-anonymous">
        <ToggleSwitch
          :model-value="configStore.sections.stats.anonymous_upload"
          :aria-label="t.stats.anonymousTitle"
          @update:model-value="updateAnonymousUpload"
        />
        <div class="stats-anonymous-text">
          <span>{{ t.stats.anonymousTitle }}</span>
          <span class="stats-anonymous-desc">{{ t.stats.anonymousDesc }}</span>
        </div>
      </div>
    </section>

    <section class="settings-group">
      <div class="settings-card stats-danger">
        <Message v-if="clearNotice" severity="success" class="settings-error">
          {{ clearNotice.text }}
        </Message>
        <Button
          type="button"
          icon="pi pi-trash"
          severity="danger"
          :label="t.stats.clear"
          :loading="loading"
          @click="onClear"
        />
      </div>
    </section>
  </div>
</template>

<style scoped>
.stats-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--vt-spacing-md);
  flex-wrap: wrap;
  margin-bottom: var(--vt-spacing-md);
}

.stats-speed {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
}

.stats-speed-label {
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.stats-cards {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: var(--vt-spacing-md);
  padding: var(--vt-spacing-md);
}

.stats-card {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-xs);
}

.stats-card-value {
  font-size: 22px;
  font-weight: 600;
  color: var(--vt-color-text);
}

.stats-card-label {
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.stats-formula {
  margin: var(--vt-spacing-sm) 0 0;
  font-size: 12px;
  color: var(--vt-color-text-muted);
}

.stats-trend {
  display: flex;
  align-items: flex-end;
  gap: var(--vt-spacing-md);
  padding: var(--vt-spacing-md);
  overflow-x: auto;
}

.stats-trend-col {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--vt-spacing-xs);
  min-width: 48px;
}

.stats-trend-value {
  font-size: 12px;
  color: var(--vt-color-text-muted);
}

.stats-trend-bar-track {
  display: flex;
  align-items: flex-end;
  width: 28px;
  height: 120px;
  border-radius: var(--vt-radius-control);
  background: var(--vt-color-nav-active-bg);
}

.stats-trend-bar {
  width: 100%;
  border-radius: var(--vt-radius-control);
  background: var(--vt-color-primary);
  min-height: 2px;
}

.stats-trend-date {
  font-size: 11.5px;
  color: var(--vt-color-text-muted);
}

.stats-empty {
  margin: 0;
  padding: var(--vt-spacing-md);
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.stats-dist-row {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-md);
  padding: var(--vt-spacing-sm) var(--vt-spacing-md);
}

.stats-dist-name {
  min-width: 96px;
  font-size: 12.5px;
  color: var(--vt-color-text);
}

.stats-dist-track {
  flex: 1;
  height: 10px;
  border-radius: var(--vt-radius-control);
  background: var(--vt-color-nav-active-bg);
  overflow: hidden;
}

.stats-dist-bar {
  height: 100%;
  border-radius: var(--vt-radius-control);
  background: var(--vt-color-primary);
}

.stats-dist-bar-secondary {
  background: var(--vt-color-success);
}

.stats-dist-count {
  min-width: 72px;
  text-align: right;
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.stats-two-col {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--vt-spacing-md);
}

.stats-col {
  margin: 0;
}

.stats-top-list {
  margin: 0;
  padding: var(--vt-spacing-sm) var(--vt-spacing-md) var(--vt-spacing-sm) var(--vt-spacing-lg);
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-sm);
}

.stats-top-list li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--vt-spacing-md);
  font-size: 12.5px;
}

.stats-top-name {
  overflow-wrap: anywhere;
}

.stats-top-count {
  color: var(--vt-color-text-muted);
}

.stats-anonymous {
  display: flex;
  align-items: flex-start;
  gap: var(--vt-spacing-md);
  padding: var(--vt-spacing-md);
}

.stats-anonymous-text {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-xs);
  font-size: 12.5px;
}

.stats-anonymous-desc {
  color: var(--vt-color-text-muted);
}

.stats-danger {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--vt-spacing-sm);
  padding: var(--vt-spacing-md);
}
</style>
