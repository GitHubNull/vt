/**
 * 设置主窗口路由（tech/03 3.6）：每个设置模块一条路由，后续任务按需扩展并启用懒加载。
 */
import { createRouter, createWebHashHistory } from 'vue-router'
import HomeView from './views/HomeView.vue'

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [{ path: '/', name: 'home', component: HomeView }],
})
