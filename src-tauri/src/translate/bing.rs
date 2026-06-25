//! 必应翻译 —— 免费网页端接口（bing.com/ttranslatev3）。
//!
//! 注意：这是非官方端点。需要先抓取翻译页拿到一次性令牌（IG / IID / key / token），
//! 再带令牌 POST 翻译文本。可能随时失效或被限流。

use std::time::Duration;

use serde_json::Value;

use super::shared_client;

const TRANSLATOR_URL: &str = "https://www.bing.com/translator";
const TRANSLATE_URL: &str = "https://www.bing.com/ttranslatev3";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// 翻译页里抓取的一次性令牌。
struct BingToken {
    ig: String,
    iid: String,
    key: String,
    token: String,
}

/// 把内部语言代码映射成必应使用的代码。
fn map_lang(code: &str) -> &str {
    match code {
        "auto" => "auto-detect",
        "zh-CN" => "zh-Hans",
        "zh-TW" => "zh-Hant",
        other => other,
    }
}

/// 调用必应免费端接口，返回完整译文。
pub async fn translate(text: &str, source: &str, target: &str) -> Result<String, String> {
    let client = shared_client(REQUEST_TIMEOUT, true);

    let token = fetch_token(&client).await?;

    let url = format!(
        "{TRANSLATE_URL}?isVertical=1&IG={ig}&IID={iid}",
        ig = token.ig,
        iid = token.iid,
    );

    let resp = client
        .post(&url)
        .header("User-Agent", USER_AGENT)
        .header("Referer", TRANSLATOR_URL)
        .form(&[
            ("fromLang", map_lang(source)),
            ("to", map_lang(target)),
            ("text", text),
            ("token", &token.token),
            ("key", &token.key),
        ])
        .send()
        .await
        .map_err(|e| format!("必应翻译请求失败: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("必应翻译返回状态 {}", resp.status()));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("必应翻译响应解析失败: {e}"))?;

    parse_response(&body).ok_or_else(|| "必应翻译未返回译文".to_string())
}

/// 抓取翻译页并解析出一次性令牌。
async fn fetch_token(client: &reqwest::Client) -> Result<BingToken, String> {
    let html = client
        .get(TRANSLATOR_URL)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("必应翻译页请求失败: {e}"))?
        .text()
        .await
        .map_err(|e| format!("必应翻译页读取失败: {e}"))?;

    parse_token(&html).ok_or_else(|| "必应翻译令牌解析失败（端点可能已变更）".to_string())
}

/// 从翻译页 HTML 解析 IG / IID / key / token。
fn parse_token(html: &str) -> Option<BingToken> {
    let ig = extract_between(html, "IG:\"", "\"")?;
    let iid = extract_between(html, "data-iid=\"", "\"")?;

    // params_AbusePreventionHelper = [key,"token",time];
    let helper = extract_between(html, "params_AbusePreventionHelper = [", "]")?;
    let mut parts = helper.split(',');
    let key = parts.next()?.trim().to_string();
    let token = parts.next()?.trim().trim_matches('"').to_string();

    if key.is_empty() || token.is_empty() {
        return None;
    }

    Some(BingToken {
        ig,
        iid,
        key,
        token,
    })
}

/// 取 `start` 与其后第一个 `end` 之间的子串。
fn extract_between(haystack: &str, start: &str, end: &str) -> Option<String> {
    let begin = haystack.find(start)? + start.len();
    let rest = &haystack[begin..];
    let stop = rest.find(end)?;
    Some(rest[..stop].to_string())
}

/// 解析必应响应：结构为 `[{"translations":[{"text":"译文"}], ...}]`。
fn parse_response(body: &Value) -> Option<String> {
    body.get(0)?
        .get("translations")?
        .get(0)?
        .get("text")?
        .as_str()
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_internal_lang_codes() {
        assert_eq!(map_lang("auto"), "auto-detect");
        assert_eq!(map_lang("zh-CN"), "zh-Hans");
        assert_eq!(map_lang("zh-TW"), "zh-Hant");
        assert_eq!(map_lang("en"), "en");
    }

    #[test]
    fn extracts_substring_between_markers() {
        assert_eq!(
            extract_between("a IG:\"ABC123\" b", "IG:\"", "\""),
            Some("ABC123".to_string())
        );
        assert_eq!(extract_between("no marker", "IG:\"", "\""), None);
    }

    #[test]
    fn parses_token_fields() {
        let html = r#"<html>var IG:"FF00";<div data-iid="translator.5028"></div>
            <script>params_AbusePreventionHelper = [167060,"abcdef",3600000];</script></html>"#;
        let token = parse_token(html).expect("token");
        assert_eq!(token.ig, "FF00");
        assert_eq!(token.iid, "translator.5028");
        assert_eq!(token.key, "167060");
        assert_eq!(token.token, "abcdef");
    }

    #[test]
    fn returns_none_when_token_missing() {
        assert!(parse_token("<html>nothing here</html>").is_none());
    }

    #[test]
    fn parses_translation_text() {
        let body = json!([{
            "detectedLanguage": { "language": "en", "score": 1.0 },
            "translations": [{ "text": "你好", "to": "zh-Hans" }]
        }]);
        assert_eq!(parse_response(&body), Some("你好".to_string()));
    }

    #[test]
    fn returns_none_on_malformed_response() {
        assert_eq!(parse_response(&json!({ "error": "bad" })), None);
    }
}
