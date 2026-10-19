//! The original invocation's cancellation at existing native admission points.
//! Cancellation withdraws queued work; it never detaches an admitted worker.

use tokio_util::sync::CancellationToken;

pub(crate) const CANCELLED_BEFORE_DISPATCH: &str = "COMPUTER_CANCELLED_BEFORE_DISPATCH: cancellation was observed before native device dispatch; no device effect was admitted.";

pub(crate) fn require_open(cancellation: &CancellationToken) -> Result<(), String> {
    if cancellation.is_cancelled() { Err(CANCELLED_BEFORE_DISPATCH.into()) } else { Ok(()) }
}

/// Read-only preparation stays joined, but cannot authorize a new mutation
/// after Stop. The native worker checks the same token again at dispatch.
#[cfg(any(not(target_os = "macos"), test))]
pub(crate) async fn after_preparation<T, R, P, D, F>(
    cancellation: &CancellationToken, preparation: P, dispatch: D,
) -> Result<R, String>
where
    P: std::future::Future<Output = Result<T, String>>,
    D: FnOnce(T, CancellationToken) -> F,
    F: std::future::Future<Output = Result<R, String>>,
{
    require_open(cancellation)?;
    let prepared = preparation.await?;
    require_open(cancellation)?;
    dispatch(prepared, cancellation.clone()).await
}

/// Linux's existing portal/native worker uses this flag for its own queued
/// operations and exact pressed-state cleanup. Forward Stop while retaining
/// that same worker future, including its actual cleanup acknowledgement.
#[cfg(any(target_os = "linux", test))]
pub(crate) async fn join_with_cancellation<T>(
    cancellation: &CancellationToken,
    native: &std::sync::atomic::AtomicBool,
    completion: impl std::future::Future<Output = T>,
) -> T {
    tokio::pin!(completion);
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            native.store(true, std::sync::atomic::Ordering::Release);
            completion.await
        }
        result = &mut completion => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};

    #[tokio::test]
    async fn stop_during_readonly_window_lookup_joins_it_without_dispatching_focus() {
        let cancellation = CancellationToken::new();
        let (lookup_started, started) = tokio::sync::oneshot::channel();
        let (release_lookup, lookup) = tokio::sync::oneshot::channel();
        let mutations = Arc::new(AtomicUsize::new(0));
        let seen = mutations.clone();
        let task = tokio::spawn({ let cancellation = cancellation.clone(); async move {
            after_preparation(&cancellation, async move {
                lookup_started.send(()).unwrap();
                lookup.await.unwrap();
                Ok(42)
            }, move |window, _| async move { seen.fetch_add(window, Ordering::SeqCst); Ok(()) }).await
        }});
        started.await.unwrap();
        cancellation.cancel();
        assert!(!task.is_finished(), "the original metadata worker still owns its result");
        release_lookup.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err(), CANCELLED_BEFORE_DISPATCH);
        assert_eq!(mutations.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stop_reaches_existing_portal_worker_and_joins_its_cleanup_receipt() {
        let cancellation = CancellationToken::new();
        let native = Arc::new(AtomicBool::new(false));
        let (entered, started) = tokio::sync::oneshot::channel();
        let (cleanup, receipt) = tokio::sync::oneshot::channel();
        let task = tokio::spawn({ let cancellation = cancellation.clone(); let native = native.clone(); async move {
            join_with_cancellation(&cancellation, &native, async move {
                entered.send(()).unwrap();
                receipt.await.unwrap()
            }).await
        }});
        started.await.unwrap();
        cancellation.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !native.load(Ordering::Acquire) { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert!(!task.is_finished(), "Stop is not a native cleanup acknowledgement");
        cleanup.send("original cleanup").unwrap();
        assert_eq!(task.await.unwrap(), "original cleanup");
    }
}
