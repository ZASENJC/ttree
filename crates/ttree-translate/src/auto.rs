//! 免费网页翻译智能竞速路由（Hedged Request Racing + 延迟加权主备优选 + 并发分块）。
//!
//! 关键提速：
//! 1. 根据最近一次响应延迟选出最快的服务作为 Primary，另一方作为 Secondary；
//! 2. Primary 发出后若在 `HEDGE_DELAY (550ms)` 内未返回，立即并发启动 Secondary 竞速（谁先成功用谁）；
//! 3. 长文本分块后支持并发请求，而非旧版的串行逐块阻塞等待。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::future::join_all;

use crate::{bing, google};

const HEDGE_DELAY: Duration = Duration::from_millis(550);
const BASE_COOLDOWN: Duration = Duration::from_secs(45);
const MAX_COOLDOWN: Duration = Duration::from_secs(300);
const CHUNK_UTF16_LIMIT: usize = 1_200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
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

    fn other(self) -> Self {
        match self {
            Self::Google => Self::Bing,
            Self::Bing => Self::Google,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Google => "谷歌",
            Self::Bing => "必应",
        }
    }
}

fn combine_provider_errors(
    primary: Provider,
    primary_err: String,
    secondary: Provider,
    secondary_err: String,
) -> String {
    format!(
        "{}: {primary_err} ｜ {}: {secondary_err}",
        primary.label(),
        secondary.label()
    )
}

#[derive(Clone, Copy)]
struct ProviderStats {
    ewma_ms: u64,
    failures: u32,
    retry_after: Option<Instant>,
}

impl Default for ProviderStats {
    fn default() -> Self {
        Self {
            ewma_ms: 400,
            failures: 0,
            retry_after: None,
        }
    }
}

#[derive(Default)]
struct RouterState {
    stats: [ProviderStats; 2],
}

impl RouterState {
    fn is_available(&self, p: Provider, now: Instant) -> bool {
        self.stats[p.index()].retry_after.is_none_or(|t| now >= t)
    }

    /// 优先返回当前可用且历史延迟更低的服务。
    fn ordered_candidates(&self, now: Instant) -> Vec<Provider> {
        let mut list: Vec<Provider> = [Provider::Google, Provider::Bing]
            .into_iter()
            .filter(|&p| self.is_available(p, now))
            .collect();
        if list.is_empty() {
            // 若两个都在冷却期，允许全部重探
            list.push(Provider::Bing);
            list.push(Provider::Google);
        } else if list.len() == 2 {
            list.sort_by_key(|&p| self.stats[p.index()].ewma_ms);
        }
        list
    }

    fn record_success(&mut self, p: Provider, elapsed: Duration) {
        let st = &mut self.stats[p.index()];
        st.failures = 0;
        st.retry_after = None;
        let sample = elapsed.as_millis() as u64;
        st.ewma_ms = (st.ewma_ms * 3 + sample) / 4;
    }

    fn record_failure(&mut self, p: Provider, now: Instant) {
        let st = &mut self.stats[p.index()];
        st.failures = st.failures.saturating_add(1);
        let mult = 1 << st.failures.saturating_sub(1).min(3);
        let cooldown = (BASE_COOLDOWN * mult).min(MAX_COOLDOWN);
        st.retry_after = Some(now + cooldown);
        st.ewma_ms = st.ewma_ms.saturating_add(1500);
    }
}

static ROUTER: OnceLock<Mutex<RouterState>> = OnceLock::new();

fn router() -> &'static Mutex<RouterState> {
    ROUTER.get_or_init(|| Mutex::new(RouterState::default()))
}

pub async fn translate(text: &str, source: &str, target: &str) -> Result<String, String> {
    let chunks = split_text(text);
    if chunks.len() == 1 {
        return translate_chunk_hedged(chunks[0].trim(), source, target).await;
    }

    // 多段长文本并发分块翻译，保留段间空白与换行
    let futs = chunks.iter().map(|&chunk| async move {
        let content = chunk.trim();
        if content.is_empty() {
            return Ok::<String, String>(chunk.to_string());
        }
        let translated = translate_chunk_hedged(content, source, target).await?;
        let mut piece = String::new();
        piece.push_str(&chunk[..chunk.len() - chunk.trim_start().len()]);
        piece.push_str(translated.trim());
        piece.push_str(&chunk[chunk.trim_end().len()..]);
        Ok(piece)
    });

    let results = join_all(futs).await;
    let mut output = String::new();
    for res in results {
        output.push_str(&res?);
    }
    Ok(output)
}

async fn translate_chunk_hedged(text: &str, source: &str, target: &str) -> Result<String, String> {
    let candidates = router()
        .lock()
        .map(|g| g.ordered_candidates(Instant::now()))
        .unwrap_or_else(|_| vec![Provider::Bing, Provider::Google]);

    let primary = candidates[0];
    let secondary = candidates
        .get(1)
        .copied()
        .unwrap_or_else(|| primary.other());

    let primary_fut = call_provider(primary, text, source, target);
    tokio::pin!(primary_fut);

    // 先给主服务 HEDGE_DELAY (550ms) 单独响应窗口
    tokio::select! {
        res = &mut primary_fut => {
            match res {
                Ok(out) => return Ok(out),
                Err(primary_err) => {
                    // 主服务立即报错，直接等待备用服务
                    return call_provider(secondary, text, source, target)
                        .await
                        .map_err(|secondary_err| {
                            combine_provider_errors(primary, primary_err, secondary, secondary_err)
                        });
                }
            }
        }
        _ = tokio::time::sleep(HEDGE_DELAY) => {}
    }

    // 主服务超过 550ms 未返回：并发拉起备用服务竞速！
    let secondary_fut = call_provider(secondary, text, source, target);
    tokio::pin!(secondary_fut);

    tokio::select! {
        res1 = &mut primary_fut => {
            match res1 {
                Ok(out) => Ok(out),
                Err(primary_err) => secondary_fut
                    .await
                    .map_err(|secondary_err| {
                        combine_provider_errors(primary, primary_err, secondary, secondary_err)
                    }),
            }
        }
        res2 = &mut secondary_fut => {
            match res2 {
                Ok(out) => Ok(out),
                Err(secondary_err) => primary_fut
                    .await
                    .map_err(|primary_err| {
                        combine_provider_errors(primary, primary_err, secondary, secondary_err)
                    }),
            }
        }
    }
}

async fn call_provider(
    provider: Provider,
    text: &str,
    source: &str,
    target: &str,
) -> Result<String, String> {
    let start = Instant::now();
    let result = match provider {
        Provider::Google => google::translate(text, source, target).await,
        Provider::Bing => bing::translate(text, source, target).await,
    };
    if let Ok(mut guard) = router().lock() {
        match &result {
            Ok(_) => guard.record_success(provider, start.elapsed()),
            Err(_) => guard.record_failure(provider, Instant::now()),
        }
    }
    result
}

pub fn split_text(mut text: &str) -> Vec<&str> {
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

    #[test]
    fn router_prefers_lower_latency_healthy_provider() {
        let mut state = RouterState::default();
        let now = Instant::now();
        state.record_success(Provider::Bing, Duration::from_millis(120));
        state.record_success(Provider::Google, Duration::from_millis(900));
        assert_eq!(state.ordered_candidates(now)[0], Provider::Bing);

        state.record_failure(Provider::Bing, now);
        assert_eq!(state.ordered_candidates(now), vec![Provider::Google]);
    }

    #[test]
    fn splits_long_text_without_losing_characters() {
        let input = format!(
            "{}\n{}",
            "测试文本 ".repeat(400),
            "hello world. ".repeat(300)
        );
        let chunks = split_text(&input);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), input);
    }

    #[test]
    fn combines_primary_and_secondary_errors_when_both_fail() {
        let msg = combine_provider_errors(
            Provider::Bing,
            "必应翻译页返回状态 302 Found".into(),
            Provider::Google,
            "谷歌翻译返回状态 429 Too Many Requests".into(),
        );
        assert_eq!(
            msg,
            "必应: 必应翻译页返回状态 302 Found ｜ 谷歌: 谷歌翻译返回状态 429 Too Many Requests"
        );
    }
}
