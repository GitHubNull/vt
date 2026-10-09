/**
 * 界面文案（zh-CN，v0.1 默认语言）。
 * 约定（tech/03 3.2 / AGENTS.md 3.5）：用户可见文案一律集中于此，不散落在组件中。
 */
export const zhCN = {
  app: {
    title: 'VibeType 码语',
    subtitle: '面向程序员的隐私优先 AI 语音输入法',
  },
  shell: {
    backToMini: '回到语音输入小窗',
    backToMiniAria: '回到语音输入小窗',
    sectionNav: '分组导航',
    settingsNav: '设置导航',
    comingSoonTitle: '敬请期待',
    comingSoonDesc: '该功能将在后续版本提供。',
  },
  nav: {
    coding: '编程',
    dict: '词典',
    ai: 'AI',
    general: '通用',
    me: '我的',
    stats: '统计',
    faq: '常见问题',
    about: '关于',
  },
  navSubtitle: {
    coding: '场景规则、编程口令与技术词库',
    dict: '替换词典与专有名词',
    ai: '提示词模板、MCP 服务与 AI 供应商',
    general: '界面、录音与文本输出设置',
    me: '个人档案与数据备份',
  },
  navChildren: {
    coding: {
      scenes: '场景规则',
      commands: '编程口令',
      glossary: '技术词库',
    },
    dict: {
      replace: '替换词典',
      properNouns: '专有名词',
    },
    ai: {
      templates: '提示词管理',
      mcp: 'MCP 服务',
      providers: 'AI 供应商',
    },
    general: {
      interface: '界面',
      input: '输入与录音',
      output: '文本输出',
      recording: '录音环境',
      system: '系统与数据',
      models: '模型管理',
    },
    me: {
      profile: '个人档案',
      backup: '数据备份与恢复',
    },
  },
  common: {
    open: '打开',
    close: '关闭',
    cancel: '取消',
    confirm: '确定',
    retry: '重试',
    errorCode: '错误编号',
    v1Coming: 'v1.0 提供',
    loadFailed: '加载失败',
  },
  general: {
    interface: {
      title: '界面',
      desc: '设置界面语言与主题。',
      group: '界面',
      language: {
        label: '语言',
        desc: '选择显示语言 / Select display language',
        options: {
          system: '跟随系统',
          zhCN: '简体中文',
          enUS: 'English（v1.0 提供）',
          zhTW: '繁體中文（v1.0 提供）',
        },
      },
      theme: {
        label: '主题',
        desc: '选择界面主题配色。',
        options: {
          system: '跟随系统',
          light: '浅色',
          dark: '深色（v1.0 提供）',
        },
      },
    },
    input: {
      title: '输入与录音',
      desc: '设置录音快捷键与录音行为。',
      group: '输入与录音',
      recordHotkey: {
        label: '录音快捷键',
        desc: '设置开始 / 结束录音的全局快捷键，此快捷键不会进行 AI 处理，支持鼠标侧键。',
      },
      recordMode: {
        label: '触发模式',
        desc: '按住说话：按住快捷键录音，松开停止；切换模式：按一次开始，再按停止。',
        options: {
          hold: '按住说话',
          toggle: '按一次开始，再按停止',
        },
      },
      cancelHotkey: {
        label: '取消录音快捷键',
        desc: '录音中按下此键放弃本次输入。',
      },
      maxDuration: {
        label: '最长录音时长',
        desc: '达到上限后自动结束录音。',
        unit: '分钟',
      },
      floatBall: {
        label: '悬浮球',
        desc: '开启后，录音时会显示一个带动效的悬浮球指示器。',
      },
      floatBallPosition: {
        label: '悬浮球位置',
        desc: '选择悬浮球出现的位置。',
        options: {
          cursor: '跟随光标',
          bottomCenter: '屏幕底部居中',
          fixed: '固定位置',
        },
      },
      soundFeedback: {
        label: '提示音',
        desc: '开始 / 结束录音时播放轻提示音。',
      },
      hotkeyPlaceholder: '未设置',
      hotkeyRecording: '请按下新的快捷键…',
      hotkeyChange: '更改',
      hotkeyAriaLabel: '快捷键输入框',
      hotkeyInvalid: '无效组合：请至少包含一个非修饰键。',
    },
    output: {
      title: '文本输出',
      desc: '设置识别结果的文本处理选项。',
      group: '文本输出',
      smartNumber: {
        label: '智能数字转换',
        desc: '中文数字自动转为阿拉伯数字（如“一百二十”→“120”）。注：复杂连续数字可能出错。',
      },
      filterFiller: {
        label: '过滤语气词',
        desc: '自动删除“嗯”“啊”等语音输入中的语气词。',
      },
      traditionalChinese: {
        label: '输出繁体中文',
        desc: '开启后，实时语音输入会输出繁体中文。',
      },
      removeEndingPunctuation: {
        label: '删除结尾标点',
        desc: '开启后，转录结果会自动删除结尾的标点（包括“。”和“？”）。场景规则中的设置优先。',
      },
      pasteInjection: {
        label: '使用复制粘贴输入',
        desc: '开启后，始终通过复制粘贴输入文本，速度更快、更稳定。',
      },
      keepInClipboard: {
        label: '保留识别文本到剪贴板',
        desc: '开启后，语音输入完成后会将识别结果保留在剪贴板中（会替换系统当前剪贴板内容）。',
      },
    },
    recording: {
      title: '录音环境',
      desc: '选择麦克风并配置录音环境。',
      group: '录音环境',
      mic: {
        label: '选择麦克风',
        desc: '手动指定优先使用的麦克风；选择系统默认则跟随系统设置。指定的麦克风断开时自动回退到系统默认并提示。',
        default: '系统默认',
        empty: '未检测到麦克风',
        refresh: '刷新',
        refreshAria: '刷新麦克风列表',
      },
      mute: {
        label: '录音时静音',
        desc: '开启后，录音时自动静音系统扬声器输出，录音结束后自动恢复。',
      },
      keepWarm: {
        label: '麦克风常驻',
        desc: '保持麦克风预热状态，减少录音启动延迟（建议麦克风启动慢的用户开启）。麦克风已预热，但仅在您按下快捷键时才会录音。',
      },
      mobileMic: {
        label: '移动麦克风',
        desc: '把手机 / 平板作为桌面端的无线麦克风使用，支持扫码配对与局域网发现。',
        note: 'v1.0 提供',
      },
    },
    system: {
      title: '系统与数据',
      desc: '配置启动、数据保存与诊断选项。',
      group: '系统与数据',
      launchAtLogin: {
        label: '开机自启动',
        desc: '默认关闭。按需开启以便开机后自动运行。',
      },
      saveDataset: {
        label: '保存语音数据集',
        desc: '开启后，每次转录会自动保存音频文件和文本到数据集目录。',
        hint: '注意：保存语音数据集会占用磁盘空间，请留意隐私风险。',
        openDir: '打开数据集目录',
      },
      dirs: {
        label: '数据目录',
        desc: '配置、数据与日志的保存位置。',
        config: '配置目录',
        data: '数据目录',
        logs: '日志目录',
        open: '打开所在位置',
      },
      diagnostic: {
        label: '诊断模式',
        desc: '开启后记录更详细的调试日志（日志级别 debug），仅用于排查问题，日常使用建议关闭。',
      },
    },
  },
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
  hotkey: {
    keys: {
      AltRight: '右 Alt',
      AltRightMac: '右 Option',
      AltLeft: 'Alt',
      ControlLeft: 'Ctrl',
      ControlRight: '右 Ctrl',
      ShiftLeft: 'Shift',
      ShiftRight: '右 Shift',
      MetaLeft: 'Meta',
      MetaLeftMac: '⌘ Command',
      MetaRight: '右 Meta',
      Escape: 'Esc',
      Space: '空格',
      Enter: '回车',
      Tab: 'Tab',
      Backspace: '退格',
      Delete: 'Delete',
      Home: 'Home',
      End: 'End',
      PageUp: 'PageUp',
      PageDown: 'PageDown',
      ArrowUp: '↑',
      ArrowDown: '↓',
      ArrowLeft: '←',
      ArrowRight: '→',
      MouseBack: '鼠标侧键（后退）',
      MouseForward: '鼠标侧键（前进）',
    },
  },
  notices: {
    configRecovered: '配置文件损坏，已自动回退到上次良好的配置或默认配置。',
  },
  mini: {
    placeholder: '语音输入小窗（占位）',
  },
  float: {
    placeholder: '录音悬浮球（占位）',
  },
}

export type Messages = typeof zhCN
