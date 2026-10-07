//! Failure isolation for native owner cleanup callbacks. The caller retains
//! task ownership and decides whether settlement has actually been proven.
use std::{future::Future, panic::AssertUnwindSafe};
use futures_util::FutureExt;
use nomifun_common::AppError;

fn failure() -> AppError {
    AppError::Conflict("Effect owner cleanup is not proven settled".into())
}

/// Isolate one cleanup callback, including a panic while constructing its
/// future. This does not retain work, retry effects or establish settlement:
/// the caller must keep ownership and reject the boundary on any error.
/// Only unwinding panics are caught; process aborts remain crash recovery.
pub async fn guard_effect_settlement<T, F, Fut>(operation: F) -> Result<T, AppError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, AppError>>,
{
    AssertUnwindSafe(async { operation().await })
        .catch_unwind()
        .await
        .map_err(|_| failure())?
}
