<script setup lang="ts">
import { ref } from 'vue'
import { RouterView } from 'vue-router'
import { t } from './i18n'

// 联调状态展示（ping 调用在 HomeView 中进行，此处仅承载路由出口）
const title = ref(t.app.title)
</script>

<template>
  <div class="app-shell">
    <header class="app-header">
      <h1>{{ title }}</h1>
    </header>
    <RouterView />
  </div>
</template>
