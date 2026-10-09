import pluginVue from 'eslint-plugin-vue'

export default [
  {
    ignores: [
      'dist/**',
      'node_modules/**',
      'src-tauri/**',
      'tmp/**',
      // TypeScript 文件由 vue-tsc（typecheck）覆盖；tech/02 依赖清单未含 @typescript-eslint，
      // ESLint 仅负责 .vue SFC 与配置文件（tech/02 2.2：eslint + eslint-plugin-vue）。
      'src/**/*.ts',
    ],
  },
  ...pluginVue.configs['flat/essential'],
  {
    files: ['*.config.js', '*.js'],
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'module',
    },
  },
  {
    rules: {
      'vue/multi-word-component-names': 'off',
    },
  },
]
