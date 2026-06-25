//! 边界错误映射：把 [`crate::progress`] 的取消/超时（`ProgressError`）翻译成引擎层
//! 可路由的 [`BrowserError`]（§22 taxonomy）。
//!
//! 设计与 [`crate::backend::cdp::map_transport_err`] 同形：**集中、穷尽（无 `_`）、绝不
//! panic**。`progress.rs` 故意与 `BrowserError` 解耦（`ProgressError` 自成一体），所以两层
//! 的语义缝合放在这里——动作层（act/actionability，后续任务）在把受控操作的取消结果上抛给
//! 引擎调用方时，经本映射统一成 `BrowserError`，让模型读到精确分类而非裸取消。
//!
//! 穷尽 match（不写 `_`）：progress.rs 给 `AbortReason` 新增变体时编译期就逼我们在这里补
//! 语义，而非静默归入 `Other`。

use crate::engine::{BrowserError, DetachKind, NavPhase};
use crate::progress::{AbortReason, ProgressError};

/// 把 Progress 的取消/超时映射到引擎错误（动作层边界，仿
/// [`crate::backend::cdp::map_transport_err`]）。**绝不 panic**；穷尽 match。
///
/// - `Timeout` → `Timeout{phase: Action}`（受控操作没在 deadline 内完成；动作层语义）。
/// - `Aborted(PageClosed)` → `TargetClosed`（page 关闭 → 该 target 没了）。
/// - `Aborted(FrameDetached)` → `Detached{kind: Frame}`（frame detach → 帧没了）。
/// - `Aborted(Cancelled)` → `NavigationInterrupted`（主动/父取消，多见于新导航打断旧操作）。
/// - `Aborted(Other(m))` → `Other(m)`（自由文本取消原因，原样透传供诊断）。
pub fn map_progress_err(e: ProgressError) -> BrowserError {
    match e {
        ProgressError::Timeout => BrowserError::Timeout {
            phase: NavPhase::Action,
        },
        ProgressError::Aborted(AbortReason::PageClosed) => BrowserError::TargetClosed,
        ProgressError::Aborted(AbortReason::FrameDetached) => BrowserError::Detached {
            kind: DetachKind::Frame,
        },
        ProgressError::Aborted(AbortReason::Cancelled) => BrowserError::NavigationInterrupted,
        ProgressError::Aborted(AbortReason::Other(m)) => BrowserError::Other(m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::{AbortReason, ProgressError};

    #[test]
    fn map_progress_err_classifies() {
        assert!(matches!(
            map_progress_err(ProgressError::Aborted(AbortReason::PageClosed)),
            BrowserError::TargetClosed
        ));
        assert!(matches!(
            map_progress_err(ProgressError::Aborted(AbortReason::FrameDetached)),
            BrowserError::Detached {
                kind: DetachKind::Frame
            }
        ));
        assert!(matches!(
            map_progress_err(ProgressError::Timeout),
            BrowserError::Timeout {
                phase: NavPhase::Action
            }
        ));
    }

    #[test]
    fn map_progress_err_cancelled_and_other() {
        // Cancelled（主动/父取消）→ NavigationInterrupted。
        assert!(matches!(
            map_progress_err(ProgressError::Aborted(AbortReason::Cancelled)),
            BrowserError::NavigationInterrupted
        ));
        // Other(m) 原样透传原因。
        match map_progress_err(ProgressError::Aborted(AbortReason::Other("custom".into()))) {
            BrowserError::Other(m) => assert_eq!(m, "custom"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn new_error_variants_display_carry_semantics() {
        assert!(format!(
            "{}",
            BrowserError::Timeout {
                phase: NavPhase::NetworkIdle
            }
        )
        .contains("timeout"));
        assert!(format!(
            "{}",
            BrowserError::Detached {
                kind: DetachKind::Frame
            }
        )
        .to_lowercase()
        .contains("detach"));
        let _ = (
            BrowserError::TargetCrashed,
            BrowserError::NavigationInterrupted,
        );
    }
}
