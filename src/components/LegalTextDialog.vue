<script setup>
/**
 * 法律文本展示对话框（T-016，F-ABOUT-03）：渲染 LICENSE / 免责声明 / 第三方组件声明全文。
 */
import Dialog from 'primevue/dialog'
import Button from 'primevue/button'
import { t } from '../i18n'

defineProps({
  visible: { type: Boolean, default: false },
  title: { type: String, default: '' },
  text: { type: String, default: '' },
})

const emit = defineEmits(['update:visible'])

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
    <pre class="legal-text">{{ text }}</pre>
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
</style>
