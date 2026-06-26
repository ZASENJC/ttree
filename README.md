# TTREE

> 轻量 macOS AI 翻译器。快捷键秒级呼出，截图 OCR、划词翻译、AI 对话，多引擎流式输出。

基于 **Tauri 2 + Svelte 5 + Rust** 构建：窗口隐藏而非销毁，保证快捷键瞬时召回；macOS 端 OCR 直接调用系统 Vision 框架，划词捕获走原生 Accessibility，零重型依赖。

---

## ✨ 功能

- **快捷键呼出翻译** — 默认 `⌘⇧Space` 呼出/收起窗口，输入文本回车翻译，`Esc` 收起，失焦自动隐藏
- **截图 OCR 翻译** — `⌘⇧S` 框选屏幕区域，调用 macOS 原生 **Vision** 框架识别文字并自动翻译（中/英/日繁等多语言）
- **划词翻译 / 划词 AI** — 选中任意应用内的文本，一键翻译或发送给 AI（基于 macOS Accessibility，未授权时回退到临时复制并恢复剪贴板）
- **AI 对话模式** — 独立于翻译的多轮对话，支持自定义 system 提示词
- **三引擎**：
  - **谷歌翻译**（免费网页端接口，无需 key）
  - **必应翻译**（免费网页端接口，自动抓取一次性令牌）
  - **OpenAI 兼容接口**（流式逐字输出，可配置 base_url / api_key / 模型 / 翻译提示词）
- **双栏对照** — 网页译文与 AI 译文并列展示，可一键切换网页引擎、复制结果
- **固定窗口** — 置顶且失焦不隐藏，方便对照查阅
- **自定义快捷键** — 五个动作均可独立配置，支持「呼出」与「划词」共用同一组合键（有选中文本时执行划词）
- **系统托盘 + 开机自启**（可选）
- 毛玻璃磨砂界面，明暗主题自适应

## ⌨️ 快捷键

| 动作 | 默认 | 说明 |
|------|------|------|
| 呼出 / 收起翻译窗口 | `⌘⇧Space` | 显示时自动复位到翻译页 |
| 截图 OCR 翻译 | `⌘⇧S` | 框选区域 → Vision 识别 → 自动翻译 |
| 呼出 AI 对话 | —（可在设置配置） | 独立对话模式 |
| 划词翻译 | —（可在设置配置） | 捕获选中文本直接翻译 |
| 划词 AI 对话 | —（可在设置配置） | 捕获选中文本发送给 AI |

所有快捷键均可在设置中自定义，留空即禁用；必须包含 `Cmd/Ctrl/Option` 中的至少一个修饰键。

## 📋 系统权限

首次使用相关功能时，macOS 会弹出授权请求，请在 **系统设置 → 隐私与安全性** 中开启：

| 权限 | 用途 |
|------|------|
| **屏幕录制** | 截图 OCR |
| **辅助功能（Accessibility）** | 划词翻译 / 划词 AI（捕获选中文本）|

> 开发模式下授权对象可能是终端或 dev 二进制。

## 🚀 开发

**前置**：Node.js 18+、Rust（stable）、Xcode Command Line Tools。

```bash
npm install          # 安装前端依赖
npm run tauri dev    # 启动开发模式
```

> Tauri 主窗口配置为 `visible: false`，开发模式下启动可能看不到窗口——用默认快捷键 `⌘⇧Space` 呼出。

仅启动前端（无后端）：

```bash
npm run dev
```

## 📦 构建

```bash
npm run tauri build    # 产出 .app 与 .dmg
```

产物位于 `src-tauri/target/release/bundle/`（含 `bundle/macos/TTREE.app` 与 `bundle/dmg/*.dmg`）。

## 🏗️ 架构

```
src-tauri/src/            Rust 后端
├── lib.rs                插件注册、毛玻璃、失焦隐藏、setup
├── commands.rs           暴露给前端的 Tauri 命令
├── config.rs             配置读写（tauri-plugin-store）
├── shortcut.rs           全局快捷键注册与分发（含冲突校验）
├── window.rs             窗口显示/隐藏/居中/聚焦/固定
├── tray.rs               系统托盘菜单
├── screenshot.rs         screencapture -i 区域截图
├── ocr/vision.rs         Apple Vision OCR（VNRecognizeTextRequest）
├── selection/macos.rs    划词捕获（Accessibility + 剪贴板回退）
└── translate/
    ├── mod.rs            引擎分发 + 共享 HTTP 客户端
    ├── google.rs         谷歌免费端接口
    ├── bing.rs           必应免费端接口（令牌抓取）
    └── openai.rs         OpenAI 兼容 SSE 流式（翻译 + 对话）

src/                      Svelte 5 前端（runes）
├── App.svelte            翻译 / 对话 / 设置 路由
├── lib/components/       TranslatePanel / EngineTabs / LangSelector / ResultView
├── lib/views/            ChatPanel / Settings
├── lib/stores/           translation / chat（.svelte.ts runes）/ languages
├── lib/shortcuts/        快捷键录制工具（shortcutRecorder）
└── lib/api/tauri.ts      invoke / event 封装
```

## ✅ 测试与检查

```bash
# Rust
cd src-tauri && cargo test          # 翻译解析 / SSE 解析 / 快捷键 / 划词归一化单测
cd src-tauri && cargo clippy -- -D warnings

# 前端
npm test          # Vitest（快捷键录制器）
npm run check     # svelte-check 类型检查
```

## ⚠️ 说明与限制

- 谷歌 / 必应走的是**非官方免费端点**，可能随时被限流或失效；OpenAI 兼容接口需自备 API Key。
- API Key 当前以明文存于本地 Tauri store（用户目录），后续可接入 macOS Keychain。
- 仅支持 **macOS 11.0+**（依赖 Vision、Accessibility、screencapture 等系统框架）。

## 📄 许可证

MIT
