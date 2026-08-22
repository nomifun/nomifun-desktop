use std::future::Future;
use std::time::Duration;

use reqwest::header::HeaderMap;
use serde_json::Value;
use tokio::sync::mpsc;

use nomi_types::llm::LlmEvent;
use nomifun_net::secret_redaction::SecretRedactor;

use super::ProviderError;
use super::anthropic_shared::StreamOutcome;

pub const MAX_STREAM_RETRIES: u32 = 2;
pub const MAX_INITIAL_REQUEST_RETRIES: u32 = 2;
const MAX_BACKOFF: Duration = Duration::from_secs(15);
const INITIAL_REQUEST_BACKOFF: Duration = Duration::from_millis(300);
const MAX_INITIAL_REQUEST_BACKOFF: Duration = Duration::from_secs(2);

/// Race provider work against the downstream receiver lifecycle.
///
/// Provider stream drains run in detached tasks, so dropping the consumer's
/// `Receiver` does not cancel them automatically. Every active drain, retry
/// send, response processor, and backoff uses this boundary; `None` means the
/// receiver closed and the owned future was dropped immediately.
pub(crate) async fn until_receiver_closed<T, F>(
    tx: &mpsc::Sender<LlmEvent>,
    future: F,
) -> Option<T>
where
    F: Future<Output = T>,
{
    if tx.is_closed() {
        return None;
    }
    tokio::select! {
        biased;
        _ = tx.closed() => None,
        value = future => Some(value),
    }
}

async fn emit_if_open(tx: &mpsc::Sender<LlmEvent>, event: LlmEvent) -> bool {
    matches!(until_receiver_closed(tx, tx.send(event)).await, Some(Ok(())))
}

/// Retry bounded initial failures before any response is exposed locally:
/// connection failures and transient gateway/service 500/502/503/504
/// responses. The upstream may still have spent work before returning an
/// error, so attempts stay deliberately low; client errors and rate limits are
/// surfaced immediately.
pub async fn with_initial_request_retry<F, Fut, T>(f: F) -> Result<T, ProviderError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, ProviderError>>,
{
    let mut backoff = INITIAL_REQUEST_BACKOFF;
    for attempt in 0..=MAX_INITIAL_REQUEST_RETRIES {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) if is_retryable_initial_request_error(&e) && attempt < MAX_INITIAL_REQUEST_RETRIES => {
                let (error_kind, status) = retry_log_classification(&e);
                tracing::warn!(
                    attempt = attempt + 1,
                    max_retries = MAX_INITIAL_REQUEST_RETRIES,
                    error_kind,
                    status = status.unwrap_or_default(),
                    "retrying transient initial provider request failure"
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_INITIAL_REQUEST_BACKOFF);
            }
            Err(e) => {
                if attempt > 0 {
                    let (error_kind, status) = retry_log_classification(&e);
                    tracing::warn!(
                        attempts = attempt + 1,
                        max_retries = MAX_INITIAL_REQUEST_RETRIES,
                        error_kind,
                        status = status.unwrap_or_default(),
                        "provider initial request retries exhausted"
                    );
                }
                return Err(e);
            }
        }
    }
    unreachable!()
}

fn retry_log_classification(error: &ProviderError) -> (&'static str, Option<u16>) {
    match error {
        ProviderError::Http(_) => ("http_transport", None),
        ProviderError::Connection(_) => ("connection", None),
        ProviderError::Api { status, .. } => ("transient_api", Some(*status)),
        _ => ("other", None),
    }
}

fn is_retryable_initial_request_error(error: &ProviderError) -> bool {
    match error {
        // No response stream exists yet, so retrying cannot duplicate visible
        // model output or tool progress. It can still duplicate upstream work,
        // which is why the shared retry budget is intentionally small.
        ProviderError::Http(err) => err.is_connect() || err.is_timeout() || err.is_request(),
        ProviderError::Connection(_) => true,
        ProviderError::Api { status, .. } => {
            matches!(status, 500 | 502 | 503 | 504) && !error.is_tool_schema_incompatible()
        }
        _ => false,
    }
}

/// Send an HTTP request and check status, returning the response on success.
/// Used by provider-specific retry loops to avoid duplicating request logic.
pub async fn send_and_check(
    client: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
    body: &Value,
    redactor: &SecretRedactor,
) -> Result<reqwest::Response, ProviderError> {
    let response = client
        .post(url)
        .headers(headers.clone())
        .json(body)
        .send()
        .await
        .map_err(ProviderError::from)?;

    let response = crate::reject_non_api_response(response)?;
    let status = response.status();
    if !status.is_success() {
        let body_text = crate::read_provider_error_body(response, redactor).await;
        return Err(ProviderError::Api {
            status: status.as_u16(),
            message: body_text,
        });
    }

    Ok(response)
}

/// Sleep with exponential backoff and log the retry attempt.
/// Returns the next backoff duration.
pub async fn backoff_sleep(attempt: u32, current_backoff: Duration) -> Duration {
    tracing::warn!(
        attempt,
        max = MAX_STREAM_RETRIES,
        "retrying provider stream after an empty retryable failure"
    );
    tokio::time::sleep(current_backoff).await;
    (current_backoff * 2).min(MAX_BACKOFF)
}

/// Cancellation-aware retry backoff. The downstream close signal interrupts
/// the timer, so a timed-out consumer never waits out a provider retry delay.
async fn backoff_sleep_until_closed(
    tx: &mpsc::Sender<LlmEvent>,
    attempt: u32,
    current_backoff: Duration,
) -> Option<Duration> {
    if tx.is_closed() {
        return None;
    }
    tracing::warn!(
        attempt,
        max = MAX_STREAM_RETRIES,
        "retrying provider stream after an empty retryable failure"
    );
    until_receiver_closed(tx, tokio::time::sleep(current_backoff)).await?;
    Some((current_backoff * 2).min(MAX_BACKOFF))
}

/// Evaluate a `StreamOutcome` within a retry loop. Returns:
/// - `Ok(None)` — stream succeeded, stop retrying
/// - `Ok(Some(err))` — non-retryable failure, caller should emit error
/// - `Err(err)` — retryable failure, caller should continue loop
pub fn evaluate_outcome(
    outcome: StreamOutcome,
    attempt: u32,
) -> Result<Option<ProviderError>, ProviderError> {
    match outcome {
        StreamOutcome::Ok => Ok(None),
        StreamOutcome::FailedPartial(e) => Ok(Some(e)),
        StreamOutcome::FailedEmpty(e) => {
            if !e.is_retryable() || attempt == MAX_STREAM_RETRIES {
                Ok(Some(e))
            } else {
                Err(e)
            }
        }
    }
}

/// Drive a completed stream outcome to resolution, retrying empty-content
/// failures with exponential backoff. Shared by all providers' spawned
/// stream tasks.
///
/// - `Ok` — nothing to do.
/// - `FailedPartial` — content already reached the consumer; replaying
///   would duplicate it, so the error is surfaced immediately.
/// - `FailedEmpty` — nothing was emitted yet; if the error is retryable,
///   re-send the request via `send` and re-process the response via
///   `process`, up to `MAX_STREAM_RETRIES` times.
pub async fn finish_stream_with_retry<S, SFut, P, PFut, R>(
    outcome: StreamOutcome,
    tx: &mpsc::Sender<LlmEvent>,
    send: S,
    mut process: P,
) where
    S: Fn() -> SFut,
    SFut: Future<Output = Result<R, ProviderError>>,
    P: FnMut(R) -> PFut,
    PFut: Future<Output = StreamOutcome>,
{
    if tx.is_closed() {
        return;
    }
    let initial_err = match outcome {
        StreamOutcome::Ok => return,
        StreamOutcome::FailedPartial(e) => {
            // Content already emitted — replaying would duplicate it.
            emit_if_open(tx, LlmEvent::Error(e.to_string())).await;
            return;
        }
        StreamOutcome::FailedEmpty(e) => e,
    };

    if !initial_err.is_retryable() {
        emit_if_open(tx, LlmEvent::Error(initial_err.to_string())).await;
        return;
    }

    let mut backoff = Duration::from_secs(1);
    let mut final_err = Some(initial_err);
    let mut attempts_made = 0;
    for attempt in 1..=MAX_STREAM_RETRIES {
        if tx.is_closed() {
            return;
        }
        attempts_made = attempt;
        let Some(next_backoff) = backoff_sleep_until_closed(tx, attempt, backoff).await else {
            return;
        };
        backoff = next_backoff;
        let Some(send_result) = until_receiver_closed(tx, send()).await else {
            return;
        };
        match send_result {
            Ok(resp) => {
                if tx.is_closed() {
                    return;
                }
                let Some(process_outcome) = until_receiver_closed(tx, process(resp)).await else {
                    return;
                };
                match evaluate_outcome(process_outcome, attempt) {
                Ok(None) => {
                    final_err = None;
                    break;
                }
                Ok(Some(e)) => {
                    final_err = Some(e);
                    break;
                }
                Err(_) => continue,
                }
            }
            Err(e) if !e.is_retryable() || attempt == MAX_STREAM_RETRIES => {
                final_err = Some(e);
                break;
            }
            Err(_) => continue,
        }
    }
    if let Some(err) = final_err {
        if tx.is_closed() {
            return;
        }
        let (error_kind, status) = retry_log_classification(&err);
        tracing::warn!(
            attempts = attempts_made,
            max_retries = MAX_STREAM_RETRIES,
            error_kind,
            status = status.unwrap_or_default(),
            "provider empty-stream retry ended with an error"
        );
        emit_if_open(tx, LlmEvent::Error(err.to_string())).await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use super::*;
    use crate::ProviderError;

    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn dropped_receiver_cancels_active_drain_future() {
        let (tx, rx) = mpsc::channel(1);
        let started = Arc::new(tokio::sync::Semaphore::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn({
            let started = Arc::clone(&started);
            let dropped = Arc::clone(&dropped);
            async move {
                until_receiver_closed(&tx, async move {
                    let _probe = DropProbe(dropped);
                    started.add_permits(1);
                    std::future::pending::<()>().await;
                })
                .await
            }
        });

        started.acquire().await.unwrap().forget();
        drop(rx);
        let result = tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .expect("active stream drain ignored receiver cancellation")
            .unwrap();
        assert!(result.is_none());
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn dropped_receiver_interrupts_retry_backoff() {
        let (tx, rx) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            backoff_sleep_until_closed(&tx, 1, Duration::from_secs(30)).await
        });
        tokio::task::yield_now().await;
        drop(rx);

        let result = tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .expect("retry backoff waited after receiver cancellation")
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn already_dropped_receiver_never_sends_or_processes_a_retry() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let send_count = Arc::new(AtomicU32::new(0));
        let process_count = Arc::new(AtomicU32::new(0));

        finish_stream_with_retry(
            StreamOutcome::FailedEmpty(ProviderError::Connection("empty".into())),
            &tx,
            || {
                let send_count = Arc::clone(&send_count);
                async move {
                    send_count.fetch_add(1, Ordering::SeqCst);
                    Ok::<(), ProviderError>(())
                }
            },
            |()| {
                let process_count = Arc::clone(&process_count);
                async move {
                    process_count.fetch_add(1, Ordering::SeqCst);
                    StreamOutcome::Ok
                }
            },
        )
        .await;

        assert_eq!(send_count.load(Ordering::SeqCst), 0);
        assert_eq!(process_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dropped_receiver_cancels_inflight_retry_send_and_prevents_more_attempts() {
        tokio::time::pause();
        let (tx, rx) = mpsc::channel(1);
        let send_count = Arc::new(AtomicU32::new(0));
        let process_count = Arc::new(AtomicU32::new(0));
        let send_started = Arc::new(tokio::sync::Semaphore::new(0));
        let task = tokio::spawn({
            let send_count = Arc::clone(&send_count);
            let process_count = Arc::clone(&process_count);
            let send_started = Arc::clone(&send_started);
            async move {
                finish_stream_with_retry(
                    StreamOutcome::FailedEmpty(ProviderError::Connection("empty".into())),
                    &tx,
                    || {
                        let send_count = Arc::clone(&send_count);
                        let send_started = Arc::clone(&send_started);
                        async move {
                            send_count.fetch_add(1, Ordering::SeqCst);
                            send_started.add_permits(1);
                            std::future::pending::<Result<(), ProviderError>>().await
                        }
                    },
                    move |()| {
                        let process_count = Arc::clone(&process_count);
                        async move {
                            process_count.fetch_add(1, Ordering::SeqCst);
                            StreamOutcome::Ok
                        }
                    },
                )
                .await;
            }
        });

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(1)).await;
        send_started.acquire().await.unwrap().forget();
        drop(rx);
        tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .expect("in-flight retry send ignored receiver cancellation")
            .unwrap();
        tokio::time::advance(Duration::from_secs(60)).await;
        assert_eq!(send_count.load(Ordering::SeqCst), 1);
        assert_eq!(process_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dropped_receiver_cancels_inflight_retry_processor() {
        tokio::time::pause();
        let (tx, rx) = mpsc::channel(1);
        let send_count = Arc::new(AtomicU32::new(0));
        let process_count = Arc::new(AtomicU32::new(0));
        let process_started = Arc::new(tokio::sync::Semaphore::new(0));
        let task = tokio::spawn({
            let send_count = Arc::clone(&send_count);
            let process_count = Arc::clone(&process_count);
            let process_started = Arc::clone(&process_started);
            async move {
                finish_stream_with_retry(
                    StreamOutcome::FailedEmpty(ProviderError::Connection("empty".into())),
                    &tx,
                    || {
                        let send_count = Arc::clone(&send_count);
                        async move {
                            send_count.fetch_add(1, Ordering::SeqCst);
                            Ok::<(), ProviderError>(())
                        }
                    },
                    move |()| {
                        let process_count = Arc::clone(&process_count);
                        let process_started = Arc::clone(&process_started);
                        async move {
                            process_count.fetch_add(1, Ordering::SeqCst);
                            process_started.add_permits(1);
                            std::future::pending::<StreamOutcome>().await
                        }
                    },
                )
                .await;
            }
        });

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(1)).await;
        process_started.acquire().await.unwrap().forget();
        drop(rx);
        tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .expect("in-flight response processor ignored receiver cancellation")
            .unwrap();
        tokio::time::advance(Duration::from_secs(60)).await;
        assert_eq!(send_count.load(Ordering::SeqCst), 1);
        assert_eq!(process_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_initial_connect_retry_succeeds_after_connection_failures() {
        tokio::time::pause();

        let counter = Arc::new(AtomicU32::new(0));
        let result = with_initial_request_retry(|| {
            let counter = Arc::clone(&counter);
            async move {
                let attempt = counter.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    Err(ProviderError::Connection("connection refused".into()))
                } else {
                    Ok(attempt)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 2);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_initial_connect_retry_does_not_retry_rate_limit() {
        let counter = Arc::new(AtomicU32::new(0));
        let result = with_initial_request_retry(|| {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(ProviderError::RateLimited {
                    retry_after_ms: 5000,
                    message: "Too Many Requests".into(),
                })
            }
        })
        .await;

        assert!(matches!(
            result.unwrap_err(),
            ProviderError::RateLimited {
                retry_after_ms: 5000,
                ..
            }
        ));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_initial_request_retries_transient_502_then_succeeds() {
        tokio::time::pause();
        let counter = Arc::new(AtomicU32::new(0));
        let result = with_initial_request_retry(|| {
            let counter = Arc::clone(&counter);
            async move {
                let attempt = counter.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    Err(ProviderError::Api {
                        status: 502,
                        message: "bad gateway".into(),
                    })
                } else {
                    Ok(())
                }
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_initial_request_does_not_retry_400() {
        let counter = Arc::new(AtomicU32::new(0));
        let result = with_initial_request_retry(|| {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(ProviderError::Api {
                    status: 400,
                    message: "bad request".into(),
                })
            }
        })
        .await;

        assert!(matches!(result, Err(ProviderError::Api { status: 400, .. })));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    // --- evaluate_outcome tests ---

    #[test]
    fn test_evaluate_outcome_ok_stops_retry() {
        let result = evaluate_outcome(StreamOutcome::Ok, 1);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn test_evaluate_outcome_failed_partial_always_stops() {
        let err = ProviderError::Connection("disconnect".into());
        let result = evaluate_outcome(StreamOutcome::FailedPartial(err), 1);
        // FailedPartial means content was already emitted — cannot retry regardless of attempt
        let Ok(Some(e)) = result else {
            panic!("expected Ok(Some(err))")
        };
        assert!(matches!(e, ProviderError::Connection(_)));
    }

    #[test]
    fn test_evaluate_outcome_failed_partial_on_last_attempt() {
        let err = ProviderError::Connection("disconnect".into());
        let result = evaluate_outcome(StreamOutcome::FailedPartial(err), MAX_STREAM_RETRIES);
        let Ok(Some(_)) = result else {
            panic!("expected Ok(Some(err))")
        };
    }

    #[test]
    fn test_evaluate_outcome_failed_empty_retries_when_not_exhausted() {
        let err = ProviderError::Connection("disconnect".into());
        // attempt 1 < MAX_STREAM_RETRIES(2), should signal "continue retrying"
        let result = evaluate_outcome(StreamOutcome::FailedEmpty(err), 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_evaluate_outcome_failed_empty_stops_on_last_attempt() {
        let err = ProviderError::Connection("disconnect".into());
        // attempt == MAX_STREAM_RETRIES, should stop and return error
        let result = evaluate_outcome(StreamOutcome::FailedEmpty(err), MAX_STREAM_RETRIES);
        let Ok(Some(e)) = result else {
            panic!("expected Ok(Some(err))")
        };
        assert!(matches!(e, ProviderError::Connection(_)));
    }

    #[test]
    fn test_evaluate_outcome_failed_empty_non_retryable_stops_immediately() {
        let err = ProviderError::Parse("malformed SSE frame".into());
        let result = evaluate_outcome(StreamOutcome::FailedEmpty(err), 1);
        let Ok(Some(e)) = result else {
            panic!("expected Ok(Some(err))")
        };
        assert!(matches!(e, ProviderError::Parse(_)));
    }

    // --- backoff_sleep tests ---

    #[tokio::test]
    async fn test_backoff_sleep_doubles_duration() {
        tokio::time::pause();

        let next = backoff_sleep(1, Duration::from_secs(1)).await;
        assert_eq!(next, Duration::from_secs(2));

        let next = backoff_sleep(2, Duration::from_secs(4)).await;
        assert_eq!(next, Duration::from_secs(8));
    }

    #[tokio::test]
    async fn test_backoff_sleep_caps_at_max() {
        tokio::time::pause();

        // 10s * 2 = 20s, but MAX_BACKOFF is 15s
        let next = backoff_sleep(1, Duration::from_secs(10)).await;
        assert_eq!(next, Duration::from_secs(15));

        // Already at max
        let next = backoff_sleep(2, Duration::from_secs(15)).await;
        assert_eq!(next, Duration::from_secs(15));
    }
}
