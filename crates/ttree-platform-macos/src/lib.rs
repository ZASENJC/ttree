//! `ttree-platform-macos` —— macOS 原生系统能力封装：
//! - Accessibility + 剪贴板回退的划词捕获
//! - 全局热键解析、冲突校验与内存级 O(1) 分发
//! - LaunchAgent 开机自启管理

pub mod autostart;
pub mod hotkey;
pub mod selection;
