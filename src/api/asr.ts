/**
 * 模型管理命令封装（T-006 契约，src-tauri/src/commands/asr.rs）。
 * 命令名与参数名与后端逐字一致（tech/03 3.5）。
 */
import { invoke } from '@tauri-apps/api/core'
import type { ModelInfo } from './types'

/** 模型清单（含下载 / 激活 / 损坏状态）。 */
export async function listModels(): Promise<ModelInfo[]> {
  return invoke<ModelInfo[]>('list_models')
}

/** 下载模型；进度经 model:progress 事件推送（stores/asr 消费）。 */
export async function downloadModel(modelId: string): Promise<void> {
  await invoke('download_model', { modelId })
}

/** 取消进行中的下载。 */
export async function cancelDownload(modelId: string): Promise<void> {
  await invoke('cancel_download', { modelId })
}

/** 删除模型；删除激活模型时后端回退到首个可用模型并广播 asr:notice。 */
export async function deleteModel(modelId: string): Promise<void> {
  await invoke('delete_model', { modelId })
}

/** 切换激活模型；切换后下一次识别即时生效。 */
export async function setActiveModel(modelId: string): Promise<void> {
  await invoke('set_active_model', { modelId })
}

/** 识别指定 16 kHz 单声道 WAV 文件（本地验证命令，返回识别文本）。 */
export async function asrRecognizeWav(wavPath: string): Promise<string> {
  return invoke<string>('asr_recognize_wav', { wavPath })
}

/* ---- 识别标点恢复（T-545，后端 services/punct.rs / commands/asr.rs） ---- */

/** 标点模型稳定 ID（model:progress 事件的 model_id 过滤用；与后端常量一致）。 */
export const PUNCT_MODEL_ID = 'punct-ct-transformer-zh-en-int8'

/**
 * 标点模型卡片状态（后端 PunctModelInfo 序列化形态，src-tauri/src/services/punct.rs）。
 * 注：与 DownloadProgress 复用同一进度结构（src/api/types.ts）。
 */
export interface PunctModelInfo {
  id: string
  name: string
  size_bytes: number
  /** 已下载且哈希校验通过。 */
  downloaded: boolean
  /** 模型文件存在但校验失败（损坏，引导重新下载）。 */
  invalid: boolean
  /** 下载中进度（非下载中为 null）。 */
  progress: {
    downloaded_bytes: number
    total_bytes: number
    bytes_per_sec: number
  } | null
  /** 生效的模型存储目录。 */
  models_dir: string
}

/** 标点模型卡片状态（未下载 / 已下载 / 损坏 / 下载中）。 */
export async function punctModelStatus(): Promise<PunctModelInfo> {
  return invoke<PunctModelInfo>('punct_model_status')
}

/** 下载标点模型（幂等；进度经 model:progress 事件推送）。 */
export async function downloadPunctModel(): Promise<void> {
  await invoke('download_punct_model')
}

/** 取消进行中的标点模型下载。 */
export async function cancelPunctModel(): Promise<void> {
  await invoke('cancel_punct_model')
}

/** 删除标点模型；删除后状态回退为未下载，标点恢复降级为原文直出。 */
export async function deletePunctModel(): Promise<void> {
  await invoke('delete_punct_model')
}
