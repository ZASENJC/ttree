<p align="center"><img src="assets/icons/128x128.png" width="80" alt="TTREE 图标"></p>
<h1 align="center">TTREE</h1>
<p align="center"><strong>让语言退后。让思考向前。</strong></p>
<p align="center">划词。截屏。对照译文。顺着上下文继续追问。<br>一个安静驻留在桌面的原生翻译与 AI 助手，用完即隐。</p>
<p align="center"><a href="https://github.com/ZASENJC/ttree/releases/latest">下载正式版</a> · <a href="https://github.com/ZASENJC/ttree/releases">历史版本</a> · <a href="docs/development.md">参与开发</a></p>

<p align="center"><img src="docs/images/product.png" width="1200" alt="TTREE Mac 与 Windows 界面渲染示意：原文、免费译文和 AI 译文在同一窗口呈现"></p>
<p align="center"><sub>macOS 与 Windows · 界面渲染示意</sub></p>

## 随叫随到。读完即隐。

好的工具，不打断正在发生的事。

阅读论文、浏览文档或排查代码时遇到陌生语言，按下快捷键，TTREE 即刻浮现于当前屏幕；读完按下 `Esc`，窗口随即收起，焦点自动回到你刚才停留的地方。需要长文比对时，点按窗口右上角的置顶按钮，它便安静地留在手边。

平时，它只是一枚安静停留在 Mac 菜单栏或 Windows 托盘里的符号，不占任务栏，也不制造干扰。

## 三种入口。把文字接进来。

无论文字在网页、终端，还是在无法复制的图片与视频里，都可以用最顺手的方式交给 TTREE：

- **输入或粘贴**：按下 **⌘ ⇧ Space**（Mac）或 **Ctrl ⇧ Space**（Windows）唤起面板，直接输入文本，按 `Enter` 即刻翻译。
- **划词即译**：在任意应用中选中文本并按下划词快捷键，无需手动复制粘贴，文字直接进入翻译或 AI 对话。
- **框选截屏识字**：按下 **⌘ ⇧ S**（Mac）或 **Ctrl ⇧ S**（Windows），框选屏幕任意区域。Mac 调用 Apple Vision，Windows 调用系统原生 OCR，识别后自动衔接翻译。

## 两份译文。一眼印证。

单一译文难免偏颇，来回切换窗口又太繁琐。TTREE 将两种翻译能力放在同一视野：

- **免费双路快译**：内置 Google 与 Bing 免费翻译，根据响应延迟与健康状态自动分流；当一路网络受阻或遇到区域重定向时，无缝回退至另一路，第一时间给出参考译文。
- **AI 深度精译**：开启 AI 翻译后，同一窗口下方同步流式呈现大模型译文。支持调节精炼或自然风格，让快速直译与语境润色一目了然。

## 顺着语境。继续深聊。

理解往往不止于字面翻译。切换到 **AI 对话**，你可以围绕刚才的段落继续提问、要求解释术语或提炼摘要。

- **独立配置，互不干扰**：AI 翻译与 AI 对话各自拥有独立的模型、接口与系统提示词设置。你可以用轻量模型做极速翻译，用推理模型做深度问答。
- **云端与本地皆可接入**：原生兼容 OpenAI 接口规范、Ollama 及各类本地模型服务。
- **内置离线翻译模型**：支持在「设置 → 模型」中按需下载内置 **HY-MT** 离线翻译模型（提供 FP16 与 Q8_0 / Q4_K_M 量化规格）。模型文件独立存放在本机目录，不增加应用安装包体积。

## 原生打造。一处核心，两套体验。

没有内嵌浏览器，也没有厚重的跨端渲染层。TTREE 使用纯 Rust 编写，将翻译路由、流式 AI、配置与历史存储沉淀为共享核心，再分别为两个桌面系统打造真正的原生界面：

- **在 Mac 上**：基于 AppKit 与系统毛玻璃材质构建。面板不仅跟随系统深浅外观，还会实时感知窗口下方的背景明暗，在深色与浅色底面之间平滑过渡文字对比度，始终保持清晰易读。
- **在 Windows 上**：基于 Win32 原生绘制，完整适配 Per-Monitor V2 高分屏缩放与任务栏明暗主题，提供无边框悬浮面板与标准窗口两种形态。

| 特性 | macOS | Windows |
| --- | --- | --- |
| 原生界面 | AppKit · 毛玻璃与背景明暗自适应文字 | Win32 · 高分屏缩放与系统明暗主题 |
| 截图识字 | 系统原生框选 + Apple Vision | 原生屏幕框选 + Windows OCR |
| 划词方式 | Accessibility 接口，必要时自动回退复制 | 临时复制并自动还原剪贴板 |
| 发行格式 | Universal DMG（Apple 芯片与 Intel） | x64 便携版 ZIP（解压即用） |

![共享能力与双平台目录关系](docs/images/workspace.svg)

## 顺手的操作。可自定义的节奏。

常用动作全部浓缩在键盘指尖。所有全局热键均可在「设置 → 快捷键」中直接点按录入，按下 `Backspace` / `Delete` 即可清空停用。

| 动作 | macOS | Windows |
| --- | --- | --- |
| 呼出 / 收起翻译 | ⌘ ⇧ Space | Ctrl ⇧ Space |
| 截图 OCR 翻译 | ⌘ ⇧ S | Ctrl ⇧ S |
| 划词翻译 / 划词 AI | 设置中按需录制 | 设置中按需录制 |
| 发送 / 翻译 | Enter | Enter |
| 输入换行 | Shift + Enter | Shift + Enter |
| 收起窗口 / 取消录入 | Esc | Esc |

## 开始使用。

### macOS

1. 前往 [Releases](https://github.com/ZASENJC/ttree/releases/latest) 下载 `TTREE_*_universal.dmg`，将 **TTREE** 拖入「应用程序」。
2. 首次打开若提示无法验证开发者，请前往 **系统设置 → 隐私与安全性**，点击 **仍要打开**。
3. 首次使用截图翻译时，请按系统提示授予 **屏幕与系统音频录制** 权限，随后完全退出并重启 TTREE。如需更顺滑的划词体验，可按需授予 **辅助功能** 权限。

> Mac 发行包使用固定社区代码签名证书签名，尚未经过 Apple 公证；固定证书用于在后续版本更新时维持系统的录屏授权记录，避免每次升级重复授权。

### Windows

1. 前往 [Releases](https://github.com/ZASENJC/ttree/releases/latest) 下载 `TTREE_*_windows_x64.zip`，解压至固定目录。
2. 双击运行 `TTREE.exe` 即可打开窗口；按 **Ctrl ⇧ Space** 随时呼出或隐藏。开启「开机自启」后，应用将以静默方式驻留系统托盘。
3. 截图 OCR 依赖 Windows 系统的对应语言识别包；若系统缺失目标语言组件，可按界面提示补充安装。

> Windows 发行包为无商业代码签名的便携版本，首次启动若触发 SmartScreen 提示，请核对官方发布页与附件中的 `SHA256SUMS` 校验值后选择继续运行。启用开机自启后请勿移动解压目录。

## 为创作者留出清晰结构。

一个 Cargo Workspace，清晰的职责边界。业务逻辑在共享库中一次验证，原生交互在各自平台目录中独立打磨。

```text
apps/macos/       macOS AppKit 窗口、菜单与原生控件
apps/windows/     Windows Win32 窗口、绘制与交互布局
crates/           翻译路由、AI 流式通信、本地模型、OCR、热键与本机存储
scripts/macos/    macOS 双架构构建、固定证书签名与验证
scripts/windows/  Windows 便携包构建与启动冒烟验证
scripts/release/  跨平台发布、校验和签名与资产核验
docs/             开发指南、发布规范与架构图示
```

本地调试可直接运行 `cargo run -p ttree-macos -- --show`（macOS）或 `cargo run -p ttree-windows -- --show`（Windows）。构建、测试与发布流程详见 [开发指南](docs/development.md) 与 [发布指南](docs/releasing.md) · [查看构建状态](https://github.com/ZASENJC/ttree/actions/workflows/build.yml)。

<sub>免费翻译依赖公开非官方服务端点，实际可用性受网络与服务状态影响。内置离线模型需在应用内独立下载，并遵循对应模型许可协议。对话历史与个人设置保存在本机；当前 API Key 保存在本机 JSON 配置文件中，请勿对外分享配置文件。macOS 支持 11.0 及以上系统（Universal Binary）；Windows 提供 64 位（x64）构建。本项目基于 MIT 协议开源，第三方图标许可详见 assets/licenses。</sub>
