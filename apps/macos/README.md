# TTREE for macOS

AppKit 原生界面，翻译、AI 和设置使用同一控制器。共享业务库位于根 `crates/`。

```sh
cargo run -p ttree-macos -- --show
# Universal 发行包；签名材料沿用本机既有配置
source scripts/macos/signing-env.sh
scripts/macos/package.sh
```

在仓库根目录执行。开发构建不会证明发行包的录屏权限可延续。签名身份、首次安装和验证步骤见 [发布指南](../../docs/releasing.md)。
