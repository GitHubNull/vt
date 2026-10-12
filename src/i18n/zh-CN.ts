/**
 * 界面文案（zh-CN，v0.1 默认语言）。
 * 约定（tech/03 3.2 / AGENTS.md 3.5）：用户可见文案一律集中于此，不散落在组件中。
 * 行数门禁：本文件仅为组装入口，正文按域拆在 sections/zh/ 下（新增文案放对应 section）。
 */
import { base } from './sections/zh/base'
import { general } from './sections/zh/general'
import { content } from './sections/zh/content'
import { misc } from './sections/zh/misc'

export const zhCN = {
  ...base,
  ...general,
  ...content,
  ...misc,
}

export type Messages = typeof zhCN
