/**
 * 界面文案（zh-CN）。
 * 约定（tech/03 3.2 / AGENTS.md 3.5）：用户可见文案一律集中于此，不散落在组件中。
 */
export const zhCN = {
  app: {
    title: 'VibeType 码语',
    scaffoldHint: '工程脚手架已就绪，后续功能将在此基础上构建。',
  },
  settings: {
    pingButton: '联调测试（ping）',
    pingRunning: '正在调用后端…',
    pingOk: '后端响应',
    pingFailed: '调用失败',
  },
  mini: {
    placeholder: '语音输入小窗（占位）',
  },
  float: {
    placeholder: '录音悬浮球（占位）',
  },
}

export type Messages = typeof zhCN
