//! Turning one claimed task into one outcome.
//!
//! The executor is the seam the L2 stage plugs into: a remote worker gets the
//! same `CrawlTask` and returns the same [`TaskOutcome`], so the runner cannot
//! tell local work from remote work.

use std::sync::Arc;
use std::time::Duration;

use nomifun_knowledge::source_url::Validators;
use tracing::debug;

use crate::error::CrawlError;
use crate::extract;
use crate::fetcher::CrawlFetcher;
use crate::frontier::{ScopeMatcher, normalize, plan_discoveries};
use crate::model::{CrawlJob, CrawlTask, TaskOutcome};
use crate::politeness::{Politeness, Verdict};
use crate::sink::{CrawlSinkWriter, IngestPage};

#[async_trait::async_trait]
pub trait CrawlExecutor: Send + Sync {
    async fn execute(&self, task: &CrawlTask, job: &CrawlJob) -> TaskOutcome;
}

pub struct LocalExecutor {
    fetcher: Arc<dyn CrawlFetcher>,
    politeness: Arc<Politeness>,
    sink: Arc<dyn CrawlSinkWriter>,
    matcher: Arc<ScopeMatcher>,
}

impl LocalExecutor {
    pub fn new(
        fetcher: Arc<dyn CrawlFetcher>,
        politeness: Arc<Politeness>,
        sink: Arc<dyn CrawlSinkWriter>,
        matcher: Arc<ScopeMatcher>,
    ) -> Self {
        Self { fetcher, politeness, sink, matcher }
    }
}

#[async_trait::async_trait]
impl CrawlExecutor for LocalExecutor {
    async fn execute(&self, task: &CrawlTask, job: &CrawlJob) -> TaskOutcome {
        match self.run(task, job).await {
            Ok(outcome) => outcome,
            Err(err) => TaskOutcome::Failed {
                error_code: "executor_error".into(),
                error_detail: err.to_string(),
                retryable: true,
            },
        }
    }
}

impl LocalExecutor {
    async fn run(&self, task: &CrawlTask, job: &CrawlJob) -> Result<TaskOutcome, CrawlError> {
        let url = normalize(&task.url)?;

        match self.politeness.acquire(&url).await? {
            Verdict::RobotsDenied => {
                return Ok(TaskOutcome::Skipped { reason: "robots.txt disallows this path".into() });
            }
            Verdict::CircuitOpen(retry_in) => {
                // Requeue rather than fail: the host is sick, the URL is not.
                return Ok(TaskOutcome::Failed {
                    error_code: "host_circuit_open".into(),
                    error_detail: format!("host paused for {}s", retry_in.as_secs()),
                    retryable: true,
                });
            }
            Verdict::Wait(delay) => {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
        }

        let validators = Validators {
            etag: task.etag.clone(),
            last_modified: task.last_modified.clone(),
        };
        let fetched = match self.fetcher.fetch(&task.url, &validators, job.render_mode).await {
            Ok(out) => out,
            Err(err) => {
                self.politeness.note_transport_failure(&task.host);
                return Ok(TaskOutcome::Failed {
                    error_code: "fetch_failed".into(),
                    error_detail: err.to_string(),
                    retryable: is_retryable_transport(&err),
                });
            }
        };

        self.politeness
            .note_response(&task.host, fetched.status, fetched.retry_after_ms);

        if fetched.is_not_modified() {
            return Ok(TaskOutcome::Unchanged { http_status: fetched.status });
        }
        if !fetched.is_success() {
            return Ok(TaskOutcome::Failed {
                error_code: format!("http_{}", fetched.status),
                error_detail: format!("HTTP {} for {}", fetched.status, fetched.final_url),
                // 4xx other than 429 is the URL's own fault; retrying wastes a
                // request and burns the attempt budget for nothing.
                retryable: fetched.status == 429 || fetched.status >= 500,
            });
        }
        if !fetched.is_html() {
            return Ok(TaskOutcome::Skipped {
                reason: format!(
                    "unsupported content type: {}",
                    fetched.content_type.as_deref().unwrap_or("unknown")
                ),
            });
        }

        let base = normalize(&fetched.final_url).unwrap_or(url);
        let page = extract::extract(&fetched.body, &base);

        if fetched.wanted_render {
            debug!(url = %task.url, "page looks client-rendered; stage A fetched it as-is");
        }

        // An unchanged hash means the server did not honour our validators but
        // the content really is the same. Skip the write, keep the links.
        let unchanged = task
            .content_hash
            .as_deref()
            .is_some_and(|previous| previous == page.content_hash);

        if job.respect_robots && page.noindex {
            return Ok(TaskOutcome::Skipped { reason: "meta robots noindex".into() });
        }

        if !unchanged {
            self.sink
                .write(
                    job,
                    &IngestPage {
                        url: fetched.final_url.clone(),
                        title: page.title.clone(),
                        markdown: page.markdown.clone(),
                    },
                )
                .await?;
        }

        let discovered = plan_discoveries(
            &page.links,
            &self.matcher,
            task.depth.saturating_add(1),
            job.max_depth,
        );

        Ok(TaskOutcome::Fetched {
            http_status: fetched.status,
            content_hash: page.content_hash,
            etag: fetched.etag,
            last_modified: fetched.last_modified,
            discovered,
        })
    }
}

/// A URL the guard rejected will be rejected identically next time; a timeout
/// or DNS blip will not.
fn is_retryable_transport(err: &CrawlError) -> bool {
    !matches!(err, CrawlError::UrlRejected(_))
        && !err.to_string().contains("private or local address")
        && !err.to_string().contains("only http(s)")
}

/// Convenience for callers that want the default local stack.
pub fn local_executor(
    fetcher: Arc<dyn CrawlFetcher>,
    politeness: Arc<Politeness>,
    sink: Arc<dyn CrawlSinkWriter>,
    matcher: Arc<ScopeMatcher>,
) -> Arc<dyn CrawlExecutor> {
    Arc::new(LocalExecutor::new(fetcher, politeness, sink, matcher))
}

/// Lease renewals happen at half the lease so one lost renewal is survivable.
pub fn renew_interval(lease_ms: i64) -> Duration {
    Duration::from_millis((lease_ms.max(2) / 2) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::FetchOutput;
    use crate::model::{CrawlScope, CrawlSink, JobStatus, RenderMode, TaskStatus};
    use crate::politeness::{RobotsFetch, RobotsSource};
    use nomifun_common::{CrawlJobId, CrawlTaskId, UserId};
    use std::sync::Mutex;

    struct StubFetcher(Mutex<Vec<Result<FetchOutput, CrawlError>>>);

    #[async_trait::async_trait]
    impl CrawlFetcher for StubFetcher {
        async fn fetch(
            &self,
            _url: &str,
            _validators: &Validators,
            _mode: RenderMode,
        ) -> Result<FetchOutput, CrawlError> {
            self.0.lock().unwrap().remove(0)
        }
    }

    struct AllowAll;

    #[async_trait::async_trait]
    impl RobotsSource for AllowAll {
        async fn fetch(&self, _u: &str) -> RobotsFetch {
            RobotsFetch::Unavailable
        }
    }

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<IngestPage>>);

    #[async_trait::async_trait]
    impl CrawlSinkWriter for RecordingSink {
        async fn write(
            &self,
            _job: &CrawlJob,
            page: &IngestPage,
        ) -> Result<Option<crate::sink::IngestReceipt>, CrawlError> {
            self.0.lock().unwrap().push(page.clone());
            Ok(None)
        }
    }

    fn job() -> CrawlJob {
        CrawlJob {
            job_id: CrawlJobId::new(),
            user_id: UserId::new(),
            name: "t".into(),
            seeds: vec!["https://example.com/".into()],
            scope: CrawlScope::default(),
            max_depth: 3,
            max_urls: 100,
            render_mode: RenderMode::Http,
            concurrency: 1,
            per_host_concurrency: 1,
            delay_ms: 0,
            respect_robots: true,
            user_agent: None,
            sink: CrawlSink::default(),
            status: JobStatus::Running,
            error_detail: None,
            started_at: None,
            finished_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn task(url: &str) -> CrawlTask {
        CrawlTask {
            task_id: CrawlTaskId::new(),
            job_id: CrawlJobId::new(),
            parent_task_id: None,
            url: url.into(),
            url_fingerprint: "0".repeat(64),
            host: "example.com".into(),
            depth: 0,
            priority: 0,
            status: TaskStatus::InProgress,
            attempt_count: 1,
            claim_generation: 1,
            owner_node_id: Some("n".into()),
            claimed_at: Some(0),
            lease_expires_at: Some(0),
            http_status: None,
            content_hash: None,
            etag: None,
            last_modified: None,
            error_code: None,
            error_detail: None,
            completed_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn ok_html(body: &str) -> Result<FetchOutput, CrawlError> {
        Ok(FetchOutput {
            final_url: "https://example.com/p".into(),
            status: 200,
            content_type: Some("text/html".into()),
            etag: Some("\"v1\"".into()),
            last_modified: None,
            retry_after_ms: None,
            body: body.into(),
            truncated: false,
            wanted_render: false,
        })
    }

    fn build(
        responses: Vec<Result<FetchOutput, CrawlError>>,
        sink: Arc<RecordingSink>,
    ) -> LocalExecutor {
        let j = job();
        let matcher = Arc::new(ScopeMatcher::build(&j.scope, &j.seeds).unwrap());
        LocalExecutor::new(
            Arc::new(StubFetcher(Mutex::new(responses))),
            Arc::new(Politeness::new(Arc::new(AllowAll), "t", true, Duration::ZERO)),
            sink,
            matcher,
        )
    }

    #[tokio::test]
    async fn successful_fetch_ingests_and_reports_links() {
        let sink = Arc::new(RecordingSink::default());
        let ex = build(
            vec![ok_html(r#"<article><p>hello world body</p></article><a href="/next">n</a>"#)],
            sink.clone(),
        );
        let outcome = ex.execute(&task("https://example.com/p"), &job()).await;
        match outcome {
            TaskOutcome::Fetched { discovered, .. } => {
                assert_eq!(discovered.len(), 1);
                assert_eq!(discovered[0].depth, 1);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(sink.0.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unchanged_content_is_not_rewritten_but_links_still_flow() {
        let sink = Arc::new(RecordingSink::default());
        let html = r#"<article><p>stable body text</p></article><a href="/n">n</a>"#;
        let ex = build(vec![ok_html(html)], sink.clone());
        let mut t = task("https://example.com/p");
        t.content_hash = Some(extract::extract(html, &normalize("https://example.com/p").unwrap()).content_hash);

        let outcome = ex.execute(&t, &job()).await;
        assert!(matches!(outcome, TaskOutcome::Fetched { .. }));
        assert!(sink.0.lock().unwrap().is_empty(), "unchanged page must not be rewritten");
    }

    #[tokio::test]
    async fn not_modified_short_circuits() {
        let sink = Arc::new(RecordingSink::default());
        let mut resp = ok_html("");
        if let Ok(r) = &mut resp {
            r.status = 304;
        }
        let ex = build(vec![resp], sink.clone());
        assert!(matches!(
            ex.execute(&task("https://example.com/p"), &job()).await,
            TaskOutcome::Unchanged { http_status: 304 }
        ));
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn client_errors_are_not_retried_but_server_errors_are() {
        for (status, expect_retry) in [(404u16, false), (403, false), (429, true), (503, true)] {
            let mut resp = ok_html("");
            if let Ok(r) = &mut resp {
                r.status = status;
            }
            let ex = build(vec![resp], Arc::new(RecordingSink::default()));
            match ex.execute(&task("https://example.com/p"), &job()).await {
                TaskOutcome::Failed { retryable, .. } => {
                    assert_eq!(retryable, expect_retry, "status {status}");
                }
                other => panic!("status {status} gave {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn non_html_is_skipped_in_stage_a() {
        let mut resp = ok_html("%PDF-1.7");
        if let Ok(r) = &mut resp {
            r.content_type = Some("application/pdf".into());
        }
        let ex = build(vec![resp], Arc::new(RecordingSink::default()));
        assert!(matches!(
            ex.execute(&task("https://example.com/p"), &job()).await,
            TaskOutcome::Skipped { .. }
        ));
    }

    #[tokio::test]
    async fn meta_noindex_skips_ingestion() {
        let sink = Arc::new(RecordingSink::default());
        let ex = build(
            vec![ok_html(r#"<meta name="robots" content="noindex"><p>body</p>"#)],
            sink.clone(),
        );
        assert!(matches!(
            ex.execute(&task("https://example.com/p"), &job()).await,
            TaskOutcome::Skipped { .. }
        ));
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn offsite_links_are_not_followed() {
        let sink = Arc::new(RecordingSink::default());
        let ex = build(
            vec![ok_html(
                r#"<article><p>body copy</p></article><a href="https://elsewhere.com/x">o</a><a href="/in">i</a>"#,
            )],
            sink,
        );
        match ex.execute(&task("https://example.com/p"), &job()).await {
            TaskOutcome::Fetched { discovered, .. } => {
                assert_eq!(discovered.len(), 1);
                assert!(discovered[0].url.contains("example.com"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn depth_limit_stops_discovery() {
        let sink = Arc::new(RecordingSink::default());
        let ex = build(vec![ok_html(r#"<p>b</p><a href="/deep">d</a>"#)], sink);
        let mut j = job();
        j.max_depth = 0;
        match ex.execute(&task("https://example.com/p"), &j).await {
            TaskOutcome::Fetched { discovered, .. } => assert!(discovered.is_empty()),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn ssrf_rejection_is_not_retried() {
        let ex = build(
            vec![Err(CrawlError::UrlRejected("private address".into()))],
            Arc::new(RecordingSink::default()),
        );
        match ex.execute(&task("https://example.com/p"), &job()).await {
            TaskOutcome::Failed { retryable, .. } => assert!(!retryable),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
