//! Owned, bounded ASR preparation. The Actor owns the returned abort handle
//! and fences the single completion callback with its own attempt identity.

use std::{future::Future, time::Instant};
use tokio::task::AbortHandle;

const TIMED_OUT: &str = "实时识别准备超时";
const CANCELLED: &str = "实时识别准备已取消";
const PANICKED: &str = "实时识别准备任务异常退出";

/// Requires a running Tokio runtime. Aborting actually drops the pending
/// future; dropping the handle alone does not cancel it. The monitor calls
/// `on_ready` exactly once while the runtime remains alive. A completion that
/// already won before `abort()` stays complete; the Actor remains the final
/// generation fence for a completion racing cancellation.
pub(crate) fn spawn<T, F, C>(future: F, deadline: Instant, on_ready: C) -> AbortHandle
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + Send + 'static,
    C: FnOnce(Result<T, String>) + Send + 'static,
{
    let primary = tokio::spawn(async move {
        // Tokio timeout may poll an already-ready future before checking its
        // timer. Include executor queue time and reject it without polling.
        if Instant::now() >= deadline {
            return Err(TIMED_OUT.to_owned());
        }
        let result =
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), future).await;
        match result {
            Ok(result) if Instant::now() < deadline => result,
            _ => Err(TIMED_OUT.to_owned()),
        }
    });
    let abort = primary.abort_handle();
    // This task owns the JoinHandle so panics/cancellation cannot silently
    // leave the Actor waiting. Never format a JoinError or its panic payload.
    tokio::spawn(async move {
        let result = match primary.await {
            Ok(result) => result,
            Err(error) if error.is_cancelled() => Err(CANCELLED.to_owned()),
            Err(_) => Err(PANICKED.to_owned()),
        };
        on_ready(result);
    });
    abort
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    use tokio::sync::oneshot;

    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    async fn receive<T>(rx: oneshot::Receiver<Result<T, String>>) -> Result<T, String> {
        tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .expect("owned preparation callback must not hang")
            .expect("completion callback must run")
    }

    #[tokio::test]
    async fn abort_drops_a_polled_pending_future_and_calls_back_once() {
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = DropFlag(dropped.clone());
        let (started_tx, started_rx) = oneshot::channel();
        let (tx, rx) = oneshot::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let abort = spawn(
            async move {
                let _guard = guard;
                let _ = started_tx.send(());
                std::future::pending::<Result<(), String>>().await
            },
            Instant::now() + Duration::from_secs(5),
            move |result| {
                count.fetch_add(1, Ordering::AcqRel);
                let _ = tx.send(result);
            },
        );
        started_rx.await.unwrap();
        abort.abort();
        assert_eq!(receive(rx).await.unwrap_err(), CANCELLED);
        assert!(dropped.load(Ordering::Acquire));
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn abort_closes_a_real_pending_loopback_socket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            socket.read_to_end(&mut bytes).await.unwrap();
            bytes.len()
        });
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = DropFlag(dropped.clone());
        let (connected_tx, connected_rx) = oneshot::channel();
        let (tx, rx) = oneshot::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let abort = spawn(
            async move {
                let _guard = guard;
                let mut socket = tokio::net::TcpStream::connect(address)
                    .await
                    .map_err(|_| "loopback connect failed".to_owned())?;
                let _ = connected_tx.send(());
                socket
                    .read_u8()
                    .await
                    .map(|_| ())
                    .map_err(|_| "loopback read ended".to_owned())
            },
            Instant::now() + Duration::from_secs(5),
            move |result| {
                count.fetch_add(1, Ordering::AcqRel);
                let _ = tx.send(result);
            },
        );
        connected_rx.await.unwrap();
        abort.abort();
        assert_eq!(receive(rx).await.unwrap_err(), CANCELLED);
        assert!(dropped.load(Ordering::Acquire));
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn panic_is_a_fixed_payload_free_error_instead_of_a_hung_callback() {
        let (tx, rx) = oneshot::channel();
        spawn(
            async {
                panic!("fixed synthetic panic marker must not enter callback");
                #[allow(unreachable_code)]
                Ok::<(), String>(())
            },
            Instant::now() + Duration::from_secs(5),
            move |result| {
                let _ = tx.send(result);
            },
        );
        assert_eq!(receive(rx).await.unwrap_err(), PANICKED);
    }

    #[tokio::test]
    async fn expired_and_executor_queued_deadlines_never_poll_the_inner_future() {
        for queued in [false, true] {
            let polled = Arc::new(AtomicBool::new(false));
            let poll_flag = polled.clone();
            let (tx, rx) = oneshot::channel();
            let deadline = if queued {
                Instant::now() + Duration::from_millis(5)
            } else {
                Instant::now() - Duration::from_millis(1)
            };
            spawn(
                async move {
                    poll_flag.store(true, Ordering::Release);
                    Ok::<_, String>(17)
                },
                deadline,
                move |result| {
                    let _ = tx.send(result);
                },
            );
            if queued {
                // Current-thread test deliberately delays the first poll.
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(receive(rx).await.unwrap_err(), TIMED_OUT);
            assert!(!polled.load(Ordering::Acquire));
        }
    }

    #[tokio::test]
    async fn a_ready_future_that_overruns_its_budget_is_still_timed_out() {
        let deadline = Instant::now() + Duration::from_millis(100);
        let polled = Arc::new(AtomicBool::new(false));
        let poll_flag = polled.clone();
        let (tx, rx) = oneshot::channel();
        spawn(
            async move {
                poll_flag.store(true, Ordering::Release);
                std::thread::sleep(
                    deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(5),
                );
                Ok::<_, String>(17)
            },
            deadline,
            move |result| {
                let _ = tx.send(result);
            },
        );
        assert_eq!(receive(rx).await.unwrap_err(), TIMED_OUT);
        assert!(polled.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn successful_completion_then_late_abort_does_not_call_back_twice() {
        let (tx, rx) = oneshot::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let abort = spawn(
            async { Ok::<_, String>(17) },
            Instant::now() + Duration::from_secs(5),
            move |result| {
                count.fetch_add(1, Ordering::AcqRel);
                let _ = tx.send(result);
            },
        );
        assert_eq!(receive(rx).await.unwrap(), 17);
        abort.abort();
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn abort_before_the_next_poll_wins_over_simultaneously_ready_input() {
        let (started_tx, started_rx) = oneshot::channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (tx, rx) = oneshot::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let abort = spawn(
            async move {
                let _ = started_tx.send(());
                ready_rx.await.map_err(|_| "test input closed".to_owned())
            },
            Instant::now() + Duration::from_secs(5),
            move |result| {
                count.fetch_add(1, Ordering::AcqRel);
                let _ = tx.send(result);
            },
        );
        started_rx.await.unwrap();
        ready_tx.send(17).unwrap();
        // No await between making input ready and marking the task cancelled.
        abort.abort();
        assert_eq!(receive(rx).await.unwrap_err(), CANCELLED);
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }
}
