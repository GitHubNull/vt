<script setup>
/**
 * 词典导入向导（T-523，F-DICT-03）：选文件 → 校验结果 → 冲突策略 → 报告。
 * 替换词典页与专有名词页共用（kind 区分）；文件路径由系统原生对话框选定
 * （plugin-dialog open）；文件级校验失败整体拒绝并展示原因（NFR-S-07）。
 * 文案经 importWizard 的 wizardText（i18n key 见 T-523 报告片段）。
 */
import { computed, ref, watch } from 'vue'
import Button from 'primevue/button'
import Dialog from 'primevue/dialog'
import Message from 'primevue/message'
import RadioButton from 'primevue/radiobutton'
import { open } from '@tauri-apps/plugin-dialog'
import { dictImportApply, dictImportInspect } from '../../api/dict'
import { toAppError } from '../../api/client'
import { useDictStore } from '../../stores/dict'
import { t } from '../../i18n'
import {
  importFilters,
  initialWizardState,
  issueText,
  reportSummary,
  strategyOptions,
  summarizeInspection,
  wizardApplyFailure,
  wizardBackToFile,
  wizardInspectFailure,
  wizardInspectSuccess,
  wizardReport,
  wizardText,
} from './importWizard'

const props = defineProps({
  /** 词典类型：'replace' | 'proper_nouns'（与后端 DictKind 字面量一致）。 */
  kind: { type: String, required: true },
  visible: { type: Boolean, default: false },
})
const emit = defineEmits(['update:visible', 'done'])

const store = useDictStore()
const txt = wizardText(props.kind)
const strategies = strategyOptions()
const stepLabels = [txt.stepFile, txt.stepReview, txt.stepStrategy, txt.stepReport]

const state = ref(initialWizardState())
const strategy = ref('skip')
const inspecting = ref(false)
const applying = ref(false)

const activeIndex = computed(() => {
  switch (state.value.step) {
    case 'review':
      return 1
    case 'strategy':
      return 2
    case 'report':
      return 3
    default:
      return 0
  }
})
const summary = computed(() =>
  state.value.inspection ? summarizeInspection(state.value.inspection) : null,
)
const reportLine = computed(() => (state.value.report ? reportSummary(state.value.report) : ''))

/** 打开时重置向导状态（关闭不清理无碍，下次打开会重置）。 */
watch(
  () => props.visible,
  (visible) => {
    if (visible) {
      state.value = initialWizardState()
      strategy.value = 'skip'
      inspecting.value = false
      applying.value = false
    }
  },
)

/** 后端结构化错误（含编号）或兜底文案。 */
function errorTextOf(error) {
  const err = toAppError(error)
  return `${err.message}（${t.common.errorCode} ${err.code}）`
}

/** 选择文件并请求后端校验（校验通过进入「校验结果」，失败展示原因）。 */
async function pickAndInspect() {
  inspecting.value = true
  state.value = { ...state.value, error: null }
  try {
    const path = await open({
      title: txt.title,
      multiple: false,
      filters: importFilters(props.kind),
    })
    if (!path) return
    const inspection = await dictImportInspect(props.kind, String(path))
    state.value = wizardInspectSuccess(state.value, String(path), inspection)
  } catch (error) {
    state.value = wizardInspectFailure(state.value, errorTextOf(error))
  } finally {
    inspecting.value = false
  }
}

function toStrategy() {
  state.value = wizardToStrategy(state.value)
}

function backToFile() {
  state.value = wizardBackToFile(state.value)
}

/** 执行导入并刷新词典镜像；完成（含部分跳过 / 失败）进入报告步骤。 */
async function applyImport() {
  const path = state.value.path
  if (!path) {
    state.value = wizardInspectFailure(state.value, txt.needFile)
    return
  }
  applying.value = true
  state.value = { ...state.value, error: null }
  try {
    const report = await dictImportApply(props.kind, path, strategy.value)
    await refreshMirror()
    state.value = wizardReport(state.value, report)
    emit('done', report)
  } catch (error) {
    state.value = wizardApplyFailure(state.value, errorTextOf(error))
  } finally {
    applying.value = false
  }
}

/** 导入成功后刷新对应词典页镜像（替换规则 / 专有名词）。 */
async function refreshMirror() {
  if (props.kind === 'proper_nouns') {
    await store.refreshNouns()
  } else {
    await store.refresh()
  }
}

function closeWizard() {
  emit('update:visible', false)
}
</script>

<template>
  <Dialog
    :visible="props.visible"
    modal
    :header="txt.title"
    class="import-wizard-dialog"
    @update:visible="(value) => emit('update:visible', value)"
  >
    <ol class="iw-steps">
      <li
        v-for="(label, index) in stepLabels"
        :key="label"
        class="iw-step"
        :class="{ 'iw-step-active': activeIndex === index, 'iw-step-done': activeIndex > index }"
      >
        <span class="iw-step-index">{{ index + 1 }}</span>{{ label }}
      </li>
    </ol>

    <div v-if="state.step === 'file'" class="iw-body">
      <p class="iw-hint">{{ txt.pickHint }}</p>
      <Button
        type="button"
        icon="pi pi-folder-open"
        :label="inspecting ? txt.inspecting : txt.pick"
        :loading="inspecting"
        @click="pickAndInspect"
      />
      <Message v-if="state.error" severity="error" class="settings-error">{{
        state.error
      }}</Message>
    </div>

    <div v-else-if="state.step === 'review'" class="iw-body">
      <ul class="iw-summary">
        <li>{{ txt.formatLabel }}：{{ summary.format }}</li>
        <li>{{ txt.versionLabel }}：{{ summary.version }}</li>
        <li>{{ txt.totalLabel }}：{{ summary.total }}</li>
        <li>{{ txt.invalidLabel }}：{{ summary.invalid }}</li>
        <li>{{ txt.conflictsLabel }}：{{ summary.conflicts }}</li>
      </ul>
      <p v-if="state.inspection && state.inspection.invalid > 0" class="iw-hint">
        {{ txt.invalidHint }}
      </p>
      <p v-if="state.path" class="iw-path">{{ state.path }}</p>
      <div class="iw-actions">
        <Button type="button" severity="secondary" :label="txt.repick" @click="backToFile" />
        <Button type="button" :label="txt.next" @click="toStrategy" />
      </div>
    </div>

    <div v-else-if="state.step === 'strategy'" class="iw-body">
      <p class="iw-hint">{{ txt.strategyHint }}</p>
      <div class="iw-strategies">
        <label v-for="option in strategies" :key="option.value" class="iw-strategy">
          <RadioButton
            v-model="strategy"
            :value="option.value"
            :input-id="`iw-strategy-${option.value}`"
          />
          <span class="iw-strategy-text">
            <span class="iw-strategy-label">{{ option.label }}</span>
            <span class="iw-strategy-desc">{{ option.description }}</span>
          </span>
        </label>
      </div>
      <Message v-if="state.error" severity="error" class="settings-error">{{
        state.error
      }}</Message>
      <div class="iw-actions">
        <Button type="button" severity="secondary" :label="txt.back" @click="backToFile" />
        <Button
          type="button"
          icon="pi pi-upload"
          :label="applying ? txt.applying : txt.apply"
          :loading="applying"
          @click="applyImport"
        />
      </div>
    </div>

    <div v-else class="iw-body">
      <Message severity="success" class="iw-report-line">{{ reportLine }}</Message>
      <ul v-if="state.report && state.report.issues.length" class="iw-issues">
        <li v-for="issue in state.report.issues" :key="issue.index">{{ issueText(issue) }}</li>
      </ul>
      <div class="iw-actions">
        <Button type="button" :label="txt.close" @click="closeWizard" />
      </div>
    </div>
  </Dialog>
</template>

<style scoped>
.iw-steps {
  display: flex;
  gap: var(--vt-spacing-md);
  margin: 0 0 var(--vt-spacing-md);
  padding: 0 0 var(--vt-spacing-sm);
  list-style: none;
  border-bottom: 1px solid var(--vt-color-border);
}

.iw-step {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.iw-step-active {
  color: var(--vt-color-text);
  font-weight: 600;
}

.iw-step-done {
  color: var(--vt-color-text);
}

.iw-step-index {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  border: 1px solid currentColor;
  font-size: 11px;
}

.iw-body {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-sm);
  min-width: 380px;
  padding-top: var(--vt-spacing-sm);
}

.iw-hint {
  margin: 0;
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
  white-space: pre-line;
}

.iw-summary {
  margin: 0;
  padding-left: 18px;
  font-size: 13px;
  line-height: 1.9;
}

.iw-path {
  margin: 0;
  font-size: 12px;
  color: var(--vt-color-text-muted);
  word-break: break-all;
}

.iw-strategies {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-xs);
}

.iw-strategy {
  display: flex;
  align-items: flex-start;
  gap: var(--vt-spacing-sm);
  padding: 8px 10px;
  border: 1px solid var(--vt-color-border);
  border-radius: var(--vt-radius-control);
  cursor: pointer;
}

.iw-strategy-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.iw-strategy-label {
  font-size: 13px;
}

.iw-strategy-desc {
  font-size: 12px;
  color: var(--vt-color-text-muted);
}

.iw-issues {
  margin: 0;
  padding-left: 18px;
  font-size: 12.5px;
  color: var(--vt-color-warning);
  max-height: 160px;
  overflow-y: auto;
  white-space: pre-line;
}

.iw-report-line {
  margin: 0;
}

.iw-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--vt-spacing-sm);
  padding-top: var(--vt-spacing-xs);
}
</style>
