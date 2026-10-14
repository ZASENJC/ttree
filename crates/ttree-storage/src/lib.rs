//! `ttree-storage` —— 内存级配置缓存（兼容既有 `settings.json` 与 Keychain）与 JSONL 对话历史持久化。

pub mod history;
pub mod settings;

pub use history::HistoryStore;
pub use settings::SettingsStore;
