<script setup>
/**
 * 法律文本展示对话框（T-016，F-ABOUT-03）：渲染 LICENSE / 免责声明 / 第三方组件声明全文。
 * DISCLAIMER.md 为 Markdown，渲染后展示；LICENSE / 第三方声明为纯文本，保持原样排版。
 */
import { computed } from 'vue'
import Dialog from 'primevue/dialog'
import Button from 'primevue/button'
import { marked } from 'marked'
import { t } from '../i18n'

const props = defineProps({
  visible: { type: Boolean, default: false },
  title: { type: String, default: '' },
  text: { type: String, default: '' },
  markdown: { type: Boolean, default: false },
})

const emit = defineEmits(['update:visible'])

// 文本为构建期 include_str! 打包的本地文档，无外部输入，可直接作为 HTML 插入。
const renderedHtml = computed(() => (props.markdown ? marked.parse(props.text) : ''))

function close() {
  emit('update:visible', false)
}
</script>

<template>
  <Dialog
    modal
    :visible="visible"
    :header="title"
    :style="{ width: '640px', maxWidth: '90vw' }"
    @update:visible="emit('update:visible', $event)"
  >
    <div v-if="markdown" class="legal-md" v-html="renderedHtml" />
    <pre v-else class="legal-text">{{ text }}</pre>
    <template #footer>
      <Button type="button" :label="t.common.close" @click="close" />
    </template>
  </Dialog>
</template>

<style scoped>
.legal-text {
  margin: 0;
  max-height: 60vh;
  overflow: auto;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: ui-monospace, 'Cascadia Mono', Menlo, Consolas, monospace;
  font-size: 12px;
  line-height: 1.6;
  color: var(--vt-color-text);
}

.legal-md {
  max-height: 60vh;
  overflow: auto;
  font-size: 13px;
  line-height: 1.8;
  color: var(--vt-color-text);
  word-break: break-word;
}

.legal-md :deep(h1),
.legal-md :deep(h2),
.legal-md :deep(h3) {
  margin: 16px 0 8px;
  font-size: 15px;
  line-height: 1.4;
}

.legal-md :deep(h1:first-child),
.legal-md :deep(h2:first-child),
.legal-md :deep(h3:first-child) {
  margin-top: 0;
}

.legal-md :deep(p) {
  margin: 8px 0;
}

.legal-md :deep(ol),
.legal-md :deep(ul) {
  margin: 8px 0;
  padding-inline-start: 22px;
}

.legal-md :deep(li) {
  margin: 4px 0;
}

.legal-md :deep(strong) {
  font-weight: 600;
}

.legal-md :deep(code) {
  font-family: ui-monospace, 'Cascadia Mono', Menlo, Consolas, monospace;
  font-size: 12px;
  background: var(--vt-color-bg);
  border-radius: 4px;
  padding: 1px 4px;
}
</style>
