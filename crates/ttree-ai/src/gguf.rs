//! 纯 Rust `Candle` 本地 `.gguf` 量化模型直接加载与流式推理（支持 macOS Metal GPU 加速）。
//!
//! 核心亮点：
//! 1. 直接从 `.gguf` 文件内置的 `tokenizer.ggml.tokens` 元数据构建分词器与解码器，无需额外准备 `tokenizer.json`；
//! 2. 自动识别 `qwen2`、`qwen3`、`llama`（含 Llama3 / Mistral / DeepSeek 等）主流 GGUF 架构；
//! 3. 优先启用 Apple Silicon `Device::new_metal(0)` GPU 加速，并在内存中常驻缓存已加载的模型权重，二次翻译 0 加载延迟；
//! 4. 每生成一个 Token 即刻回调 UI 并检查 `CancelToken`，支持随时毫秒级打断。

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use candle_core::quantized::gguf_file::{self, Value as GgufValue};
use candle_core::{Device, Tensor};
use candle_transformers::models::{quantized_llama, quantized_qwen2, quantized_qwen3};
use ttree_core::{CancelToken, ChatMessage};

const MAX_NEW_TOKENS: usize = 512;

enum QuantizedArch {
    Qwen2(quantized_qwen2::ModelWeights),
    Qwen3(quantized_qwen3::ModelWeights),
    Llama(quantized_llama::ModelWeights),
}

impl QuantizedArch {
    fn clear_kv_cache(&mut self) {
        match self {
            Self::Qwen2(m) => m.clear_kv_cache(),
            Self::Qwen3(m) => m.clear_kv_cache(),
            Self::Llama(_) => {}
        }
    }

    fn forward(&mut self, input: &Tensor, index_pos: usize) -> candle_core::Result<Tensor> {
        match self {
            Self::Qwen2(m) => m.forward(input, index_pos),
            Self::Qwen3(m) => m.forward(input, index_pos),
            Self::Llama(m) => m.forward(input, index_pos),
        }
    }
}

/// 从 GGUF 内嵌的 `tokenizer.ggml.tokens` 词表构建的轻量分词/解码器。
pub struct GgufEmbeddedTokenizer {
    id_to_token: Vec<String>,
    token_to_id: HashMap<String, u32>,
    eos_tokens: Vec<u32>,
    bos_token: Option<u32>,
}

impl GgufEmbeddedTokenizer {
    pub fn from_gguf_metadata(md: &HashMap<String, GgufValue>) -> Result<Self, String> {
        let raw_tokens = md
            .get("tokenizer.ggml.tokens")
            .and_then(|v| v.to_vec().ok())
            .ok_or_else(|| "GGUF 文件缺少内置词表 (tokenizer.ggml.tokens)".to_string())?;

        let mut id_to_token = Vec::with_capacity(raw_tokens.len());
        let mut token_to_id = HashMap::with_capacity(raw_tokens.len());

        for (idx, val) in raw_tokens.iter().enumerate() {
            let s = val.to_string().cloned().unwrap_or_default();
            token_to_id.insert(s.clone(), idx as u32);
            id_to_token.push(s);
        }

        let mut eos_tokens = Vec::new();
        if let Some(eos_id) = md
            .get("tokenizer.ggml.eos_token_id")
            .and_then(|v| v.to_u32().ok())
        {
            eos_tokens.push(eos_id);
        }
        for special in ["<|im_end|>", "<|endoftext|>", "</s>", "<|eot_id|>", "<end_of_turn>"] {
            if let Some(&id) = token_to_id.get(special) {
                if !eos_tokens.contains(&id) {
                    eos_tokens.push(id);
                }
            }
        }

        let bos_token = md
            .get("tokenizer.ggml.bos_token_id")
            .and_then(|v| v.to_u32().ok());

        Ok(Self {
            id_to_token,
            token_to_id,
            eos_tokens,
            bos_token,
        })
    }

    pub fn is_eos(&self, token_id: u32) -> bool {
        self.eos_tokens.contains(&token_id)
    }

    /// 最长前缀贪心匹配 + GPT2/SentencePiece 字节回退分词。
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let mut ids = Vec::new();
        if let Some(bos) = self.bos_token {
            ids.push(bos);
        }
        // 将空格映射为 SentencePiece `▁` 或 GPT-2 `Ġ` 候选以匹配主流 GGUF 词表
        let uses_sp_space = self.token_to_id.contains_key("▁the")
            || self.token_to_id.contains_key("▁")
            || self.token_to_id.contains_key("▁a");
        let normalized = if uses_sp_space {
            text.replace(' ', "▁")
        } else {
            text.replace(' ', "Ġ").replace('\n', "Ċ")
        };

        let mut rest = normalized.as_str();
        while !rest.is_empty() {
            // 先匹配特殊控制标记（如 `<|im_start|>`、`<|im_end|>`）
            if rest.starts_with("<|") {
                if let Some(end_idx) = rest.find("|>") {
                    let special = &rest[..end_idx + 2];
                    if let Some(&id) = self.token_to_id.get(special) {
                        ids.push(id);
                        rest = &rest[end_idx + 2..];
                        continue;
                    }
                }
            }

            let mut matched = None;
            let max_bytes = rest.len().min(32);
            let mut end = max_bytes;
            while end > 0 {
                if rest.is_char_boundary(end) {
                    let candidate = &rest[..end];
                    if let Some(&id) = self.token_to_id.get(candidate) {
                        matched = Some((id, end));
                        break;
                    }
                }
                end -= 1;
            }

            if let Some((id, len)) = matched {
                ids.push(id);
                rest = &rest[len..];
            } else {
                // 单字节 `<0xXX>` 回退
                let ch = rest.chars().next().unwrap();
                let ch_len = ch.len_utf8();
                for &b in &rest.as_bytes()[..ch_len] {
                    let hex_tok = format!("<0x{b:02X}>");
                    if let Some(&id) = self.token_to_id.get(&hex_tok) {
                        ids.push(id);
                    }
                }
                rest = &rest[ch_len..];
            }
        }
        ids
    }

    pub fn decode_token_bytes(&self, token_id: u32) -> Vec<u8> {
        let Some(piece) = self.id_to_token.get(token_id as usize) else {
            return Vec::new();
        };
        if piece.starts_with("<|") && piece.ends_with("|>") {
            return Vec::new();
        }
        if piece.len() == 6 && piece.starts_with("<0x") && piece.ends_with('>') {
            if let Ok(b) = u8::from_str_radix(&piece[3..5], 16) {
                return vec![b];
            }
        }
        let decoded = piece
            .replace('▁', " ")
            .replace("Ġ", " ")
            .replace("Ċ", "\n");
        decoded.into_bytes()
    }
}

struct CachedSession {
    model_path: String,
    device: Device,
    arch: QuantizedArch,
    tokenizer: GgufEmbeddedTokenizer,
}

static GGUF_SESSION: OnceLock<Mutex<Option<CachedSession>>> = OnceLock::new();

fn select_device() -> Device {
    #[cfg(target_os = "macos")]
    if let Ok(metal) = Device::new_metal(0) {
        return metal;
    }
    Device::Cpu
}

fn load_session(model_path: &str) -> Result<CachedSession, String> {
    let path = Path::new(model_path);
    if !path.is_file() {
        return Err(format!("找不到本地 GGUF 模型文件: {model_path}"));
    }
    let file = File::open(path).map_err(|e| format!("打开 GGUF 文件失败: {e}"))?;
    let mut reader = BufReader::new(file);
    let content = gguf_file::Content::read(&mut reader)
        .map_err(|e| format!("解析 GGUF 文件头失败: {e}"))?;

    let arch_name = content
        .metadata
        .get("general.architecture")
        .and_then(|v| v.to_string().ok())
        .cloned()
        .unwrap_or_else(|| "llama".to_string());

    let tokenizer = GgufEmbeddedTokenizer::from_gguf_metadata(&content.metadata)?;
    let device = select_device();

    let arch = match arch_name.as_str() {
        "qwen2" => {
            let weights = quantized_qwen2::ModelWeights::from_gguf(content, &mut reader, &device)
                .map_err(|e| format!("加载 Qwen2 GGUF 权重失败: {e}"))?;
            QuantizedArch::Qwen2(weights)
        }
        "qwen3" => {
            let weights = quantized_qwen3::ModelWeights::from_gguf(content, &mut reader, &device)
                .map_err(|e| format!("加载 Qwen3 GGUF 权重失败: {e}"))?;
            QuantizedArch::Qwen3(weights)
        }
        _ => {
            let weights = quantized_llama::ModelWeights::from_gguf(content, &mut reader, &device)
                .map_err(|e| format!("加载 {arch_name} GGUF 权重失败: {e}"))?;
            QuantizedArch::Llama(weights)
        }
    };

    Ok(CachedSession {
        model_path: model_path.to_string(),
        device,
        arch,
        tokenizer,
    })
}

/// 将对话消息格式化为通用 ChatML 提示词模板（兼容 Qwen / Llama / OpenChat 等 GGUF 模型）。
pub fn format_chatml_prompt(messages: &[ChatMessage]) -> String {
    let mut prompt = String::new();
    for msg in messages {
        prompt.push_str("<|im_start|>");
        prompt.push_str(&msg.role);
        prompt.push('\n');
        prompt.push_str(&msg.content);
        prompt.push_str("<|im_end|>\n");
    }
    prompt.push_str("<|im_start|>assistant\n");
    prompt
}

pub async fn stream_gguf_inference<F>(
    gguf_model_path: &str,
    _gguf_tokenizer_path: &str,
    messages: &[ChatMessage],
    cancel: &CancelToken,
    on_delta: &mut F,
) -> Result<(), String>
where
    F: FnMut(&str) + Send,
{
    let model_path = gguf_model_path.trim().to_string();
    if model_path.is_empty() {
        return Err("未选择本地 .gguf 模型文件路径，请在设置中指定文件位置".to_string());
    }

    let prompt = format_chatml_prompt(messages);
    let cancel_clone = cancel.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, String>>();

    tokio::task::spawn_blocking(move || {
        let slot = GGUF_SESSION.get_or_init(|| Mutex::new(None));
        let mut guard = match slot.lock() {
            Ok(g) => g,
            Err(e) => {
                let _ = tx.send(Err(format!("GGUF 引擎锁异常: {e}")));
                return;
            }
        };

        let needs_reload = guard
            .as_ref()
            .is_none_or(|s| s.model_path != model_path);
        if needs_reload {
            match load_session(&model_path) {
                Ok(session) => *guard = Some(session),
                Err(err) => {
                    let _ = tx.send(Err(err));
                    return;
                }
            }
        }

        let session = guard.as_mut().expect("session loaded");
        session.arch.clear_kv_cache();

        let prompt_tokens = session.tokenizer.encode(&prompt);
        if prompt_tokens.is_empty() {
            let _ = tx.send(Err("提示词分词为空".to_string()));
            return;
        }

        let mut next_token = match run_step(&mut session.arch, &session.device, &prompt_tokens, 0) {
            Ok(tok) => tok,
            Err(err) => {
                let _ = tx.send(Err(err));
                return;
            }
        };

        let mut utf8_pending = Vec::new();

        for pos in (prompt_tokens.len()..).take(MAX_NEW_TOKENS) {
            if cancel_clone.is_cancelled() {
                return;
            }
            if session.tokenizer.is_eos(next_token) {
                break;
            }
            utf8_pending.extend_from_slice(&session.tokenizer.decode_token_bytes(next_token));
            if let Ok(valid_str) = std::str::from_utf8(&utf8_pending) {
                if !valid_str.is_empty() {
                    let _ = tx.send(Ok(valid_str.to_string()));
                }
                utf8_pending.clear();
            }

            next_token = match run_step(&mut session.arch, &session.device, &[next_token], pos) {
                Ok(tok) => tok,
                Err(err) => {
                    let _ = tx.send(Err(err));
                    return;
                }
            };
        }
    });

    while let Some(item) = rx.recv().await {
        if cancel.is_cancelled() {
            return Err("请求已取消".to_string());
        }
        let delta = item?;
        on_delta(&delta);
    }
    Ok(())
}

/// 执行一步前向推理并采用 Argmax 贪心采样（翻译任务使用 temperature=0 最准确稳定）。
fn run_step(
    arch: &mut QuantizedArch,
    device: &Device,
    tokens: &[u32],
    index_pos: usize,
) -> Result<u32, String> {
    let input = Tensor::new(tokens, device)
        .and_then(|t| t.unsqueeze(0))
        .map_err(|e| format!("构造输入张量失败: {e}"))?;
    let logits = arch
        .forward(&input, index_pos)
        .map_err(|e| format!("GGUF 模型前向推理失败: {e}"))?;
    let logits = logits
        .squeeze(0)
        .map_err(|e| format!("张量维度压缩失败: {e}"))?;
    let next_id = logits
        .argmax(0)
        .and_then(|t| t.to_scalar::<u32>())
        .map_err(|e| format!("采样下一个 Token 失败: {e}"))?;
    Ok(next_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_chatml_prompt_correctly() {
        let msgs = vec![
            ChatMessage::system("Translate to Chinese."),
            ChatMessage::user("Hello"),
        ];
        let p = format_chatml_prompt(&msgs);
        assert!(p.contains("<|im_start|>system\nTranslate to Chinese.<|im_end|>"));
        assert!(p.ends_with("<|im_start|>assistant\n"));
    }
}
