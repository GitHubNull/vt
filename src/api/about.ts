/**
 * 关于页命令封装（T-016 SPEC 命令契约；后端实现见 src-tauri/src/commands/about.rs）。
 * getAppInfo 同时服务于 T-013 系统与数据页（数据目录展示）。
 */
import { invoke } from '@tauri-apps/api/core'
import type { AppInfo, ExportLogsResult, LegalDocument, UpdateCheckResult } from './types'

/** 应用信息：版本、构建信息、三个平台标准目录（tech/08 8.1）。 */
export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('get_app_info')
}

/**
 * 检查更新（F-ABOUT-01、NFR-PR-06）。
 * 请求载荷只含版本号 / OS / 架构，由后端保证；v0.1 更新服务器未就绪时返回 unavailable。
 */
export async function checkUpdate(): Promise<UpdateCheckResult> {
  return invoke<UpdateCheckResult>('check_update')
}

/** 下载并安装更新（Tauri Updater 接入，NFR-S-06 签名校验失败拒绝安装并报错）。 */
export async function applyUpdate(): Promise<void> {
  await invoke('apply_update')
}

/**
 * 导出脱敏日志（F-ABOUT-02、NFR-S-02）：打包最近 1000 行 + 系统信息头，
 * 经脱敏层处理，写入 target_path（由 plugin-dialog 选择的保存位置）。
 */
export async function exportLogs(targetPath: string): Promise<ExportLogsResult> {
  return invoke<ExportLogsResult>('export_logs', { targetPath })
}

/** 法律文本（LICENSE / DISCLAIMER.md / 第三方组件声明，构建时打包进资源）。 */
export async function getLegalText(document: LegalDocument): Promise<string> {
  return invoke<string>('get_legal_text', { document })
}
