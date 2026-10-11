# 第 9 章 跨平台与构建打包

> 本章描述 Windows / macOS / Linux 三平台的差异处理、构建打包与分发。
> 需求依据：PRD 第 5 章兼容性（NFR 兼容性一节）、NFR-P-09（安装包 ≤120 MB）、NFR-S-06（更新签名）。

## 9.1 平台支持矩阵

| 平台 | 版本 | 架构 | 支持级别 |
| --- | --- | --- | --- |
| Windows | 10 22H2+ / 11 | x64、ARM64 | 完整支持 |
| macOS | 13+ | Apple Silicon、Intel | 完整支持 |
| Linux | Ubuntu 22.04+、Debian 12+、Fedora 39+ | x64 | X11 完整支持；Wayland 提供替代方案（v1.0） |

## 9.2 平台差异处理

所有差异收敛在 `platform/` 适配层（见 [第 4 章](./04-backend.md) 4.5），本节列出各能力的平台实现要点。

### 9.2.1 全局快捷键

| 平台 | 实现 | 注意事项 |
| --- | --- | --- |
| Windows | `global-shortcut`（RegisterHotKey）+ 自研低级键盘钩子（WH_KEYBOARD_LL，`platform/hotkey/windows_llhook.rs`） | 单个左右修饰键作触发键时 global-hotkey 的 `key_to_vk` 不映射修饰键 Code 且裸修饰键不可靠（右 Alt 兼 AltGr），改由低级钩子覆盖；钩子路径实现 PRD 3.1.1 规则 3（组合键抑制）与规则 4（0.3 s 误触门）；组合键 / 功能键仍走 RegisterHotKey |
| macOS | `global-shortcut`（Carbon / NSEvent 监听） | 需要辅助功能权限；首次注册时引导授权 |
| Linux X11 | `global-shortcut`（XRecord / XGrabKey） | 与窗口管理器快捷键冲突检测（F-SYS-03） |
| Linux Wayland | 协议限制无法全局监听 | v1.0 提供替代方案：桌面门户（GlobalShortcuts portal）或提示用户改用小窗触发 |

### 9.2.2 文本上屏（InjectService）

| 平台 | 模拟键入 | 剪贴板粘贴回退 |
| --- | --- | --- |
| Windows | SendInput（enigo 封装） | 支持 |
| macOS | CGEvent（需辅助功能权限） | 支持 |
| Linux X11 | XTest / XSendEvent | 支持 |
| Linux Wayland | 受 compositor 限制 | 剪贴板路径为主；ydotool / portal 作为可选后端 |

权限缺失时的降级链：模拟键入 → 剪贴板粘贴 → 无焦点兜底（F-INPUT-09），每级降级都提示原因与授权引导。

### 9.2.3 场景识别（F-CODE-01，v0.5）

前台窗口与应用识别：Windows（GetForegroundWindow + 进程名）、macOS（NSWorkspace / Accessibility API）、Linux X11（_NET_ACTIVE_WINDOW）；Wayland 下能力受限，提供手动场景切换兜底。

### 9.2.4 悬浮球不抢焦点

| 平台 | 行为 |
| --- | --- |
| Windows | `WS_EX_NOACTIVATE` 窗口样式，Tauri `focusable: false` 生效 |
| macOS | `NSWindowStyleMaskNonactivatingPanel`，生效 |
| Linux | 各 WM 行为不一致；X11 下用 `_NET_WM_STATE_ABOVE` + 不接受焦点 hint；异常时降级为"录音时显示、点击穿透" |

### 9.2.5 托盘

Tauri 内置 TrayIcon 三平台可用；Linux 需要 AppIndicator 支持（GNOME 需扩展，设置 FAQ 中说明，F-FAQ-01）。

### 9.2.6 音频采集

`cpal` 统一抽象；Linux 下 ALSA / PulseAudio / PipeWire 均经 cpal 后端；设备热插拔事件三平台行为不同，AudioService 统一做枚举刷新与错误恢复（NFR-R-03）。

## 9.3 构建与打包

### 本地构建

```bash
pnpm tauri build        # 当前平台安装包
```

### CI 矩阵（规划）

| 目标 | 产物 |
| --- | --- |
| windows-latest (x64 / ARM64) | `.msi` / `.exe`（NSIS） |
| macos-latest (Universal) | `.dmg` / `.app` |
| ubuntu-22.04 (x64) | `.deb` / `.AppImage` |

### 体积控制（NFR-P-09：≤120 MB 不含模型）

- ASR 模型**不打进安装包**，首启引导下载（F-SET-06 模型管理）；
- Rust 侧：`release` profile 开 LTO、`strip` 符号、按需裁剪 tokio features；
- 前端侧：Vite 分包、PrimeVue 按需引入、图标字体子集化；
- 发布前在 CI 记录产物体积，超标报警。

### 签名与更新（NFR-S-06）

- Tauri Updater：更新包以项目私钥签名，客户端内置公钥校验，失败不得安装；
- macOS：Apple 开发者签名与公证（如需）；Windows：代码签名证书（发布阶段落实，归属发布流程文档）；
- 卸载流程：提示是否删除本地数据（NFR-PR-08），由各平台安装器脚本实现。

## 9.4 首次运行与权限引导

- 首次使用引导（F-SYS-02，v0.5）覆盖各平台权限：麦克风、辅助功能（macOS）、自启动（可选）；
- 权限缺失时功能降级而非崩溃：无麦克风权限 → 录音按钮禁用 + 引导；无辅助功能权限 → 上屏走剪贴板路径。

## 9.5 平台相关测试要求

- 每个涉及平台能力的任务 SPEC 必须包含三平台手动验证清单；
- CI 跑三平台编译 + 单元测试；真实硬件兼容性（目标应用上屏测试矩阵，PRD 第 5 章）在版本发布前手动执行。
