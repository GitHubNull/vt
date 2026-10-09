/**
 * 设置主窗口路由（tech/03 3.6）：与 PRD 第 2 章导航体系一一对应，全部懒加载。
 * 侧边栏 / 分组导航由本文件的路由元信息派生（单一数据源，杜绝重复渲染）。
 */
import {
  createRouter,
  createWebHashHistory,
  type Router,
  type RouteRecordRaw,
} from 'vue-router'

/** 一级导航模块（PRD 2.2 顺序，编程置于首位；不含已移除的“音视频转录”）。 */
export const NAV_ORDER = ['coding', 'dict', 'ai', 'general', 'me', 'stats', 'faq', 'about'] as const
export type NavModuleKey = (typeof NAV_ORDER)[number]

declare module 'vue-router' {
  interface RouteMeta {
    /** 一级导航模块元信息：存在即出现在侧边栏。 */
    navModule?: { key: NavModuleKey; icon: string; order: number }
    /** 二级分组导航元信息。 */
    navChild?: { titleKey: string }
  }
}

const comingSoon = () => import('./components/ComingSoon.vue')

export const routes: RouteRecordRaw[] = [
  { path: '/', redirect: '/general/interface' },
  {
    path: '/coding',
    meta: { navModule: { key: 'coding', icon: 'pi pi-code', order: 1 } },
    children: [
      { path: '', redirect: '/coding/scenes' },
      {
        path: 'scenes',
        name: 'coding-scenes',
        component: comingSoon,
        meta: { navChild: { titleKey: 'navChildren.coding.scenes' } },
      },
      {
        path: 'commands',
        name: 'coding-commands',
        component: comingSoon,
        meta: { navChild: { titleKey: 'navChildren.coding.commands' } },
      },
      {
        path: 'glossary',
        name: 'coding-glossary',
        component: comingSoon,
        meta: { navChild: { titleKey: 'navChildren.coding.glossary' } },
      },
    ],
  },
  {
    path: '/dict',
    meta: { navModule: { key: 'dict', icon: 'pi pi-book', order: 2 } },
    children: [
      { path: '', redirect: '/dict/replace' },
      {
        path: 'replace',
        name: 'dict-replace',
        component: comingSoon,
        meta: { navChild: { titleKey: 'navChildren.dict.replace' } },
      },
      {
        path: 'proper-nouns',
        name: 'dict-proper-nouns',
        component: comingSoon,
        meta: { navChild: { titleKey: 'navChildren.dict.properNouns' } },
      },
    ],
  },
  {
    path: '/ai',
    meta: { navModule: { key: 'ai', icon: 'pi pi-bolt', order: 3 } },
    children: [
      { path: '', redirect: '/ai/templates' },
      {
        path: 'templates',
        name: 'ai-templates',
        component: comingSoon,
        meta: { navChild: { titleKey: 'navChildren.ai.templates' } },
      },
      {
        path: 'mcp',
        name: 'ai-mcp',
        component: comingSoon,
        meta: { navChild: { titleKey: 'navChildren.ai.mcp' } },
      },
      {
        path: 'providers',
        name: 'ai-providers',
        component: comingSoon,
        meta: { navChild: { titleKey: 'navChildren.ai.providers' } },
      },
    ],
  },
  {
    path: '/general',
    meta: { navModule: { key: 'general', icon: 'pi pi-sliders-h', order: 4 } },
    children: [
      { path: '', redirect: '/general/interface' },
      {
        path: 'interface',
        name: 'general-interface',
        component: () => import('./views/general/InterfaceView.vue'),
        meta: { navChild: { titleKey: 'navChildren.general.interface' } },
      },
      {
        path: 'input',
        name: 'general-input',
        component: () => import('./views/general/InputView.vue'),
        meta: { navChild: { titleKey: 'navChildren.general.input' } },
      },
      {
        path: 'output',
        name: 'general-output',
        component: () => import('./views/general/OutputView.vue'),
        meta: { navChild: { titleKey: 'navChildren.general.output' } },
      },
      {
        path: 'recording',
        name: 'general-recording',
        component: () => import('./views/general/RecordingView.vue'),
        meta: { navChild: { titleKey: 'navChildren.general.recording' } },
      },
      {
        path: 'system',
        name: 'general-system',
        component: () => import('./views/general/SystemView.vue'),
        meta: { navChild: { titleKey: 'navChildren.general.system' } },
      },
      {
        path: 'models',
        name: 'general-models',
        component: comingSoon,
        meta: { navChild: { titleKey: 'navChildren.general.models' } },
      },
    ],
  },
  {
    path: '/me',
    meta: { navModule: { key: 'me', icon: 'pi pi-user', order: 5 } },
    children: [
      { path: '', redirect: '/me/profile' },
      {
        path: 'profile',
        name: 'me-profile',
        component: comingSoon,
        meta: { navChild: { titleKey: 'navChildren.me.profile' } },
      },
      {
        path: 'backup',
        name: 'me-backup',
        component: comingSoon,
        meta: { navChild: { titleKey: 'navChildren.me.backup' } },
      },
    ],
  },
  {
    path: '/stats',
    name: 'stats',
    component: comingSoon,
    meta: { navModule: { key: 'stats', icon: 'pi pi-chart-bar', order: 6 } },
  },
  {
    path: '/faq',
    name: 'faq',
    component: comingSoon,
    meta: { navModule: { key: 'faq', icon: 'pi pi-question-circle', order: 7 } },
  },
  {
    path: '/about',
    name: 'about',
    component: () => import('./views/about/AboutView.vue'),
    meta: { navModule: { key: 'about', icon: 'pi pi-info-circle', order: 8 } },
  },
  { path: '/:pathMatch(.*)*', redirect: '/general/interface' },
]

export interface NavChildItem {
  path: string
  titleKey: string
}

export interface NavModuleItem {
  key: NavModuleKey
  icon: string
  order: number
  /** 侧边栏点击跳转的路径（有分组时取第一个分组）。 */
  path: string
  children: NavChildItem[]
}

/**
 * 从路由表派生侧边栏导航（单一数据源）。
 * 传入 routes 便于在 Node 环境（Vitest）中测试，无需实例化 router。
 */
export function buildNavModules(routeList: RouteRecordRaw[] = routes): NavModuleItem[] {
  const modules: NavModuleItem[] = []
  for (const route of routeList) {
    const nav = route.meta?.navModule
    if (!nav) continue
    const children: NavChildItem[] = (route.children ?? [])
      .filter((child) => child.meta?.navChild && typeof child.path === 'string' && child.path !== '')
      .map((child) => ({
        path: `${route.path}/${child.path}`,
        titleKey: child.meta!.navChild!.titleKey,
      }))
    modules.push({
      key: nav.key,
      icon: nav.icon,
      order: nav.order,
      path: children[0]?.path ?? route.path,
      children,
    })
  }
  return modules.sort((a, b) => a.order - b.order)
}

/** 收集路由表中的全部可导航路径（含子路由），用于校验导航与路由一致。 */
export function collectRoutePaths(routeList: RouteRecordRaw[] = routes): string[] {
  const paths: string[] = []
  const walk = (list: RouteRecordRaw[], prefix: string) => {
    for (const route of list) {
      const full = route.path.startsWith('/') ? route.path : `${prefix}/${route.path}`
      if (route.redirect === undefined) paths.push(full)
      if (route.children) walk(route.children as RouteRecordRaw[], full)
    }
  }
  walk(routeList, '')
  return paths
}

/**
 * 创建 router 实例。做成工厂函数而非模块级单例，
 * 以便路由表与导航派生函数可在 Node 环境（Vitest）中直接测试。
 */
export function createAppRouter(): Router {
  return createRouter({
    history: createWebHashHistory(),
    routes,
  })
}
