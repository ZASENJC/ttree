# TTREE for Windows

Windows 10/11 x64 原生 Win32 界面，支持托盘、快捷键、截图 OCR、AI 翻译和多轮对话。

## 便携版

将整个 TTREE 文件夹解压到固定目录，双击 `TTREE.exe` 即可打开窗口；已经运行时会唤起已有窗口。使用 `Ctrl+Shift+Space` 呼出或收起，`Ctrl+Shift+S` 截图翻译。也可从托盘菜单打开。开机自启使用 `--hidden` 参数，静默驻留托盘。

开机自启记录当前可执行路径；移动目录后应重新设置。OCR 使用系统文字识别组件，缺少语言时在 Windows 中补装对应的 OCR 可选功能。本包未使用商业代码签名证书，可能触发 SmartScreen；请只从项目官方 Releases 下载并校验 SHA256SUMS。

配置位于 `%APPDATA%/com.samwstu.ttree/`；API Key 当前随 JSON 配置保存。模型和推理运行时首次使用时独立下载，不在安装包内。不要分享本机配置文件。

## 开发

安装 Rust stable MSVC 与 Visual Studio C++ Build Tools 后，在仓库根运行：

```powershell
cargo run -p ttree-windows -- --show
cargo test --workspace --locked
./scripts/windows/package.ps1
./scripts/windows/smoke.ps1 -Exe target/package/windows/TTREE/TTREE.exe -Output target/windows-smoke
```

产物位于 `dist/windows/`。Mac 上的交叉类型检查不能替代此平台的链接与运行验证。
