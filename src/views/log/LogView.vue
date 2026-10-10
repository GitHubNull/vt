<script setup>
/**
 * 日志 › 日志查看（T-018，F-LOG-01 ~ F-LOG-03，PRD 3.13）：
 * 记录目标配置（文本 / 数据库双写，至少一项）+ 关键词 / 正则 / 大小写敏感搜索
 * + 级别 / 时间范围过滤 + 语法高亮 + 按条件导出。
 */
import { computed, onMounted, ref, watch } from 'vue'
import Button from 'primevue/button'
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import MultiSelect from 'primevue/multiselect'
import ToggleSwitch from 'primevue/toggleswitch'
import { save } from '@tauri-apps/plugin-dialog'
import SettingsRow from '../../components/SettingsRow.vue'
import { useConfigStore } from '../../stores/config'
import { useLogStore } from '../../stores/log'
import { toAppError } from '../../api/client'
import { t } from '../../i18n'
import {
  buildHighlightPattern,
  computeSegments,
  formatLocalTime,
  levelClass,
} from '../../utils/logHighlight'

const configStore = useConfigStore()
const store = useLogStore()

/** 级别过滤选项（PRD 3.13：DEBUG / INFO / WARN / ERROR）。 */
const LEVEL_OPTIONS = ['DEBUG', 'INFO', 'WARN', 'ERROR']

const targets = computed(() => configStore.sections.log.targets)
const targetHint = ref('')

onMounted(() => {
  void configStore.ensureLoaded()
  // 默认展示最近日志（数据库启用前提下）。
  if (store.dbEnabled) void store.search()
})

/** 高亮匹配器：非法正则在 store.search 预检处拦截，这里兜底为不高亮。 */
const highlightPattern = computed(() => {
  const kw = store.keyword.trim()
  if (!kw) return null
  try {
    return buildHighlightPattern(kw, store.regex, store.caseSensitive)
  } catch {
    return null
  }
})

function segmentsOf(body) {
  return computeSegments(body, highlightPattern.value)
}

// 过滤条件变化即重新查询（防抖 300ms，避免输入过程频繁 invoke）。
let debounceTimer = 0
watch(
  () => [store.keyword, store.regex, store.caseSensitive, store.levels, store.fromLocal, store.toLocal],
  () => {
    if (!store.dbEnabled) return
    window.clearTimeout(debounceTimer)
    debounceTimer = window.setTimeout(() => void store.search(), 300)
  },
)

async function toggleTarget(target, enabled) {
  targetHint.value = ''
  const current = [...targets.value]
  const next = enabled ? [...current, target] : current.filter((item) => item !== target)
  // F-LOG-02：至少保留一项（后端 E-CONFIG-004 兜底，前端先行提示）。
  if (next.length === 0) {
    targetHint.value = t.log.targets.needOne
    return
  }
  const ok = await store.updateTargets(next)
  if (!ok) targetHint.value = ''
}

const searchDisabled = computed(() => !store.dbEnabled || store.loading)

const errorText = computed(() =>
  store.lastError ? `${store.lastError.message}（${t.common.errorCode} ${store.lastError.code}）` : '',
)

const exportMessage = ref(null)
const exporting = ref(false)

async function onExport() {
  exporting.value = true
  exportMessage.value = null
  try {
    const path = await save({
      title: t.log.search.saveTitle,
      defaultPath: 'vibetype-logs.txt',
      filters: [{ name: 'Text', extensions: ['txt'] }],
    })
    if (!path) return
    const lines = await store.exportResults(path)
    if (lines !== null) {
      exportMessage.value = { ok: true, text: `${t.log.search.exportDone}${lines}` }
    } else if (store.lastError) {
      exportMessage.value = {
        ok: false,
        text: `${t.log.search.exportFailed}（${t.common.errorCode} ${store.lastError.code}）`,
      }
    }
  } catch (error) {
    exportMessage.value = {
      ok: false,
      text: `${t.log.search.exportFailed}（${t.common.errorCode} ${toAppError(error).code}）`,
    }
  } finally {
    exporting.value = false
  }
}
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.log.title }}</h2>
    <p class="settings-page-desc">{{ t.log.desc }}</p>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.log.targets.group }}</h3>
      <div class="settings-card">
        <SettingsRow :title="t.log.targets.file.label" :desc="t.log.targets.file.desc">
          <ToggleSwitch
            :model-value="targets.includes('file')"
            :aria-label="t.log.targets.file.label"
            @update:model-value="(v) => toggleTarget('file', v)"
          />
        </SettingsRow>
        <SettingsRow
          :title="t.log.targets.database.label"
          :desc="t.log.targets.database.desc"
        >
          <ToggleSwitch
            :model-value="targets.includes('database')"
            :aria-label="t.log.targets.database.label"
            @update:model-value="(v) => toggleTarget('database', v)"
          />
        </SettingsRow>
        <p v-if="targetHint" class="targets-hint">{{ targetHint }}</p>
        <Message
          v-if="configStore.lastError"
          severity="error"
          class="settings-error"
          :closable="false"
        >
          {{ configStore.lastError.message }}（{{ t.common.errorCode }} {{ configStore.lastError.code }}）
        </Message>
      </div>
    </section>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.log.search.group }}</h3>
      <div class="settings-card">
        <Message v-if="!store.dbEnabled" severity="info" :closable="false" class="log-db-hint">
          {{ t.log.search.dbDisabled }}
        </Message>
        <div v-else class="log-search-bar">
          <InputText
            v-model="store.keyword"
            class="log-keyword"
            :placeholder="t.log.search.keywordPlaceholder"
            :aria-label="t.log.search.keywordPlaceholder"
            @keyup.enter="store.search()"
          />
          <div class="log-toggles">
            <label class="log-toggle">
              <ToggleSwitch v-model="store.regex" :aria-label="t.log.search.regex" />
              <span>{{ t.log.search.regex }}</span>
            </label>
            <label class="log-toggle">
              <ToggleSwitch v-model="store.caseSensitive" :aria-label="t.log.search.caseSensitive" />
              <span>{{ t.log.search.caseSensitive }}</span>
            </label>
          </div>
          <MultiSelect
            v-model="store.levels"
            :options="LEVEL_OPTIONS"
            class="log-levels"
            :placeholder="t.log.search.levels"
            :aria-label="t.log.search.levels"
          />
          <div class="log-time-range">
            <input
              v-model="store.fromLocal"
              type="datetime-local"
              class="log-time-input"
              :aria-label="t.log.search.from"
            />
            <span class="log-time-sep">—</span>
            <input
              v-model="store.toLocal"
              type="datetime-local"
              class="log-time-input"
              :aria-label="t.log.search.to"
            />
          </div>
          <Button
            type="button"
            icon="pi pi-search"
            :label="store.loading ? t.log.search.loading : t.log.search.query"
            :loading="store.loading"
            :disabled="searchDisabled && !store.loading"
            @click="store.search()"
          />
          <Button
            type="button"
            icon="pi pi-download"
            :label="exporting ? t.log.search.exporting : t.log.search.export"
            :loading="exporting"
            :disabled="!store.dbEnabled || exporting"
            severity="secondary"
            @click="onExport"
          />
        </div>
        <p v-if="store.searchHint === 'invalid_regex'" class="log-hint log-hint-error">
          {{ t.log.search.invalidRegex }}
        </p>
        <Message v-if="errorText" severity="error" :closable="false" class="settings-error">
          {{ errorText }}
        </Message>
        <p v-if="exportMessage" class="log-hint" :class="{ 'log-hint-error': !exportMessage.ok }">
          {{ exportMessage.text }}
        </p>

        <div class="log-list" :aria-busy="store.loading">
          <p v-if="!store.loading && store.entries.length === 0" class="log-empty">
            {{ t.log.search.empty }}
          </p>
          <div v-for="entry in store.entries" :key="entry.id" class="log-line">
            <span class="log-ts">{{ formatLocalTime(entry.ts_millis) }}</span>
            <span class="log-level" :class="levelClass(entry.level)">{{ entry.level }}</span>
            <span class="log-target">{{ entry.target }}</span>
            <span class="log-body">
              <template v-for="(seg, i) in segmentsOf(entry.body)" :key="i">
                <mark v-if="seg.match" class="log-mark">{{ seg.text }}</mark>
                <template v-else>{{ seg.text }}</template>
              </template>
            </span>
          </div>
        </div>
        <div v-if="store.hasMore" class="log-more">
          <Button
            type="button"
            :label="t.log.search.loadMore"
            :loading="store.loading"
            severity="secondary"
            size="small"
            @click="store.loadMore()"
          />
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.targets-hint {
  margin: 0;
  padding: 10px 20px;
  font-size: 12.5px;
  color: var(--vt-color-warning);
  border-top: 1px solid var(--vt-color-border);
}

.log-db-hint,
.settings-error {
  margin: 10px 20px;
}

.log-search-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--vt-spacing-sm);
  padding: 14px 20px;
  border-bottom: 1px solid var(--vt-color-border);
}

.log-keyword {
  width: 240px;
}

.log-toggles {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-md);
}

.log-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
  white-space: nowrap;
}

.log-levels {
  min-width: 180px;
}

.log-time-range {
  display: flex;
  align-items: center;
  gap: 6px;
}

.log-time-input {
  padding: 8px 10px;
  font-size: 12.5px;
  font-family: inherit;
  color: var(--vt-color-text);
  background: var(--vt-color-bg);
  border: 1px solid var(--vt-color-border);
  border-radius: 6px;
}

.log-time-sep {
  color: var(--vt-color-text-muted);
}

.log-hint {
  margin: 8px 20px 0;
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.log-hint-error {
  color: var(--vt-color-error);
}

.log-list {
  max-height: 480px;
  overflow-y: auto;
  padding: 8px 0;
  font-family: 'SF Mono', 'Cascadia Mono', Consolas, monospace;
  font-size: 12px;
  line-height: 1.7;
}

.log-empty {
  margin: 12px 20px;
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.log-line {
  display: flex;
  gap: 10px;
  padding: 0 20px;
  white-space: pre-wrap;
  word-break: break-all;
}

.log-line:hover {
  background: var(--vt-color-hover, rgba(0, 0, 0, 0.04));
}

.log-ts {
  flex-shrink: 0;
  color: var(--vt-color-text-muted);
}

.log-level {
  flex-shrink: 0;
  width: 48px;
  font-weight: 600;
}

.lv-debug {
  color: var(--vt-color-text-muted);
}

.lv-info {
  color: var(--vt-color-primary, #2f7cf6);
}

.lv-warn {
  color: var(--vt-color-warning);
}

.lv-error {
  color: var(--vt-color-error);
}

.log-target {
  flex-shrink: 0;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  color: #8e5bd0;
}

.log-body {
  flex: 1;
  color: var(--vt-color-text);
}

.log-mark {
  padding: 0 1px;
  background: var(--vt-color-warning);
  color: inherit;
  border-radius: 2px;
}

.log-more {
  display: flex;
  justify-content: center;
  padding: 10px 0 14px;
}
</style>
