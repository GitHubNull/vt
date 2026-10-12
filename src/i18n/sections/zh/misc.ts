/**
 * 界面文案（zh · misc 组）：由原 zh.ts 按域拆分而来（行数门禁）。
 * 键面与组合文件互为一体：新增文案按域放入对应 section。
 */
export const misc = {
  about: {
    title: '关于',
    versionCard: {
      checkUpdate: '检查更新',
      checking: '正在检查更新…',
      official: '官网',
      openSource: '项目开源',
      autoCheck: {
        label: '自动检查更新',
        desc: '每天最多一次，仅发送当前版本号、操作系统类型与架构。',
      },
    },
    update: {
      upToDate: '已是最新版本。',
      newVersionPrefix: '发现新版本：',
      notesTitle: '更新摘要',
      applyNow: '立即更新',
      later: '稍后提醒',
      skip: '跳过此版本',
      applying: '正在下载并安装更新…',
      skipped: '已跳过该版本。',
      failedPrefix: '检查更新失败',
      applyFailedPrefix: '更新失败',
      unavailable: '更新服务器暂不可用，请稍后再试。',
    },
    changelog: {
      title: '更新日志',
      desc: '查看版本更新记录。',
      view: '查看',
      versions: [
        {
          version: 'v0.1.10',
          date: '2026-10-11',
          items: ['修复 Windows 下右 Alt 触发键偶发无响应的问题。'],
        },
        {
          version: 'v0.1.9',
          date: '2026-10-10',
          items: [
            '修复模型下载进度卡在 0% 与取消下载后残留假进度的问题。',
            '日志关键词输入框新增一键清除按钮。',
          ],
        },
        {
          version: 'v0.1.8',
          date: '2026-10-10',
          items: ['修复日志查看列表高度自适应与级别筛选下拉面板无底色的问题。'],
        },
      ],
    },
    exportLogs: {
      title: '导出日志',
      desc: '导出最近 1000 行应用日志，自动过滤转录文本等敏感信息。',
      button: '导出',
      exporting: '正在导出…',
      saveTitle: '导出日志',
      donePrefix: '日志已导出：',
      failedPrefix: '导出失败',
    },
    legal: {
      title: '法律信息',
      license: '开源许可证',
      thirdParty: '第三方开源组件声明',
      disclaimer: '免责声明',
      privacy: '隐私说明',
      loadFailed: '内容加载失败',
    },
  },
  notices: {
    configRecovered: '配置文件损坏，已自动回退到上次良好的配置或默认配置。',
  },
  mini: {
    status: {
      hold: '按住麦克风或 {key} 说话',
      toggle: '点击麦克风或按 {key} 开始',
      recording: '正在录音…',
      processing: '识别中…',
      cancelled: '已取消',
    },
    cancel: '取消录音',
    lastResult: '上次结果',
    copy: '复制',
    copySuccess: '已复制到剪贴板',
    copyFailed: '复制失败，请手动选择文本复制',
    openSettings: '打开设置',
  },
  float: {
    modeHold: '按住',
    modeToggle: '切换',
    processing: '识别中',
    dragCancelHint: '松开取消',
    feedback: {
      success: '已上屏',
      degraded: '已输入原始结果',
      failed: '识别失败',
      cancelled: '已取消',
    },
  },
}
