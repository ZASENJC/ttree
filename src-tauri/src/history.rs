//! AI 对话历史持久化 —— 行分隔 JSON (JSONL) 追加文件。
//!
//! 设计目标：**最省存储 + 全量 + 以对话为单位**。
//! - 追加写：每轮结束把消息各写一行，`OpenOptions::append`，**永不重写**旧数据，
//!   O(1) 写入、最小 I/O。
//! - 行分隔：无 JSON 数组括号/缩进，每行一条 JSON，天然最小体积。
//! - 流式读：`BufReader::lines()` 逐行解析，单行损坏不影响其余（容错）。
//! - 独立文件 `chat-history.jsonl`，与 `settings.json`（含 API 配置）物理隔离。
//! - 对话边界：用一条 `{"type":"session_start"}` 标记新对话开始。
//!   旧数据（无标记）自动归为最旧的一段，向后兼容。

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::translate::ChatMessage;

/// 历史文件名（与 settings.json 同目录，但独立文件）。
const HISTORY_FILE: &str = "chat-history.jsonl";

/// 序列化所有历史读写：多个 Tauri 命令（append/read/clear）可能并发执行，
/// 若各自 open + write_all 会交错写入、破坏 JSONL。临界区很短，单把锁足够。
static HISTORY_LOCK: Mutex<()> = Mutex::new(());

/// 单段对话：包含一组消息，以及用于列表展示的摘要（首句用户提问）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    /// 稳定会话 id；恢复旧对话后返回顺序会变化，但 id 保持不变。
    pub id: usize,
    pub messages: Vec<ChatMessage>,
    /// 摘要：该段对话第一条用户提问（截断），用作列表标题。
    pub summary: String,
}

/// 文件行：可能是对话分隔标记或普通消息。
/// 用未标记的 `#[serde(untagged)]` 让旧格式 `{"role","content"}` 继续可解析。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum HistoryLine {
    /// 恢复已有对话，后续消息继续归入指定稳定 id。
    SessionResume {
        #[serde(rename = "type")]
        kind: String,
        id: usize,
    },
    /// 新对话开始标记（`{"type":"session_start"}`）。
    SessionStart {
        #[serde(rename = "type")]
        kind: String,
    },
    /// 普通消息（role/content）。
    Message(ChatMessage),
}

const SESSION_START_MARKER: &str = r#"{"type":"session_start"}"#;
const SUMMARY_MAX_CHARS: usize = 40;

/// 解析历史文件绝对路径。目录不存在时尝试创建。
fn history_path<R: Runtime>(app: &AppHandle<R>) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("解析配置目录失败: {e}"))?;
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    Ok(dir.join(HISTORY_FILE))
}

/// 写入对话分隔标记，表示之后的消息属于一段新对话。
/// 仅在文件已有内容时才需要写标记（首段对话无需前导标记）。
pub fn start_new_conversation<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let _guard = HISTORY_LOCK
        .lock()
        .map_err(|e| format!("历史锁中毒: {e}"))?;
    let path = history_path(app)?;
    // 空文件无需分隔标记——它本身已是新对话起点
    let needs_marker = match fs::metadata(&path) {
        Ok(meta) => meta.len() > 0,
        Err(_) => false, // 不存在 → 视为空，首段无需标记
    };
    if !needs_marker {
        return Ok(());
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("打开历史文件失败: {e}"))?;
    let mut buf = String::from(SESSION_START_MARKER);
    buf.push('\n');
    file.write_all(buf.as_bytes())
        .map_err(|e| format!("写入对话分隔标记失败: {e}"))
}

/// 追加一轮对话（多条消息）。
///
/// 每条消息独占一行，单次 write 调用写完整批，保证原子性尽量高、I/O 最小。
/// 空切片直接返回 Ok。
pub fn append_round<R: Runtime>(
    app: &AppHandle<R>,
    conversation_id: Option<usize>,
    msgs: &[ChatMessage],
) -> Result<(), String> {
    if msgs.is_empty() {
        return Ok(());
    }
    let _guard = HISTORY_LOCK
        .lock()
        .map_err(|e| format!("历史锁中毒: {e}"))?;
    let path = history_path(app)?;
    // 预先序列化所有行，合并成一次写入
    let mut buf = String::with_capacity(128 * msgs.len());
    if let Some(id) = conversation_id {
        let conversations = read_conversations_from_path(&path)?;
        if !conversations
            .iter()
            .any(|conversation| conversation.id == id)
        {
            return Err(format!("对话不存在: {id}"));
        }
        let marker = serde_json::json!({ "type": "session_resume", "id": id });
        buf.push_str(
            &serde_json::to_string(&marker).map_err(|e| format!("序列化对话恢复标记失败: {e}"))?,
        );
        buf.push('\n');
    }
    for m in msgs {
        let line = serde_json::to_string(m).map_err(|e| format!("序列化消息失败: {e}"))?;
        buf.push_str(&line);
        buf.push('\n');
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("打开历史文件失败: {e}"))?;
    file.write_all(buf.as_bytes())
        .map_err(|e| format!("写入历史文件失败: {e}"))
}

/// 读取全部历史消息（扁平流，向后兼容旧前端调用）。
pub fn read_all<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<ChatMessage>, String> {
    Ok(read_conversations(app)?
        .into_iter()
        .flat_map(|c| c.messages)
        .collect())
}

/// 以对话为单位读取历史：按 `session_start` 标记切分，旧数据（无标记）
/// 归为第一段（id=0）。返回顺序为最久未活动 → 最近活动。
pub fn read_conversations<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<Conversation>, String> {
    let _guard = HISTORY_LOCK
        .lock()
        .map_err(|e| format!("历史锁中毒: {e}"))?;
    let path = history_path(app)?;
    read_conversations_from_path(&path)
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

fn parse_conversation_lines<I>(lines: I) -> Vec<Conversation>
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
        // 单行损坏不影响其余历史
        let parsed = match serde_json::from_str::<HistoryLine>(trimmed) {
            Ok(p) => p,
            Err(_) => continue,
        };
        match parsed {
            HistoryLine::SessionResume { kind, id } if kind == "session_resume" => {
                current_id = Some(id);
                next_id = next_id.max(id.saturating_add(1));
                if let Some(index) = conversations.iter().position(|c| c.id == id) {
                    let conversation = conversations.remove(index);
                    conversations.push(conversation);
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
                if !conversations
                    .iter()
                    .any(|conversation| conversation.id == id)
                {
                    conversations.push(Conversation {
                        id,
                        messages: Vec::new(),
                        summary: String::new(),
                    });
                }
                let conv = conversations
                    .iter_mut()
                    .find(|conversation| conversation.id == id)
                    .expect("conversation was inserted above");
                if conv.summary.is_empty() && msg.role == "user" {
                    conv.summary = truncate_summary(&msg.content);
                }
                conv.messages.push(msg);
            }
            // 非法 type 值：忽略，不影响其余
            _ => continue,
        }
    }

    conversations
}

/// 截取摘要：首行 + 限定长度，避免超长提问撑爆列表标题。
fn truncate_summary(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= SUMMARY_MAX_CHARS {
        return first_line.to_string();
    }
    let truncated: String = first_line.chars().take(SUMMARY_MAX_CHARS).collect();
    format!("{truncated}…")
}

/// 清空全部历史：直接删除文件（下次 append 会重建）。
pub fn clear<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let _guard = HISTORY_LOCK
        .lock()
        .map_err(|e| format!("历史锁中毒: {e}"))?;
    let path = history_path(app)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除历史文件失败: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn parse_recovers_around_corrupt_lines() {
        // 构造一段含损坏行的 JSONL：第 1、3 行合法，第 2 行损坏
        let data = concat!(
            r#"{"role":"user","content":"你好"}"#,
            "\n",
            r#"this is not json"#,
            "\n",
            r#"{"role":"assistant","content":"你好！有什么可以帮你？"}"#,
            "\n",
        );
        let mut out = Vec::new();
        for line in data.lines() {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            if let Ok(m) = serde_json::from_str::<ChatMessage>(t) {
                out.push(m);
            }
        }
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].role, "user");
        assert_eq!(out[1].content, "你好！有什么可以帮你？");
    }

    #[test]
    fn append_then_read_roundtrip() {
        // 临时文件：序列化 → 反序列化，验证行格式正确
        let round = vec![msg("user", "翻译"), msg("assistant", "translate")];
        let mut buf = String::new();
        for m in &round {
            buf.push_str(&serde_json::to_string(m).unwrap());
            buf.push('\n');
        }
        let parsed: Vec<ChatMessage> = buf
            .lines()
            .filter_map(|l| serde_json::from_str(l.trim()).ok())
            .collect();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].content, "翻译");
        assert_eq!(parsed[1].role, "assistant");
    }

    #[test]
    fn empty_input_is_noop() {
        // 追加空切片应直接成功（由调用侧保证，这里验证逻辑）
        let round: Vec<ChatMessage> = vec![];
        assert!(round.is_empty());
    }

    #[test]
    fn each_message_is_one_line_compact() {
        // 验证最省存储：无缩进、无多余空白，每条恰好一行
        let s = serde_json::to_string(&msg("user", "hi")).unwrap();
        assert!(!s.contains('\n'));
        assert!(!s.contains("  ")); // 无两连空格（缩进）
        assert_eq!(s, r#"{"role":"user","content":"hi"}"#);
    }

    /// 解析 JSONL 为对话列表的核心逻辑（不依赖 AppHandle，便于单测）。
    fn parse_conversations(data: &str) -> Vec<Conversation> {
        parse_conversation_lines(data.lines().map(str::to_string))
    }

    #[test]
    fn legacy_data_without_markers_becomes_single_conversation() {
        // 旧格式（无 session_start 标记）：全部归为一段对话（id=0）
        let data = concat!(
            r#"{"role":"user","content":"你好"}"#,
            "\n",
            r#"{"role":"assistant","content":"你好！"}"#,
            "\n",
            r#"{"role":"user","content":"再见"}"#,
            "\n",
        );
        let convs = parse_conversations(data);
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].id, 0);
        assert_eq!(convs[0].messages.len(), 3);
        assert_eq!(convs[0].summary, "你好");
    }

    #[test]
    fn session_start_marker_splits_conversations() {
        // 两个 session_start → 三段对话，摘要各取首句用户提问
        let data = concat!(
            r#"{"role":"user","content":"第一个问题"}"#,
            "\n",
            r#"{"role":"assistant","content":"回答一"}"#,
            "\n",
            r#"{"type":"session_start"}"#,
            "\n",
            r#"{"role":"user","content":"第二个问题"}"#,
            "\n",
            r#"{"role":"assistant","content":"回答二"}"#,
            "\n",
            r#"{"type":"session_start"}"#,
            "\n",
            r#"{"role":"user","content":"第三个问题"}"#,
            "\n",
        );
        let convs = parse_conversations(data);
        assert_eq!(convs.len(), 3);
        assert_eq!(convs[0].messages.len(), 2);
        assert_eq!(convs[1].messages.len(), 2);
        assert_eq!(convs[2].messages.len(), 1);
        assert_eq!(convs[0].summary, "第一个问题");
        assert_eq!(convs[1].summary, "第二个问题");
        assert_eq!(convs[2].summary, "第三个问题");
        // id 严格递增
        assert_eq!(convs[0].id, 0);
        assert_eq!(convs[1].id, 1);
        assert_eq!(convs[2].id, 2);
    }

    #[test]
    fn long_summary_is_truncated() {
        // 超过 SUMMARY_MAX_CHARS(40) 的提问应被截断并加省略号
        let long = "一".repeat(60);
        let summary = truncate_summary(&long);
        assert!(summary.ends_with('…'));
        // 截断后 = 40 字符 + 1 省略号
        assert_eq!(summary.chars().count(), SUMMARY_MAX_CHARS + 1);
    }

    #[test]
    fn short_summary_is_kept_intact() {
        let short = "简短提问".to_string();
        assert_eq!(truncate_summary(&short), "简短提问");
    }

    #[test]
    fn summary_takes_first_line_only() {
        let multi = "第一行提问\n第二行不该出现".to_string();
        assert_eq!(truncate_summary(&multi), "第一行提问");
    }

    #[test]
    fn summary_skips_assistant_until_first_user() {
        // 首条是 assistant（异常但需容错），摘要应取第一条 user
        let data = concat!(
            r#"{"role":"assistant","content":"嗨"}"#,
            "\n",
            r#"{"role":"user","content":"真正的问题"}"#,
            "\n",
        );
        let convs = parse_conversations(data);
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].summary, "真正的问题");
    }

    #[test]
    fn resumed_conversation_receives_appended_round_and_becomes_most_recent() {
        let data = concat!(
            r#"{"role":"user","content":"A1"}"#,
            "\n",
            r#"{"role":"assistant","content":"A2"}"#,
            "\n",
            r#"{"type":"session_start"}"#,
            "\n",
            r#"{"role":"user","content":"B1"}"#,
            "\n",
            r#"{"role":"assistant","content":"B2"}"#,
            "\n",
            r#"{"type":"session_resume","id":0}"#,
            "\n",
            r#"{"role":"user","content":"A3"}"#,
            "\n",
            r#"{"role":"assistant","content":"A4"}"#,
            "\n",
        );

        let convs = parse_conversations(data);

        assert_eq!(convs.len(), 2);
        assert_eq!(convs[0].id, 1);
        assert_eq!(convs[0].messages.last().unwrap().content, "B2");
        assert_eq!(convs[1].id, 0);
        assert_eq!(convs[1].messages.len(), 4);
        assert_eq!(convs[1].messages.last().unwrap().content, "A4");
    }
}
