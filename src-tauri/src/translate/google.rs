//! 谷歌翻译 —— 免费网页端接口（translate.googleapis.com）。
//!
//! 注意：这是非官方端点，可能被限流。设置项预留官方 Cloud API key 入口。

use std::time::Duration;

use serde_json::Value;

use super::shared_client;

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// 调用谷歌免费端接口，返回完整译文。
pub async fn translate(
    text: &str,
    source: &str,
    target: &str,
) -> Result<String, String> {
    let url = format!(
        "{ENDPOINT}?client=gtx&sl={sl}&tl={tl}&dt=t&q={q}",
        sl = urlencoding::encode(source),
        tl = urlencoding::encode(target),
        q = urlencoding::encode(text),
    );

    let resp = shared_client(REQUEST_TIMEOUT, false)
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .map_err(|e| format!("谷歌翻译请求失败: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("谷歌翻译返回状态 {}", resp.status()));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("谷歌翻译响应解析失败: {e}"))?;

    Ok(parse_response(&body))
}

/// 解析谷歌响应：结构为 `[[[译文, 原文, ...], ...], ...]`。
/// 拼接第一段数组中所有片段的译文。
fn parse_response(body: &Value) -> String {
    let mut out = String::new();
    if let Some(segments) = body.get(0).and_then(|v| v.as_array()) {
        for seg in segments {
            if let Some(t) = seg.get(0).and_then(|v| v.as_str()) {
                out.push_str(t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_single_segment() {
        let body = json!([[["你好", "hello", null, null, 10]], null, "en"]);
        assert_eq!(parse_response(&body), "你好");
    }

    #[test]
    fn concatenates_multiple_segments() {
        let body = json!([
            [["你好", "hello"], ["世界", "world"]],
            null,
            "en"
        ]);
        assert_eq!(parse_response(&body), "你好世界");
    }

    #[test]
    fn returns_empty_on_malformed() {
        let body = json!({ "error": "bad" });
        assert_eq!(parse_response(&body), "");
    }
}
