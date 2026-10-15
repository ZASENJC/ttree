# Windows

继承根 AGENTS.md。这里只放 Win32 UI、DWM、对话框及 Windows 交互测试。

- `app.rs` 是控制器与消息循环；`app/ui.rs` 负责绘制/控件；`app/dialogs.rs` 负责系统对话框；`design.rs` 保存可独立测试的布局规则。
- 业务逻辑复用共享 crates。系统划词放 `crates/ttree-platform/src/windows/`，截图/OCR 放 `crates/ttree-ocr/src/windows/`。
- 保持 DPI 缩放、主题、滚动、hover 重绘和启动隐藏行为；不要用强制重绘掩盖残影。
- Mac 交叉 `cargo check` 不等于 Windows 原生验证。发布必须在 Windows MSVC runner 上编译、测试、启动便携包。
- 发行时 ZIP 内为 `TTREE/TTREE.exe`。不把用户配置、下载模型或托管运行时放进 ZIP。
- `tests/repaint-regression.py` 用于锁定具体重绘回归，需交互桌面、pywinauto 与 Pillow，按需运行。
