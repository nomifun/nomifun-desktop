// Real Terminal storage/hooks behind the host's retained shutdown worker.
// Only the unrelated Agent producer's completion is controlled.
use super::*;
use nomifun_api_types::CreateTerminalRequest;
use nomifun_common::OnTerminalDelete;
use tokio::sync::{Notify, Semaphore};

struct DeleteBarrier {
    calls: AtomicUsize,
    completed: AtomicUsize,
    cancelled: Arc<AtomicUsize>,
    observed: StdMutex<Vec<(String, String)>>,
    entered: Notify,
    released: Notify,
    release: Semaphore,
}

impl Default for DeleteBarrier {
    fn default() -> Self {
        Self { calls: 0.into(), completed: 0.into(), cancelled: Arc::new(0.into()),
            observed: Default::default(), entered: Notify::new(), released: Notify::new(), release: Semaphore::new(0) }
    }
}

struct DeleteContinuation {
    cancelled: Arc<AtomicUsize>,
    completed: bool,
}

impl Drop for DeleteContinuation {
    fn drop(&mut self) {
        if !self.completed { self.cancelled.fetch_add(1, Ordering::SeqCst); }
    }
}

#[async_trait::async_trait]
impl OnTerminalDelete for DeleteBarrier {
    async fn on_terminal_deleted(&self, owner: &str, terminal: &str) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.observed.lock().unwrap().push((owner.to_owned(), terminal.to_owned()));
        let mut continuation = DeleteContinuation { cancelled: self.cancelled.clone(), completed: false };
        self.entered.notify_one();
        self.release.acquire().await.unwrap().forget();
        continuation.completed = true;
        self.completed.fetch_add(1, Ordering::SeqCst);
        self.released.notify_one();
    }
}

struct BlockedShutdownRegistry {
    inner: Arc<dyn AgentRuntimeSessions>,
    entered: Arc<Notify>,
    release: Arc<Semaphore>,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl AgentRuntimeSessions for BlockedShutdownRegistry {
    fn get_runtime(&self, session: &str) -> Option<nomifun_ai_agent::AgentRuntimeHandle> {
        self.inner.get_runtime(session)
    }
    async fn get_or_create_runtime(&self, session: &str, options: nomifun_ai_agent::types::AgentRuntimeBuildOptions)
        -> Result<nomifun_ai_agent::AgentRuntimeHandle, nomifun_common::AppError> {
        self.inner.get_or_create_runtime(session, options).await
    }
    fn terminate(&self, session: &str, reason: Option<nomifun_common::AgentKillReason>) -> Result<(), nomifun_common::AppError> {
        self.inner.terminate(session, reason)
    }
    fn terminate_all(&self) { self.inner.terminate_all(); }
    fn active_runtime_count(&self) -> usize { self.inner.active_runtime_count() }
    fn shutdown_and_wait(&self) -> nomifun_ai_agent::RuntimeTeardown {
        // Preserve the real registry's immediate admission fence and cleanup
        // owner. This barrier delays acknowledgement, never supplies proof.
        let original = self.inner.shutdown_and_wait();
        self.calls.fetch_add(1, Ordering::SeqCst);
        let entered = self.entered.clone();
        let release = self.release.clone();
        Box::pin(async move {
            entered.notify_one();
            release.acquire().await.unwrap().forget();
            original.await
        })
    }
}

fn deferred_request(root: &Path) -> CreateTerminalRequest {
    serde_json::from_value(serde_json::json!({
        "name":"Original deferred terminal", "cwd":root, "command":std::env::current_exe().unwrap(),
        "args":[], "defer_spawn":true, "cols":80, "rows":24
    })).unwrap()
}

async fn add_deferred_terminal(services: &AppServices, root: &Path, hook: &Arc<DeleteBarrier>) -> String {
    let response = services.terminal_service.create(services.authoritative_user_id.as_ref(), deferred_request(root))
        .await.unwrap();
    services.terminal_service.with_delete_hook(hook.clone());
    let terminal = response.terminal_id.to_string();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM terminal_sessions WHERE terminal_id=?")
        .bind(&terminal).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(count, 1, "this is the original real repository row, not a mock cleanup result");
    terminal
}

async fn assert_terminal_admission_closed(services: &AppServices, root: &Path) {
    let rejected = services.terminal_service.create(services.authoritative_user_id.as_ref(), deferred_request(root)).await.unwrap_err();
    assert!(matches!(rejected, nomifun_terminal::error::TerminalError::ShuttingDown),
        "the original service admission must already be fenced: {rejected}");
}

#[cfg(feature = "browser-use")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocked_original_mcp_auth_owner_does_not_delay_terminal_or_browser_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let mut services = AppServices::from_config(database, &test_config(directory.path())).await.unwrap();
    let hook = Arc::new(DeleteBarrier::default());
    add_deferred_terminal(&services, directory.path(), &hook).await;
    let browser = Arc::new(BrowserResourceShutdownProbe::default());
    browser.allow_close.store(true, Ordering::Release);
    let _workspace = install_shutdown_browser(&mut services, browser.clone()).await;

    // Hold the real authority's existing per-connection observation guard.
    // The original logout is admitted, then waits on that exact guard; its
    // activity token makes the actual McpAuthService::shutdown remain pending.
    let server = uuid::Uuid::now_v7().to_string();
    let revision = services.mcp_auth.binding_revision(&server).await.unwrap();
    let observation = services.mcp_auth.acquire_observation_guard(&server, &revision).await.unwrap();
    let auth = services.mcp_auth.clone();
    let mut logout = Box::pin(async move { auth.logout(&server).await });
    let initial = std::future::poll_fn(|context| std::task::Poll::Ready(
        std::future::Future::poll(logout.as_mut(), context),
    )).await;
    assert!(initial.is_pending(), "the original logout must already own its activity token");
    let logout = tokio::spawn(logout);
    let services = Arc::new(services);
    let closing = services.clone();
    let shutdown = tokio::spawn(async move { closing.shutdown_nomi_core_host().await });
    tokio::time::timeout(Duration::from_secs(5), hook.entered.notified()).await
        .expect("the real Terminal delete hook must run while authentication still joins");
    tokio::time::timeout(Duration::from_secs(5), browser.close_entered.notified()).await
        .expect("the original Browser Resource must close independently of authentication");
    assert_terminal_admission_closed(&services, directory.path()).await;
    assert!(!logout.is_finished());
    assert!(!shutdown.is_finished());
    assert!(services.mcp_auth.binding_revision("after-auth-fence").await.is_err());
    assert_eq!(browser.closes.load(Ordering::Acquire), 1);
    assert_eq!(browser.native_shutdowns.load(Ordering::Acquire), 0);
    assert!(!services.database.pool().is_closed());
    hook.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), hook.released.notified()).await.unwrap();

    // Dropping the host waiter must preserve the same auth shutdown worker,
    // not run another shutdown that can lose the first service's task handles.
    shutdown.abort();
    assert!(shutdown.await.unwrap_err().is_cancelled());
    let original = services.host_shutdown_stage.lock().await.mcp_auth_task.as_ref().unwrap().id();
    assert!(!services.host_shutdown_stage.lock().await.mcp_auth_task.as_ref().unwrap().is_finished());
    drop(observation);
    assert!(logout.await.unwrap().is_err(), "shutdown rejects the late logout's nested admission");
    services.shutdown_nomi_core_host().await.unwrap();
    let stage = services.host_shutdown_stage.lock().await;
    assert!(stage.mcp_auth_closed);
    assert!(stage.mcp_auth_task.is_none(), "the original worker {original:?} was joined");
    assert_eq!(hook.calls.load(Ordering::SeqCst), 1);
    assert_eq!(hook.cancelled.load(Ordering::SeqCst), 0);
    assert_eq!(browser.closes.load(Ordering::Acquire), 1);
    assert_eq!(browser.native_shutdowns.load(Ordering::Acquire), 1);
    assert!(services.database.pool().is_closed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn host_terminal_cleanup_reaches_real_delete_hook_before_a_blocked_runtime_owner_finishes() {
    let directory = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let mut services = AppServices::from_config(database, &test_config(directory.path())).await.unwrap();
    let hook = Arc::new(DeleteBarrier::default());
    let terminal = add_deferred_terminal(&services, directory.path(), &hook).await;
    let producer = Arc::new(BlockedShutdownRegistry { inner:services.agent_runtime_sessions.clone(),
        entered:Arc::new(Notify::new()), release:Arc::new(Semaphore::new(0)), calls:0.into() });
    services.agent_runtime_sessions = producer.clone();
    let services = Arc::new(services);
    let closing = services.clone();
    let shutdown = tokio::spawn(async move { closing.shutdown_nomi_core_host().await });
    tokio::time::timeout(Duration::from_secs(5), producer.entered.notified()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), hook.entered.notified()).await
        .expect("Terminal must execute its real delete_all and hooks while the unrelated Runtime acknowledgement is still blocked");
    assert!(!shutdown.is_finished());
    assert_eq!(producer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(hook.calls.load(Ordering::SeqCst), 1);
    assert_eq!(*hook.observed.lock().unwrap(), [(services.authoritative_user_id.to_string(), terminal.clone())]);
    assert_terminal_admission_closed(&services, directory.path()).await;
    assert!(!services.database.pool().is_closed(), "pending owners cannot authorize storage close");
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM terminal_sessions WHERE terminal_id=?")
        .bind(&terminal).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(rows, 0, "the original Terminal repository deletion has actually committed before the hook");
    hook.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), hook.released.notified()).await.unwrap();
    assert!(!shutdown.is_finished(), "finishing Terminal alone is not Agent cleanup proof");
    assert!(!services.database.pool().is_closed());
    producer.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(15), shutdown).await.unwrap().unwrap().unwrap();
    assert_eq!(hook.completed.load(Ordering::SeqCst), 1);
    assert_eq!(hook.cancelled.load(Ordering::SeqCst), 0);
    assert!(services.database.pool().is_closed());
    services.shutdown_nomi_core_host().await.unwrap();
    assert_eq!(producer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(hook.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn terminal_shutdown_timeout_and_dropped_waiter_keep_the_same_real_hook_continuation() {
    let directory = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(database, &test_config(directory.path())).await.unwrap();
    let hook = Arc::new(DeleteBarrier::default());
    add_deferred_terminal(&services, directory.path(), &hook).await;
    let mut stage = HostShutdownStage::default();
    stage.start_terminal_cleanup(services.terminal_service.clone()).await;
    tokio::time::timeout(Duration::from_secs(5), hook.entered.notified()).await.unwrap();
    let original = stage.terminal_task.as_ref().unwrap().id();
    assert_terminal_admission_closed(&services, directory.path()).await;

    // Pause only after SQLite has committed and the real hook has entered a
    // pure semaphore wait. SQLx worker I/O is never timed by a virtual clock.
    tokio::time::pause();
    let mut observing = Box::pin(stage.wait_terminal_cleanup());
    assert!(futures_util::poll!(observing.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(5)).await;
    let error = observing.await.unwrap_err();
    assert!(error.contains("original worker is retained"), "{error}");
    assert_eq!(stage.terminal_task.as_ref().unwrap().id(), original);
    assert!(!stage.terminal_closed && !stage.storage_closed && !stage.complete);
    assert!(stage.storage_task.is_none());
    assert!(stage.terminal_unreported_failure.is_none(), "an observation timeout is not a permanent Terminal failure");
    {
        let mut abandoned = Box::pin(stage.wait_terminal_cleanup());
        assert!(futures_util::poll!(abandoned.as_mut()).is_pending());
        drop(abandoned);
    }
    stage.start_terminal_cleanup(services.terminal_service.clone()).await;
    assert_eq!(stage.terminal_task.as_ref().unwrap().id(), original);
    assert_eq!(hook.calls.load(Ordering::SeqCst), 1);
    assert_eq!(hook.completed.load(Ordering::SeqCst), 0);
    assert_eq!(hook.cancelled.load(Ordering::SeqCst), 0, "neither timeout nor dropped waiter can drop the original native-domain future");
    assert!(!services.database.pool().is_closed());
    hook.release.add_permits(1);
    tokio::task::yield_now().await;
    stage.wait_terminal_cleanup().await.unwrap();
    assert!(stage.terminal_closed && stage.terminal_task.is_none());
    assert_eq!(hook.completed.load(Ordering::SeqCst), 1);
    assert_eq!(hook.cancelled.load(Ordering::SeqCst), 0, "natural hook completion is not cancellation");
    stage.start_terminal_cleanup(services.terminal_service.clone()).await;
    stage.wait_terminal_cleanup().await.unwrap();
    assert!(stage.terminal_task.is_none());
    assert_eq!(hook.calls.load(Ordering::SeqCst), 1, "successful retry only joins the original worker");
    assert!(!stage.storage_closed && !stage.complete);
    tokio::time::resume();
    services.shutdown_nomi_core_host().await.unwrap();
    assert!(services.database.pool().is_closed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_sqlite_writer_lock_retains_terminal_row_and_both_domain_failures_until_explicit_retry() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let path = config.database_path();
    let database = nomifun_db::init_database(&path).await.unwrap();
    let mut services = AppServices::from_config(database, &config).await.unwrap();
    let hook = Arc::new(DeleteBarrier::default());
    let terminal = add_deferred_terminal(&services, directory.path(), &hook).await;
    let allow = Arc::new(AtomicBool::new(false));
    services.agent_runtime_sessions = Arc::new(ShutdownRegistryProbe { inner:services.agent_runtime_sessions.clone(), allow:allow.clone() });
    let lock_pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(&path).busy_timeout(Duration::from_secs(5))
    ).await.unwrap();
    let mut writer = lock_pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("UPDATE terminal_sessions SET name=name WHERE terminal_id=?").bind(&terminal).execute(&mut *writer).await.unwrap();
    let failure = tokio::time::timeout(Duration::from_secs(20), services.shutdown_nomi_core_host()).await.unwrap().unwrap_err().to_string();
    assert!(failure.contains("Agent runtime cleanup failed") && failure.contains("Agent exit is not proven"), "{failure}");
    assert!(failure.contains("terminal cleanup") && (failure.contains("locked") || failure.contains("timed out")), "{failure}");
    assert!(!services.database.pool().is_closed());
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM terminal_sessions WHERE terminal_id=?")
        .bind(&terminal).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(rows, 1, "failure cannot erase the Terminal's original management row");
    assert_eq!(hook.calls.load(Ordering::SeqCst), 0, "a failed real delete_all cannot issue a fictitious deletion hook");
    {
        let stage = services.host_shutdown_stage.lock().await;
        assert!(!stage.terminal_closed && !stage.storage_closed && !stage.complete);
        assert!(stage.storage_task.is_none());
        assert!(stage.terminal_unreported_failure.is_none(), "the actual aggregate error returned the original Terminal cause to this caller");
    }
    writer.rollback().await.unwrap();
    lock_pool.close().await;
    allow.store(true, Ordering::SeqCst);
    hook.release.add_permits(1);
    // If the observation deadline won the same instant as SQLITE_BUSY, one
    // explicit retry first consumes that retained failed result. It must stay
    // visible; only the next explicit retry starts a new cleanup attempt.
    let mut completed = false;
    for _ in 0..2 {
        match tokio::time::timeout(Duration::from_secs(15), services.shutdown_nomi_core_host()).await.unwrap() {
            Ok(()) => { completed = true; break; }
            Err(error) => {
                let cause = error.to_string();
                assert!(cause.contains("terminal cleanup failed") && cause.contains("locked"), "{cause}");
                assert!(!services.database.pool().is_closed());
            }
        }
    }
    assert!(completed, "the original Terminal cleanup must complete after the writer is released and explicitly retried");
    assert_eq!(hook.calls.load(Ordering::SeqCst), 1);
    assert_eq!(hook.completed.load(Ordering::SeqCst), 1);
    assert_eq!(hook.cancelled.load(Ordering::SeqCst), 0);
    assert_eq!(*hook.observed.lock().unwrap(), [(services.authoritative_user_id.to_string(), terminal)]);
    assert!(services.database.pool().is_closed());
    services.shutdown_nomi_core_host().await.unwrap();
    assert_eq!(hook.calls.load(Ordering::SeqCst), 1);
}
