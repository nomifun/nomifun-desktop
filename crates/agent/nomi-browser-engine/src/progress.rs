//! Progress / deadline / abort 核心：所有 CDP 调用与浏览器动作的取消地基。
//!
//! 镜像 Playwright `server/progress.ts` 的 **ProgressController**：单一 deadline +
//! 每个 await 与 abort 竞速。核心不变量——**page.close / frame.detach → 立即 abort**，
//! 对标 PW `LongStandingScope`（父作用域取消则其下所有进行中的操作立即被取消）。
//!
//! Rust 实现用 [`tokio_util::sync::CancellationToken`]：它原生支持父 token→子 token 的
//! 层级取消（取消父 → 所有子立即取消），天然建模「page 关闭 → 该 page 下所有 frame
//! 操作立即 abort」。每个受控 await 都与 (deadline, token.cancelled()) 三方竞速。
//!
//! 本模块**只**提供并发原语，不接真实的 page.close/frame.detach 事件源（那是传输/会话
//! 层的职责，它们届时会持有本作用域的 token 并在事件到达时调用 [`Progress::abort`] 或
//! 取消 token）。也**不**依赖 `BrowserError`：`ProgressError` 自成一体。

use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

/// abort 原因（可扩展）。`PageClosed` / `FrameDetached` 是规范来源，对应 PW
/// `LongStandingScope` 关闭父作用域的两个触发点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbortReason {
    /// page 被关闭：该 page 下所有进行中的操作立即取消。
    PageClosed,
    /// frame 被 detach：该 frame 下所有进行中的操作立即取消。
    FrameDetached,
    /// 主动/用户取消。
    Cancelled,
    /// 其它原因（自由文本）。
    Other(String),
}

/// 受控操作可能的失败：要么超时，要么被 abort。与 `BrowserError` 解耦。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProgressError {
    #[error("operation timed out")]
    Timeout,
    #[error("operation aborted: {0:?}")]
    Aborted(AbortReason),
}

/// 一次受控操作的作用域：持有 deadline 预算 + 一个（可为子）取消 token。
///
/// 把任意 future 经 [`Progress::race`] 与 (deadline, abort) 竞速。abort 通过取消
/// `CancellationToken` 实现，并记录 [`AbortReason`] 供 race 返回精确分类。
///
/// 层级：用 [`Progress::child`] 从父作用域派生子作用域，父被取消时子立即一起取消
/// （`CancellationToken::child_token` 语义）——这就是 `LongStandingScope` 不变量。
pub struct Progress {
    timeout: Duration,
    token: CancellationToken,
    /// 取消时记录的原因。`abort()` 写入；token 自身被外部（含父）取消时为 `None`，
    /// 此时 race 用 `AbortReason::Cancelled` 兜底。
    reason: Mutex<Option<AbortReason>>,
}

impl Progress {
    /// 新建一个独立作用域，给定超时预算。无父 token。
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            token: CancellationToken::new(),
            reason: Mutex::new(None),
        }
    }

    /// 从父 token 派生一个子作用域：父被取消 → 本作用域立即一起取消
    /// （建模 `LongStandingScope` 层级：page.close 取消 page-token → 其下所有 frame
    /// 作用域随之取消）。
    pub fn child(timeout: Duration, parent: &CancellationToken) -> Self {
        Self {
            timeout,
            token: parent.child_token(),
            reason: Mutex::new(None),
        }
    }

    /// 暴露本作用域的取消 token：供派生子作用域，或供传输/会话层在 page.close /
    /// frame.detach 时直接取消（也可用 [`Progress::abort`] 以携带原因）。
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// 本作用域的 deadline 预算。供派生子作用域时继承同一超时（如动作层的 detach/crash
    /// 接线据父作用域 timeout + token 派生一个子作用域跑动作，见
    /// [`crate::backend::cdp::CdpBackend::arm_act_abort`]）。
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// 主动 abort（携带原因）。进行中的 race 会立即以 `Err(Aborted(reason))` 返回；
    /// 之后到来的 race 同样立即返回（已取消是粘性的）。
    pub fn abort(&self, reason: AbortReason) {
        // 先记录原因再取消，确保被唤醒的 race 一定能读到原因。
        *self.reason.lock().unwrap() = Some(reason);
        self.token.cancel();
    }

    /// 当前已记录的 abort 原因；未显式 `abort()`（如被父/外部取消 token）则为 `None`。
    fn abort_reason(&self) -> AbortReason {
        self.reason
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(AbortReason::Cancelled)
    }

    /// 把单个 future 与 (deadline, abort) 三方竞速：
    /// - future 先完成 → `Ok(value)`；
    /// - token 被取消（含父取消） → `Err(Aborted(reason))`，**立即**返回，远早于 deadline；
    /// - deadline 先到 → `Err(Timeout)`。
    ///
    /// 已取消的作用域上调用 → 立即 `Err(Aborted(..))`（晚到的等待者不会漏掉取消）。
    pub async fn race<F: Future>(&self, fut: F) -> Result<F::Output, ProgressError> {
        // 已取消是粘性的：晚到的等待者也立即观察到，绝不漏。
        if self.token.is_cancelled() {
            return Err(ProgressError::Aborted(self.abort_reason()));
        }

        // 三方竞速。`biased` + 取消分支在前 → 当取消与 deadline/future 同时就绪时，
        // 取消稳定胜出（契约 6：abort 优先于 timeout，分类不抖动）。deadline 用
        // `tokio::time::sleep`，在测试中受可控时钟驱动；真实运行用真实时钟。
        tokio::select! {
            biased;
            () = self.token.cancelled() => Err(ProgressError::Aborted(self.abort_reason())),
            () = tokio::time::sleep(self.timeout) => Err(ProgressError::Timeout),
            out = fut => Ok(out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::pending;
    use tokio::time::Duration as TDuration;
    use tokio_util::sync::CancellationToken;

    /// 契约 1：deadline 前完成的 future → Ok(value)。
    #[tokio::test]
    async fn completes_before_deadline_returns_ok() {
        let p = Progress::new(Duration::from_secs(60));
        let out = p.race(async { 42 }).await;
        assert!(matches!(out, Ok(42)), "expected Ok(42), got {out:?}");
    }

    /// 契约 2：永不完成的 future + 很短 deadline → Err(Timeout)，约在 deadline 内返回。
    /// 用可控时钟：pause 后 race 永挂的 future，advance 越过 deadline 即触发超时，
    /// 不真实 sleep，快速且确定。
    #[tokio::test(start_paused = true)]
    async fn never_completes_hits_timeout() {
        let p = Progress::new(Duration::from_millis(30));
        let handle = tokio::spawn(async move { p.race(pending::<()>()).await });
        // 推进虚拟时钟越过 deadline。
        tokio::time::advance(Duration::from_millis(31)).await;
        let out = handle.await.expect("task panicked");
        assert!(
            matches!(out, Err(ProgressError::Timeout)),
            "expected Timeout, got {out:?}"
        );
    }

    /// 契约 3：进行中 abort → race 立即（远早于 deadline）以 Aborted(PageClosed) 返回。
    #[tokio::test]
    async fn in_flight_abort_returns_immediately() {
        use std::sync::Arc;
        let p = Arc::new(Progress::new(Duration::from_secs(3600)));
        let p2 = Arc::clone(&p);

        let race = tokio::spawn(async move { p2.race(pending::<()>()).await });

        // 让 race 真正进入等待，然后在另一处 abort。
        tokio::task::yield_now().await;
        p.abort(AbortReason::PageClosed);

        // 真实墙钟：必须远早于 3600s 的 deadline 返回；用一个保守上限断言「立即」。
        let out = tokio::time::timeout(TDuration::from_secs(5), race)
            .await
            .expect("race did not return promptly after abort")
            .expect("task panicked");
        assert_eq!(
            out.err(),
            Some(ProgressError::Aborted(AbortReason::PageClosed)),
            "expected Aborted(PageClosed)"
        );
    }

    /// 契约 4：先 abort 再 race → 立即 Err(Aborted(..))（晚到的等待者也观察到取消）。
    #[tokio::test]
    async fn race_after_abort_is_immediate() {
        let p = Progress::new(Duration::from_secs(3600));
        p.abort(AbortReason::Cancelled);

        let out = tokio::time::timeout(TDuration::from_secs(5), p.race(pending::<()>()))
            .await
            .expect("race did not return promptly on already-cancelled scope");
        assert_eq!(out.err(), Some(ProgressError::Aborted(AbortReason::Cancelled)));
    }

    /// 契约 5：层级取消（LongStandingScope）。父 token 派生子 Progress，取消父 →
    /// 子 Progress 的进行中 race 立即 Err(Aborted(..))。
    #[tokio::test]
    async fn parent_cancel_aborts_child_scope() {
        use std::sync::Arc;
        let parent = CancellationToken::new();
        let child = Arc::new(Progress::child(Duration::from_secs(3600), &parent));
        let child2 = Arc::clone(&child);

        let race = tokio::spawn(async move { child2.race(pending::<()>()).await });

        tokio::task::yield_now().await;
        parent.cancel(); // 取消父 → 子作用域立即一起取消。

        let out = tokio::time::timeout(TDuration::from_secs(5), race)
            .await
            .expect("child race did not return after parent cancel")
            .expect("task panicked");
        // 父取消未经子的 abort() 携带原因 → 兜底为 Cancelled。
        assert_eq!(out.err(), Some(ProgressError::Aborted(AbortReason::Cancelled)));
    }

    /// 契约 6：abort 与 deadline 同时逼近时分类稳定。用可控时钟：虚拟时钟不自行推进，
    /// deadline 永不触发，abort 成为唯一就绪分支 → 始终分类为 Aborted（abort 优先）。
    /// 这把「优先级」与「时间流逝」解耦：不依赖墙钟调度时序，消除潜在 flaky。
    #[tokio::test(start_paused = true)]
    async fn abort_before_deadline_classifies_as_aborted_not_timeout() {
        use std::sync::Arc;
        let p = Arc::new(Progress::new(Duration::from_millis(200)));
        let p2 = Arc::clone(&p);

        let race = tokio::spawn(async move { p2.race(pending::<()>()).await });

        // 让 race 进入 select；虚拟时钟未推进，200ms deadline 不会触发。
        // 即便 race 尚未进入 select，abort 的取消是粘性的，入口守卫亦会命中——
        // 无论调度顺序如何，结果都只能是 Aborted，绝不会是 Timeout。
        tokio::task::yield_now().await;
        p.abort(AbortReason::FrameDetached);

        let out = race.await.expect("task panicked");
        assert_eq!(
            out.err(),
            Some(ProgressError::Aborted(AbortReason::FrameDetached)),
            "abort must win; deadline must not fire on a non-advanced virtual clock"
        );
    }
}
