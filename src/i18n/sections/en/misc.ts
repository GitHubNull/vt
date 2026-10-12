/**
 * 界面文案（en · misc 组）：由原 en.ts 按域拆分而来（行数门禁）。
 * 键面与组合文件互为一体：新增文案按域放入对应 section。
 */
export const misc = {
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
      versions: [
        {
          version: 'v0.1.10',
          date: '2026-10-11',
          items: ['Fixed the occasional unresponsive Right Alt trigger key on Windows.'],
        },
        {
          version: 'v0.1.9',
          date: '2026-10-10',
          items: [
            'Fixed model download progress stuck at 0% and stale fake progress after cancelling a download.',
            'Added a one-click clear button to the log keyword input.',
          ],
        },
        {
          version: 'v0.1.8',
          date: '2026-10-10',
          items: [
            'Fixed the log viewer list height adaptation and the level filter dropdown background.',
          ],
        },
      ],
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
  notices: {
    configRecovered:
      'The config file was corrupted and has been restored from the last good copy or defaults.',
  },
  mini: {
    status: {
      hold: 'Hold the mic or {key} to talk',
      toggle: 'Click the mic or press {key} to start',
      recording: 'Recording…',
      processing: 'Recognizing…',
      cancelled: 'Cancelled',
    },
    cancel: 'Cancel recording',
    lastResult: 'Last result',
    copy: 'Copy',
    copySuccess: 'Copied to clipboard',
    copyFailed: 'Copy failed. Please select the text and copy it manually.',
    openSettings: 'Open settings',
  },
  float: {
    modeHold: 'Hold',
    modeToggle: 'Toggle',
    processing: 'Recognizing',
    dragCancelHint: 'Release to cancel',
    feedback: {
      success: 'Inserted',
      degraded: 'Inserted raw result',
      failed: 'Recognition failed',
      cancelled: 'Cancelled',
    },
  },
}
