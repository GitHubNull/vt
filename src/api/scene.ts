/**
 * 场景规则命令封装（T-501 契约 + T-502 扩展，src-tauri/src/commands/scene.rs）。
 * 命令名与参数名与后端逐字一致（tech/03 3.5）；组件不直接 invoke。
 */
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { SceneEntry, SceneKind } from './types'

/** 场景判定来源（与 Rust SceneSource 逐字对应）。 */
export type SceneSource = 'override' | 'auto'

/**
 * 当前场景状态（`scene_get_current` 返回载荷；T-501 契约 + T-502 条目扩展）。
 * `app` / `process` 仅用于"当前识别到：×× → ××场景"展示；锁定（override）时为空。
 */
export interface SceneStatus {
  scene: SceneKind
  source: SceneSource
  app: string
  process: string
  /** 生效场景规则条目 ID（T-502；null = 无匹配条目）。 */
  entry_id: string | null
  /** 生效场景规则条目名称（T-502，展示用）。 */
  entry_name: string | null
}

/**
 * 场景变更事件名（events.rs `scene:changed`，只携带场景标识字符串）。
 * 说明：api/events.ts 的 EVENTS 尚未收录，待集成时并入（报告已给出片段）。
 */
export const SCENE_CHANGED_EVENT = 'scene:changed'

/** 查询当前场景（override 优先；自动识别走后端短缓存）。 */
export async function sceneGetCurrent(): Promise<SceneStatus> {
  return invoke<SceneStatus>('scene_get_current')
}

/** 临时锁定场景（PRD 3.2.1 规则 3：锁定期间自动切换不生效，直到解除）。 */
export async function sceneSetOverride(scene: SceneKind): Promise<void> {
  await invoke('scene_set_override', { scene })
}

/** 解除锁定，恢复自动识别。 */
export async function sceneClearOverride(): Promise<void> {
  await invoke('scene_clear_override')
}

/** 生效场景规则条目清单（配置为空 = 内置五场景默认；内置条目始终在列）。 */
export async function sceneListEntries(): Promise<SceneEntry[]> {
  return invoke<SceneEntry[]>('scene_list_entries')
}

/**
 * 新增自定义场景条目（ID / 内置标记由后端生成与裁决）。
 * 非法输入（名称为空 / 颜色格式 / 应用超限等）抛 AppError（E-CONFIG-004）。
 */
export async function sceneAddEntry(entry: SceneEntry): Promise<SceneEntry> {
  return invoke<SceneEntry>('scene_add_entry', { entry })
}

/** 更新场景条目（内置条目可编辑；ID 与内置标记不变）。 */
export async function sceneUpdateEntry(entry: SceneEntry): Promise<void> {
  await invoke('scene_update_entry', { entry })
}

/** 删除场景条目（仅自定义条目；内置条目抛 E-CONFIG-005）。 */
export async function sceneDeleteEntry(entryId: string): Promise<void> {
  await invoke('scene_delete_entry', { entryId })
}

/** 恢复内置条目为 PRD 默认规则（返回恢复后的条目；非内置条目抛 E-CONFIG-005）。 */
export async function sceneRestoreEntryDefault(entryId: string): Promise<SceneEntry> {
  return invoke<SceneEntry>('scene_restore_entry_default', { entryId })
}

/** 持久化拖动排序；entryIds 须为当前全部条目 ID 的排列（否则抛 E-CONFIG-004）。 */
export async function sceneReorderEntries(entryIds: string[]): Promise<void> {
  await invoke('scene_reorder_entries', { entryIds })
}

/** 订阅场景变更事件（设置页"当前识别到"联动）。 */
export async function onSceneChanged(handler: (scene: string) => void): Promise<UnlistenFn> {
  return listen<{ scene: string }>(SCENE_CHANGED_EVENT, (event) => handler(event.payload.scene))
}
