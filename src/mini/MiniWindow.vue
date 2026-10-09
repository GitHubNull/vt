<script setup lang="ts">
import { t } from '../i18n'
</script>

<template>
  <div class="mini-window">{{ t.mini.placeholder }}</div>
</template>

<style scoped>
.mini-window {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
}
</style>
