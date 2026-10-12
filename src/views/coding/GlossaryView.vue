<script setup>
/**
 * 编程 › 技术词库（T-521，F-CODE-06、PRD 3.3）。
 * 内置术语包按技术栈卡片展示：独立启停（即时生效，写入 dict.tech_lexicon.enabled_stacks）、
 * 条目浏览与搜索（对话框内二次过滤）；搜索框带清除按钮（沿用词典页约定）。
 * 词库为只读内置数据（项目术语导入为 v1.0 F-CODE-07）。
 */
import { computed, onMounted, ref } from 'vue'
import Button from 'primevue/button'
import Dialog from 'primevue/dialog'
import InputText from 'primevue/inputtext'
import Message from 'primevue/message'
import ToggleSwitch from 'primevue/toggleswitch'
import { techLexiconStacks } from '../../api/lexicon'
import { toAppError } from '../../api/client'
import { useConfigStore } from '../../stores/config'
import { getMessage, t } from '../../i18n'
import {
  STACK_ORDER,
  filterStacks,
  filterTerms,
  stackNameKey,
  toggleStack,
} from './glossary'

const configStore = useConfigStore()

const stacks = ref([])
const loading = ref(false)
const lastError = ref(null)
const search = ref('')
/** 词条浏览对话框（null = 关闭）。 */
const viewer = ref(null)
const viewerQuery = ref('')

const viewerVisible = computed({
  get: () => viewer.value !== null,
  set: (visible) => {
    if (!visible) viewer.value = null
  },
})

/** 技术栈标识 → 显示名（i18n 映射，缺失回退标识本身）。 */
const stackNames = computed(() => {
  const names = {}
  for (const stack of stacks.value) {
    names[stack.id] = getMessage(stackNameKey(stack.id))
  }
  return names
})

onMounted(() => {
  void load()
  void configStore.ensureLoaded()
})

async function load() {
  loading.value = true
  lastError.value = null
  try {
    const list = await techLexiconStacks()
    const order = new Map(STACK_ORDER.map((id, index) => [id, index]))
    stacks.value = [...list].sort(
      (a, b) => (order.get(a.id) ?? 99) - (order.get(b.id) ?? 99),
    )
  } catch (error) {
    lastError.value = toAppError(error)
  } finally {
    loading.value = false
  }
}

const enabledStacks = computed(() => configStore.sections.dict.tech_lexicon.enabled_stacks)

const filteredStacks = computed(() =>
  filterStacks(stacks.value, stackNames.value, search.value),
)

const enabledCount = computed(() =>
  stacks.value.filter((stack) => enabledStacks.value.includes(stack.id)).length,
)

const enabledCountText = computed(() =>
  t.coding.glossary.enabledCount
    .replace('{enabled}', String(enabledCount.value))
    .replace('{total}', String(stacks.value.length)),
)

function isStackEnabled(id) {
  return enabledStacks.value.includes(id)
}

function onToggleStack(stack, on) {
  void configStore.updateSection('dict', {
    ...configStore.sections.dict,
    tech_lexicon: { enabled_stacks: toggleStack(enabledStacks.value, stack.id, on) },
  })
}

function openViewer(stack) {
  viewerQuery.value = ''
  viewer.value = stack
}

const viewerTerms = computed(() => {
  if (!viewer.value) return []
  return filterTerms(viewer.value.terms, viewerQuery.value)
})

const viewerTitle = computed(() =>
  viewer.value
    ? t.coding.glossary.dialogTitle.replace('{stack}', stackNames.value[viewer.value.id] ?? viewer.value.id)
    : '',
)

function errorText() {
  return lastError.value
    ? `${lastError.value.message}（${t.common.errorCode} ${lastError.value.code}）`
    : ''
}
</script>

<template>
  <div class="settings-page">
    <h2 class="settings-page-title">{{ t.coding.glossary.title }}</h2>
    <p class="settings-page-desc">{{ t.coding.glossary.desc }}</p>

    <section class="settings-group">
      <h3 class="settings-group-title">{{ t.coding.glossary.group }}</h3>
      <div class="settings-card">
        <div class="dict-toolbar">
          <span class="dict-counter">{{ enabledCountText }}</span>
          <div class="dict-search-wrap">
            <InputText
              v-model="search"
              class="dict-search"
              :placeholder="t.coding.glossary.searchPlaceholder"
              :aria-label="t.coding.glossary.searchPlaceholder"
            />
            <button
              v-if="search"
              type="button"
              class="dict-search-clear"
              :aria-label="t.common.clear"
              :title="t.common.clear"
              @click="search = ''"
            >
              <i class="pi pi-times" aria-hidden="true" />
            </button>
          </div>
        </div>
        <p class="glossary-hint">{{ t.coding.glossary.enabledHint }}</p>

        <p v-if="!filteredStacks.length" class="glossary-empty">
          {{ t.coding.glossary.empty }}
        </p>

        <div class="glossary-grid">
          <div v-for="stack in filteredStacks" :key="stack.id" class="glossary-card">
            <div class="glossary-card-head">
              <span class="glossary-card-name">{{
                stackNames[stack.id] ?? stack.id
              }}</span>
              <ToggleSwitch
                :model-value="isStackEnabled(stack.id)"
                :aria-label="`${t.coding.glossary.enableStack}: ${stackNames[stack.id] ?? stack.id}`"
                @update:model-value="(v) => onToggleStack(stack, v)"
              />
            </div>
            <p class="glossary-card-meta">
              {{ t.coding.glossary.termsCount.replace('{count}', String(stack.terms.length)) }}
            </p>
            <Button
              type="button"
              severity="secondary"
              size="small"
              icon="pi pi-search"
              :label="t.coding.glossary.browse"
              @click="openViewer(stack)"
            />
          </div>
        </div>
      </div>
    </section>

    <Message v-if="lastError" severity="error" class="settings-error">{{
      errorText()
    }}</Message>

    <Dialog v-model:visible="viewerVisible" :header="viewerTitle" modal class="glossary-dialog">
      <div v-if="viewer" class="glossary-viewer">
        <div class="dict-search-wrap">
          <InputText
            v-model="viewerQuery"
            class="dict-search"
            :placeholder="t.coding.glossary.dialogSearchPlaceholder"
            :aria-label="t.coding.glossary.dialogSearchPlaceholder"
          />
          <button
            v-if="viewerQuery"
            type="button"
            class="dict-search-clear"
            :aria-label="t.common.clear"
            :title="t.common.clear"
            @click="viewerQuery = ''"
          >
            <i class="pi pi-times" aria-hidden="true" />
          </button>
        </div>
        <p v-if="!viewerTerms.length" class="glossary-empty">
          {{ t.coding.glossary.empty }}
        </p>
        <ul class="glossary-terms">
          <li v-for="term in viewerTerms" :key="term.term" class="glossary-term">
            <span class="glossary-term-main">{{ term.term }}</span>
            <span v-if="term.aliases.length" class="glossary-term-aliases">
              {{ term.aliases.join(t.coding.glossary.aliasSeparator) }}
            </span>
          </li>
        </ul>
      </div>
      <template #footer>
        <Button
          type="button"
          severity="secondary"
          :label="t.common.close"
          @click="viewer = null"
        />
      </template>
    </Dialog>
  </div>
</template>

<style scoped>
.dict-toolbar {
  display: flex;
  align-items: center;
  gap: var(--vt-spacing-sm);
  padding: var(--vt-spacing-md);
  border-bottom: 1px solid var(--vt-color-border);
  flex-wrap: wrap;
}

.dict-counter {
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
  min-width: 96px;
}

.dict-search-wrap {
  position: relative;
  flex: 1;
  min-width: 160px;
}

.dict-search {
  width: 100%;
  padding-right: 30px;
}

.dict-search-clear {
  position: absolute;
  top: 50%;
  right: 8px;
  transform: translateY(-50%);
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--vt-color-text-muted);
  font-size: 12px;
  cursor: pointer;
}

.glossary-hint {
  margin: 0;
  padding: 8px var(--vt-spacing-md);
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
  border-bottom: 1px solid var(--vt-color-border);
}

.glossary-empty {
  margin: 0;
  padding: var(--vt-spacing-md);
  font-size: 12.5px;
  color: var(--vt-color-text-muted);
}

.glossary-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: var(--vt-spacing-md);
  padding: var(--vt-spacing-md);
}

.glossary-card {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-sm);
  padding: var(--vt-spacing-md);
  border: 1px solid var(--vt-color-border);
  border-radius: var(--vt-radius-card);
  background: var(--vt-color-surface);
}

.glossary-card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--vt-spacing-sm);
}

.glossary-card-name {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--vt-color-text);
}

.glossary-card-meta {
  margin: 0;
  font-size: 12px;
  color: var(--vt-color-text-muted);
}

.glossary-viewer {
  display: flex;
  flex-direction: column;
  gap: var(--vt-spacing-sm);
  min-width: 420px;
  max-height: 420px;
  overflow-y: auto;
}

.glossary-terms {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
}

.glossary-term {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--vt-spacing-md);
  padding: 6px 2px;
  border-bottom: 1px solid var(--vt-color-border);
  font-size: 12.5px;
}

.glossary-term-main {
  color: var(--vt-color-text);
}

.glossary-term-aliases {
  color: var(--vt-color-text-muted);
  text-align: right;
  overflow-wrap: anywhere;
}
</style>
