/**
 * AI 命令封装（T-532 模型列表管理 / T-533 提示词模板引擎；
 * 契约：src-tauri/src/commands/ai.rs）。命令名与参数名与后端逐字一致（tech/03 3.5）。
 *
 * TS 类型为本地定义（随命令契约独立演进，不扩散到 api/types.ts 的配置镜像）。
 * 安全（NFR-S-01/02）：任何返回值都不含 API Key 明文，仅"已设置"布尔状态。
 */
import { invoke } from '@tauri-apps/api/core'

/** 供应商视图（`ai_list_providers`；不含任何 Key 内容）。 */
export interface ProviderView {
  id: string
  name: string
  protocol: string
  base_url: string
  enabled: boolean
  /** 当前默认模型（T-532：未显式指定模型的调用使用它）。 */
  model: string | null
  /** 已拉取 / 手动维护的模型标识清单（T-532，去重保序；模板绑定下拉数据源）。 */
  available_models: string[]
  /** 本地供应商（Ollama / LM Studio，数据不出本机）。 */
  local: boolean
  /** 云端供应商需要 Key。 */
  key_required: boolean
  /** Key 是否已设置（只回状态，不回内容）。 */
  key_set: boolean
  /** 预置推荐项。 */
  recommended: boolean
}

/** 模型拉取合并结果（`ai_fetch_and_merge_models`，T-532）。 */
export interface ModelMergeReport {
  /** 本次新加入的模型标识（按拉取顺序）。 */
  added: string[]
  /** 合并后的完整模型清单（去重保序）。 */
  models: string[]
}

/** AI 提示词模板（T-533；与 templates.json 结构一致）。 */
export interface AiTemplate {
  id: string
  name: string
  prompt: string
  /** 绑定供应商 ID；null = 无（纯转录）。 */
  provider_id: string | null
  /** 绑定模型标识；选择供应商后必填。 */
  model: string | null
  /** 快捷键（存储字段预留；绑定机制在 T-534）。 */
  hotkey: string | null
  /** 超时时间（毫秒，1000–60000）。 */
  timeout_ms: number
  /** 内置模板标记（只读、可复制，不可修改 / 删除）。 */
  builtin: boolean
}

/** 供应商清单（首次调用自动灌入预置）。 */
export async function aiListProviders(): Promise<ProviderView[]> {
  return invoke<ProviderView[]>('ai_list_providers')
}

/** 拉取供应商模型清单并合并持久化（T-532，F-AI-08；一键拉取）。 */
export async function aiFetchAndMergeModels(id: string): Promise<ModelMergeReport> {
  return invoke<ModelMergeReport>('ai_fetch_and_merge_models', { id })
}

/** 手动添加模型条目（供应商不支持列表 API 时的兜底）。 */
export async function aiAddProviderModel(id: string, model: string): Promise<void> {
  await invoke('ai_add_provider_model', { id, model })
}

/** 删除模型条目（删除当前默认模型时默认选择一并清空）。 */
export async function aiRemoveProviderModel(id: string, model: string): Promise<void> {
  await invoke('ai_remove_provider_model', { id, model })
}

/** 设置 / 清除供应商默认模型（`null` 清除；T-532）。 */
export async function aiSetProviderModel(id: string, model: string | null): Promise<void> {
  await invoke('ai_set_provider_model', { id, model })
}

/** 模板清单（首次调用自动初始化内置模板，内置在前）。 */
export async function aiListTemplates(): Promise<AiTemplate[]> {
  return invoke<AiTemplate[]>('ai_list_templates')
}

/** 新建模板（名称唯一与绑定校验在后端；`id` 为空由后端生成）。 */
export async function aiCreateTemplate(template: AiTemplate): Promise<AiTemplate> {
  return invoke<AiTemplate>('ai_create_template', { template })
}

/** 更新模板（整体替换；内置模板只读 → E-CONFIG-004）。 */
export async function aiUpdateTemplate(template: AiTemplate): Promise<AiTemplate> {
  return invoke<AiTemplate>('ai_update_template', { template })
}

/** 删除模板（内置模板不可删除 → E-CONFIG-004）。 */
export async function aiDeleteTemplate(id: string): Promise<void> {
  await invoke('ai_delete_template', { id })
}

/** 复制模板为自定义（内置模板的唯一定制路径，名称自动唯一化）。 */
export async function aiCopyTemplate(id: string): Promise<AiTemplate> {
  return invoke<AiTemplate>('ai_copy_template', { id })
}
