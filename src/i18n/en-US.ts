/**
 * UI strings (en-US). v0.1 ships zh-CN only; this file is a placeholder kept
 * structurally in sync with zh-CN so the Messages type stays complete.
 */
import type { Messages } from './zh-CN'

export const enUS: Messages = {
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
    about: 'About',
  },
  navSubtitle: {
    coding: 'Scene rules, coding commands and tech glossary',
    dict: 'Replacement rules and proper nouns',
    ai: 'Prompt templates, MCP services and AI providers',
    general: 'Interface, recording and text output settings',
    me: 'Profile and data backup',
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
    errorCode: 'Error code',
    v1Coming: 'Available in v1.0',
    loadFailed: 'Failed to load',
  },
  general: {
    interface: {
      title: 'Interface',
      desc: 'Set display language and theme.',
      group: 'Interface',
      language: {
        label: 'Language',
        desc: 'Select display language / 选择显示语言',
        options: {
          system: 'System',
          zhCN: '简体中文',
          enUS: 'English (in v1.0)',
          zhTW: '繁體中文（v1.0 提供）',
        },
      },
      theme: {
        label: 'Theme',
        desc: 'Choose the UI theme.',
        options: {
          system: 'System',
          light: 'Light',
          dark: 'Dark (in v1.0)',
        },
      },
    },
    input: {
      title: 'Input & Recording',
      desc: 'Configure the recording hotkey and recording behavior.',
      group: 'Input & Recording',
      recordHotkey: {
        label: 'Recording Hotkey',
        desc: 'Global hotkey to start / stop recording. It is never sent for AI processing. Mouse side buttons are supported.',
      },
      recordMode: {
        label: 'Trigger Mode',
        desc: 'Hold to talk: hold the hotkey to record, release to stop. Toggle: press once to start, press again to stop.',
        options: {
          hold: 'Hold to talk',
          toggle: 'Press once to start, again to stop',
        },
      },
      cancelHotkey: {
        label: 'Cancel Recording Hotkey',
        desc: 'Press during recording to discard the current input.',
      },
      maxDuration: {
        label: 'Max Recording Duration',
        desc: 'Recording stops automatically when the limit is reached.',
        unit: 'min',
      },
      floatBall: {
        label: 'Float Ball',
        desc: 'Show an animated float ball indicator while recording.',
      },
      floatBallPosition: {
        label: 'Float Ball Position',
        desc: 'Where the float ball appears.',
        options: {
          cursor: 'Follow cursor',
          bottomCenter: 'Bottom center of screen',
          fixed: 'Fixed position',
        },
      },
      soundFeedback: {
        label: 'Sound Feedback',
        desc: 'Play a subtle sound when recording starts / stops.',
      },
      hotkeyPlaceholder: 'Not set',
      hotkeyRecording: 'Press a new hotkey…',
      hotkeyChange: 'Change',
      hotkeyAriaLabel: 'Hotkey input',
      hotkeyInvalid: 'Invalid combo: a non-modifier key is required.',
    },
    output: {
      title: 'Text Output',
      desc: 'Configure how recognized text is processed.',
      group: 'Text Output',
      smartNumber: {
        label: 'Smart Number Conversion',
        desc: 'Convert Chinese numbers to Arabic digits (e.g. “一百二十” → “120”). Note: complex consecutive numbers may be wrong.',
      },
      filterFiller: {
        label: 'Filter Filler Words',
        desc: 'Automatically remove filler words such as “嗯” and “啊”.',
      },
      traditionalChinese: {
        label: 'Output Traditional Chinese',
        desc: 'Output Traditional Chinese for real-time voice input.',
      },
      removeEndingPunctuation: {
        label: 'Remove Ending Punctuation',
        desc: 'Automatically remove ending punctuation (including “。” and “？”). Scene rule settings take precedence.',
      },
      pasteInjection: {
        label: 'Paste to Input',
        desc: 'Always input text via copy & paste. Faster and more stable.',
      },
      keepInClipboard: {
        label: 'Keep Recognized Text in Clipboard',
        desc: 'After voice input, keep the result in the clipboard (replaces current clipboard content).',
      },
    },
    recording: {
      title: 'Recording Environment',
      desc: 'Select the microphone and configure the recording environment.',
      group: 'Recording Environment',
      mic: {
        label: 'Microphone',
        desc: 'Manually pick a preferred microphone; “System default” follows OS settings. If the selected device disconnects, it falls back to system default with a notice.',
        default: 'System default',
        empty: 'No microphone detected',
        refresh: 'Refresh',
        refreshAria: 'Refresh microphone list',
      },
      mute: {
        label: 'Mute While Recording',
        desc: 'Mute system speaker output while recording; restore afterwards.',
      },
      keepWarm: {
        label: 'Keep Microphone Warm',
        desc: 'Keep the microphone pre-warmed to reduce recording start latency (recommended if your microphone starts slowly). The mic is warmed up but only records when you press the hotkey.',
      },
      mobileMic: {
        label: 'Mobile Microphone',
        desc: 'Use your phone / tablet as a wireless microphone for the desktop, with QR pairing and LAN discovery.',
        note: 'Available in v1.0',
      },
    },
    system: {
      title: 'System & Data',
      desc: 'Configure startup, data saving and diagnostics.',
      group: 'System & Data',
      launchAtLogin: {
        label: 'Launch at Login',
        desc: 'Off by default. Enable to run automatically after login.',
      },
      saveDataset: {
        label: 'Save Voice Dataset',
        desc: 'Save audio files and text to the dataset directory after each transcription.',
        hint: 'Note: saving the voice dataset uses disk space. Mind the privacy risk.',
        openDir: 'Open dataset directory',
      },
      dirs: {
        label: 'Data Directories',
        desc: 'Where config, data and logs are stored.',
        config: 'Config directory',
        data: 'Data directory',
        logs: 'Log directory',
        open: 'Open location',
      },
      diagnostic: {
        label: 'Diagnostic Mode',
        desc: 'Log at debug level for troubleshooting. Keep it off for daily use.',
      },
    },
  },
  about: {
    title: 'About',
    versionCard: {
      checkUpdate: 'Check for Updates',
      checking: 'Checking for updates…',
      official: 'Website',
      openSource: 'Open Source',
      autoCheck: {
        label: 'Check for updates automatically',
        desc: 'At most once a day. Only the current version, OS type and architecture are sent.',
      },
    },
    update: {
      upToDate: 'You are up to date.',
      newVersionPrefix: 'New version available: ',
      notesTitle: 'Release notes',
      applyNow: 'Update Now',
      later: 'Later',
      skip: 'Skip this version',
      applying: 'Downloading and installing the update…',
      skipped: 'Version skipped.',
      failedPrefix: 'Update check failed',
      applyFailedPrefix: 'Update failed',
      unavailable: 'Update server is unavailable. Please try again later.',
    },
    changelog: {
      title: 'Changelog',
      desc: 'View version release notes.',
      view: 'View',
    },
    exportLogs: {
      title: 'Export Logs',
      desc: 'Export the latest 1000 lines of application logs. Sensitive data such as transcribed text is filtered automatically.',
      button: 'Export',
      exporting: 'Exporting…',
      saveTitle: 'Export logs',
      donePrefix: 'Logs exported: ',
      failedPrefix: 'Export failed',
    },
    legal: {
      title: 'Legal',
      license: 'Open Source License',
      thirdParty: 'Third-party Notices',
      disclaimer: 'Disclaimer',
      privacy: 'Privacy Notes',
      loadFailed: 'Failed to load content',
    },
  },
  hotkey: {
    keys: {
      AltRight: 'Right Alt',
      AltRightMac: 'Right Option',
      AltLeft: 'Alt',
      ControlLeft: 'Ctrl',
      ControlRight: 'Right Ctrl',
      ShiftLeft: 'Shift',
      ShiftRight: 'Right Shift',
      MetaLeft: 'Meta',
      MetaLeftMac: '⌘ Command',
      MetaRight: 'Right Meta',
      Escape: 'Esc',
      Space: 'Space',
      Enter: 'Enter',
      Tab: 'Tab',
      Backspace: 'Backspace',
      Delete: 'Delete',
      Home: 'Home',
      End: 'End',
      PageUp: 'PageUp',
      PageDown: 'PageDown',
      ArrowUp: 'Up',
      ArrowDown: 'Down',
      ArrowLeft: 'Left',
      ArrowRight: 'Right',
      MouseBack: 'Mouse side button (back)',
      MouseForward: 'Mouse side button (forward)',
    },
  },
  notices: {
    configRecovered: 'The config file was corrupted and has been restored from the last good copy or defaults.',
  },
  mini: {
    placeholder: 'Voice input mini window (placeholder)',
  },
  float: {
    placeholder: 'Recording float ball (placeholder)',
  },
}
