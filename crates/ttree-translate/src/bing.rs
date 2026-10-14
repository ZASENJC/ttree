//! 必应翻译 —— 带有内存级 `BingToken` 缓存（默认 45 分钟复用，失效自动刷新）。
//!
//! 关键提速：
//! 旧版每次翻译都先请求并下载数 MB 的 `https://www.bing.com/translator` 网页解析 Token，
//! 导致必应翻译耗时翻倍且分块时重复抓网页。新版将 Token 缓存在内存中，后续请求全部单跳直达。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::{
    read_error_body, read_limited_response, shared_client, MAX_BING_PAGE_BYTES,
    MAX_TRANSLATION_RESPONSE_BYTES,
};

const TRANSLATOR_URL: &str = "https://www.bing.com/translator";
const TRANSLATE_URL: &str = "https://www.bing.com/ttranslatev3";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const TOKEN_TTL: Duration = Duration::from_secs(45 * 60);
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BingToken {
    pub ig: String,
    pub iid: String,
    pub key: String,
    pub token: String,
}

#[derive(Clone)]
struct CachedToken {
    token: BingToken,
    expires_at: Instant,
}

static TOKEN_CACHE: OnceLock<Mutex<Option<CachedToken>>> = OnceLock::new();

fn get_cached_token(now: Instant) -> Option<BingToken> {
    let cache = TOKEN_CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().ok()?;
    if let Some(entry) = guard.as_ref() {
        if now < entry.expires_at {
            return Some(entry.token.clone());
        }
    }
    *guard = None;
    None
}

fn store_cached_token(token: BingToken, now: Instant) {
    let cache = TOKEN_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedToken {
            token,
            expires_at: now + TOKEN_TTL,
        });
    }
}

fn invalidate_cached_token() {
    if let Some(cache) = TOKEN_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = None;
        }
    }
}

fn map_lang(code: &str) -> &str {
    match code {
        "auto" => "auto-detect",
        "zh-CN" => "zh-Hans",
        "zh-TW" => "zh-Hant",
        other => other,
    }
}

pub async fn translate(text: &str, source: &str, target: &str) -> Result<String, String> {
    let client = shared_client(REQUEST_TIMEOUT, true);

    let (token, from_cache) = match get_cached_token(Instant::now()) {
        Some(t) => (t, true),
        None => {
            let fresh = fetch_token(&client).await?;
            store_cached_token(fresh.clone(), Instant::now());
            (fresh, false)
        }
    };

    match translate_with_token(&client, &token, text, source, target).await {
        Ok(out) => Ok(out),
        Err(_first_err) if from_cache => {
            // 缓存令牌可能提前失效，立即淘汰并刷新重试一次
            invalidate_cached_token();
            let fresh = fetch_token(&client).await?;
            store_cached_token(fresh.clone(), Instant::now());
            translate_with_token(&client, &fresh, text, source, target).await
        }
        Err(err) => Err(err),
    }
}

async fn translate_with_token(
    client: &reqwest::Client,
    token: &BingToken,
    text: &str,
    source: &str,
    target: &str,
) -> Result<String, String> {
    let url = format!(
        "{TRANSLATE_URL}?isVertical=1&IG={ig}&IID={iid}",
        ig = urlencoding::encode(&token.ig),
        iid = urlencoding::encode(&token.iid),
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
        let status = resp.status();
        let body = read_error_body(resp).await;
        return Err(format!("必应翻译返回状态 {status}: {body}"));
    }

    let body = read_limited_response(resp, MAX_TRANSLATION_RESPONSE_BYTES, "必应翻译响应").await?;
    let body: Value =
        serde_json::from_slice(&body).map_err(|e| format!("必应翻译响应解析失败: {e}"))?;

    parse_response(&body)
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| "必应翻译未返回译文".to_string())
}

async fn fetch_token(client: &reqwest::Client) -> Result<BingToken, String> {
    let response = client
        .get(TRANSLATOR_URL)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("必应翻译页请求失败: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = read_error_body(response).await;
        return Err(format!("必应翻译页返回状态 {status}: {body}"));
    }
    let html = read_limited_response(response, MAX_BING_PAGE_BYTES, "必应翻译页").await?;
    let html = std::str::from_utf8(&html).map_err(|e| format!("必应翻译页不是 UTF-8: {e}"))?;

    parse_token(html).ok_or_else(|| "必应翻译令牌解析失败（端点可能已变更）".to_string())
}

pub fn parse_token(html: &str) -> Option<BingToken> {
    let ig = extract_between(html, "IG:\"", "\"")?;
    let iid = format!(
        "translator.{}",
        extract_between(html, "data-iid=\"translator.", "\"")?
    );

    let assignment = html
        .split_once("params_AbusePreventionHelper")?
        .1
        .trim_start();
    let assignment = assignment.strip_prefix('=')?.trim_start();
    let helper = assignment.strip_prefix('[')?.split_once(']')?.0;
    let parts: Vec<Value> = serde_json::from_str(&format!("[{helper}]")).ok()?;
    let key = parts.first()?.as_u64()?.to_string();
    let token = parts.get(1)?.as_str()?.to_string();

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

fn extract_between(haystack: &str, start: &str, end: &str) -> Option<String> {
    let begin = haystack.find(start)? + start.len();
    let rest = &haystack[begin..];
    let stop = rest.find(end)?;
    Some(rest[..stop].to_string())
}

pub fn parse_response(body: &Value) -> Option<String> {
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

    #[test]
    fn parses_bing_html_token_and_caches_with_ttl() {
        let html = r#"IG:"FF00"; <div data-iid="translator.5028"></div>
            params_AbusePreventionHelper = [167060,"abcdef",3600000];"#;
        let tok = parse_token(html).unwrap();
        assert_eq!(tok.ig, "FF00");
        assert_eq!(tok.iid, "translator.5028");
        assert_eq!(tok.token, "abcdef");

        let now = Instant::now();
        store_cached_token(tok.clone(), now);
        assert_eq!(get_cached_token(now + Duration::from_secs(60)), Some(tok));
        assert_eq!(
            get_cached_token(now + TOKEN_TTL + Duration::from_secs(1)),
            None
        );
    }
}
