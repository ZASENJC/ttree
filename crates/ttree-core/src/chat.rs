//! 对话消息与会话上下文裁剪。

use serde::{Deserialize, Serialize};

const ALLOWED_ROLES: &[&str] = &["user", "assistant", "system"];
pub const MAX_CONTENT_BYTES: usize = 64 * 1024;
pub const MAX_MESSAGES: usize = 200;
pub const MAX_CONTEXT_MESSAGES: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: usize,
    pub messages: Vec<ChatMessage>,
    pub summary: String,
}

/// 校验消息角色白名单、单条长度与总条数。
pub fn validate_messages(msgs: &[ChatMessage]) -> Result<(), String> {
    if msgs.len() > MAX_MESSAGES {
        return Err(format!("消息数超出上限 {MAX_MESSAGES} 条"));
    }
    for m in msgs {
        if !ALLOWED_ROLES.contains(&m.role.as_str()) {
            return Err(format!("非法消息角色: {}", m.role));
        }
        if m.content.len() > MAX_CONTENT_BYTES {
            return Err(format!("单条消息超出 {MAX_CONTENT_BYTES} 字节上限"));
        }
    }
    Ok(())
}

/// 截取最近 24 条非空消息，并丢弃前导孤立的 `assistant` 消息，确保上下文以 `user` 开头。
pub fn recent_chat_context(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let filtered: Vec<ChatMessage> = messages
        .iter()
        .filter(|m| !m.content.trim().is_empty())
        .cloned()
        .collect();
    let start = filtered.len().saturating_sub(MAX_CONTEXT_MESSAGES);
    let mut slice = &filtered[start..];
    while let Some(first) = slice.first() {
        if first.role == "assistant" {
            slice = &slice[1..];
        } else {
            break;
        }
    }
    slice.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_context_and_drops_leading_orphan_assistant() {
        let mut msgs = Vec::new();
        for i in 0..30 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            msgs.push(ChatMessage {
                role: role.to_string(),
                content: format!("msg-{i}"),
            });
        }
        let ctx = recent_chat_context(&msgs);
        assert!(ctx.len() <= MAX_CONTEXT_MESSAGES);
        assert_eq!(ctx.first().unwrap().role, "user");
    }

    #[test]
    fn validates_roles_and_lengths() {
        assert!(validate_messages(&[ChatMessage::user("hi")]).is_ok());
        assert!(validate_messages(&[ChatMessage {
            role: "tool".to_string(),
            content: "x".to_string()
        }])
        .is_err());
    }
}
