<script setup>
/**
 * 更新日志对话框（F-ABOUT，验收 4.1.5）：应用内展示最近版本条目（i18n 文案）。
 * 项目不引外部网页、不做网页托管，更新日志不走外链（AGENTS.md 3.3）。
 */
import Dialog from 'primevue/dialog'
import Button from 'primevue/button'
import { t } from '../i18n'

defineProps({
  visible: { type: Boolean, default: false },
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
    :header="t.about.changelog.title"
    :style="{ width: '560px', maxWidth: '90vw' }"
    @update:visible="emit('update:visible', $event)"
  >
    <ol class="changelog-list">
      <li v-for="entry in t.about.changelog.versions" :key="entry.version" class="changelog-entry">
        <p class="changelog-version">
          {{ entry.version }}
          <span class="changelog-date">{{ entry.date }}</span>
        </p>
        <ul class="changelog-items">
          <li v-for="(item, index) in entry.items" :key="index">{{ item }}</li>
        </ul>
      </li>
    </ol>
    <template #footer>
      <Button type="button" :label="t.common.close" @click="close" />
    </template>
  </Dialog>
</template>

<style scoped>
.changelog-list {
  margin: 0;
  padding: 0;
  list-style: none;
  max-height: 60vh;
  overflow-y: auto;
}

.changelog-entry + .changelog-entry {
  margin-top: var(--vt-spacing-md);
  padding-top: var(--vt-spacing-md);
  border-top: 1px solid var(--vt-color-border);
}

.changelog-version {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  color: var(--vt-color-text);
}

.changelog-date {
  margin-inline-start: var(--vt-spacing-sm);
  font-size: 12px;
  font-weight: 400;
  color: var(--vt-color-text-muted);
}

.changelog-items {
  margin: var(--vt-spacing-xs) 0 0;
  padding-inline-start: 20px;
  font-size: 13px;
  line-height: 1.8;
  color: var(--vt-color-text);
}

.changelog-items li + li {
  margin-top: 2px;
}
</style>
