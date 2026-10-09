<script setup lang="ts">
/**
 * v0.5+ 模块占位页（T-002）：统一渲染“敬请期待”。
 */
import { t } from '../i18n'
</script>

<template>
  <div class="coming-soon">
    <i class="pi pi-clock coming-soon-icon" aria-hidden="true" />
    <h2 class="coming-soon-title">{{ t.shell.comingSoonTitle }}</h2>
    <p class="coming-soon-desc">{{ t.shell.comingSoonDesc }}</p>
  </div>
</template>

<style scoped>
.coming-soon {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-height: 320px;
  color: var(--vt-color-text-muted);
}

.coming-soon-icon {
  font-size: 32px;
  color: var(--vt-color-primary);
}

.coming-soon-title {
  margin: 0;
  font-size: 16px;
  color: var(--vt-color-text);
}

.coming-soon-desc {
  margin: 0;
  font-size: 13px;
}
</style>
