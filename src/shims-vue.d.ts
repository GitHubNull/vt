/**
 * .vue SFC 模块声明：v0.1 约定 SFC script 为纯 JS（ESLint 无 TS parser，见 eslint.config.js 注释），
 * 由 vue-tsc 负责 .ts 类型检查；此处提供通配声明使 .ts 文件可导入 SFC。
 */
declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<Record<string, never>, Record<string, never>, unknown>
  export default component
}
