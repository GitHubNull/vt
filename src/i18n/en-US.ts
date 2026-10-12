/**
 * UI strings (en-US). v0.1 ships zh-CN only; this file keeps the English
 * mirror structurally in sync with zh-CN so the Messages type stays complete.
 * 行数门禁：本文件仅为组装入口，正文按域拆在 sections/en/ 下（新增文案放对应 section）。
 */
import type { Messages } from './zh-CN'
import { base } from './sections/en/base'
import { general } from './sections/en/general'
import { content } from './sections/en/content'
import { misc } from './sections/en/misc'

export const enUS: Messages = {
  ...base,
  ...general,
  ...content,
  ...misc,
}
