<script setup lang="ts">
import { t } from '../i18n'
</script>

<template>
  <!-- 悬浮球只渲染录音反馈，不含任何可聚焦控件（tech/03 3.1） -->
  <div class="float-ball">{{ t.float.placeholder }}</div>
</template>

<style scoped>
.float-ball {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  border-radius: 50%;
  background: rgba(99, 102, 241, 0.9);
  color: #fff;
  font-size: 12px;
  user-select: none;
}
</style>
