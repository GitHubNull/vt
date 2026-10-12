/**
 * 技术词库命令封装（T-521 契约，src-tauri/src/commands/dict.rs）。
 * 命令名与参数名与后端逐字一致（tech/03 3.5）。
 */
import { invoke } from '@tauri-apps/api/core'
import type { TechStackLexicon } from './types'

/** 内置技术词库全部栈清单（只读数据；栈启停经配置 dict 节的 enabled_stacks）。 */
export async function techLexiconStacks(): Promise<TechStackLexicon[]> {
  return invoke<TechStackLexicon[]>('tech_lexicon_stacks')
}
