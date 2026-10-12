//! 免费翻译自动分流：健康服务轮询，失败切换，故障服务指数退避。

use std::future::Future;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{bing, google};

const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);
const BASE_COOLDOWN: Duration = Duration::from_secs(30);
const MAX_COOLDOWN: Duration = Duration::from_secs(300);
// Bing 网页端限制较小；同时避免 Google GET 的 URL 随长文本膨胀。
const CHUNK_UTF16_LIMIT: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provider {
    Google,
    Bing,
}

impl Provider {
    fn index(self) -> usize {
        match self {
            Self::Google => 0,
            Self::Bing => 1,
        }
    }
}

#[derive(Default)]
struct Health {
    failures: u32,
    retry_after: Option<Instant>,
}

#[derive(Default)]
struct Pool {
    health: [Health; 2],
    next: usize,
}

impl Pool {
    fn candidates(&mut self, now: Instant) -> Vec<Provider> {
        let providers = [Provider::Google, Provider::Bing];
        let start = self.next;
        self.next = (self.next + 1) % providers.len();
        (0..providers.len())
            .map(|offset| providers[(start + offset) % providers.len()])
            .filter(|provider| self.available(*provider, now))
            .collect()
    }

    fn available(&self, provider: Provider, now: Instant) -> bool {
        self.health[provider.index()]
            .retry_after
            .is_none_or(|retry_after| now >= retry_after)
    }

    fn succeeded(&mut self, provider: Provider) {
        self.health[provider.index()] = Health::default();
    }

    fn failed(&mut self, provider: Provider, now: Instant) {
        let health = &mut self.health[provider.index()];
        health.failures = health.failures.saturating_add(1);
        let multiplier = 1 << health.failures.saturating_sub(1).min(4);
        let cooldown = (BASE_COOLDOWN * multiplier).min(MAX_COOLDOWN);
        health.retry_after = Some(now + cooldown);
    }
}

pub async fn translate(text: &str, source: &str, target: &str) -> Result<String, String> {
    static POOL: OnceLock<Mutex<Pool>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(Pool::default()));
    let chunks = split_text(text);
    let timeout = ATTEMPT_TIMEOUT
        .saturating_mul(chunks.len() as u32)
        .min(Duration::from_secs(120));
    let chunks = &chunks;
    translate_with(pool, timeout, |provider| async move {
        let mut output = String::new();
        for &chunk in chunks {
            let content = chunk.trim();
            if content.is_empty() {
                output.push_str(chunk);
                continue;
            }
            let translated = match provider {
                Provider::Google => google::translate(content, source, target).await?,
                Provider::Bing => bing::translate(content, source, target).await?,
            };
            // 保留分块边界上的空白和换行，避免多段译文粘在一起。
            output.push_str(&chunk[..chunk.len() - chunk.trim_start().len()]);
            output.push_str(translated.trim());
            output.push_str(&chunk[chunk.trim_end().len()..]);
        }
        Ok(output)
    })
    .await
}

async fn translate_with<F, Fut>(
    pool: &Mutex<Pool>,
    timeout: Duration,
    mut call: F,
) -> Result<String, String>
where
    F: FnMut(Provider) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let candidates = pool
        .lock()
        .map_err(|_| "翻译服务状态不可用".to_string())?
        .candidates(Instant::now());

    for provider in candidates {
        // 另一个并发请求可能已将此服务熔断；不在锁内等待网络。
        if !pool
            .lock()
            .map_err(|_| "翻译服务状态不可用".to_string())?
            .available(provider, Instant::now())
        {
            continue;
        }
        let result = tokio::time::timeout(timeout, call(provider))
            .await
            .unwrap_or_else(|_| Err("翻译服务请求超时".to_string()))
            .and_then(|text| {
                if text.trim().is_empty() {
                    Err("翻译服务未返回译文".to_string())
                } else {
                    Ok(text)
                }
            });
        let mut state = pool
            .lock()
            .map_err(|_| "翻译服务状态不可用".to_string())?;
        match result {
            Ok(text) => {
                state.succeeded(provider);
                return Ok(text);
            }
            Err(error) => {
                state.failed(provider, Instant::now());
                eprintln!("免费翻译服务 {provider:?} 失败，暂时停用: {error}");
            }
        }
    }
    Err("翻译服务暂时不可用，已自动尝试其他服务，请稍后重试。".to_string())
}

fn split_text(mut text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    while !text.is_empty() {
        let mut units = 0;
        let mut end = text.len();
        let mut boundary = None;
        for (index, ch) in text.char_indices() {
            units += ch.len_utf16();
            if units > CHUNK_UTF16_LIMIT {
                end = boundary.unwrap_or(index);
                break;
            }
            if units >= CHUNK_UTF16_LIMIT / 2
                && (ch.is_whitespace() || matches!(ch, '.' | '!' | '?' | '。' | '！' | '？'))
            {
                boundary = Some(index + ch.len_utf8());
            }
        }
        let (chunk, remaining) = text.split_at(end);
        chunks.push(chunk);
        text = remaining;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn distributes_successful_requests_between_providers() {
        let pool = Mutex::new(Pool::default());
        let mut called = Vec::new();
        for _ in 0..4 {
            translate_with(&pool, ATTEMPT_TIMEOUT, |provider| {
                called.push(provider);
                async { Ok("译文".to_string()) }
            })
            .await
            .unwrap();
        }
        assert_eq!(
            called,
            [Provider::Google, Provider::Bing, Provider::Google, Provider::Bing]
        );
    }

    #[tokio::test]
    async fn fails_over_and_skips_a_service_during_cooldown() {
        let pool = Mutex::new(Pool::default());
        let mut called = Vec::new();
        for _ in 0..3 {
            let result = translate_with(&pool, ATTEMPT_TIMEOUT, |provider| {
                called.push(provider);
                async move {
                    match provider {
                        Provider::Google => Err("429 Too Many Requests".to_string()),
                        Provider::Bing => Ok("备用译文".to_string()),
                    }
                }
            })
            .await;
            assert_eq!(result.unwrap(), "备用译文");
        }
        assert_eq!(
            called,
            [Provider::Google, Provider::Bing, Provider::Bing, Provider::Bing]
        );
    }

    #[tokio::test]
    async fn empty_results_trigger_failover() {
        let pool = Mutex::new(Pool::default());
        let result = translate_with(&pool, ATTEMPT_TIMEOUT, |provider| async move {
            Ok(if provider == Provider::Google {
                "   "
            } else {
                "备用译文"
            }
            .to_string())
        })
        .await;
        assert_eq!(result.unwrap(), "备用译文");
    }

    #[tokio::test]
    async fn timeout_triggers_failover() {
        let pool = Mutex::new(Pool::default());
        let result = translate_with(&pool, Duration::from_millis(10), |provider| async move {
            if provider == Provider::Google {
                std::future::pending::<()>().await;
            }
            Ok("备用译文".to_string())
        })
        .await;
        assert_eq!(result.unwrap(), "备用译文");
    }

    #[tokio::test]
    async fn stops_requesting_when_all_providers_are_cooling_down() {
        let pool = Mutex::new(Pool::default());
        let mut count = 0;
        for _ in 0..2 {
            assert!(translate_with(&pool, ATTEMPT_TIMEOUT, |_| {
                count += 1;
                async { Err("offline".to_string()) }
            })
            .await
            .is_err());
        }
        assert_eq!(count, 2);
    }

    #[test]
    fn recovers_after_cooldown_and_resets_backoff_on_success() {
        let mut pool = Pool::default();
        let now = Instant::now();
        pool.failed(Provider::Google, now);
        assert!(!pool.available(Provider::Google, now));
        assert!(pool.available(Provider::Google, now + BASE_COOLDOWN));
        pool.failed(Provider::Google, now + BASE_COOLDOWN);
        assert!(!pool.available(Provider::Google, now + BASE_COOLDOWN * 2));
        pool.succeeded(Provider::Google);
        assert!(pool.available(Provider::Google, now));
        assert_eq!(pool.health[0].failures, 0);
    }

    #[test]
    fn chunks_unicode_within_service_limits_without_losing_text() {
        let text = format!("{}\n{}", "截图😀中文 ".repeat(500), "long text ".repeat(300));
        let chunks = split_text(&text);
        assert!(chunks.len() > 1);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.encode_utf16().count() <= CHUNK_UTF16_LIMIT));
        assert_eq!(chunks.concat(), text);
        assert_eq!(split_text("short text"), ["short text"]);
    }
}
