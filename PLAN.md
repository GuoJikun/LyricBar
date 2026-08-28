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
