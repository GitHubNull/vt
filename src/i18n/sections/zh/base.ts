/**
 * 界面文案（zh · base 组）：由原 zh.ts 按域拆分而来（行数门禁）。
 * 键面与组合文件互为一体：新增文案按域放入对应 section。
 */
export const base = {
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
    log: '日志',
    about: '关于',
  },
  navSubtitle: {
    coding: '场景规则、编程口令与技术词库',
    dict: '替换词典与专有名词',
    ai: '提示词模板、MCP 服务与 AI 供应商',
    general: '界面、录音与文本输出设置',
    me: '个人档案与数据备份',
    log: '查看、搜索与导出应用日志',
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
    clear: '清除',
    errorCode: '错误编号',
    v1Coming: 'v1.0 提供',
    loadFailed: '加载失败',
  },
}
