# 构建与发布

## 触发策略

| 事件 | 构建验证 | GitHub Release |
| --- | --- | --- |
| 推送 main | Mac Universal + Windows x64 | `vX.Y.Z-dev.RUN.ATTEMPT`，预发布，不改变 Latest |
| 推送其他分支 | 同上 | 不发布 |
| PR | 两端编译/测试、Windows 打包启动；Mac 无签名编译 | 不发布，不读取签名 Secrets |
| 推送 vX.Y.Z 标签 | 同上并执行发行验证 | 正式版，更新 Latest |
| 手动 workflow_dispatch | 构建与临时 artifacts | 不发布 |

唯一工作流为 `.github/workflows/build.yml`。Mac 和 Windows 同时成功是发布前提，任一失败都不会发布单边版本。

## 正式发布

版本唯一来源为根 `Cargo.toml` 的 `[workspace.package].version`，所有 package 继承它。先更新版本并刷新 Cargo.lock，通过验证后提交至 main，再推送对应标签：

```sh
cargo check --workspace --locked
cargo test --workspace --locked
bash scripts/tests/run-release-tests.sh
git tag vX.Y.Z
git push origin vX.Y.Z
```

标签与版本不符会失败。不要移动已发布标签或覆盖已有 Release。脚本先创建草稿，上传后重新下载实际附件、校验哈希和更新签名，再公开；失败清理此次创建的草稿。发布正文包含双平台下载说明、权限注意事项、GitHub 自动变更说明及提交列表。

## 附件

| 文件 | 用途 |
| --- | --- |
| `TTREE_X.Y.Z_universal.dmg` | Mac 安装包，包含 arm64 + x86_64 |
| `TTREE_universal.app.tar.gz` | 同一个签名 App 的更新归档 |
| `TTREE_universal.app.tar.gz.sig` | 更新归档 minisign 签名 |
| `latest.json` | 旧更新客户端兼容清单，保留原公钥及平台键 |
| `TTREE_X.Y.Z_windows_x64.zip` | Windows 便携包，内含 `TTREE/TTREE.exe` 与许可/说明 |
| `SHA256SUMS` | 全部五项附件的 SHA-256 |

当前纯 Rust UI 未实现自动更新界面，用户从 Release 下载手动替换。保留旧更新附件协议不代表新 UI 已实现更新功能。

## 两层 Mac 签名

社区证书固定为 `TTREE Open Source Code Signing`，SHA-1 `B8D254561BB6CAD78AA71FA18D62A32D10F50C08`，Bundle ID 为 `com.samwstu.ttree`。指定要求必须始终绑定此证书指纹，以延续录屏 TCC 身份。禁止替换、重建或把指纹改成可由 Secret 覆盖的值。

CI 复用已有四项 Secrets：

- `MACOS_COMMUNITY_CERTIFICATE`：加密 P12 的 base64。
- `MACOS_COMMUNITY_CERTIFICATE_PASSWORD`：P12 强口令。
- `TAURI_SIGNING_PRIVATE_KEY`：加密的 minisign/rsign 更新私钥。
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：至少 16 字符的更新密钥口令。

旧仓库内其他身份/指纹 Secrets 不作为可信预期来源。公钥位于 `scripts/release/updater.pub`，从原发布配置原样迁移，不生成新密钥。

编译前先在临时小型 bundle 上预检证书、私钥与指定要求；预检编译器同样不接收签名材料。编译步骤没有上述 Secret 环境变量；编译完成后临时导入固定证书，签署 App，随后仅独立 signer 接收更新私钥。退出时删除临时签名 Keychain。

发布硬门禁包含 deep/strict 验证、Authority、Info.plist、封印资源、指定要求；并解开 tar.gz、挂载 DMG 验证容器内的 App。签名失败、ad-hoc 或证书变化都阻止发布。Mac 包非 Apple 公证，首次安装需“隐私与安全性 → 仍要打开”，随后录屏授权并重启应用。

本机沿用既有加密材料及 Keychain 口令：

```sh
source scripts/macos/signing-env.sh
scripts/macos/package.sh universal-apple-darwin 1
# 仅编译，不读取签名材料
scripts/macos/package.sh universal-apple-darwin 1 --build-only
```

Mac 发行输出为 `dist/macos/`，Windows 为 `dist/windows/`；中间 App 在 `target/universal-apple-darwin/release/bundle/macos/TTREE.app`。Windows 本机打包命令为 `./scripts/windows/package.ps1`。

## 验证与证据

Actions 保存每次 Mac/Windows 产物七天；Windows 另保存原生启动检查结果与界面截图。正式附件从这些已验证 artifact 合并产生，再下载草稿中的附件复验。

TCC 权限延续需另做物理机 A→B 实测：授权 A，安装不同 CDHash 但同证书/指定要求的 B，不重置 TCC，执行 OCR 不应再要求授权。签名验证通过只能证明身份一致，不能宣称已完成这项交互验证。
