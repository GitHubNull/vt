/**
 * 界面文案（en · base 组）：由原 en.ts 按域拆分而来（行数门禁）。
 * 键面与组合文件互为一体：新增文案按域放入对应 section。
 */
export const base = {
  app: {
    title: 'VibeType',
    subtitle: 'A privacy-first AI voice input method for programmers',
  },
  shell: {
    backToMini: 'Back to voice mini window',
    backToMiniAria: 'Back to voice mini window',
    sectionNav: 'Section navigation',
    settingsNav: 'Settings navigation',
    comingSoonTitle: 'Coming soon',
    comingSoonDesc: 'This feature will be available in a future release.',
  },
  nav: {
    coding: 'Coding',
    dict: 'Dictionary',
    ai: 'AI',
    general: 'General',
    me: 'Me',
    stats: 'Stats',
    faq: 'FAQ',
    log: 'Logs',
    about: 'About',
  },
  navSubtitle: {
    coding: 'Scene rules, coding commands and tech glossary',
    dict: 'Replacement rules and proper nouns',
    ai: 'Prompt templates, MCP services and AI providers',
    general: 'Interface, recording and text output settings',
    me: 'Profile and data backup',
    log: 'View, search and export app logs',
  },
  navChildren: {
    coding: {
      scenes: 'Scene Rules',
      commands: 'Coding Commands',
      glossary: 'Tech Glossary',
    },
    dict: {
      replace: 'Replacement Rules',
      properNouns: 'Proper Nouns',
    },
    ai: {
      templates: 'Prompt Templates',
      mcp: 'MCP Services',
      providers: 'AI Providers',
    },
    general: {
      interface: 'Interface',
      input: 'Input & Recording',
      output: 'Text Output',
      recording: 'Recording Environment',
      system: 'System & Data',
      models: 'Model Management',
    },
    me: {
      profile: 'Profile',
      backup: 'Backup & Restore',
    },
  },
  common: {
    open: 'Open',
    close: 'Close',
    cancel: 'Cancel',
    confirm: 'OK',
    retry: 'Retry',
    clear: 'Clear',
    errorCode: 'Error code',
    v1Coming: 'Available in v1.0',
    loadFailed: 'Failed to load',
  },
}
