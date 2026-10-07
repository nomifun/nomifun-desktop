//! Anonymous, bounded render operations owned by the application. This runtime
//! grants no Agent authority: Knowledge and the Browser Module owner call it
//! only after their own routing/Action admission. It is never a Hub lane or an
//! interactive Browser Resource.
use futures_util::FutureExt;
use nomi_browser_engine::headless_page::{self, HeadlessPageError, RenderedContent};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{sync::Semaphore, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use url::Url;
const MAX_RENDER_JOBS: usize = 16;
const QUEUE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

#[async_trait::async_trait]
pub(crate) trait RenderEngine: Send + Sync {
    fn binding(&self) -> serde_json::Value;
    async fn render(
        &self,
        url: Url,
        cancel: CancellationToken,
    ) -> Result<RenderedContent, HeadlessPageError>;
}
struct InstalledEngine {
    path: PathBuf,
    product: String,
    digest: String,
}
#[async_trait::async_trait]
impl RenderEngine for InstalledEngine {
    fn binding(&self) -> serde_json::Value {
        serde_json::json!({"browser_product":self.product,"browser_binary_digest":self.digest,"runtime_build_digest":headless_page::implementation_digest()})
    }
    async fn render(
        &self,
        url: Url,
        cancel: CancellationToken,
    ) -> Result<RenderedContent, HeadlessPageError> {
        if headless_page::binary_digest(self.path.clone()).await? != self.digest {
            return Err(HeadlessPageError::BindingChanged);
        }
        headless_page::render_content(
            self.path.clone(),
            url,
            self.product.clone(),
            "en-US".into(),
            cancel,
        )
        .await
    }
}
type RenderResult = Result<RenderedContent, HeadlessPageError>;
struct Job {
    cancel: CancellationToken,
    task: tokio::sync::Mutex<(Option<JoinHandle<RenderResult>>, Option<RenderResult>)>,
}
impl Job {
    async fn wait(&self) -> RenderResult {
        let mut task = self.task.lock().await;
        if let Some(result) = &task.1 {
            return result.clone();
        }
        let result = task
            .0
            .as_mut()
            .expect("owned render job")
            .await
            .unwrap_or(Err(HeadlessPageError::Cleanup));
        task.0.take();
        task.1 = Some(result.clone());
        result
    }
}
struct State {
    closed: bool,
    next: u64,
    jobs: BTreeMap<u64, Arc<Job>>,
}
pub struct HeadlessRenderRuntime {
    engine: Arc<dyn RenderEngine>,
    slots: Arc<Semaphore>,
    state: Mutex<State>,
}
impl HeadlessRenderRuntime {
    pub async fn from_installed_release(
        path: PathBuf,
        product: String,
    ) -> Result<Arc<Self>, HeadlessPageError> {
        let digest = headless_page::installed_release_digest(&path, &product).await?;
        Ok(Self::with_engine(Arc::new(InstalledEngine {
            path,
            product,
            digest,
        })))
    }
    pub(crate) fn with_engine(engine: Arc<dyn RenderEngine>) -> Arc<Self> {
        Arc::new(Self {
            engine,
            slots: Arc::new(Semaphore::new(2)),
            state: Mutex::new(State {
                closed: false,
                next: 0,
                jobs: BTreeMap::new(),
            }),
        })
    }
    pub(crate) fn binding(&self) -> serde_json::Value {
        let mut binding = self.engine.binding();
        binding["host_digest"] = serde_json::json!(nomifun_agent_contracts::digest_bytes(
            [
                include_bytes!("headless_render.rs").as_slice(),
                include_bytes!("router/knowledge_browser.rs").as_slice()
            ]
            .concat()
            .as_slice()
        ));
        binding
    }
    pub(crate) async fn render(self: &Arc<Self>, url: Url) -> RenderResult {
        // Reap completed jobs whose callers disappeared; each handle remains
        // owned until joined, rather than accumulating finished tasks forever.
        let prior: Vec<_> = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .jobs
            .iter()
            .map(|(id, job)| (*id, job.clone()))
            .collect();
        for (id, job) in prior {
            let finished = job.task.try_lock().ok().is_some_and(|task| {
                task.1.is_some() || task.0.as_ref().is_some_and(JoinHandle::is_finished)
            });
            if finished {
                let result = job.wait().await;
                self.retire(id, &result);
            }
        }
        struct CancelOnDrop(CancellationToken);
        impl Drop for CancelOnDrop {
            fn drop(&mut self) {
                self.0.cancel();
            }
        }
        let cancel = CancellationToken::new();
        let _cancel = CancelOnDrop(cancel.clone());
        let (id, job) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(HeadlessPageError::Canceled);
            }
            if state.jobs.len() >= MAX_RENDER_JOBS {
                return Err(HeadlessPageError::Unavailable);
            }
            state.next = state
                .next
                .checked_add(1)
                .ok_or(HeadlessPageError::Unavailable)?;
            let id = state.next;
            let engine = self.engine.clone();
            let slots = self.slots.clone();
            let child_cancel = cancel.clone();
            let owner = Arc::downgrade(self);
            let task = tokio::spawn(async move {
                let _permit = tokio::select! {biased;
                    _=child_cancel.cancelled()=>return Err(HeadlessPageError::Canceled),
                    result=tokio::time::timeout(QUEUE_DEADLINE,slots.acquire_owned())=>result.map_err(|_|HeadlessPageError::Timeout)?.map_err(|_|HeadlessPageError::Unavailable)?,
                };
                let result = std::panic::AssertUnwindSafe(engine.render(url, child_cancel))
                    .catch_unwind()
                    .await
                    .unwrap_or(Err(HeadlessPageError::Cleanup));
                // Fence and cancel waiting jobs before releasing this permit.
                // The caller may already have disappeared; it cannot own this step.
                if matches!(result, Err(HeadlessPageError::Cleanup)) {
                    if let Some(owner) = owner.upgrade() {
                        owner.retire(id, &result);
                    }
                }
                result
            });
            let job = Arc::new(Job {
                cancel,
                task: tokio::sync::Mutex::new((Some(task), None)),
            });
            state.jobs.insert(id, job.clone());
            (id, job)
        };
        let result = job.wait().await;
        self.retire(id, &result);
        result
    }
    fn retire(&self, id: u64, result: &RenderResult) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(result, Err(HeadlessPageError::Cleanup)) {
            state.closed = true;
            for job in state.jobs.values() {
                job.cancel.cancel();
            }
        } else {
            state.jobs.remove(&id);
        }
    }
    pub(crate) async fn shutdown(&self) -> Result<(), HeadlessPageError> {
        let jobs: Vec<_> = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            state
                .jobs
                .iter()
                .map(|(id, job)| (*id, job.clone()))
                .collect()
        };
        for (_, job) in &jobs {
            job.cancel.cancel();
        }
        let mut failed = false;
        for (id, job) in jobs {
            let result = job.wait().await;
            failed |= matches!(result, Err(HeadlessPageError::Cleanup));
            self.retire(id, &result);
        }
        if failed {
            Err(HeadlessPageError::Cleanup)
        } else {
            Ok(())
        }
    }
}

impl Drop for HeadlessRenderRuntime {
    fn drop(&mut self) {
        for job in self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .jobs
            .values()
        {
            job.cancel.cancel();
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Recorded(Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl RenderEngine for Recorded {
        fn binding(&self) -> serde_json::Value {
            serde_json::json!({"fixture":"recorded-render"})
        }
        async fn render(&self, url: Url, _cancel: CancellationToken) -> RenderResult {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(RenderedContent {final_url:url.to_string(),html:"<html><title>Canonical render</title><body>Rendered by the selected Provider</body></html>".into(),html_truncated:false})
        }
    }
    pub(crate) fn runtime() -> (Arc<HeadlessRenderRuntime>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            HeadlessRenderRuntime::with_engine(Arc::new(Recorded(calls.clone()))),
            calls,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Waiting {
        started: tokio::sync::Notify,
        stopped: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl RenderEngine for Waiting {
        fn binding(&self) -> serde_json::Value {
            serde_json::json!({"fixture":"waiting"})
        }
        async fn render(&self, _url: Url, cancel: CancellationToken) -> RenderResult {
            self.started.notify_one();
            cancel.cancelled().await;
            self.stopped.fetch_add(1, Ordering::SeqCst);
            Err(HeadlessPageError::Canceled)
        }
    }
    #[tokio::test]
    async fn bounded_admission_and_shutdown_join_owned_render_jobs() {
        let engine = Arc::new(Waiting {
            started: tokio::sync::Notify::new(),
            stopped: AtomicUsize::new(0),
        });
        let runtime = HeadlessRenderRuntime::with_engine(engine.clone());
        let mut jobs = vec![];
        for index in 0..MAX_RENDER_JOBS {
            let owner = runtime.clone();
            jobs.push(tokio::spawn(async move {
                owner
                    .render(Url::parse("https://example.com/").unwrap())
                    .await
            }));
            if index < 2 {
                engine.started.notified().await;
            }
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.state.lock().unwrap().jobs.len() < MAX_RENDER_JOBS {
            assert!(
                tokio::time::Instant::now() < deadline,
                "queued jobs were not admitted"
            );
            tokio::task::yield_now().await;
        }
        assert!(matches!(
            runtime
                .render(Url::parse("https://example.com/").unwrap())
                .await,
            Err(HeadlessPageError::Unavailable)
        ));
        runtime.shutdown().await.unwrap();
        for job in jobs {
            let _ = job.await;
        }
        assert_eq!(engine.stopped.load(Ordering::SeqCst), 2);
        assert_eq!(runtime.slots.available_permits(), 2);
        assert!(runtime.state.lock().unwrap().jobs.is_empty());
        assert!(matches!(
            runtime
                .render(Url::parse("https://example.com/").unwrap())
                .await,
            Err(HeadlessPageError::Canceled)
        ));
    }
    #[tokio::test]
    async fn caller_abort_retains_the_job_until_cleanup_is_joined() {
        let engine = Arc::new(Waiting {
            started: tokio::sync::Notify::new(),
            stopped: AtomicUsize::new(0),
        });
        let runtime = HeadlessRenderRuntime::with_engine(engine.clone());
        let owner = runtime.clone();
        let caller = tokio::spawn(async move {
            owner
                .render(Url::parse("https://example.com/").unwrap())
                .await
        });
        engine.started.notified().await;
        caller.abort();
        let _ = caller.await;
        runtime.shutdown().await.unwrap();
        assert_eq!(engine.stopped.load(Ordering::SeqCst), 1);
        assert!(runtime.state.lock().unwrap().jobs.is_empty());
    }
    #[tokio::test]
    async fn ordinary_four_source_batches_wait_instead_of_failing_admission() {
        struct Timed {
            active: AtomicUsize,
            peak: AtomicUsize,
        }
        #[async_trait::async_trait]
        impl RenderEngine for Timed {
            fn binding(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            async fn render(&self, url: Url, _cancel: CancellationToken) -> RenderResult {
                let count = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(count, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                self.active.fetch_sub(1, Ordering::SeqCst);
                Ok(RenderedContent {
                    final_url: url.to_string(),
                    html: "ready".into(),
                    html_truncated: false,
                })
            }
        }
        let engine = Arc::new(Timed {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let runtime = HeadlessRenderRuntime::with_engine(engine.clone());
        let mut tasks = vec![];
        for _ in 0..4 {
            let owner = runtime.clone();
            tasks.push(tokio::spawn(async move {
                owner
                    .render(Url::parse("https://example.com/").unwrap())
                    .await
            }));
        }
        for task in tasks {
            assert!(task.await.unwrap().is_ok());
        }
        assert_eq!(engine.peak.load(Ordering::SeqCst), 2);
        runtime.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn cleanup_failure_or_panic_cancels_active_and_queued_jobs_before_slot_release() {
        struct Failing {
            started: tokio::sync::Notify,
            fail: tokio::sync::Notify,
            calls: AtomicUsize,
            panic: bool,
        }
        #[async_trait::async_trait]
        impl RenderEngine for Failing {
            fn binding(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            async fn render(&self, _url: Url, cancel: CancellationToken) -> RenderResult {
                let index = self.calls.fetch_add(1, Ordering::SeqCst);
                self.started.notify_one();
                if index == 0 {
                    self.fail.notified().await;
                    assert!(!self.panic, "fixture engine panic");
                    Err(HeadlessPageError::Cleanup)
                } else {
                    cancel.cancelled().await;
                    Err(HeadlessPageError::Canceled)
                }
            }
        }
        for panic in [false, true] {
            let engine = Arc::new(Failing {
                started: tokio::sync::Notify::new(),
                fail: tokio::sync::Notify::new(),
                calls: AtomicUsize::new(0),
                panic,
            });
            let runtime = HeadlessRenderRuntime::with_engine(engine.clone());
            let mut callers = vec![];
            for index in 0..3 {
                let owner = runtime.clone();
                callers.push(tokio::spawn(async move {
                    owner
                        .render(Url::parse("https://example.com/").unwrap())
                        .await
                }));
                if index < 2 {
                    engine.started.notified().await;
                }
            }
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while runtime.state.lock().unwrap().jobs.len() < 3 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            engine.fail.notify_one();
            for (index, caller) in callers.into_iter().enumerate() {
                let result = tokio::time::timeout(std::time::Duration::from_secs(2), caller)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    result.unwrap_err(),
                    if index == 0 {
                        HeadlessPageError::Cleanup
                    } else {
                        HeadlessPageError::Canceled
                    }
                );
            }
            assert_eq!(engine.calls.load(Ordering::SeqCst), 2);
            assert_eq!(runtime.slots.available_permits(), 2);
            assert_eq!(runtime.shutdown().await, Err(HeadlessPageError::Cleanup));
            assert_eq!(runtime.state.lock().unwrap().jobs.len(), 1);
        }
    }
    #[tokio::test]
    async fn cleanup_failure_is_not_retired_as_success() {
        struct Failed;
        #[async_trait::async_trait]
        impl RenderEngine for Failed {
            fn binding(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            async fn render(&self, _url: Url, _cancel: CancellationToken) -> RenderResult {
                Err(HeadlessPageError::Cleanup)
            }
        }
        let runtime = HeadlessRenderRuntime::with_engine(Arc::new(Failed));
        assert!(matches!(
            runtime
                .render(Url::parse("https://example.com/").unwrap())
                .await,
            Err(HeadlessPageError::Cleanup)
        ));
        assert!(runtime.state.lock().unwrap().closed);
        assert_eq!(runtime.shutdown().await, Err(HeadlessPageError::Cleanup));
        assert_eq!(runtime.state.lock().unwrap().jobs.len(), 1);
    }
}
