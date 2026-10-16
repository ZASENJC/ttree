# TTREE 工作规则

## 任务边界

在本轮授权内完成交付及必要验证；明确的只读或先审批要求是停止边界。已授权且范围未变的步骤不重复索取批准。发布、对外传讯及服务变更须有覆盖该动作的授权。

选择最窄的适用技能。一般分析直接交付结论与证据；需要持久交付物时再建立文件。不因通用模板自动执行全套测试、定时重跑或发布。

## 提交约定

- 每个会话做出代码、文档或配置更改后，必须在完成必要验证后创建 Git 提交，再向用户交付结果；除非用户明确要求暂不提交，不需再次询问是否提交。
- 只提交本会话任务产生的更改。共享工作区中其他会话或用户的改动保持原样；同一文件混有其他改动时，按修改块选择提交，禁止直接全量暂存。
- 提交信息应说明实际变更；交付时报告提交哈希和验证结果。提交不代表推送或发布，这些动作仍须有对应授权。

## 当前架构与目录

TTREE 是纯 Rust Cargo workspace，Mac 用 AppKit，Windows 用 Win32。入口分别为 `apps/macos`（包 `ttree-macos`，二进制 `ttree`）与 `apps/windows`（包/二进制 `ttree-windows`，发行时命名 `TTREE.exe`）。不要恢复已移除的 Web 前端或旧框架命令。

- `apps/macos/src/`：Mac 窗口、菜单、控制器、原生控件。遵守其局部 AGENTS.md。
- `apps/windows/src/`：Windows 窗口、绘制、对话框与设计布局。遵守其局部 AGENTS.md。
- `crates/ttree-core`：配置模型、语言、消息校验、取消与布局数据。
- `crates/ttree-storage`：本机 JSON 配置与 JSONL 历史。
- `crates/ttree-translate`：免费翻译引擎、健康分流和回退。
- `crates/ttree-ai`：HTTP / SSE、本地模型、HY-MT 下载与进程管理。
- `crates/ttree-ocr/src/{macos,windows}`：平台专用截图与 OCR；公共预处理和结果类型留在上层。
- `crates/ttree-platform/src/{macos,windows}`：平台专用划词；热键解析和自启入口共享。
- `scripts/{macos,windows,release}`：各平台打包与公共发布工具。
- `docs/`：开发与发布说明；图片只放 `docs/images/`。
- `target/`、`dist/`：忽略的编译/发行产物。禁止向根目录堆放截图、日志、压缩包、临时报告。

## 修改原则

1. 翻译、AI、配置、语言等业务语义优先改共享 crate，避免在两个 UI 里复制规则。
2. 原生控件与平台权限留在对应平台目录；共享 crate 的系统实现用 `cfg` 隔离。不得让 Windows 构建依赖 Apple 框架，反之亦然。
3. 共享行为发生变化时，检查两个调用端；若另一个平台尚未实现，明确说明，不宣称已验证。
4. 保留默认隐藏、快捷键唤起、Esc 收起、置顶时不因失焦隐藏等行为。不要重启无关应用、模型服务或清空用户配置。
5. API Key 当前保存在本机 JSON 配置，不能在文档中宣称已使用 Keychain。禁止把用户配置、历史、凭据或模型文件提交到仓库，不打印私钥/口令。
6. 不改变 `com.samwstu.ttree` 配置目录或历史格式来完成目录整理。版本唯一来源是根 `Cargo.toml` 的 `[workspace.package].version`；UI 使用 Cargo 版本常量。
7. 应用图标使用黑色底板；Mac 菜单栏与 Windows 托盘只显示透明背景字形，不带底板。Mac 使用模板图标，Windows 跟随系统任务栏主题选择黑/白字形。
8. README 使用克制、简洁、以体验为主的中文文风；插图标清实机截图与示意图，不用虚构界面冒充运行结果。

## 开发与必要验证

```sh
cargo check --workspace --locked
cargo test --workspace --locked
bash scripts/tests/run-release-tests.sh
# 在对应系统运行
cargo run -p ttree-macos -- --show
cargo run -p ttree-windows -- --show
```

修改 Windows 代码可在 Mac 上先 `cargo check -p ttree-windows --target x86_64-pc-windows-msvc --locked`；它仅验证类型，不等于原生链接、启动或交互验证。实际发布必须通过 Windows runner 的编译、测试、ZIP 验证和启动检查。

验证按影响范围选择；不为低影响文档修改强制运行完整套件。发布变更须跑发布合约测试。不要通过删除有效的门禁绕过失败。构建失败先报告最小真实错误。完整说明见 [开发指南](docs/development.md)。

## 自动发布约定

`.github/workflows/build.yml` 是唯一构建发布工作流。

- 分支推送、PR：双平台构建与测试；其他分支不发布。
- `main` 推送：双平台成功后发布唯一的 `vX.Y.Z-dev.RUN.ATTEMPT` 预发布版，不覆盖 Latest。
- 推送 `vX.Y.Z` 标签：必须匹配工作区版本，双平台成功后发布正式版，更新 Latest。
- `workflow_dispatch`：只验证、上传临时 artifact，不发布。
- 发布 job 必须依赖 metadata、macos、windows 三者成功。先创建草稿，上传附件，再下载验证签名与哈希，最后公开；不覆盖既有 Release。
- 附件：Universal DMG、Mac `.app.tar.gz`、`.sig`、`latest.json`、Windows x64 ZIP、`SHA256SUMS`。
- 保留旧客户端更新签名兼容性；不能把纯 Rust 应用描述成已有自动更新界面。

## Mac 签名硬约束

- 固定证书 `TTREE Open Source Code Signing`，SHA-1 `B8D254561BB6CAD78AA71FA18D62A32D10F50C08`。绝不打印、重建、替换、轮换、提交该证书或私钥；不得用 Secret 覆盖预期身份或指纹。
- 固定 designated requirement：`identifier "com.samwstu.ttree" and certificate leaf = H"b8d254561bb6cad78aa71fa18d62a32d10f50c08"`。录屏 TCC 依赖它，不能退化为 ad-hoc、identifier-only 或 CDHash 要求。
- 每个最终 `.app` 以及 DMG/归档中的 `.app` 必须通过 `scripts/macos/verify-signature.sh`（deep + strict、证书 Authority、绑定的 Info.plist、资源封印、指定要求）。禁止失败后发布。
- Apple 原生截图必须保持 `/usr/sbin/screencapture -i -x -r` + Vision；不得以解决权限为由改造为自绘选择窗口。
- 社区签名不等于 Apple 公证；`spctl` 拒绝并不能代替上述签名验证。README/Release 须保留“隐私与安全性 → 仍要打开”、录屏授权及重启说明。
- updater minisign 与 macOS 签名是两层独立保证，均须保留。更新私钥必须加密且口令至少 16 字符，只在打包后的 signer 步骤接收密钥；编译阶段不得暴露签名材料。
- 本机加密材料在 `~/.config/ttree-code-signing/` 和 `~/.config/ttree-updater/`，口令从既有 Keychain 项读取。CI 使用已有的四项证书/更新签名 Secrets，详见 [发布指南](docs/releasing.md)。
- TCC 跨升级实测需安装 A、授权一次、再安装 CDHash 不同但同证书的 B，不执行 `tccutil reset`，确认 OCR 不再提示。普通编译或签名检查不等于完成此实测。
