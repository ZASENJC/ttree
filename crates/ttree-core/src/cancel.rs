//! 轻量任务取消令牌（`CancelToken`）与单槽抢占器（`CancelSlot`）。
//!
//! 解决痛点：当用户快速修改输入或连续触发划词/OCR 时，立即取消正在进行的旧请求
//! （尤其是耗时较长的 AI 流式生成），绝不让上一条慢请求排队阻塞新翻译。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    notify: Notify,
}

/// 可克隆的异步取消令牌。
#[derive(Clone, Default)]
pub struct CancelToken {
    request_id: u64,
    inner: Arc<Inner>,
}

impl CancelToken {
    pub fn new(request_id: u64) -> Self {
        Self {
            request_id,
            inner: Arc::new(Inner::default()),
        }
    }

    /// 当前令牌对应的单调递增请求 ID。
    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    /// 触发取消并唤醒所有正在等待的异步任务。
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_waiters();
        }
    }

    /// 是否已被取消。
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// 异步等待直至令牌被取消（配合 `tokio::select!` 中断网络流）。
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.inner.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

/// 单槽抢占管理器：每次签发新 `CancelToken` 时自动取消上一个未完成的令牌。
#[derive(Default)]
pub struct CancelSlot {
    state: Mutex<(u64, Option<CancelToken>)>,
}

impl CancelSlot {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new((0, None)),
        }
    }

    /// 取消旧令牌并签发新一代 `CancelToken`。
    pub fn next(&self) -> CancelToken {
        let mut guard = self.state.lock().expect("CancelSlot lock poisoned");
        if let Some(prev) = guard.1.take() {
            prev.cancel();
        }
        guard.0 = guard.0.wrapping_add(1);
        let token = CancelToken::new(guard.0);
        guard.1 = Some(token.clone());
        token
    }

    /// 显式取消当前槽位中的活跃任务。
    pub fn cancel_current(&self) {
        let mut guard = self.state.lock().expect("CancelSlot lock poisoned");
        if let Some(prev) = guard.1.take() {
            prev.cancel();
        }
    }

    /// 当前最新请求 ID。
    pub fn current_id(&self) -> u64 {
        self.state.lock().map(|g| g.0).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn issuing_next_token_cancels_previous_token() {
        let slot = CancelSlot::new();
        let first = slot.next();
        assert_eq!(first.request_id(), 1);
        assert!(!first.is_cancelled());

        let second = slot.next();
        assert_eq!(second.request_id(), 2);
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());

        // cancelled() future on first resolves immediately
        first.cancelled().await;
    }
}
