//! SSE (Server-Sent Events) 与 NDJSON (Ollama) 流式帧解码器。

use serde_json::Value;

pub const MAX_SSE_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_STREAM_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamDelta {
    Content(String),
    Done,
}

pub fn parse_sse_line(line: &str) -> Result<Option<StreamDelta>, String> {
    let Some(data) = line.strip_prefix("data:") else {
        return Ok(None);
    };
    let data = data.trim();
    if data.is_empty() {
        return Ok(None);
    }
    if data == "[DONE]" {
        return Ok(Some(StreamDelta::Done));
    }
    let v: Value = serde_json::from_str(data).map_err(|e| format!("SSE JSON 解析失败: {e}"))?;
    let content = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("delta"))
        .and_then(|d| d.get("content"))
        .and_then(Value::as_str);
    Ok(content.map(|s| StreamDelta::Content(s.to_string())))
}

pub fn parse_ollama_ndjson_line(line: &str) -> Result<Option<StreamDelta>, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let v: Value =
        serde_json::from_str(trimmed).map_err(|e| format!("Ollama JSON 解析失败: {e}"))?;
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        return Err(format!("Ollama 返回错误: {err}"));
    }
    let done = v.get("done").and_then(Value::as_bool).unwrap_or(false);
    let content = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if !content.is_empty() {
        return Ok(Some(StreamDelta::Content(content.to_string())));
    }
    if done {
        return Ok(Some(StreamDelta::Done));
    }
    Ok(None)
}

pub struct LineStreamDecoder {
    buffer: Vec<u8>,
    frame_limit: usize,
    output_limit: usize,
    output_bytes: usize,
    saw_done: bool,
    ndjson_mode: bool,
}

impl LineStreamDecoder {
    pub fn new_sse() -> Self {
        Self {
            buffer: Vec::new(),
            frame_limit: MAX_SSE_FRAME_BYTES,
            output_limit: MAX_STREAM_OUTPUT_BYTES,
            output_bytes: 0,
            saw_done: false,
            ndjson_mode: false,
        }
    }

    pub fn new_ollama_ndjson() -> Self {
        Self {
            buffer: Vec::new(),
            frame_limit: MAX_SSE_FRAME_BYTES,
            output_limit: MAX_STREAM_OUTPUT_BYTES,
            output_bytes: 0,
            saw_done: false,
            ndjson_mode: true,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<StreamDelta>, String> {
        let mut deltas = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                if let Some(delta) = self.take_line()? {
                    let is_done = delta == StreamDelta::Done;
                    deltas.push(delta);
                    if is_done {
                        break;
                    }
                }
                continue;
            }
            if self.buffer.len() >= self.frame_limit {
                return Err(format!("流式单帧超出 {} 字节上限", self.frame_limit));
            }
            self.buffer.push(byte);
        }
        Ok(deltas)
    }

    pub fn finish(mut self) -> Result<Vec<StreamDelta>, String> {
        let mut deltas = Vec::new();
        if !self.buffer.is_empty() {
            if let Some(delta) = self.take_line()? {
                deltas.push(delta);
            }
        }
        if !self.saw_done && self.output_bytes == 0 {
            return Err("AI 流提前结束且未返回有效内容".to_string());
        }
        Ok(deltas)
    }

    fn take_line(&mut self) -> Result<Option<StreamDelta>, String> {
        let line_bytes = std::mem::take(&mut self.buffer);
        let line = std::str::from_utf8(&line_bytes)
            .map_err(|e| format!("流式响应包含非法 UTF-8: {e}"))?
            .trim();
        let parsed = if self.ndjson_mode {
            parse_ollama_ndjson_line(line)?
        } else {
            parse_sse_line(line)?
        };
        let Some(delta) = parsed else {
            return Ok(None);
        };
        match &delta {
            StreamDelta::Content(c) => {
                self.output_bytes = self
                    .output_bytes
                    .checked_add(c.len())
                    .ok_or_else(|| "流式输出大小溢出".to_string())?;
                if self.output_bytes > self.output_limit {
                    return Err(format!("流式输出超出 {} 字节上限", self.output_limit));
                }
            }
            StreamDelta::Done => {
                self.saw_done = true;
            }
        }
        Ok(Some(delta))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_sse_and_ollama_ndjson_streams() {
        let mut sse = LineStreamDecoder::new_sse();
        let out = sse
            .push(
                "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\ndata: [DONE]\n"
                    .as_bytes(),
            )
            .unwrap();
        assert_eq!(
            out,
            vec![StreamDelta::Content("你好".to_string()), StreamDelta::Done]
        );

        let mut ndjson = LineStreamDecoder::new_ollama_ndjson();
        let out2 = ndjson
            .push("{\"message\":{\"content\":\"世界\"},\"done\":false}\n{\"message\":{\"content\":\"\"},\"done\":true}\n".as_bytes())
            .unwrap();
        assert_eq!(
            out2,
            vec![StreamDelta::Content("世界".to_string()), StreamDelta::Done]
        );
    }
}
