# 开发指南

## 一次修改，两个平台

`apps/macos` 和 `apps/windows` 各自拥有独立 Cargo package，只维护原生界面与交互。共享能力统一位于 `crates`，使用一个根 Cargo.lock 和工作区版本号。

| 需要修改的内容 | 位置 | 联动范围 |
| --- | --- | --- |
| 语言、提示词输入、布局数据、消息校验 | `crates/ttree-core` | 两端 |
| 免费翻译、分流、回退 | `crates/ttree-translate` | 两端 |
| 流式 AI、HY-MT、本地服务 | `crates/ttree-ai` | 两端；Candle/Metal 实现限 Mac |
| 版本检查、更新下载、校验与安装 | `crates/ttree-updater` | 两端；安装器按平台隔离 |
| 配置和历史 | `crates/ttree-storage` | 两端；保持旧格式 |
| 快捷键、自启和划词入口 | `crates/ttree-platform` | 两端，系统划词实现分目录 |
| OCR 公共处理/结果类型 | `crates/ttree-ocr/src` | 两端 |
| AppKit / Win32 控件 | `apps/macos` / `apps/windows` | 对应平台 |
| 平台截图/OCR | `crates/ttree-ocr/src/macos` / `windows` | 对应平台 |

![目录与依赖示意图](images/workspace.svg)

不要复制两份配置模型或翻译策略。改变公共接口后同步更新两个调用端，保持平台专用依赖受 `cfg(target_os)` 保护。

## 环境与运行

Mac 需要 Rust stable、Xcode Command Line Tools，以及 Metal 编译工具。打 Universal 包还需 `rustup target add aarch64-apple-darwin x86_64-apple-darwin`。如果 `xcrun -f metal` 失败，在完整 Xcode 环境安装 Metal Toolchain。

Windows 需要 Rust stable MSVC、Visual Studio C++ Build Tools 和 Windows SDK。平台包使用 `x86_64-pc-windows-msvc`。CI 的 Windows 2022 runner 已具备这些条件。

```sh
# 在对应系统任选一个入口；--show 显式显示默认隐藏的窗口
cargo run -p ttree-macos -- --show
cargo run -p ttree-windows -- --show

# Mac 签名 App 的原生截图 / Vision 验证（应用未运行时）
open -n /Applications/TTREE.app --args --ocr

# 编译和单元测试（workspace 成员在非对应平台使用空入口）
cargo check --workspace --locked
cargo test --workspace --locked

# 发布脚本、签名失败场景和发布策略合约
bash scripts/tests/run-release-tests.sh

# Mac 上只做 Windows 类型检查
cargo check -p ttree-windows --target x86_64-pc-windows-msvc --locked
```

不需要 Node 前端工具链。发布阶段使用固定版本的 Tauri 独立 signer，用于兼容旧客户端的更新附件签名及双平台更新清单签名。

## 验证边界

共享代码修改需检查两端。Mac 本地测试与 Windows 交叉检查通过后，仍需 Windows runner 的原生构建、单元测试、便携 ZIP 校验与启动验证。CI 保存 Windows 启动结果和截图作为 artifact；这只是启动检查，不能代替完整桌面交互回归。

Windows 重绘问题可按需运行 `apps/windows/tests/repaint-regression.py`（需 unlocked desktop、Pillow、pywinauto）；输出放到 `target/repaint-regression`。Mac 签名和录屏权限的跨升级实测是不同证据，见[发布指南](releasing.md)。

## 工作区整洁

源码在 `apps/` 与 `crates/`；文档在 `docs/`；脚本按 `macos`、`windows`、`release` 分类。日志、截图测试证据和中间构建放 `target/`，对外附件放 `dist/`，均不提交。

README 图片放 `docs/images/`，只能提交确认可公开的界面。API Key 当前保存在用户配置目录 JSON 内，Mac 文件权限设为 0600；不要把它描述成 Keychain 存储，也不要将配置或聊天历史用于测试样本。

## 品牌与渲染图

单色 Logo 源文件为 `assets/icons/icon.svg`。Mac 安装 librsvg 后运行 `python3 scripts/branding/generate-icons.py`，统一生成 PNG、ICO、ICNS。Windows 可生成 PNG/ICO；发布前仍须在 Mac 更新 ICNS。Windows build.rs 将图标及版本编译进 EXE，窗口读取带底板的应用图标；托盘读取无底板的浅色/深色字形，并跟随系统任务栏主题。Mac 菜单栏使用透明字形与 NSImage template 自动适配明暗。

README 产品图源文件为 `docs/design/product.html`，按 1600 × 1080 CSS 像素渲染，导出 `docs/images/product.png`（当前为 2x）。这是基于现有布局的界面示意，不是实机截图；不要用它作为交互验收证据。

应用内安装原生冒烟由 `updater-smoke` opt-in 验证构建执行：`--updater-smoke-install SIGNED_RELEASE_DIR vX.Y.Z`。入口仅在验证构建中存在，使用正式公钥和签名清单，安装器与发行版相同。Windows CI 验证实际 EXE 替换、新 GUI 进程、便携目录文件保留及 helper 清理；不以假程序启动代替。
