# macOS

继承根 AGENTS.md。这里只放 AppKit 窗口、控件、菜单和 Mac UI 状态。

- `app.rs` 负责入口、事件分发、窗口和菜单；`state.rs` 负责控制器；`ui_helpers.rs` 负责原生控件和 splitter。
- 系统划词放 `crates/ttree-platform/src/macos/`，截图/Vision 放 `crates/ttree-ocr/src/macos/`。
- 使用 objc2 的主线程约束；后台任务经 UI 事件回到主线程更新控件。
- 保留 Accessory 模式、LSUIElement、隐藏而非销毁、置顶与失焦行为。
- 发布必须沿用固定社区证书；详见根规则和发布指南。不要将开发运行成功等同于打包后的 TCC 升级验证。
- `cargo run -p ttree-macos -- --show` 开发；`scripts/macos/package.sh` 打包。
