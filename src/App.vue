<script setup lang="ts">
/**
 * 设置主窗口三栏外壳（T-002，PRD 4.2.1）：
 * 一级导航（侧边栏，自路由元信息派生）→ 二级分组导航 → 内容区（仅内容区滚动）。
 */
import { computed } from 'vue'
import { RouterLink, RouterView, useRoute } from 'vue-router'
import { getMessage, t } from './i18n'
import { buildNavModules } from './router'
import { showMiniWindow } from './api/window'

const route = useRoute()

// 单一数据源：侧边栏与分组导航均由路由表派生（UI-FIX-01：不重复渲染）
const modules = buildNavModules()

const activeModule = computed(
  () => modules.find((m) => route.path === `/${m.key}` || route.path.startsWith(`/${m.key}/`)) ?? null,
)
</script>

<template>
  <div class="shell">
    <aside class="shell-sidebar">
      <button type="button" class="shell-back" @click="showMiniWindow">
        <i class="pi pi-arrow-left" aria-hidden="true" />
        <span>{{ t.shell.backToMini }}</span>
      </button>
      <nav class="shell-nav" :aria-label="t.shell.settingsNav">
        <RouterLink
          v-for="m in modules"
          :key="m.key"
          :to="m.path"
          class="shell-nav-item"
          :class="{ 'shell-nav-item--active': activeModule?.key === m.key }"
        >
          <i :class="['shell-nav-icon', m.icon]" aria-hidden="true" />
          <span>{{ getMessage(`nav.${m.key}`) }}</span>
        </RouterLink>
      </nav>
    </aside>

    <div class="shell-main">
      <aside v-if="activeModule && activeModule.children.length > 0" class="shell-groups">
        <header class="shell-groups-header">
          <h2 class="shell-groups-title">{{ getMessage(`nav.${activeModule.key}`) }}</h2>
          <p class="shell-groups-subtitle">{{ getMessage(`navSubtitle.${activeModule.key}`) }}</p>
        </header>
        <nav
          class="shell-group-nav"
          :aria-label="`${getMessage(`nav.${activeModule.key}`)}${t.shell.sectionNav}`"
        >
          <RouterLink
            v-for="c in activeModule.children"
            :key="c.path"
            :to="c.path"
            class="shell-group-item"
            active-class="shell-group-item--active"
          >
            {{ getMessage(c.titleKey) }}
          </RouterLink>
        </nav>
      </aside>

      <main class="shell-content">
        <RouterView />
      </main>
    </div>
  </div>
</template>
