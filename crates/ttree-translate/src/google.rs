//! 谷歌翻译 —— 改用 POST 表单请求避免长文本 URL 膨胀，并配合 1.8s 快速连接超时。

use serde_json::Value;
use std::time::Duration;

use crate::{
    read_error_body, read_limited_response, shared_client, MAX_TRANSLATION_RESPONSE_BYTES,
};

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

pub async fn translate(text: &str, source: &str, target: &str) -> Result<String, String> {
    let url = format!(
        "{ENDPOINT}?client=gtx&sl={sl}&tl={tl}&dt=t",
        sl = urlencoding::encode(source),
        tl = urlencoding::encode(target),
    );

    let resp = shared_client(REQUEST_TIMEOUT, false)
        .post(&url)
        .header("User-Agent", "Mozilla/5.0")
        .form(&[("q", text)])
        .send()
        .await
        .map_err(|e| format!("谷歌翻译请求失败: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = read_error_body(resp).await;
        return Err(format!("谷歌翻译返回状态 {status}: {body}"));
    }

    let body = read_limited_response(resp, MAX_TRANSLATION_RESPONSE_BYTES, "谷歌翻译响应").await?;
    let body: Value =
        serde_json::from_slice(&body).map_err(|e| format!("谷歌翻译响应解析失败: {e}"))?;

    let out = parse_response(&body);
    if out.trim().is_empty() {
        return Err("谷歌翻译未返回有效译文".to_string());
    }
    Ok(out)
}

pub fn parse_response(body: &Value) -> String {
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
    fn parses_multi_segment_google_response() {
        let body = json!([[["你好", "hello"], ["世界", "world"]], null, "en"]);
        assert_eq!(parse_response(&body), "你好世界");
    }
}
