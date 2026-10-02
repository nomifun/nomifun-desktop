//! Robot gateway: LAN-attached physical robots (xiaozhi firmware) acting as the
//! physical embodiment of a desktop companion.
//!
//! Byte sources are abstracted behind [`link::RobotLinkSource`] so a future
//! public relay reuses the same session core; model capabilities sit behind the
//! [`services`] trait seam so the pipeline is testable with mocks.

pub mod audio;
pub mod capability;
pub mod dto;
pub mod endpoint;
pub mod effect_ledger;
pub mod events;
pub mod lan_source;
pub mod link;
mod lifecycle;
pub mod mcp_bridge;
pub mod pipeline;
pub mod protocol;
pub mod registry;
pub mod routes;
pub mod services;
pub mod session;
pub mod status;
pub mod tool_registry;
pub mod vad;
pub mod vision;
pub mod wiring;

use std::sync::{Arc, Mutex};
use futures_util::FutureExt;

/// Domain name used in log fields and event prefixes.
pub fn robot_domain_name() -> &'static str {
    "robot"
}

/// Owns source, session and session-child task completion, not only ingress.
pub struct RobotGateway {
    deps: session::SessionDeps,
    gate: Mutex<(bool, bool)>, // started, closing
    stop: tokio::sync::watch::Sender<bool>,
    completion: tokio::sync::watch::Sender<Option<Result<(), String>>>,
    tasks: Arc<lifecycle::OwnedTasks>,
}

impl RobotGateway {
    pub fn new(deps: session::SessionDeps) -> Self {
        Self { deps, gate: Mutex::new((false, false)), stop: tokio::sync::watch::channel(false).0,
            completion: tokio::sync::watch::channel(None).0, tasks: Arc::new(Default::default()) }
    }

    /// Register the retained worker synchronously, before the caller can be
    /// cancelled. A waiter never owns an abort handle for this worker.
    pub fn start(self: &Arc<Self>, sources: Vec<Arc<dyn link::RobotLinkSource>>) {
        let mut gate = match self.gate.lock() {
            Ok(gate) => gate,
            Err(poisoned) => {
                self.tasks.record_error("Robot gateway admission lock was poisoned".into());
                poisoned.into_inner()
            }
        };
        if gate.0 || gate.1 { return; }
        gate.0 = true;
        let owner = self.clone();
        tokio::spawn(async move {
            let result = std::panic::AssertUnwindSafe(owner.run_owned(sources)).catch_unwind().await;
            if result.is_err() { owner.tasks.record_error("Robot gateway worker panicked; cleanup remains failed".into()); }
            owner.request_shutdown();
            let joined = owner.tasks.abort_and_join().await;
            owner.completion.send_replace(Some(joined));
        });
    }

    /// Compatibility waiter. Aborting it does not drop source/session owners.
    pub async fn serve(self: Arc<Self>, sources: Vec<Arc<dyn link::RobotLinkSource>>) {
        self.start(sources);
        if let Err(error) = self.wait_completion().await { tracing::error!(%error, "robot gateway stopped uncleanly"); }
    }

    pub fn request_shutdown(&self) {
        let (mut gate, poisoned) = match self.gate.lock() {
            Ok(gate) => (gate, false),
            Err(poisoned) => {
                self.tasks.record_error("Robot gateway admission lock was poisoned".into());
                (poisoned.into_inner(), true)
            }
        };
        gate.1 = true;
        self.stop.send_replace(true);
        if !gate.0 {
            self.completion.send_replace(Some(if poisoned {
                Err("Robot gateway admission lock was poisoned".into())
            } else { Ok(()) }));
        }
    }

    /// A bounded wait, not timeout-as-cleanup. The worker and its immutable
    /// completion remain retained after a timed-out or cancelled caller.
    pub async fn shutdown_and_wait(&self) -> Result<(), String> {
        self.request_shutdown();
        tokio::time::timeout(std::time::Duration::from_secs(5), self.wait_completion()).await
            .map_err(|_| "Robot task shutdown is still pending after 5 seconds".to_owned())?
    }

    async fn wait_completion(&self) -> Result<(), String> {
        let mut completion = self.completion.subscribe();
        loop {
            if let Some(result) = completion.borrow_and_update().clone() { return result; }
            completion.changed().await.map_err(|_| "Robot shutdown completion was lost".to_owned())?;
        }
    }

    async fn run_owned(&self, sources: Vec<Arc<dyn link::RobotLinkSource>>) {
        if *self.stop.borrow() { return; }
        let (tx, mut rx) = tokio::sync::mpsc::channel::<link::AcceptedLink>(8);
        let mut source_tasks = Vec::new();
        let mut sources_join = tokio::task::JoinSet::new();
        let mut sessions_join = tokio::task::JoinSet::new();
        let mut stop = self.stop.subscribe();
        for source in sources {
            let tx = tx.clone();
            let name = source.name();
            let tasks = self.tasks.clone();
            let task = self.tasks.spawn(async move {
                if let Err(error) = source.run(tx).await {
                    tracing::error!(source = name, %error, "robot: link source stopped");
                    tasks.record_error(format!("Robot source {name} failed: {error}"));
                }
            });
            source_tasks.push(task.clone());
            sources_join.spawn(async move { task.join().await });
        }
        drop(tx);
        loop {
            if *stop.borrow_and_update() { break; }
            tokio::select! {
                biased;
                changed = stop.changed() => { if changed.is_err() { self.tasks.record_error("Robot stop channel was lost".into()); } break; }
                result = sources_join.join_next(), if !sources_join.is_empty() => {
                    if let Some(Err(error)) = result { self.tasks.record_error(format!("Robot source join failed: {error}")); }
                }
                result = sessions_join.join_next(), if !sessions_join.is_empty() => {
                    if let Some(Err(error)) = result { self.tasks.record_error(format!("Robot session join failed: {error}")); }
                }
                link = rx.recv() => {
                    let Some(link) = link else { break; };
                    let deps = self.deps.clone();
                    let tasks = self.tasks.clone();
                    let session_tasks = tasks.clone();
                    let stop = self.stop.subscribe();
                    let task = self.tasks.spawn(async move {
                        if let Err(error) = session::run_session_owned(link, deps, stop, session_tasks).await { tasks.record_error(error); }
                    });
                    sessions_join.spawn(async move { task.join().await });
                }
            }
        }
        self.request_shutdown();
        rx.close();
        drop(rx); // Drop queued links; no new session can be admitted.
        for task in source_tasks { task.abort(); }
        while let Some(result) = sources_join.join_next().await {
            if let Err(error) = result { self.tasks.record_error(format!("Robot source join failed: {error}")); }
        }
        // Sessions observe stop and run their owned tail; do not abort them
        // merely to obtain a fast result while their children still write.
        while let Some(result) = sessions_join.join_next().await {
            if let Err(error) = result { self.tasks.record_error(format!("Robot session join failed: {error}")); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use crate::link::{AcceptedLink, Frame, LinkError, RobotIdentity, RobotLinkSink, RobotLinkSource, RobotLinkStream};

    struct NullEvents;
    impl nomifun_realtime::UserEventSink for NullEvents {
        fn send_to_user(&self, _: &str, _: nomifun_api_types::WebSocketMessage<serde_json::Value>) {}
    }

    async fn gateway() -> (tempfile::TempDir, Arc<RobotGateway>) {
        let directory = tempfile::tempdir().unwrap();
        let registry = Arc::new(registry::RobotRegistry::load(directory.path()).await.unwrap());
        let status = Arc::new(status::RobotStatusRegistry::new(
            events::RobotEventEmitter::new(Arc::new(NullEvents)), "owner".into()));
        let gateway = Arc::new(RobotGateway::new(session::SessionDeps {
            registry, status,
            speech: Arc::new(services::mock::MockSpeech::new()),
            dispatcher: Arc::new(services::mock::MockDispatcher::new()),
            tools: Arc::new(tool_registry::RobotToolRegistry::default()),
        }));
        (directory, gateway)
    }

    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped { fn drop(&mut self) { self.0.store(true, Ordering::Release); } }

    struct HeldSink {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Semaphore>,
        dropped: Arc<AtomicBool>,
    }
    impl Drop for HeldSink { fn drop(&mut self) { self.dropped.store(true, Ordering::Release); } }
    #[async_trait::async_trait]
    impl RobotLinkSink for HeldSink {
        async fn send(&mut self, _: Frame) -> Result<(), LinkError> { Ok(()) }
        async fn close(&mut self) {
            self.entered.notify_one();
            self.release.acquire().await.unwrap().forget();
        }
    }
    struct IdleStream;
    #[async_trait::async_trait]
    impl RobotLinkStream for IdleStream {
        async fn next(&mut self) -> Option<Result<Frame, LinkError>> { std::future::pending().await }
    }
    struct HeldSource {
        link: Mutex<Option<AcceptedLink>>,
        dropped: Arc<AtomicBool>,
    }
    #[async_trait::async_trait]
    impl RobotLinkSource for HeldSource {
        fn name(&self) -> &'static str { "held-native-task-fixture" }
        async fn run(self: Arc<Self>, accept: tokio::sync::mpsc::Sender<AcceptedLink>) -> anyhow::Result<()> {
            let _drop = Dropped(self.dropped.clone());
            let link = self.link.lock().unwrap().take().unwrap();
            accept.send(link).await.map_err(|_| anyhow::anyhow!("accept closed"))?;
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn robot_shutdown_joins_source_and_session_children_after_waiter_drop() {
        let (_directory, gateway) = gateway().await;
        let source_dropped = Arc::new(AtomicBool::new(false));
        let sink_dropped = Arc::new(AtomicBool::new(false));
        let close_entered = Arc::new(tokio::sync::Notify::new());
        let close_release = Arc::new(tokio::sync::Semaphore::new(0));
        let source = Arc::new(HeldSource {
            link: Mutex::new(Some(AcceptedLink {
                identity: RobotIdentity { robot_id: "fixture".into(), client_id: "fixture".into(), peer: "isolated".into(), vision_base: None, device_token: String::new() },
                sink: Box::new(HeldSink { entered: close_entered.clone(), release: close_release.clone(), dropped: sink_dropped.clone() }),
                stream: Box::new(IdleStream),
            })),
            dropped: source_dropped.clone(),
        });
        gateway.start(vec![source]);
        // Wait until the writer has been registered, without requesting stop
        // before the test link was admitted.
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if gateway.tasks.task_count() >= 4 { break; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        let waiter_owner = gateway.clone();
        let waiter = tokio::spawn(async move { waiter_owner.shutdown_and_wait().await });
        tokio::time::timeout(std::time::Duration::from_secs(2), close_entered.notified()).await.unwrap();
        assert!(!waiter.is_finished());
        assert!(source_dropped.load(Ordering::Acquire));
        assert!(!sink_dropped.load(Ordering::Acquire));
        waiter.abort();
        let _ = waiter.await;
        assert!(gateway.completion.borrow().is_none(), "dropped waiter is not cleanup proof");
        close_release.add_permits(1);
        gateway.shutdown_and_wait().await.unwrap();
        assert!(sink_dropped.load(Ordering::Acquire));
        gateway.shutdown_and_wait().await.unwrap();
    }

    struct PanickingSource;
    #[async_trait::async_trait]
    impl RobotLinkSource for PanickingSource {
        fn name(&self) -> &'static str { "panic-fixture" }
        async fn run(self: Arc<Self>, _: tokio::sync::mpsc::Sender<AcceptedLink>) -> anyhow::Result<()> { panic!("controlled source panic") }
    }

    #[tokio::test]
    async fn robot_shutdown_retains_source_panic_as_failed_completion() {
        let (_directory, gateway) = gateway().await;
        gateway.start(vec![Arc::new(PanickingSource)]);
        tokio::time::timeout(std::time::Duration::from_secs(2), gateway.wait_completion()).await.unwrap().unwrap_err();
        assert!(gateway.shutdown_and_wait().await.is_err());
        assert!(gateway.shutdown_and_wait().await.is_err());
    }

    #[test]
    fn domain_name_is_robot() {
        assert_eq!(robot_domain_name(), "robot");
    }
}
