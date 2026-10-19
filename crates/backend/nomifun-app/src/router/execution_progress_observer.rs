//! Notifications only wake exact canonical reads; they never settle work.
use super::*;
use nomifun_agent_execution::AgentExecutionTurnProgress;

pub(super) async fn observe<Read, ReadFuture>(
    after_cursor: Option<u64>, mut stream: Option<broadcast::Receiver<AgentStreamEvent>>, mut read: Read,
) -> Result<AgentExecutionTurnProgress, AppError>
where
    Read: FnMut() -> ReadFuture + Send,
    ReadFuture: Future<Output = Result<AgentExecutionTurnProgress, AppError>> + Send,
{
    // The caller has subscribed before this read. A completion committed on
    // either side of subscription is seen here or by the bounded re-read.
    let current = read().await?;
    if !current.found || current.terminal || after_cursor != Some(current.cursor) { return Ok(current); }
    tokio::select! {
        _ = async {
            if let Some(stream) = stream.as_mut() {
                match stream.recv().await {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => return,
                    // A closed owner transport is not terminal proof and must
                    // not turn the next observation into a tight polling loop.
                    Err(broadcast::error::RecvError::Closed) => {}
                }
            }
            std::future::pending::<()>().await;
        } => {}
        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
    }
    read().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn progress(terminal: bool) -> AgentExecutionTurnProgress {
        AgentExecutionTurnProgress { found: true, terminal, cursor: if terminal { 2 } else { 1 }, suspended: false }
    }

    #[tokio::test(start_paused = true)]
    async fn completion_between_subscription_and_first_read_is_observed_without_another_notification() {
        let (sender, receiver) = broadcast::channel(1);
        drop(sender);
        let reads = AtomicUsize::new(0);
        let result = observe(Some(1), Some(receiver), || async {
            reads.fetch_add(1, Ordering::SeqCst);
            Ok(progress(true))
        }).await.unwrap();
        assert!(result.terminal);
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn lost_owner_notification_is_recovered_by_the_canonical_cursor_fallback() {
        let (_sender, receiver) = broadcast::channel(1);
        let terminal = AtomicBool::new(false);
        let reads = AtomicUsize::new(0);
        let read_started = tokio::sync::Notify::new();
        let observer = observe(Some(1), Some(receiver), || async {
            let result = progress(terminal.load(Ordering::SeqCst));
            reads.fetch_add(1, Ordering::SeqCst);
            read_started.notify_one();
            Ok(result)
        });
        tokio::pin!(observer);
        tokio::select! {
            result = &mut observer => panic!("no canonical terminal yet: {result:?}"),
            _ = read_started.notified() => {}
        }
        terminal.store(true, Ordering::SeqCst);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(observer.await.unwrap().terminal);
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn closed_owner_stream_waits_for_fallback_and_does_not_spin_or_claim_completion() {
        let (sender, receiver) = broadcast::channel(1);
        drop(sender);
        let terminal = AtomicBool::new(false);
        let reads = AtomicUsize::new(0);
        let read_started = tokio::sync::Notify::new();
        let observer = observe(Some(1), Some(receiver), || async {
            let result = progress(terminal.load(Ordering::SeqCst));
            reads.fetch_add(1, Ordering::SeqCst);
            read_started.notify_one();
            Ok(result)
        });
        tokio::pin!(observer);
        tokio::select! {
            result = &mut observer => panic!("closed transport cannot prove terminal: {result:?}"),
            _ = read_started.notified() => {}
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        terminal.store(true, Ordering::SeqCst);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(observer.await.unwrap().terminal);
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn absent_owner_stream_uses_the_same_receipt_fallback() {
        let reads = AtomicUsize::new(0);
        let observer = observe(Some(1), None, || async {
            Ok(progress(reads.fetch_add(1, Ordering::SeqCst) > 0))
        });
        tokio::pin!(observer);
        tokio::select! {
            result = &mut observer => panic!("no receipt change before fallback: {result:?}"),
            _ = tokio::task::yield_now() => {}
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(observer.await.unwrap().terminal);
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }
}
