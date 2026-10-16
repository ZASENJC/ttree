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

const DEFAULT_ORIGIN: &str = "https://www.bing.com";
const TRANSLATOR_URL: &str = "https://www.bing.com/translator";
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
    pub origin: String,
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

fn is_allowed_bing_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "bing.com" || host.ends_with(".bing.com")
}

fn origin_from_url(url: &reqwest::Url) -> Option<String> {
    if url.scheme() != "https" && url.scheme() != "http" {
        return None;
    }
    let host = url.host_str()?;
    let mut origin = format!("{}://{}", url.scheme(), host);
    if let Some(port) = url.port() {
        origin.push_str(&format!(":{port}"));
    }
    Some(origin)
}

pub async fn translate(text: &str, source: &str, target: &str) -> Result<String, String> {
    let client = shared_client(REQUEST_TIMEOUT, true);

    let (token, from_cache) = match get_cached_token(Instant::now()) {
        Some(t) => (t, true),
        None => {
            let fresh = fetch_token(&client, TRANSLATOR_URL).await?;
            store_cached_token(fresh.clone(), Instant::now());
            (fresh, false)
        }
    };

    match translate_with_token(&client, &token, text, source, target).await {
        Ok(out) => Ok(out),
        Err(_first_err) if from_cache => {
            // 缓存令牌可能提前失效，立即淘汰并刷新重试一次
            invalidate_cached_token();
            let fresh = fetch_token(&client, TRANSLATOR_URL).await?;
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
    let origin = if token.origin.is_empty() {
        DEFAULT_ORIGIN
    } else {
        token.origin.as_str()
    };
    let referer = format!("{origin}/translator");
    let mut url = format!(
        "{origin}/ttranslatev3?isVertical=1&IG={ig}&IID={iid}",
        ig = urlencoding::encode(&token.ig),
        iid = urlencoding::encode(&token.iid),
    );
    let form = [
        ("fromLang", map_lang(source)),
        ("to", map_lang(target)),
        ("text", text),
        ("token", &token.token),
        ("key", &token.key),
    ];

    let mut resp = None;
    for _ in 0..=3 {
        let current = client
            .post(&url)
            .header("User-Agent", USER_AGENT)
            .header("Referer", &referer)
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("必应翻译请求失败: {e}"))?;

        if current.status().is_redirection() {
            if let Some(next_url) = resolve_redirect_url(&url, current.headers()) {
                url = next_url;
                continue;
            }
        }
        resp = Some(current);
        break;
    }
    let resp = resp.ok_or_else(|| "必应翻译重定向次数过多".to_string())?;

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

fn resolve_redirect_url(current_url: &str, headers: &reqwest::header::HeaderMap) -> Option<String> {
    let loc = headers.get(reqwest::header::LOCATION)?.to_str().ok()?;
    let base = reqwest::Url::parse(current_url).ok()?;
    let target = base.join(loc).ok()?;
    let host = target.host_str()?;
    let same_loopback = base.host_str() == Some(host) && (host == "127.0.0.1" || host == "localhost");
    if (target.scheme() == "https" && is_allowed_bing_host(host)) || same_loopback {
        Some(target.into())
    } else {
        None
    }
}

async fn fetch_token(client: &reqwest::Client, translator_url: &str) -> Result<BingToken, String> {
    let mut url = translator_url.to_string();
    let mut response = None;
    for _ in 0..=3 {
        let current = client
            .get(&url)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .map_err(|e| format!("必应翻译页请求失败: {e}"))?;
        if current.status().is_redirection() {
            if let Some(next_url) = resolve_redirect_url(&url, current.headers()) {
                url = next_url;
                continue;
            }
        }
        response = Some(current);
        break;
    }
    let response = response.ok_or_else(|| "必应翻译页重定向次数过多".to_string())?;
    if !response.status().is_success() {
        let status = response.status();
        let body = read_error_body(response).await;
        return Err(format!("必应翻译页返回状态 {status}: {body}"));
    }
    let final_origin = origin_from_url(response.url()).unwrap_or_else(|| DEFAULT_ORIGIN.to_string());
    let html = read_limited_response(response, MAX_BING_PAGE_BYTES, "必应翻译页").await?;
    let html = std::str::from_utf8(&html).map_err(|e| format!("必应翻译页不是 UTF-8: {e}"))?;

    let mut token =
        parse_token(html).ok_or_else(|| "必应翻译令牌解析失败（端点可能已变更）".to_string())?;
    token.origin = final_origin;
    Ok(token)
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
        origin: DEFAULT_ORIGIN.to_string(),
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

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

    #[tokio::test]
    async fn follows_302_redirects_for_token_and_translation_post() {
        let redirect_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_addr = redirect_listener.local_addr().unwrap();
        let target_addr = target_listener.local_addr().unwrap();

        let redirect_srv = tokio::spawn(async move {
            let (mut socket, _) = redirect_listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await.unwrap();
            let resp = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/translator-cn\r\nSet-Cookie: SNRHOP=1; Path=/\r\nContent-Length: 0\r\n\r\n",
                target_addr.port()
            );
            socket.write_all(resp.as_bytes()).await.unwrap();
        });

        let target_srv = tokio::spawn(async move {
            for step in 0..3 {
                let (mut socket, _) = target_listener.accept().await.unwrap();
                let mut buf = vec![0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]);
                match step {
                    0 => {
                        assert!(req.starts_with("GET /translator-cn "), "{req}");
                        let body = r#"IG:"FF00"; <div data-iid="translator.5028"></div> params_AbusePreventionHelper = [167060,"abcdef",3600000];"#;
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        socket.write_all(resp.as_bytes()).await.unwrap();
                    }
                    1 => {
                        assert!(req.starts_with("POST /ttranslatev3?isVertical=1&IG=FF00&IID=translator.5028 "), "{req}");
                        let resp = format!(
                            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/ttranslatev3-cn?isVertical=1&IG=FF00&IID=translator.5028\r\nContent-Length: 0\r\n\r\n",
                            target_addr.port()
                        );
                        socket.write_all(resp.as_bytes()).await.unwrap();
                    }
                    _ => {
                        assert!(req.starts_with("POST /ttranslatev3-cn?isVertical=1&IG=FF00&IID=translator.5028 "), "{req}");
                        assert!(req.contains("text=hello"), "{req}");
                        let body = r#"[{"translations":[{"text":"你好","to":"zh-Hans"}]}]"#;
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        socket.write_all(resp.as_bytes()).await.unwrap();
                    }
                }
            }
        });

        let client = shared_client(Duration::from_secs(5), true);
        let initial_url = format!("http://127.0.0.1:{}/translator", redirect_addr.port());
        let token = fetch_token(&client, &initial_url).await.unwrap();
        assert_eq!(token.origin, format!("http://127.0.0.1:{}", target_addr.port()));

        let translated = translate_with_token(&client, &token, "hello", "en", "zh-CN")
            .await
            .unwrap();
        assert_eq!(translated, "你好");

        redirect_srv.await.unwrap();
        target_srv.await.unwrap();
    }
}
