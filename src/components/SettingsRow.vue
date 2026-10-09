<script setup>
/**
 * 设置行（PRD 4.4）：左侧标题 + 说明，右侧控件（控件由插槽提供）。
 */
defineProps({
  title: { type: String, required: true },
  desc: { type: String, default: '' },
})
</script>

<template>
  <div class="settings-row">
    <div class="settings-row-text">
      <div class="settings-row-title">{{ title }}</div>
      <p v-if="desc" class="settings-row-desc">{{ desc }}</p>
    </div>
    <div class="settings-row-control">
      <slot />
    </div>
  </div>
</template>
