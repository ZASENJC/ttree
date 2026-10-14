//! AI 对话历史持久化（JSONL 追加写，100% 兼容既有 `chat-history.jsonl`）。

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Deserialize;
use ttree_core::{validate_messages, ChatMessage, Conversation};

use crate::settings::SettingsStore;

pub const HISTORY_FILE: &str = "chat-history.jsonl";
const SESSION_START_MARKER: &str = r#"{"type":"session_start"}"#;
const SUMMARY_MAX_CHARS: usize = 40;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum HistoryLine {
    SessionResume {
        #[serde(rename = "type")]
        kind: String,
        id: usize,
    },
    SessionStart {
        #[serde(rename = "type")]
        kind: String,
    },
    Message(ChatMessage),
}

pub struct HistoryStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl HistoryStore {
    pub fn open_default() -> Self {
        let dir = SettingsStore::default_config_dir();
        let _ = fs::create_dir_all(&dir);
        Self::open_at(dir.join(HISTORY_FILE))
    }

    pub fn open_at(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    pub fn start_new_conversation(&self) -> Result<(), String> {
        let _guard = self.lock.lock().map_err(|e| format!("历史锁中毒: {e}"))?;
        let needs_marker = fs::metadata(&self.path)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
        if !needs_marker {
            return Ok(());
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("打开历史文件失败: {e}"))?;
        let mut buf = String::from(SESSION_START_MARKER);
        buf.push('\n');
        file.write_all(buf.as_bytes())
            .map_err(|e| format!("写入对话分隔标记失败: {e}"))
    }

    pub fn append_round(
        &self,
        conversation_id: Option<usize>,
        msgs: &[ChatMessage],
    ) -> Result<(), String> {
        if msgs.is_empty() {
            return Ok(());
        }
        validate_messages(msgs)?;
        let _guard = self.lock.lock().map_err(|e| format!("历史锁中毒: {e}"))?;
        let mut buf = String::with_capacity(128 * msgs.len());
        if let Some(id) = conversation_id {
            let conversations = read_conversations_from_path(&self.path)?;
            if !conversations.iter().any(|c| c.id == id) {
                return Err(format!("对话不存在: {id}"));
            }
            let marker = serde_json::json!({ "type": "session_resume", "id": id });
            buf.push_str(
                &serde_json::to_string(&marker)
                    .map_err(|e| format!("序列化对话恢复标记失败: {e}"))?,
            );
            buf.push('\n');
        }
        for m in msgs {
            let line = serde_json::to_string(m).map_err(|e| format!("序列化消息失败: {e}"))?;
            buf.push_str(&line);
            buf.push('\n');
        }
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("打开历史文件失败: {e}"))?;
        file.write_all(buf.as_bytes())
            .map_err(|e| format!("写入历史文件失败: {e}"))
    }

    pub fn read_conversations(&self) -> Result<Vec<Conversation>, String> {
        let _guard = self.lock.lock().map_err(|e| format!("历史锁中毒: {e}"))?;
        read_conversations_from_path(&self.path)
    }

    pub fn clear(&self) -> Result<(), String> {
        let _guard = self.lock.lock().map_err(|e| format!("历史锁中毒: {e}"))?;
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("删除历史文件失败: {e}")),
        }
    }
}

fn read_conversations_from_path(path: &Path) -> Result<Vec<Conversation>, String> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("打开历史文件失败: {e}")),
    };
    let reader = BufReader::new(file);
    Ok(parse_conversation_lines(
        reader.lines().map_while(Result::ok),
    ))
}

pub fn parse_conversation_lines<I>(lines: I) -> Vec<Conversation>
where
    I: IntoIterator<Item = String>,
{
    let mut conversations: Vec<Conversation> = Vec::new();
    let mut current_id: Option<usize> = None;
    let mut next_id: usize = 0;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<HistoryLine>(trimmed) else {
            continue;
        };
        match parsed {
            HistoryLine::SessionResume { kind, id } if kind == "session_resume" => {
                current_id = Some(id);
                next_id = next_id.max(id.saturating_add(1));
                if let Some(index) = conversations.iter().position(|c| c.id == id) {
                    let conv = conversations.remove(index);
                    conversations.push(conv);
                }
            }
            HistoryLine::SessionStart { kind } if kind == "session_start" => {
                current_id = Some(next_id);
                next_id = next_id.saturating_add(1);
            }
            HistoryLine::Message(msg) => {
                let id = current_id.unwrap_or_else(|| {
                    let id = next_id;
                    next_id = next_id.saturating_add(1);
                    current_id = Some(id);
                    id
                });
                if !conversations.iter().any(|c| c.id == id) {
                    conversations.push(Conversation {
                        id,
                        messages: Vec::new(),
                        summary: String::new(),
                    });
                }
                let conv = conversations
                    .iter_mut()
                    .find(|c| c.id == id)
                    .expect("conversation exists");
                if conv.summary.is_empty() && msg.role == "user" {
                    conv.summary = truncate_summary(&msg.content);
                }
                conv.messages.push(msg);
            }
            _ => continue,
        }
    }
    conversations
}

fn truncate_summary(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= SUMMARY_MAX_CHARS {
        return first_line.to_string();
    }
    let truncated: String = first_line.chars().take(SUMMARY_MAX_CHARS).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_conversations_and_session_splits() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = HistoryStore::open_at(tmp.path().to_path_buf());

        store
            .append_round(
                None,
                &[
                    ChatMessage::user("第一段问题"),
                    ChatMessage::assistant("第一段回答"),
                ],
            )
            .unwrap();
        store.start_new_conversation().unwrap();
        store
            .append_round(
                None,
                &[
                    ChatMessage::user("第二段问题"),
                    ChatMessage::assistant("第二段回答"),
                ],
            )
            .unwrap();

        let convs = store.read_conversations().unwrap();
        assert_eq!(convs.len(), 2);
        assert_eq!(convs[0].summary, "第一段问题");
        assert_eq!(convs[1].summary, "第二段问题");
    }
}
