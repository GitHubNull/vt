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
