# LyricBar 实现计划

纯 Rust 的 Windows 任务栏桌面歌词。通过 SMTC 统一读取播放器元数据，独立解析歌词并做时间轴同步，用 sysmon 的 `SetParent(Shell_TrayWnd)` 方案把透明窗口嵌入任务栏。

## 技术栈（已确认）

- **语言**：Rust **edition 2024**（stable 1.97+）
- **Windows 交互**：`windows` crate **0.62**（WinRT SMTC + Win32 窗口/任务栏 API）
- **异步/HTTP**：`tokio` + `reqwest`（歌词 API）
- **解析**：`serde` / `serde_json`（API 响应）；LRC 自写解析器
- **错误**：`anyhow` + `thiserror`
- **UI 渲染**：纯 `windows-rs` 透明分层窗口（GDI `DrawText` 文本，后续可升级 DirectWrite），**不使用 Windows Reactor**（理由见下）

## 关于 UI 框架的决策

原计划探讨过 Windows Reactor（windows-rs 内的 React 风格 WinUI 3 框架）。MVP **不采用**，原因：

1. Reactor 基于 WinUI 3，需 Windows App SDK 2.0.1+ 运行时，且当前为 git-only 依赖，本环境构建风险高。
2. 将 WinUI 3 窗口 `SetParent` 进 `Shell_TrayWnd` 未经验证，DWM 合成行为不确定。
3. 任务栏歌词本质是"一行带样式的文字"，原生分层窗口足够且最可靠、全开源。

渲染模块（`ui/overlay.rs`）保持隔离，后续若验证可行可替换为 Reactor 设置窗口。

## 架构

```md
SMTC 层        → 元数据(标题/歌手/专辑/状态/进度) + 源 App 标识
   ↓
Song Resolver  →  (歌名,歌手,源App) → 选歌词 provider → LRC 文本
   ↓
Lyrics Engine  →  LRC 解析 + 时间轴同步 + 偏移 + 双语
   ↓
UI 层          →  SetParent(Shell_TrayWnd) 嵌入 + 分层窗口渲染 + 托盘菜单
```

## 关键技术方案

### SMTC（windows-rs 0.62）

- `GlobalSystemMediaTransportControlsSessionManager::RequestAsync()`
- 订阅 `MediaPropertiesChanged` / `TimelinePropertiesChanged` / `PlaybackInfoChanged`
- `TryGetMediaPropertiesAsync()` → `Title` / `Artist` / `AlbumTitle` / `Thumbnail`
- `GetTimelineProperties()` → `Position` / `Duration`
- `GetPlaybackInfo()` → `PlaybackStatus`
- 播放器识别：`GetSourceAppInfo()` / `SourceAppUserModelId` 判断是否为网易云（含 `NetEaseCloudMusic`）

### 歌词解析

- **LRCLib**（通用 fallback）：`GET https://lrclib.net/api/search?artist=&track=` → `syncedLyrics`（带时间戳 LRC），免费无鉴权
- **网易云**（网易云会话优先）：搜 song id 再取歌词（含 `tlyric` 翻译 / `romalrc` 音译）；MVP 先用老 `/api/` 端点做 best-effort，weapi 加密作为后续增强
- provider 链可扩展 QQ/Spotify 等

### 任务栏嵌入（sysmon 方案）

- 创建无装饰、`WS_EX_LAYERED` 透明、`skip_taskbar` 窗口
- `SetParent(hwnd, Shell_TrayWnd)` 将窗口变为任务栏子窗口
- 每 ~500ms 用 `SHAppBarMessage(ABM_GETTASKBARPOS)` 判断方向，`FindWindowExW(TrayNotifyWnd)` 定位，紧贴其左侧（横向）或上方（纵向），DPI 自适应
- `WS_EX_TRANSPARENT` 实现鼠标穿透

### 同步引擎

- LRC → `Vec<(Duration, String)>`，支持 `[offset:]` 与一行多时间戳
- ~100ms 定时器读 `Position` 定位当前行；暂停则冻结
- 偏移校准：全局 + 每首歌偏移（后续托盘菜单调整）

## 目录结构

```md
LyricBar/
├── Cargo.toml
├── PLAN.md
├── src/
│   ├── main.rs
│   ├── smtc/{mod.rs, session.rs, metadata.rs}
│   ├── lyrics/{mod.rs, provider.rs, lrclib.rs, netease.rs, lrc.rs, engine.rs}
│   ├── ui/{mod.rs, taskbar.rs, overlay.rs, tray.rs}
│   ├── config.rs
│   └── error.rs
└── assets/  (icon, 后续)
```

## 分阶段

- **阶段 0 脚手架**：Cargo.toml / 目录 / 错误类型
- **阶段 1 SMTC MVP**：订阅 SMTC，控制台打印元数据+进度+源App（验证可行性）
- **阶段 2 歌词引擎**：LRCLib + LRC 解析 + 同步（进度由 SMTC 喂入）
- **阶段 3 网易云源**：接入网易云歌词（含翻译/音译）
- **阶段 4 任务栏窗口**：SetParent 嵌入 + 分层窗口渲染当前歌词行 + 自适应
- **阶段 5 体验**：托盘菜单、偏移校准、双语切换、多播放器、配置持久化、自启

## 验收

- `cargo check` / `cargo build` 通过（编译验证）
- 运行期需在真实 Windows 桌面 + 网易云播放下验证：任务栏出现跟随播放的歌词
- 阶段 1 可先以控制台输出验证 SMTC 可取数据

## 风险

- windows-rs 0.62 的 SMTC feature 名称需以编译器反馈为准（`Media_Control` 等）
- 网易云老 `/api/` 端点可能失效，需回退 LRCLib 或补 weapi
- 任务栏嵌入在 explorer 重启/任务栏移动时需重定位（已有 500ms 定时器兜底）

---

# Tauri 迁移计划

将 LyricBar 从纯 WebView2 实现迁移到 Tauri 框架，采用 Vue.js 作为前端技术栈。

## 迁移收益

| 方面 | 当前实现 | Tauri 迁移后 |
|------|----------|--------------|
| WebView2 管理 | 手动创建 `webview2-com` | Tauri 自动管理 |
| 系统托盘 | 手写 Win32 API（212行） | Tauri 原生 `tray-icon` feature |
| 日志系统 | 手写 fern + RotatingWriter（150行） | Tauri `tauri-plugin-log`（15行配置） |
| 窗口管理 | 手动创建窗口类+消息循环 | Tauri `WebviewWindowBuilder` |
| 前端渲染 | Rust 生成 HTML 字符串 | Vue.js 独立前端项目 |
| 构建/打包 | 无 | Tauri CLI 提供打包支持 |

## 模块迁移策略

### ✅ 完全保留（Rust 后端）
- `src/smtc/` — SMTC 会话管理（无需改动）
- `src/lyrics/` — 歌词获取/解析/同步引擎（无需改动）
- `src/crash.rs` — 崩溃捕获（无需改动）

### 🔄 适配改造
- `src/main.rs` → `src-tauri/src/lib.rs` — 改为 Tauri 应用入口
- `src/ui/overlay.rs` → 保留任务栏嵌入逻辑，窗口创建改用 Tauri

### ❌ 删除重写
- `src/ui/tray.rs` — 使用 Tauri 托盘 API
- `src/ui/taskbar.rs` — 简化为辅助函数
- `src/log_writer.rs` — 使用 `tauri-plugin-log`
- WebView2 直接渲染 → 改为 Vue.js 前端

## 新项目结构

```md
LyricBar/
├── Cargo.toml                    # 根 workspace
├── PLAN.md
├── package.json                  # 前端依赖
├── index.html                    # 入口 HTML
├── vite.config.js                # Vite 配置
├── src/                          # Vue.js 前端
│   ├── main.js
│   ├── App.vue
│   ├── components/
│   │   └── LyricDisplay.vue
│   └── assets/
│       └── styles.css
├── src-tauri/
│   ├── Cargo.toml                # Rust 后端依赖
│   ├── tauri.conf.json           # Tauri 配置
│   ├── build.rs
│   ├── capabilities/
│   │   └── default.json          # 权限配置
│   └── src/
│       ├── lib.rs                # Tauri 应用入口
│       ├── main.rs               # 程序入口
│       ├── smtc/                 # 保留
│       │   ├── mod.rs
│       │   ├── session.rs
│       │   └── metadata.rs
│       ├── lyrics/               # 保留
│       │   ├── mod.rs
│       │   ├── provider.rs
│       │   ├── lrclib.rs
│       │   ├── netease.rs
│       │   ├── lrc.rs
│       │   └── engine.rs
│       ├── overlay.rs            # 任务栏嵌入逻辑
│       └── crash.rs              # 保留
└── assets/                       # 图标等资源
```

## 分阶段实施

### 阶段 1：脚手架搭建
1. 初始化 Tauri 项目结构
2. 配置 `tauri.conf.json`（透明窗口、无边框）
3. 配置 Vue.js + Vite 前端
4. 验证 `cargo tauri dev` 能运行

### 阶段 2：后端迁移
1. 将 `smtc/`、`lyrics/` 模块移入 `src-tauri/src/`
2. 改造 `main.rs` → `lib.rs`，使用 Tauri 生命周期
3. 保留 `overlay.rs` 任务栏嵌入逻辑，通过 `window.hwnd()` 获取句柄
4. 集成 `tauri-plugin-log` 替换手写日志系统
5. 使用 Tauri 托盘 API 替换手写托盘代码

### 阶段 3：前端开发
1. 创建 Vue.js 歌词显示组件
2. 实现歌词样式（白色粗体、阴影、双语显示）
3. 通过 Tauri 事件系统接收后端歌词数据

### 阶段 4：联调测试
1. SMTC → 歌词获取 → 前端渲染完整链路
2. 任务栏嵌入定位测试
3. 播放/暂停/切歌状态同步

## 关键配置

### Tauri 配置 (`src-tauri/tauri.conf.json`)

```json
{
  "$schema": "https://raw.githubusercontent.com/tauri-apps/tauri/dev/crates/tauri-config-schema/schema.json",
  "productName": "LyricBar",
  "version": "0.1.0",
  "identifier": "com.lyricbar.app",
  "app": {
    "windows": [
      {
        "label": "lyric",
        "title": "LyricBar",
        "transparent": true,
        "decorations": false,
        "skipTaskbar": true,
        "width": 240,
        "height": 28,
        "resizable": false,
        "alwaysOnTop": true,
        "visible": true
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ]
  }
}
```

### 权限配置 (`src-tauri/capabilities/default.json`)

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the main window",
  "windows": ["lyric"],
  "permissions": [
    "core:default",
    "log:default"
  ]
}
```

### Cargo.toml (`src-tauri/Cargo.toml`)

```toml
[package]
name = "lyricbar"
version = "0.1.0"
edition = "2024"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-log = "2"
windows = { version = "0.62", features = [
    "Foundation",
    "Foundation_Collections",
    "Media_Control",
    "Win32_Foundation",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Graphics_Gdi",
    "Win32_Graphics_Dwm",
    "Win32_UI_Shell",
    "Win32_UI_HiDpi",
    "Win32_UI_Controls",
    "Win32_System_LibraryLoader",
    "Win32_System_Diagnostics_Debug",
    "Win32_System_Kernel",
    "Win32_System_Com",
    "Win32_System_Ole",
] }
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "2"
log = "0.4"

[lib]
name = "lyricbar_lib"
crate-type = ["lib", "cdylib", "staticlib"]
```

### package.json (根目录)

```json
{
  "name": "lyricbar",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "tauri": "tauri"
  },
  "dependencies": {
    "vue": "^3.4",
    "@tauri-apps/api": "^2.0",
    "@tauri-apps/plugin-log": "^2.0"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0",
    "vite": "^5.0",
    "@vitejs/plugin-vue": "^5.0"
  }
}
```

## 日志系统迁移

### 当前实现 vs Tauri 日志插件

| 方面 | 当前（fern + 自定义 RotatingWriter） | Tauri 插件 |
|------|--------------------------------------|------------|
| 代码量 | ~150 行（`log_writer.rs` + `setup_log()`） | ~15 行配置 |
| 日志轮转 | 手动实现 | 内置 `rotation_strategy` |
| 输出目标 | 手动配置 stdout + 文件 | `TargetKind::Stdout` / `LogDir` / `Webview` |
| 时间格式 | 手动用 `chrono` | 内置 `timezone_strategy` |
| 前端日志 | 不支持 | 支持 `Webview` 目标 |

### 删除文件

- `src/log_writer.rs`（~101 行）

### 日志配置代码

```rust
// src-tauri/src/lib.rs
use tauri_plugin_log::{Target, TargetKind};

pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                    Target::new(TargetKind::Webview),
                ])
                .max_file_size(1_000_000)  // 1MB
                .rotation_strategy(tauri_plugin_log::RotationStrategy::Keep(5))
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .level(log::LevelFilter::Debug)
                .build(),
        )
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### 日志文件位置

| 平台 | 路径 |
|------|------|
| Windows | `%LOCALAPPDATA%/{bundleId}/logs` |
| macOS | `~/Library/Logs/{bundleId}` |
| Linux | `~/.local/share/{bundleId}/logs` |

### 前端日志查看

```javascript
// src/main.js
import { attachConsole } from '@tauri-apps/plugin-log';

// 将 Rust 日志转发到前端 console
const detach = await attachConsole();
```

## 验收标准

- `cargo tauri dev` 能正常启动
- 任务栏出现歌词悬浮窗
- SMTC 播放时歌词实时更新
- 系统托盘图标和菜单正常
- 日志文件正确生成在指定目录
- `cargo tauri build` 能生成安装包

## 风险

- windows-rs 0.62 的 SMTC feature 名称需以编译器反馈为准
- 任务栏嵌入需要通过 `window.hwnd()` 获取句柄，需验证 Tauri 窗口兼容性
- 网易云老 `/api/` 端点可能失效，需回退 LRCLib
