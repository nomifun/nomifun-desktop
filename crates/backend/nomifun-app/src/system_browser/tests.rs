use super::*;
use nomi_browser_engine::attached_browser::UserTabChoice;
use std::sync::atomic::{AtomicUsize, Ordering};

struct MockConnection {
    connected: std::sync::atomic::AtomicBool,
    id: usize,
    lists: AtomicUsize,
    grants: AtomicUsize,
    closes: AtomicUsize,
    fail_closes: AtomicUsize,
    close_gate: tokio::sync::Semaphore,
}
#[async_trait]
impl SystemBrowserConnection for MockConnection {
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
    fn request_disconnect(&self) {
        self.connected.store(false, Ordering::SeqCst);
    }
    async fn choices(&self) -> Result<UserTabInventory, AttachError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        Ok(UserTabInventory {
            tabs: vec![UserTabChoice {
                choice_id: format!("choice-{}", self.id),
                title: "Private page".into(),
                url: "https://example.test/private".into(),
            }],
        })
    }
    async fn grant(&self, choice_id: &str) -> Result<SystemBrowserTab, AttachError> {
        if choice_id != format!("choice-{}", self.id) {
            return Err(AttachError::StaleSelection);
        }
        self.grants.fetch_add(1, Ordering::SeqCst);
        Ok(SystemBrowserTab {
            tab_id: format!("grant-{}", self.id),
            title: "Private page".into(),
            url: "https://example.test/private".into(),
        })
    }
    async fn disconnect(&self) -> Result<(), AttachError> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        let _permit = self.close_gate.acquire().await.unwrap();
        if self
            .fail_closes
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return Err(AttachError::ConnectionFailed);
        }
        Ok(())
    }
}
struct MockFactory {
    calls: AtomicUsize,
    gate: tokio::sync::Semaphore,
    connections: Mutex<Vec<Arc<MockConnection>>>,
}
#[async_trait]
impl SystemBrowserConnectionFactory for MockFactory {
    async fn connect(&self) -> Result<Arc<dyn SystemBrowserConnection>, AttachError> {
        let id = self.calls.fetch_add(1, Ordering::SeqCst);
        self.gate.acquire().await.unwrap().forget();
        let connection = Arc::new(MockConnection {
            connected: std::sync::atomic::AtomicBool::new(true),
            id,
            lists: AtomicUsize::new(0),
            grants: AtomicUsize::new(0),
            closes: AtomicUsize::new(0),
            fail_closes: AtomicUsize::new(0),
            close_gate: tokio::sync::Semaphore::new(1),
        });
        self.connections.lock().unwrap().push(connection.clone());
        Ok(connection)
    }
}
fn fixture(permits: usize) -> (Arc<SystemBrowserService>, Arc<MockFactory>) {
    let factory = Arc::new(MockFactory {
        calls: AtomicUsize::new(0),
        gate: tokio::sync::Semaphore::new(permits),
        connections: Mutex::new(Vec::new()),
    });
    (SystemBrowserService::with_factory(factory.clone()), factory)
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("operation reached expected state");
}

#[tokio::test]
async fn snapshots_are_memory_only_and_inventory_is_not_authorization() {
    let (service, factory) = fixture(1);
    assert!(service.snapshot("alice", "chat").is_none());
    assert!(matches!(
        service.choices("alice", "chat", "missing").await,
        Err(SystemBrowserError::NotConnected)
    ));
    assert_eq!(factory.calls.load(Ordering::SeqCst), 0);
    let connected = service.connect("alice", "chat", None).await.unwrap();
    assert_eq!(connected.state, SystemBrowserState::Connected);
    assert!(connected.tabs.is_empty());
    let inventory = service
        .choices("alice", "chat", &connected.incarnation)
        .await
        .unwrap();
    assert_eq!(inventory.tabs.len(), 1);
    for _ in 0..3 {
        assert!(service.snapshot("alice", "chat").unwrap().tabs.is_empty());
    }
    let connection = factory.connections.lock().unwrap()[0].clone();
    assert_eq!(connection.lists.load(Ordering::SeqCst), 1);
    let granted = service
        .grant(
            "alice",
            "chat",
            &connected.incarnation,
            &inventory.tabs[0].choice_id,
        )
        .await
        .unwrap();
    assert_eq!(granted.tabs.len(), 1);
    let wire = serde_json::to_string(&granted).unwrap();
    assert!(!wire.contains("choice_id"));
    let disconnected = service
        .disconnect("alice", "chat", &connected.incarnation)
        .await
        .unwrap();
    assert_eq!(disconnected.state, SystemBrowserState::Disconnected);
    assert!(disconnected.tabs.is_empty());
    assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn lost_connection_clears_cached_authorization_without_io_or_reconnect() {
    let (service, factory) = fixture(1);
    let snapshot = service.connect("alice", "chat", None).await.unwrap();
    service
        .grant("alice", "chat", &snapshot.incarnation, "choice-0")
        .await
        .unwrap();
    let connection = factory.connections.lock().unwrap()[0].clone();
    connection.connected.store(false, Ordering::SeqCst);
    let lost = service.snapshot("alice", "chat").unwrap();
    assert_eq!(lost.state, SystemBrowserState::ConnectionLost);
    assert!(lost.tabs.is_empty());
    assert!(matches!(
        service
            .choices("alice", "chat", &snapshot.incarnation)
            .await,
        Err(SystemBrowserError::NotConnected)
    ));
    assert_eq!(connection.lists.load(Ordering::SeqCst), 0);
    assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        service
            .connect("alice", "chat", Some(&snapshot.incarnation))
            .await,
        Err(SystemBrowserError::Busy)
    ));
    service
        .disconnect("alice", "chat", &snapshot.incarnation)
        .await
        .unwrap();
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn grant_capacity_is_checked_before_calling_the_connection() {
    let (service, factory) = fixture(1);
    let snapshot = service.connect("alice", "chat", None).await.unwrap();
    let entry = service
        .entry("alice", "chat", &snapshot.incarnation, false)
        .unwrap();
    {
        let mut data = entry.data.lock().unwrap();
        for index in 0..MAX_GRANTED_TABS {
            data.tabs.insert(
                format!("grant-{index}"),
                SystemBrowserTab {
                    tab_id: format!("grant-{index}"),
                    title: "fixture".into(),
                    url: "https://fixture.test".into(),
                },
            );
        }
    }
    assert!(matches!(
        service
            .grant("alice", "chat", &snapshot.incarnation, "choice-0")
            .await,
        Err(SystemBrowserError::Capacity)
    ));
    assert_eq!(
        factory.connections.lock().unwrap()[0]
            .grants
            .load(Ordering::SeqCst),
        0
    );
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn owner_conversation_and_incarnation_never_alias() {
    let (service, factory) = fixture(3);
    let first = service.connect("alice", "first", None).await.unwrap();
    assert!(service.snapshot("bob", "first").is_none());
    assert!(matches!(
        service
            .grant("bob", "first", &first.incarnation, "choice-0")
            .await,
        Err(SystemBrowserError::NotConnected)
    ));
    assert!(matches!(
        service
            .disconnect("alice", "second", &first.incarnation)
            .await,
        Err(SystemBrowserError::NotConnected)
    ));
    assert!(matches!(
        service.connect("alice", "first", None).await,
        Err(SystemBrowserError::StaleIncarnation)
    ));
    let second = service.connect("alice", "second", None).await.unwrap();
    assert!(matches!(
        service.choices("alice", "second", &first.incarnation).await,
        Err(SystemBrowserError::StaleIncarnation)
    ));
    assert!(matches!(
        service
            .grant("alice", "second", &second.incarnation, "choice-0")
            .await,
        Err(SystemBrowserError::Connection(AttachError::StaleSelection))
    ));
    service
        .disconnect("alice", "first", &first.incarnation)
        .await
        .unwrap();
    let next = service
        .connect("alice", "first", Some(&first.incarnation))
        .await
        .unwrap();
    assert_ne!(next.incarnation, first.incarnation);
    assert!(matches!(
        service
            .disconnect("alice", "first", &first.incarnation)
            .await,
        Err(SystemBrowserError::StaleIncarnation)
    ));
    assert_eq!(factory.calls.load(Ordering::SeqCst), 3);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_connect_retains_and_cleans_the_late_connection() {
    let (service, factory) = fixture(0);
    let request = {
        let service = service.clone();
        tokio::spawn(async move { service.connect("alice", "chat", None).await })
    };
    until(|| factory.calls.load(Ordering::SeqCst) == 1).await;
    request.abort();
    let _ = request.await;
    factory.gate.add_permits(1);
    until(|| service.snapshot("alice", "chat").unwrap().state == SystemBrowserState::Disconnected)
        .await;
    assert_eq!(
        factory.connections.lock().unwrap()[0]
            .closes
            .load(Ordering::SeqCst),
        1
    );
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancellation_after_socket_ready_but_before_delivery_still_disconnects() {
    let (service, factory) = fixture(1);
    let mut request = Box::pin(service.connect("alice", "chat", None));
    assert!(futures_util::poll!(request.as_mut()).is_pending());
    until(|| {
        service
            .snapshot("alice", "chat")
            .is_some_and(|snapshot| snapshot.state == SystemBrowserState::Connected)
    })
    .await;
    drop(request);
    until(|| service.snapshot("alice", "chat").unwrap().state == SystemBrowserState::Disconnected)
        .await;
    assert_eq!(
        factory.connections.lock().unwrap()[0]
            .closes
            .load(Ordering::SeqCst),
        1
    );
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn cleanup_failure_remains_owned_and_can_only_be_retried_by_exact_owner() {
    let (service, factory) = fixture(1);
    let connected = service.connect("alice", "chat", None).await.unwrap();
    let connection = factory.connections.lock().unwrap()[0].clone();
    connection.fail_closes.store(1, Ordering::SeqCst);
    assert!(matches!(
        service
            .disconnect("alice", "chat", &connected.incarnation)
            .await,
        Err(SystemBrowserError::CleanupFailed)
    ));
    assert_eq!(
        service.snapshot("alice", "chat").unwrap().state,
        SystemBrowserState::CleanupFailed
    );
    assert!(matches!(
        service
            .connect("alice", "chat", Some(&connected.incarnation))
            .await,
        Err(SystemBrowserError::Busy)
    ));
    assert!(matches!(
        service
            .disconnect("bob", "chat", &connected.incarnation)
            .await,
        Err(SystemBrowserError::NotConnected)
    ));
    assert_eq!(
        service
            .disconnect("alice", "chat", &connected.incarnation)
            .await
            .unwrap()
            .state,
        SystemBrowserState::Disconnected
    );
    assert_eq!(connection.closes.load(Ordering::SeqCst), 2);
    service.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_waits_for_connect_and_never_publishes_its_late_socket() {
    let (service, factory) = fixture(0);
    let request = {
        let service = service.clone();
        tokio::spawn(async move { service.connect("alice", "chat", None).await })
    };
    until(|| factory.calls.load(Ordering::SeqCst) == 1).await;
    let shutdown = {
        let service = service.clone();
        tokio::spawn(async move { service.shutdown().await })
    };
    until(|| service.closed.is_cancelled()).await;
    assert!(!shutdown.is_finished());
    factory.gate.add_permits(1);
    assert!(request.await.unwrap().is_err());
    shutdown.await.unwrap().unwrap();
    assert_eq!(
        service.snapshot("alice", "chat").unwrap().state,
        SystemBrowserState::Disconnected
    );
    assert_eq!(
        factory.connections.lock().unwrap()[0]
            .closes
            .load(Ordering::SeqCst),
        1
    );
    assert!(matches!(
        service.connect("alice", "other", None).await,
        Err(SystemBrowserError::Closed)
    ));
    assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dropped_disconnect_request_does_not_abandon_cleanup_or_allow_new_connection() {
    let (service, factory) = fixture(1);
    let connected = service.connect("alice", "chat", None).await.unwrap();
    let connection = factory.connections.lock().unwrap()[0].clone();
    let pause = connection.close_gate.acquire().await.unwrap();
    let request = {
        let service = service.clone();
        let id = connected.incarnation.clone();
        tokio::spawn(async move { service.disconnect("alice", "chat", &id).await })
    };
    until(|| connection.closes.load(Ordering::SeqCst) == 1).await;
    request.abort();
    let _ = request.await;
    assert!(matches!(
        service
            .connect("alice", "chat", Some(&connected.incarnation))
            .await,
        Err(SystemBrowserError::Busy)
    ));
    drop(pause);
    until(|| service.snapshot("alice", "chat").unwrap().state == SystemBrowserState::Disconnected)
        .await;
    service.shutdown().await.unwrap();
    assert_eq!(connection.closes.load(Ordering::SeqCst), 1);
}
