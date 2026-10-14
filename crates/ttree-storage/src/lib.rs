//! `ttree-storage` —— 内存级配置缓存（直接持久化至本地 `settings.json`，无 Keychain 弹窗）与 JSONL 对话历史持久化。

pub mod history;
pub mod settings;

pub use history::HistoryStore;
pub use settings::SettingsStore;
