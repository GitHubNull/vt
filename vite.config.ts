/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { fileURLToPath, URL } from 'node:url'

// 多窗口多页构建（tech/03 3.1）：index=设置主窗口、mini=语音小窗、float=悬浮球
export default defineConfig({
  plugins: [vue()],
  // Tauri 约定（tech/02 2.5）：dev 服务器固定 1420 端口，被占用时直接失败而非换端口
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // tmp/ 存放构建期解压的 alsa 等源码软链（环境产物，已 gitignore），
      // 其中存在自引用软链导致 chokidar ELOOP 使 dev 服务器崩溃，整体排除。
      ignored: ['**/tmp/**'],
    },
  },
  build: {
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL('index.html', import.meta.url)),
        mini: fileURLToPath(new URL('mini.html', import.meta.url)),
        float: fileURLToPath(new URL('float.html', import.meta.url)),
      },
    },
  },
  test: {
    environment: 'node',
  },
})
