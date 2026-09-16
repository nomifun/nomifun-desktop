use super::*;
use std::sync::atomic::AtomicUsize;

struct Factory(AtomicUsize);
#[async_trait]
impl SystemBrowserConnectionFactory for Factory {
    async fn connect(&self) -> Result<Arc<dyn SystemBrowserConnection>, AttachError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(AttachError::ConnectionFailed)
    }
}
struct Verifier {
    allow: AtomicBool,
    calls: AtomicUsize,
}
#[async_trait]
impl SystemBrowserOwnerVerifier for Verifier {
    async fn verify(&self, user: &str, conversation: &str) -> Result<(), Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if !self.allow.load(Ordering::Acquire)
            || user != "alice"
            || !conversation.starts_with("chat")
        {
            return Err(Error::TabDenied);
        }
        Ok(())
    }
}
struct Connection {
    key: String,
    commands: Mutex<Vec<(String, Value)>>,
    invokes: AtomicUsize,
    releases: AtomicUsize,
    disconnects: AtomicUsize,
    hold: AtomicBool,
    fail_release: AtomicBool,
    entered: tokio::sync::Semaphore,
    released: tokio::sync::Semaphore,
    hold_settle: AtomicBool,
    settle_entered: tokio::sync::Semaphore,
    settle_done: tokio::sync::Semaphore,
}
impl Connection {
    fn new(key: char) -> Arc<Self> {
        Arc::new(Self {
            key: key.to_string().repeat(64),
            commands: Mutex::new(Vec::new()),
            invokes: AtomicUsize::new(0),
            releases: AtomicUsize::new(0),
            disconnects: AtomicUsize::new(0),
            hold: AtomicBool::new(false),
            fail_release: AtomicBool::new(false),
            entered: tokio::sync::Semaphore::new(0),
            released: tokio::sync::Semaphore::new(0),
            hold_settle: AtomicBool::new(false),
            settle_entered: tokio::sync::Semaphore::new(0),
            settle_done: tokio::sync::Semaphore::new(0),
        })
    }
}
#[async_trait]
impl SystemBrowserConnection for Connection {
    fn is_connected(&self) -> bool {
        true
    }
    fn request_disconnect(&self) {}
    async fn choices(&self) -> Result<UserTabInventory, AttachError> {
        panic!("run must not enumerate ungranted tabs")
    }
    async fn grant(&self, _: &str) -> Result<SystemBrowserTab, AttachError> {
        panic!("run must not grant tabs")
    }
    async fn disconnect(&self) -> Result<(), AttachError> {
        self.disconnects.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn target_key(&self, _: &str) -> Result<String, Error> {
        Ok(self.key.clone())
    }
    async fn invoke(
        &self,
        tab: &str,
        command: SystemBrowserCommand,
        cancel: &CancellationToken,
    ) -> Result<Value, Error> {
        self.invokes.fetch_add(1, Ordering::SeqCst);
        self.commands.lock().unwrap().push((tab.into(), serde_json::to_value(command).unwrap()));
        self.entered.add_permits(1);
        if self.hold.load(Ordering::Acquire) {
            self.released.acquire().await.unwrap().forget();
        }
        if cancel.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(json!({"observed":true}))
        }
    }
    async fn settle(&self, _: &str) -> Result<(), Error> {
        self.releases.fetch_add(1, Ordering::SeqCst);
        self.settle_entered.add_permits(1);
        if self.hold_settle.load(Ordering::Acquire) {
            self.settle_done.acquire().await.unwrap().forget();
        }
        if self.fail_release.load(Ordering::Acquire) {
            return Err(Error::ExecutionFailed);
        }
        Ok(())
    }
}
fn fixture() -> (Arc<SystemBrowserService>, Arc<Factory>, Arc<Verifier>) {
    let factory = Arc::new(Factory(AtomicUsize::new(0)));
    let service = SystemBrowserService::with_factory(factory.clone());
    let verifier = Arc::new(Verifier {
        allow: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
    });
    service.install_owner_verifier(verifier.clone()).unwrap();
    (service, factory, verifier)
}
fn attach(
    service: &SystemBrowserService,
    conversation: &str,
    tab: &str,
    connection: Arc<Connection>,
) {
    let tab = SystemBrowserTab {
        tab_id: tab.into(),
        title: "Authorized tab".into(),
        url: "https://example.test/".into(),
    };
    service.state.lock().unwrap().entries.insert(
        ("alice".into(), conversation.into()),
        Arc::new(Entry {
            incarnation: uuid::Uuid::now_v7().to_string(),
            data: Mutex::new(EntryData {
                state: SystemBrowserState::Connected,
                connection: Some(connection),
                tabs: [(tab.tab_id.clone(), tab)].into(),
                job: None,
            }),
            operation: tokio::sync::Mutex::new(()),
            cancel_connect: CancellationToken::new(),
        }),
    );
}
async fn begin(service: &SystemBrowserService, conversation: &str) -> Arc<dyn SystemBrowserTurn> {
    service
        .workspace("alice", conversation)
        .await
        .unwrap()
        .begin_run()
        .await
        .unwrap()
}
fn observe(tab: &str) -> SystemBrowserCommand {
    SystemBrowserCommand::Observe { tab_id: tab.into() }
}
async fn finish(turn: &Arc<dyn SystemBrowserTurn>) {
    turn.settle().await.unwrap();
    turn.finish().await.unwrap();
}
async fn until(mut test: impl FnMut() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !test() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dialog_reply_uses_the_frozen_tab_and_preserves_exact_input_fields() {
    let (service, factory, _) = fixture();
    let connection = Connection::new('d');
    attach(&service, "chat", "granted-tab", connection.clone());
    let turn = begin(&service, "chat").await;
    let reply = |tab: &str| SystemBrowserCommand::Dialog {
        tab_id: tab.into(), dialog_id: "current-dialog".into(), accept: true,
        prompt_text: Some("中文回复".into()),
    };
    assert_eq!(turn.invoke(reply("ungranted-tab")).await, Err(Error::TabDenied));
    assert!(connection.commands.lock().unwrap().is_empty());
    turn.invoke(reply("granted-tab")).await.unwrap();
    assert_eq!(*connection.commands.lock().unwrap(), vec![("granted-tab".into(), json!({
        "operation":"dialog", "tab_id":"granted-tab", "dialog_id":"current-dialog",
        "accept":true, "prompt_text":"中文回复"
    }))]);
    assert_eq!(factory.0.load(Ordering::SeqCst), 0);
    finish(&turn).await;
    assert_eq!(connection.releases.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unconnected_turn_is_lazy_unique_and_stale_after_finish() {
    let (service, factory, _) = fixture();
    let workspace = service.workspace("alice", "chat").await.unwrap();
    let run = workspace.begin_run().await.unwrap();
    assert!(matches!(workspace.begin_run().await, Err(Error::Busy)));
    assert_eq!(
        run.invoke(SystemBrowserCommand::Tabs {}).await,
        Err(Error::Unavailable)
    );
    assert_eq!(run.finish().await, Err(Error::Busy));
    finish(&run).await;
    assert_eq!(
        run.invoke(SystemBrowserCommand::Tabs {}).await,
        Err(Error::StaleRun)
    );
    let next = workspace.begin_run().await.unwrap();
    finish(&next).await;
    assert_eq!(factory.0.load(Ordering::SeqCst), 0);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn persistent_owner_is_required_and_rechecked_before_run() {
    let factory = Arc::new(Factory(AtomicUsize::new(0)));
    let closed = SystemBrowserService::with_factory(factory);
    assert!(matches!(
        closed.workspace("alice", "chat").await,
        Err(Error::Unavailable)
    ));
    let (service, _, verifier) = fixture();
    assert!(service.install_owner_verifier(verifier.clone()).is_err());
    assert!(matches!(
        service.workspace("mallory", "chat").await,
        Err(Error::TabDenied)
    ));
    let workspace = service.workspace("alice", "chat").await.unwrap();
    verifier.allow.store(false, Ordering::Release);
    assert!(matches!(workspace.begin_run().await, Err(Error::TabDenied)));
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 3);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn same_real_target_across_connections_is_exclusive_until_terminal_finish() {
    let (service, _, _) = fixture();
    let first = Connection::new('a');
    let second = Connection::new('a');
    attach(&service, "chat1", "grant1", first.clone());
    attach(&service, "chat2", "grant2", second.clone());
    let one = begin(&service, "chat1").await;
    let two = begin(&service, "chat2").await;
    one.invoke(observe("grant1")).await.unwrap();
    assert_eq!(two.invoke(observe("grant2")).await, Err(Error::Busy));
    one.settle().await.unwrap();
    assert_eq!(two.invoke(observe("grant2")).await, Err(Error::Busy));
    one.finish().await.unwrap();
    two.invoke(observe("grant2")).await.unwrap();
    finish(&two).await;
    assert_eq!(first.releases.load(Ordering::SeqCst), 1);
    assert_eq!(second.invokes.load(Ordering::SeqCst), 1);
    assert_eq!(first.disconnects.load(Ordering::SeqCst), 0);
    assert_eq!(second.disconnects.load(Ordering::SeqCst), 0);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_and_dropped_invoke_keep_atomic_input_until_it_completes() {
    let (service, _, _) = fixture();
    let connection = Connection::new('a');
    connection.hold.store(true, Ordering::Release);
    attach(&service, "chat", "grant", connection.clone());
    let run = begin(&service, "chat").await;
    let invoking = tokio::spawn({
        let run = run.clone();
        async move { run.invoke(observe("grant")).await }
    });
    connection.entered.acquire().await.unwrap().forget();
    invoking.abort();
    assert!(invoking.await.unwrap_err().is_cancelled());
    run.cancel();
    let settling = tokio::spawn({
        let run = run.clone();
        async move { run.settle().await }
    });
    tokio::task::yield_now().await;
    assert!(!settling.is_finished());
    assert_eq!(connection.releases.load(Ordering::SeqCst), 0);
    assert_eq!(service.runtime.state.lock().unwrap().claims.len(), 1);
    connection.released.add_permits(1);
    settling.await.unwrap().unwrap();
    assert_eq!(connection.releases.load(Ordering::SeqCst), 1);
    assert_eq!(service.runtime.state.lock().unwrap().claims.len(), 1);
    run.finish().await.unwrap();
    assert!(service.runtime.state.lock().unwrap().claims.is_empty());
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn run_drop_custodian_retains_job_and_releases_only_instrumentation() {
    let (service, _, _) = fixture();
    let connection = Connection::new('a');
    connection.hold.store(true, Ordering::Release);
    attach(&service, "chat", "grant", connection.clone());
    let run = begin(&service, "chat").await;
    let invoking = tokio::spawn({
        let run = run.clone();
        async move { run.invoke(observe("grant")).await }
    });
    connection.entered.acquire().await.unwrap().forget();
    invoking.abort();
    let _ = invoking.await;
    drop(run);
    assert_eq!(service.runtime.state.lock().unwrap().claims.len(), 1);
    connection.released.add_permits(1);
    until(|| service.runtime.state.lock().unwrap().runs.is_empty()).await;
    assert!(service.runtime.state.lock().unwrap().claims.is_empty());
    assert_eq!(connection.releases.load(Ordering::SeqCst), 1);
    assert_eq!(connection.disconnects.load(Ordering::SeqCst), 0);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_detach_retains_claim_and_run_until_exact_retry() {
    let (service, _, _) = fixture();
    let connection = Connection::new('a');
    attach(&service, "chat", "grant", connection.clone());
    let run = begin(&service, "chat").await;
    run.invoke(observe("grant")).await.unwrap();
    connection.fail_release.store(true, Ordering::Release);
    assert_eq!(run.settle().await, Err(Error::ExecutionFailed));
    assert_eq!(run.finish().await, Err(Error::Busy));
    assert_eq!(service.runtime.state.lock().unwrap().claims.len(), 1);
    connection.fail_release.store(false, Ordering::Release);
    finish(&run).await;
    assert!(service.runtime.state.lock().unwrap().claims.is_empty());
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn grants_and_incarnation_are_frozen_at_begin() {
    let (service, _, _) = fixture();
    let connection = Connection::new('a');
    attach(&service, "chat", "grant1", connection.clone());
    let run = begin(&service, "chat").await;
    let entry = service
        .state
        .lock()
        .unwrap()
        .entries
        .get(&("alice".into(), "chat".into()))
        .unwrap()
        .clone();
    entry.data.lock().unwrap().tabs.insert(
        "later".into(),
        SystemBrowserTab {
            tab_id: "later".into(),
            title: "Later grant".into(),
            url: "https://example.test/".into(),
        },
    );
    assert_eq!(run.invoke(observe("later")).await, Err(Error::TabDenied));
    attach(&service, "chat", "grant2", connection.clone());
    assert_eq!(
        run.invoke(observe("grant1")).await,
        Err(Error::Disconnected)
    );
    finish(&run).await;
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_settle_retains_the_same_detach_job_without_replaying_it() {
    let (service, _, _) = fixture();
    let connection = Connection::new('a');
    connection.hold_settle.store(true, Ordering::Release);
    attach(&service, "chat", "grant", connection.clone());
    let run = begin(&service, "chat").await;
    run.invoke(observe("grant")).await.unwrap();
    let first = tokio::spawn({
        let run = run.clone();
        async move { run.settle().await }
    });
    connection.settle_entered.acquire().await.unwrap().forget();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert_eq!(run.finish().await, Err(Error::Busy));
    let retry = tokio::spawn({
        let run = run.clone();
        async move { run.settle().await }
    });
    tokio::task::yield_now().await;
    assert!(!retry.is_finished());
    assert_eq!(connection.releases.load(Ordering::SeqCst), 1);
    connection.settle_done.add_permits(1);
    retry.await.unwrap().unwrap();
    run.finish().await.unwrap();
    assert_eq!(connection.releases.load(Ordering::SeqCst), 1);
    assert!(service.runtime.state.lock().unwrap().claims.is_empty());
    service.shutdown().await.unwrap();
}
