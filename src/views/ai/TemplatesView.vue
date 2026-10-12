<script setup>
/**
 * AI › 提示词管理（T-533，F-AI-01/F-AI-03、PRD 3.5）。
 * 模板卡片列表 + 展开编辑态（对照原型 PixPin_2026-10-09_21-34-15.png）：
 * - 卡片：折叠箭头、名称、“内置”标签、供应商 · 模型 / 纯转录标签、快捷键标签、操作图标；
 * - 展开字段：模板名称、快捷键（存储字段预留，绑定机制在 T-534）、AI 供应商、
 *   模型（选择供应商后必填）、超时时间、提示词内容、取消 / 保存；
 * - 内置模板只读，可复制为自定义（SPEC T-533；删除 / 修改由后端拒绝）；
 * - 绑定下拉数据来自 T-531/T-532 服务（ai_list_providers 的 available_models）。
 * 用户可见文案集中于 i18n（ai.templates.*），页面状态与编排在 stores/ai.ts。
 */
import { computed, onMounted } from 'vue'
import Button from 'primevue/button'
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import Select from 'primevue/select'
import Tag from 'primevue/tag'
import Textarea from 'primevue/textarea'
import { confirm } from '@tauri-apps/plugin-dialog'
import { openUrl } from '@tauri-apps/plugin-opener'
import {
  DEFAULT_TIMEOUT_MS,
  MAX_TEMPLATE_NAME,
  MAX_TEMPLATE_PROMPT,
  NO_PROVIDER,
  TIMEOUT_PRESETS,
  useAiStore,
} from '../../stores/ai'
import { t } from '../../i18n'

const store = useAiStore()

onMounted(() => {
  void store.ensureLoaded()
})

/** 提示词模板教程（外部链接；占位地址，正式地址发布前替换，AGENTS.md 3.5）。 */
const TUTORIAL_URL = 'https://example.com/vibetype/prompt-templates'

function openTutorial() {
  openUrl(TUTORIAL_URL).catch(() => {
    // 无外部浏览器等打开失败：无副作用，直接忽略。
  })
}

const errorText = computed(() =>
  store.lastError
    ? `${store.lastError.message}（${t.common.errorCode} ${store.lastError.code}）`
    : '',
)

/** 新建草稿卡片哨兵 ID（草稿未落盘，不占用真实模板 ID 空间）。 */
const DRAFT_CARD_ID = '__new__'

/**
 * 卡片数据源：新建草稿（存在时置顶）+ 模板清单。
 * 统一成一个编辑器模板块，避免编辑态与新建态两份字段渲染分叉。
 */
const cards = computed(() => {
  const items = store.templates.map((template) => ({ template, isDraft: false }))
  if (store.draft && store.draft.id === null) {
    items.unshift({
      template: { id: DRAFT_CARD_ID, name: '', builtin: false },
      isDraft: true,
    })
  }
  return items
})

function isExpanded(item) {
  return item.isDraft || store.expandedId === item.template.id
}

function onCardHead(item) {
  if (!item.isDraft) store.toggleExpand(item.template)
}

/** 供应商下拉选项（首项“无（纯转录）”，PRD 3.5.1；本地供应商带“本地”后缀）。 */
const providerOptions = computed(() => [
  { value: NO_PROVIDER, label: t.ai.templates.noProvider },
  ...store.providers.map((provider) => ({
    value: provider.id,
    label: provider.local ? `${provider.name}（${t.ai.templates.localTag}）` : provider.name,
  })),
])

/** 当前草稿的模型选项（供应商切换后按新清单计算，沿用不必丢当前绑定）。 */
const modelOptions = computed(() => {
  const draft = store.draft
  if (!draft) return []
  return store.modelOptions(draft.providerId, draft.model).map((model) => ({
    value: model,
    label: model,
  }))
})

/** 超时时间常用值（PRD 3.5.1：3000 / 5000 / 10000 / 30000，可手动输入）。 */
const timeoutOptions = TIMEOUT_PRESETS.map((ms) => ({ value: ms, label: String(ms) }))

/** 卡片第二行标签：纯转录 / 供应商 · 模型。 */
function providerLabel(template) {
  if (!template.provider_id) return t.ai.templates.transcriptionOnly
  const provider = store.providerById(template.provider_id)
  const name = provider ? provider.name : template.provider_id
  return template.model ? `${name} · ${template.model}` : name
}

async function onDelete(template) {
  const ok = await confirm(t.ai.templates.deleteConfirm.replace('{name}', template.name), {
    title: t.ai.templates.deleteTitle,
    kind: 'warning',
  })
  if (ok) await store.removeTemplate(template.id)
}
</script>

<template>
  <div class="settings-page">
    <div class="templates-head">
      <h2 class="settings-page-title">{{ t.ai.templates.title }}</h2>
      <Button
        type="button"
        icon="pi pi-plus"
        :label="t.ai.templates.add"
        :disabled="store.draft?.id === null"
        @click="store.openCreate"
      />
    </div>

    <div class="templates-howto">
      <span>{{ t.ai.templates.howToWrite }}</span>
      <a class="templates-howto-link" href="#" @click.prevent="openTutorial">
        {{ t.ai.templates.tutorial }}
        <i class="pi pi-external-link" aria-hidden="true" />
      </a>
    </div>

    <p v-if="store.loaded && !cards.length" class="templates-empty">
      {{ t.ai.templates.empty }}
    </p>

    <div
      v-for="item in cards"
      :key="item.template.id"
      class="template-card"
      :class="{ 'template-card-open': isExpanded(item) }"
    >
      <div class="template-card-head" @click="onCardHead(item)">
        <template v-if="item.isDraft">
          <span class="template-name">{{ t.ai.templates.newTemplate }}</span>
        </template>
        <template v-else>
          <i
            class="pi"
            :class="isExpanded(item) ? 'pi-chevron-down' : 'pi-chevron-right'"
            aria-hidden="true"
          />
          <span class="template-name">{{ item.template.name }}</span>
          <Tag v-if="item.template.builtin" :value="t.ai.templates.builtinTag" severity="info" />
          <span class="template-head-spacer" />
          <Button
            v-if="item.template.builtin"
            type="button"
            icon="pi pi-copy"
            severity="secondary"
            size="small"
            text
            :aria-label="t.ai.templates.copy"
            :title="t.ai.templates.copy"
            @click.stop="store.copyTemplate(item.template.id)"
          />
          <template v-else>
            <Button
              type="button"
              icon="pi pi-pencil"
              severity="secondary"
              size="small"
              text
              :aria-label="t.ai.templates.edit"
              :title="t.ai.templates.edit"
              @click.stop="store.openEditor(item.template)"
            />
            <Button
              type="button"
              icon="pi pi-times"
              severity="danger"
              size="small"
              text
              :aria-label="t.ai.templates.delete"
              :title="t.ai.templates.delete"
              @click.stop="onDelete(item.template)"
            />
          </template>
        </template>
      </div>

      <div v-if="!item.isDraft" class="template-card-meta">
        <Tag icon="pi pi-microphone" severity="secondary" :value="providerLabel(item.template)" />
        <Tag
          v-if="item.template.hotkey"
          icon="pi pi-key"
          severity="secondary"
          :value="item.template.hotkey"
        />
      </div>

      <div v-if="isExpanded(item) && store.draft" class="template-editor">
        <div class="template-field">
          <label class="template-field-label" :for="`tpl-name-${item.template.id}`">
            {{ t.ai.templates.nameLabel }}
            <em class="template-required">*</em>
          </label>
          <InputText
            :id="`tpl-name-${item.template.id}`"
            v-model="store.draft.name"
            :maxlength="MAX_TEMPLATE_NAME"
            :disabled="store.draft.builtin"
            :placeholder="t.ai.templates.namePlaceholder"
          />
        </div>

        <div class="template-field">
          <label class="template-field-label">
            {{ t.ai.templates.hotkeyLabel }}
          </label>
          <div class="template-hotkey-row">
            <InputText
              :model-value="store.draft.hotkey || t.ai.templates.hotkeyUnset"
              disabled
              :aria-label="t.ai.templates.hotkeyLabel"
            />
            <Button
              type="button"
              severity="secondary"
              :label="t.ai.templates.hotkeySet"
              disabled
              :title="t.ai.templates.hotkeyComingSoon"
            />
          </div>
          <p class="template-field-hint">{{ t.ai.templates.hotkeyHint }}</p>
        </div>

        <div class="template-field-grid">
          <div class="template-field">
            <label class="template-field-label">
              {{ t.ai.templates.providerLabel }}
            </label>
            <Select
              v-model="store.draft.providerId"
              :options="providerOptions"
              option-label="label"
              option-value="value"
              :disabled="store.draft.builtin"
              :aria-label="t.ai.templates.providerLabel"
            />
            <p class="template-field-hint">
              {{ t.ai.templates.providerHint }}
            </p>
          </div>
          <div class="template-field">
            <label class="template-field-label">
              {{ t.ai.templates.modelLabel }}
              <em v-if="store.draft.providerId" class="template-required">*</em>
            </label>
            <Select
              v-model="store.draft.model"
              :options="modelOptions"
              option-label="label"
              option-value="value"
              :disabled="
                store.draft.builtin ||
                store.draft.providerId === NO_PROVIDER ||
                !modelOptions.length
              "
              :placeholder="t.ai.templates.modelPlaceholder"
              :aria-label="t.ai.templates.modelLabel"
            />
          </div>
        </div>

        <div class="template-field template-field-timeout">
          <label class="template-field-label">
            {{ t.ai.templates.timeoutLabel }}
          </label>
          <Select
            v-model="store.draft.timeoutMs"
            :options="timeoutOptions"
            option-label="label"
            option-value="value"
            editable
            :disabled="store.draft.builtin"
            :placeholder="String(DEFAULT_TIMEOUT_MS)"
            :aria-label="t.ai.templates.timeoutLabel"
          />
          <p class="template-field-hint">{{ t.ai.templates.timeoutHint }}</p>
        </div>

        <div class="template-field">
          <label class="template-field-label" :for="`tpl-prompt-${item.template.id}`">
            {{ t.ai.templates.promptLabel }}
            <em class="template-required">*</em>
          </label>
          <Textarea
            :id="`tpl-prompt-${item.template.id}`"
            v-model="store.draft.prompt"
            rows="9"
            :maxlength="MAX_TEMPLATE_PROMPT"
            :disabled="store.draft.builtin"
            :placeholder="t.ai.templates.promptPlaceholder"
          />
          <p class="template-field-hint">
            {{
              t.ai.templates.promptCount
                .replace('{count}', String(store.draft.prompt.length))
                .replace('{max}', String(MAX_TEMPLATE_PROMPT))
            }}
          </p>
        </div>

        <div v-if="store.draft.builtin" class="template-editor-footer">
          <span class="template-builtin-hint">
            {{ t.ai.templates.builtinReadonlyHint }}
          </span>
          <Button
            type="button"
            icon="pi pi-copy"
            :label="t.ai.templates.copy"
            @click="store.copyTemplate(item.template.id)"
          />
        </div>
        <div v-else class="template-editor-footer">
          <Button
            type="button"
            severity="secondary"
            :label="t.common.cancel"
            @click="store.closeEditor()"
          />
          <Button
            type="button"
            :label="t.ai.templates.save"
            :disabled="!store.draftValid || store.saving"
            :loading="store.saving"
            @click="store.saveDraft()"
          />
        </div>
      </div>
    </div>

    <Message v-if="store.lastError" severity="error" class="settings-error">{{
      errorText
    }}</Message>
  </div>
</template>

<style scoped>
.templates-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--vt-spacing-md);
}

.templates-howto {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--vt-spacing-md);
  margin-bottom: var(--vt-spacing-md);
  padding: 10px var(--vt-spacing-md);
  border: 1px solid var(--vt-color-border);
  border-radius: var(--vt-radius-card);
  background: var(--vt-color-bg);
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.templates-howto-link {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--vt-color-primary);
  text-decoration: none;
  cursor: pointer;
}

.templates-howto-link:hover {
  color: var(--vt-color-primary-hover);
  text-decoration: underline;
}

.templates-empty {
  margin: 0 0 var(--vt-spacing-md);
  padding: var(--vt-spacing-md);
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.template-card {
  margin-bottom: var(--vt-spacing-sm);
  border: 1px solid var(--vt-color-border);
  border-radius: var(--vt-radius-card);
  background: var(--vt-color-surface);
  overflow: hidden;
}

.template-card-open {
  border-color: var(--vt-color-primary);
}

.template-card-head {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
  padding: 12px var(--vt-spacing-md);
  cursor: pointer;
}

.template-card-head .pi-chevron-right,
.template-card-head .pi-chevron-down {
  font-size: 12px;
  color: var(--vt-color-text-muted);
}

.template-name {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--vt-color-text);
}

.template-head-spacer {
  flex: 1;
}

.template-card-meta {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
  padding: 0 var(--vt-spacing-md) 12px 36px;
  flex-wrap: wrap;
}

.template-editor {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-md);
  padding: var(--vt-spacing-md);
  border-top: 1px solid var(--vt-color-border);
}

.template-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.template-field-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--vt-spacing-md);
}

.template-field-timeout {
  max-width: 260px;
}

.template-field-label {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--vt-color-text);
}

.template-required {
  color: var(--vt-color-danger, #dc2626);
  font-style: normal;
}

.template-field-hint {
  margin: 0;
  font-size: 12px;
  color: var(--vt-color-text-muted);
}

.template-hotkey-row {
  display: flex;
  gap: var(--vt-spacing-sm);
}

.template-hotkey-row :deep(.p-inputtext) {
  flex: 1;
}

.template-editor-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--vt-spacing-sm);
}

.template-builtin-hint {
  flex: 1;
  font-size: 12px;
  color: var(--vt-color-text-muted);
}

@media (max-width: 720px) {
  .template-field-grid {
    grid-template-columns: 1fr;
  }
}
</style>
