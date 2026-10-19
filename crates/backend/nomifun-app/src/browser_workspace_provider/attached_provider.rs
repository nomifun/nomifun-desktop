//! Installation-level attached Chrome Provider.
//!
//! The local user connects Chrome once. Canonical AgentSessions with an exact
//! Browser Module/Resource grant may then bind this provider without a second
//! Session or tab authorization flow. Runtime claims still serialize Agent
//! control of the same physical tab and retain settle authority after cancel.

use async_trait::async_trait;
use nomi_browser_engine::attached_browser::{
    AttachError, AttachedBrowser, GrantedTab,
};
use nomifun_browser_platform::attached_browser::{
    AttachedBrowserCommand, AttachedBrowserProviderBinding, AttachedBrowserProviderHost,
    AttachedBrowserResource, AttachedBrowserRuntimeError, AttachedBrowserTurn,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex, OnceLock, Weak,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use tokio_util::sync::CancellationToken;

const MAX_TABS: usize = 4096;
const MAX_ACTIVE_RUNS: usize = 128;
const ACTIVE: u8 = 0;
const SETTLING: u8 = 1;
const SETTLED: u8 = 2;
const FINISHED: u8 = 3;

type SessionOwner = (String, String);
type Error = AttachedBrowserRuntimeError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AttachedProviderError {
    #[error("Attached Chrome Provider is shutting down")]
    Closed,
    #[error("Attached Chrome Provider is already connected")]
    Busy,
    #[error("Attached Chrome Provider is not connected")]
    NotConnected,
    #[error("Attached Chrome Provider belongs to another principal")]
    OwnerMismatch,
    #[error("Attached Chrome Provider connection changed")]
    StaleIncarnation,
    #[error("Attached Chrome Provider cleanup could not be confirmed")]
    CleanupFailed,
    #[error(transparent)]
    Connection(#[from] AttachError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachedProviderState {
    Connected,
    ConnectionLost,
    Disconnecting,
    CleanupFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AttachedProviderSnapshot {
    pub incarnation: String,
    pub state: AttachedProviderState,
    pub chromium_major: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AttachedTab {
    tab_id: String,
    title: String,
    url: String,
}

#[async_trait]
pub(crate) trait AttachedProviderConnection: Send + Sync {
    fn is_connected(&self) -> bool;
    fn chromium_major(&self) -> u32;
    fn request_disconnect(&self);
    async fn tabs(&self) -> Result<Vec<AttachedTab>, AttachError>;
    fn target_key(&self, tab_id: &str) -> Result<String, Error>;
    async fn invoke(
        &self,
        tab_id: &str,
        command: AttachedBrowserCommand,
        cancel: &CancellationToken,
    ) -> Result<Value, Error>;
    async fn settle(&self, tab_id: &str) -> Result<(), Error>;
    async fn disconnect(&self) -> Result<(), AttachError>;
}

#[async_trait]
pub(crate) trait AttachedProviderConnectionFactory: Send + Sync {
    async fn connect(&self) -> Result<Arc<dyn AttachedProviderConnection>, AttachError>;
}

struct RunningChromeFactory;

#[cfg(all(test,feature="browser-conformance"))]
struct ConformancePortFileFactory(std::path::PathBuf);

#[cfg(all(test,feature="browser-conformance"))]
#[async_trait]
impl AttachedProviderConnectionFactory for ConformancePortFileFactory {
    async fn connect(&self)->Result<Arc<dyn AttachedProviderConnection>,AttachError> {
        Ok(Arc::new(RunningChromeConnection {
            browser:AttachedBrowser::connect_for_conformance(&self.0).await?,
            grants:Mutex::new(BTreeMap::new()),
        }))
    }
}

struct RunningChromeConnection {
    browser: AttachedBrowser,
    grants: Mutex<BTreeMap<String, GrantedTab>>,
}

#[async_trait]
impl AttachedProviderConnectionFactory for RunningChromeFactory {
    async fn connect(&self) -> Result<Arc<dyn AttachedProviderConnection>, AttachError> {
        Ok(Arc::new(RunningChromeConnection {
            browser: AttachedBrowser::connect_running_chrome().await?,
            grants: Mutex::new(BTreeMap::new()),
        }))
    }
}

#[async_trait]
impl AttachedProviderConnection for RunningChromeConnection {
    fn is_connected(&self) -> bool {
        self.browser.is_connected()
    }

    fn chromium_major(&self) -> u32 {
        self.browser.chromium_major()
    }

    fn request_disconnect(&self) {
        self.browser.request_disconnect();
    }

    async fn tabs(&self) -> Result<Vec<AttachedTab>, AttachError> {
        let discovered = self.browser.tabs_for_provider().await?;
        let mut grants = self.grants.lock().unwrap_or_else(|error| error.into_inner());
        let mut result = Vec::with_capacity(discovered.len());
        for tab in discovered {
            let grant = grants
                .values()
                .find(|grant| grant.same_target(&tab.grant))
                .cloned()
                .unwrap_or(tab.grant);
            let tab_id = grant.id().to_owned();
            if !grants.contains_key(&tab_id) {
                if grants.len() >= MAX_TABS {
                    return Err(AttachError::InventoryLimit);
                }
                grants.insert(tab_id.clone(), grant);
            }
            result.push(AttachedTab {
                tab_id,
                title: tab.info.title,
                url: tab.info.url,
            });
        }
        Ok(result)
    }

    fn target_key(&self, tab_id: &str) -> Result<String, Error> {
        self.grants
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(tab_id)
            .map(GrantedTab::target_key)
            .ok_or(Error::TabDenied)
    }

    async fn invoke(
        &self,
        tab_id: &str,
        command: AttachedBrowserCommand,
        cancel: &CancellationToken,
    ) -> Result<Value, Error> {
        let grant = self
            .grants
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(tab_id)
            .cloned()
            .ok_or(Error::TabDenied)?;
        self.browser.execute_granted(&grant, command, cancel).await
    }

    async fn settle(&self, tab_id: &str) -> Result<(), Error> {
        let grant = self
            .grants
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(tab_id)
            .cloned()
            .ok_or(Error::TabDenied)?;
        self.browser.release_granted(&grant).await
    }

    async fn disconnect(&self) -> Result<(), AttachError> {
        self.browser.disconnect().await?;
        self.grants
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        Ok(())
    }
}

#[async_trait]
pub trait AttachedBrowserSessionVerifier: Send + Sync {
    async fn verify(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<(), AttachedBrowserRuntimeError>;
}

struct ConnectionEntry {
    owner_id: String,
    incarnation: String,
    connection: Arc<dyn AttachedProviderConnection>,
    state: AttachedProviderState,
}

/// If a connect caller disappears after the socket is ready but before the
/// service publishes it, an owned cleanup task retains physical disposal.
struct UnpublishedConnection(Option<Arc<dyn AttachedProviderConnection>>);

impl Drop for UnpublishedConnection {
    fn drop(&mut self) {
        let Some(connection) = self.0.take() else {
            return;
        };
        connection.request_disconnect();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = connection.disconnect().await;
            });
        }
    }
}

#[derive(Default)]
struct ServiceState {
    closed: bool,
    connection: Option<ConnectionEntry>,
}

#[derive(Default)]
struct RuntimeState {
    closed: bool,
    runs: BTreeMap<SessionOwner, Arc<Run>>,
    claims: BTreeMap<String, String>,
    /// Canonical AgentSession deletion is a permanent admission fence. Keep it
    /// in process memory before draining the current run so a begin_run that
    /// was already waiting on the provider barrier cannot publish after the
    /// delete cleanup sweep completed.
    retired_sessions: BTreeSet<SessionOwner>,
}

#[derive(Default)]
struct ProviderRuntime {
    state: Mutex<RuntimeState>,
}

pub struct AttachedChromeProviderService {
    factory: Arc<dyn AttachedProviderConnectionFactory>,
    state: Mutex<ServiceState>,
    // Serialize physical connect/disconnect only. Admission and cancellation
    // use the short state lock and never wait behind this native lifecycle.
    lifecycle: tokio::sync::Mutex<()>,
    verifier: OnceLock<Arc<dyn AttachedBrowserSessionVerifier>>,
    runtime: Arc<ProviderRuntime>,
    weak: Weak<Self>,
}

impl AttachedChromeProviderService {
    pub fn new() -> Arc<Self> {
        Self::with_factory(Arc::new(RunningChromeFactory))
    }

    /// Opt-in native fixture only: the ordinary connection implementation,
    /// pointed at the separately launched disposable browser's own port file.
    #[cfg(all(test,feature="browser-conformance"))]
    pub(crate) fn with_conformance_port_file(port_file:std::path::PathBuf)->Arc<Self> {
        Self::with_factory(Arc::new(ConformancePortFileFactory(port_file)))
    }

    #[cfg(all(test,feature="browser-conformance"))]
    pub(crate) fn conformance_claim_counts(&self)->(usize,usize) {
        let state=self.runtime.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        (state.runs.len(),state.claims.len())
    }

    pub(crate) fn with_factory(factory: Arc<dyn AttachedProviderConnectionFactory>) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            factory,
            state: Mutex::new(ServiceState::default()),
            lifecycle: tokio::sync::Mutex::new(()),
            verifier: OnceLock::new(),
            runtime: Arc::new(ProviderRuntime::default()),
            weak: weak.clone(),
        })
    }

    pub fn install_session_verifier(
        &self,
        verifier: Arc<dyn AttachedBrowserSessionVerifier>,
    ) -> Result<(), AttachedProviderError> {
        self.verifier
            .set(verifier)
            .map_err(|_| AttachedProviderError::Busy)
    }

    pub fn snapshot(&self, owner_id: &str) -> Result<Option<AttachedProviderSnapshot>, AttachedProviderError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(entry) = &mut state.connection else {
            return Ok(None);
        };
        if entry.owner_id != owner_id {
            return Err(AttachedProviderError::OwnerMismatch);
        }
        if entry.state == AttachedProviderState::Connected && !entry.connection.is_connected() {
            entry.state = AttachedProviderState::ConnectionLost;
        }
        Ok(Some(AttachedProviderSnapshot {
            incarnation: entry.incarnation.clone(),
            state: entry.state,
            chromium_major: entry.connection.chromium_major(),
        }))
    }

    pub async fn connect(
        self: &Arc<Self>,
        owner_id: &str,
    ) -> Result<AttachedProviderSnapshot, AttachedProviderError> {
        if owner_id.trim().is_empty()
            || owner_id.len() > 512
            || owner_id.chars().any(char::is_control)
        {
            return Err(AttachedProviderError::OwnerMismatch);
        }
        if self.state.lock().unwrap_or_else(|error| error.into_inner()).closed {
            return Err(AttachedProviderError::Closed);
        }
        let service = self.clone();
        let owner_id = owner_id.to_owned();
        tokio::spawn(async move { service.connect_owned(owner_id).await })
            .await.map_err(|_| AttachedProviderError::CleanupFailed)?
    }

    async fn connect_owned(&self, owner_id: String) -> Result<AttachedProviderSnapshot, AttachedProviderError> {
        let _lifecycle = self.lifecycle.lock().await;
        {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.closed {
                return Err(AttachedProviderError::Closed);
            }
            if state.connection.is_some() {
                return Err(AttachedProviderError::Busy);
            }
        }
        let connection = self.factory.connect().await?;
        let mut unpublished = UnpublishedConnection(Some(connection.clone()));
        if !connection.is_connected() {
            return Err(AttachedProviderError::Connection(AttachError::ConnectionFailed));
        }
        let entry = ConnectionEntry {
            owner_id,
            incarnation: nomifun_common::generate_id(),
            connection,
            state: AttachedProviderState::Connected,
        };
        let snapshot = AttachedProviderSnapshot {
            incarnation: entry.incarnation.clone(),
            state: entry.state,
            chromium_major: entry.connection.chromium_major(),
        };
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        // Shutdown can fence admission while the constructor holds the
        // lifecycle gate. Publish its exact connection for that waiting owner
        // to dispose, never an unregistered late successful connection.
        let closed = state.closed;
        state.connection = Some(ConnectionEntry {
            state: if closed { AttachedProviderState::Disconnecting } else { entry.state },
            ..entry
        });
        unpublished.0 = None;
        if closed { Err(AttachedProviderError::Closed) } else { Ok(snapshot) }
    }

    pub async fn disconnect(
        &self,
        owner_id: &str,
        incarnation: &str,
    ) -> Result<(), AttachedProviderError> {
        let connection = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let entry = state
                .connection
                .as_mut()
                .ok_or(AttachedProviderError::NotConnected)?;
            if entry.owner_id != owner_id {
                return Err(AttachedProviderError::OwnerMismatch);
            }
            if entry.incarnation != incarnation {
                return Err(AttachedProviderError::StaleIncarnation);
            }
            entry.state = AttachedProviderState::Disconnecting;
            entry.connection.clone()
        };
        // Fence admission and signal every registered run before waiting on
        // either native work or another lifecycle caller.
        self.runtime.cancel_runs();
        let service = self.weak.upgrade().ok_or(AttachedProviderError::Closed)?;
        let incarnation = incarnation.to_owned();
        tokio::spawn(async move { service.disconnect_owned(&incarnation, connection).await })
            .await.map_err(|_| AttachedProviderError::CleanupFailed)?
    }

    async fn disconnect_owned(&self, incarnation: &str, connection: Arc<dyn AttachedProviderConnection>)
        -> Result<(), AttachedProviderError> {
        let _lifecycle = self.lifecycle.lock().await;
        {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let Some(entry) = state.connection.as_ref() else { return Ok(()); };
            if entry.incarnation != incarnation || !Arc::ptr_eq(&entry.connection, &connection) {
                return Err(AttachedProviderError::StaleIncarnation);
            }
        }
        self.runtime.drain_runs().await?;
        connection.request_disconnect();
        let result = connection.disconnect().await;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        match result {
            Ok(()) => {
                state.connection = None;
                Ok(())
            }
            Err(_) => {
                if let Some(entry) = state.connection.as_mut() {
                    entry.state = AttachedProviderState::CleanupFailed;
                }
                Err(AttachedProviderError::CleanupFailed)
            }
        }
    }

    async fn verify_session(&self, owner: &SessionOwner) -> Result<(), Error> {
        self.verifier
            .get()
            .ok_or(Error::Unavailable)?
            .verify(&owner.0, &owner.1)
            .await
    }

    pub async fn close_agent_session(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<(), AttachedProviderError> {
        let owner = (principal_id.to_owned(), agent_session_id.to_owned());
        let run = {
            let mut state = self
                .runtime
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.retired_sessions.insert(owner.clone());
            state.runs.get(&owner).cloned()
        };
        if let Some(run) = run {
            run.cancel.cancel();
            tokio::spawn(async move {
                run.settle().await.map_err(|_| AttachedProviderError::CleanupFailed)?;
                run.finish().await.map_err(|_| AttachedProviderError::CleanupFailed)
            }).await.map_err(|_| AttachedProviderError::CleanupFailed)??;
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), AttachedProviderError> {
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            if let Some(entry) = state.connection.as_mut() {
                entry.state = AttachedProviderState::Disconnecting;
            }
        }
        {
            self.runtime.state.lock().unwrap_or_else(|error| error.into_inner()).closed = true;
        }
        self.runtime.cancel_runs();
        let service = self.weak.upgrade().ok_or(AttachedProviderError::Closed)?;
        tokio::spawn(async move { service.shutdown_owned().await })
            .await.map_err(|_| AttachedProviderError::CleanupFailed)?
    }

    async fn shutdown_owned(&self) -> Result<(), AttachedProviderError> {
        let _lifecycle = self.lifecycle.lock().await;
        self.runtime.drain_runs().await?;
        let connection = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.connection.as_mut().map(|entry| {
                entry.connection.request_disconnect();
                entry.connection.clone()
            })
        };
        if let Some(connection) = connection {
            if connection.disconnect().await.is_err() {
                if let Some(entry) = self
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .connection
                    .as_mut()
                {
                    entry.state = AttachedProviderState::CleanupFailed;
                }
                return Err(AttachedProviderError::CleanupFailed);
            }
            self.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .connection = None;
        }
        Ok(())
    }
}

struct Resource {
    service: Arc<AttachedChromeProviderService>,
    owner: SessionOwner,
}

struct Run {
    id: String,
    owner: SessionOwner,
    service: Weak<AttachedChromeProviderService>,
    runtime: Weak<ProviderRuntime>,
    incarnation: String,
    connection: Arc<dyn AttachedProviderConnection>,
    tabs: OnceLock<BTreeMap<String, AttachedTab>>,
    phase: AtomicU8,
    cancel: CancellationToken,
    operation: Arc<tokio::sync::Mutex<()>>,
    touched: Mutex<BTreeSet<String>>,
    orphaned: AtomicBool,
    wake: tokio::sync::Notify,
}

struct Turn(Arc<Run>);

/// The native inventory owner is retained even if its begin_run waiter leaves.
/// Mark the same registered Run for cleanup instead of dropping native work.
struct PendingRun(Option<Arc<Run>>);

impl Drop for PendingRun {
    fn drop(&mut self) {
        if let Some(run) = &self.0 { run.abandon(); }
    }
}

#[async_trait]
impl AttachedBrowserProviderHost for AttachedChromeProviderService {
    fn binding(&self) -> AttachedBrowserProviderBinding {
        use sha2::Digest;
        let mut hash = sha2::Sha256::new();
        let automation_digest = AttachedBrowser::automation_digest();
        for source in [
            automation_digest.as_bytes(),
            include_bytes!("attached_provider.rs").as_slice(),
        ] {
            hash.update((source.len() as u64).to_le_bytes());
            hash.update(source);
        }
        AttachedBrowserProviderBinding {
            schema_version: 1,
            runtime_digest: format!("{:x}", hash.finalize()),
        }
    }

    async fn resource(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<Arc<dyn AttachedBrowserResource>, Error> {
        let owner = (principal_id.to_owned(), agent_session_id.to_owned());
        self.verify_session(&owner).await?;
        Ok(Arc::new(Resource {
            service: self.weak.upgrade().ok_or(Error::Disconnected)?,
            owner,
        }))
    }
}

#[async_trait]
impl AttachedBrowserResource for Resource {
    async fn begin_run(&self) -> Result<Arc<dyn AttachedBrowserTurn>, Error> {
        self.service.verify_session(&self.owner).await?;
        let runtime = self.service.runtime.clone();
        let run = {
            // Connection epoch, admission and Run registration are atomic.
            // The registered Run also owns a still-pending inventory read, so
            // disconnect cannot miss an operation not yet visible to callers.
            let service = self.service.state.lock().unwrap_or_else(|error| error.into_inner());
            let entry = service.connection.as_ref().ok_or(Error::Unavailable)?;
            if service.closed || entry.owner_id != self.owner.0
                || entry.state != AttachedProviderState::Connected || !entry.connection.is_connected() {
                return Err(Error::Unavailable);
            }
            let mut state = runtime.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.retired_sessions.contains(&self.owner) { return Err(Error::StaleRun); }
            if state.closed
                || state.runs.contains_key(&self.owner)
                || state.runs.len() >= MAX_ACTIVE_RUNS
            {
                return Err(Error::Busy);
            }
            let run = Arc::new(Run {
                id: nomifun_common::generate_id(),
                owner: self.owner.clone(),
                service: Arc::downgrade(&self.service),
                runtime: Arc::downgrade(&runtime),
                incarnation: entry.incarnation.clone(),
                connection: entry.connection.clone(),
                tabs: OnceLock::new(),
                phase: AtomicU8::new(ACTIVE),
                cancel: CancellationToken::new(),
                operation: Arc::new(tokio::sync::Mutex::new(())),
                touched: Mutex::new(BTreeSet::new()),
                orphaned: AtomicBool::new(false),
                wake: tokio::sync::Notify::new(),
            });
            state.runs.insert(self.owner.clone(), run.clone());
            run
        };
        let mut pending = PendingRun(Some(run.clone()));
        tokio::spawn(custodian(run.clone(), runtime));
        let result = tokio::spawn(async move { run.initialize().await }).await
            .map_err(|_| Error::ExecutionFailed)?;
        if result.is_ok() { pending.0 = None; }
        result
    }
}

impl Run {
    fn abandon(&self) {
        if self.phase.load(Ordering::Acquire) != FINISHED {
            self.orphaned.store(true, Ordering::Release);
            self.cancel.cancel();
            self.wake.notify_one();
        }
    }

    async fn initialize(self: Arc<Self>) -> Result<Arc<dyn AttachedBrowserTurn>, Error> {
        let _operation = self.operation.lock().await;
        if self.cancel.is_cancelled() { return Err(Error::Cancelled); }
        self.current_runtime()?;
        self.current_connection()?;
        let tabs = self.connection.tabs().await.map_err(|_| Error::Disconnected)?
            .into_iter().map(|tab| (tab.tab_id.clone(), tab)).collect();
        let service = self.service.upgrade().ok_or(Error::Disconnected)?;
        service.verify_session(&self.owner).await?;
        let state = service.state.lock().unwrap_or_else(|error| error.into_inner());
        self.require_connection(&state)?;
        self.current_runtime()?;
        if self.phase.load(Ordering::Acquire) != ACTIVE || self.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.tabs.set(tabs).map_err(|_| Error::StaleRun)?;
        Ok(Arc::new(Turn(self.clone())))
    }

    fn current_runtime(&self) -> Result<Arc<ProviderRuntime>, Error> {
        let runtime = self.runtime.upgrade().ok_or(Error::StaleRun)?;
        let valid = runtime
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .runs
            .get(&self.owner)
            .is_some_and(|run| run.id == self.id);
        valid.then_some(runtime).ok_or(Error::StaleRun)
    }

    fn current_connection(&self) -> Result<(), Error> {
        let service = self.service.upgrade().ok_or(Error::Disconnected)?;
        let state = service.state.lock().unwrap_or_else(|error| error.into_inner());
        self.require_connection(&state)
    }

    fn require_connection(&self, state: &ServiceState) -> Result<(), Error> {
        let entry = state.connection.as_ref().ok_or(Error::Disconnected)?;
        if state.closed || entry.state != AttachedProviderState::Connected
            || entry.incarnation != self.incarnation
            || !Arc::ptr_eq(&entry.connection, &self.connection)
            || !entry.connection.is_connected()
        {
            return Err(Error::Disconnected);
        }
        Ok(())
    }

    async fn invoke_owned(&self, command: AttachedBrowserCommand, _operation: tokio::sync::OwnedMutexGuard<()>) -> Result<Value, Error> {
        // Keep causal order within this Run, without blocking another Run or
        // the provider's synchronous disconnect/cancellation fence.
        let runtime = self.current_runtime()?;
        if self.phase.load(Ordering::Acquire) != ACTIVE || self.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let service = self.service.upgrade().ok_or(Error::Disconnected)?;
        let tab_id = {
            // This short lock is the shared linearization point with closing.
            // Register the exact claim and cleanup responsibility before
            // releasing it and entering any native call.
            let state = service.state.lock().unwrap_or_else(|error| error.into_inner());
            self.require_connection(&state)?;
            let tabs = self.tabs.get().ok_or(Error::StaleRun)?;
            let tab_id = match &command {
                AttachedBrowserCommand::Tabs {} => {
                    let tabs = tabs.values().map(|tab| json!({
                        "tab_id": tab.tab_id,
                        "title": tab.title,
                        "url": nomifun_browser_platform::url_projection::project_metadata_url(&tab.url),
                    })).collect::<Vec<_>>();
                    return Ok(json!({"tabs": tabs, "untrusted_page_content": true}));
                }
                AttachedBrowserCommand::Observe { tab_id }
                | AttachedBrowserCommand::Navigate { tab_id, .. }
                | AttachedBrowserCommand::Dialog { tab_id, .. }
                | AttachedBrowserCommand::Click { tab_id, .. }
                | AttachedBrowserCommand::Type { tab_id, .. }
                | AttachedBrowserCommand::Press { tab_id, .. }
                | AttachedBrowserCommand::Scroll { tab_id, .. } => tab_id.clone(),
            };
            if !tabs.contains_key(&tab_id) { return Err(Error::TabDenied); }
            let target_key = self.connection.target_key(&tab_id)?;
            {
                let mut runtime = runtime.state.lock().unwrap_or_else(|error| error.into_inner());
                if runtime.claims.get(&target_key).is_some_and(|run_id| run_id != &self.id) {
                    return Err(Error::Busy);
                }
                runtime.claims.insert(target_key, self.id.clone());
            }
            self.touched.lock().unwrap_or_else(|error| error.into_inner()).insert(tab_id.clone());
            tab_id
        };
        self.connection.invoke(&tab_id, command, &self.cancel).await
    }

    async fn settle(&self) -> Result<(), Error> {
        self.cancel.cancel();
        self.phase
            .compare_exchange(ACTIVE, SETTLING, Ordering::AcqRel, Ordering::Acquire)
            .ok();
        let _operation = self.operation.lock().await;
        if self.phase.load(Ordering::Acquire) >= SETTLED {
            return Ok(());
        }
        self.current_runtime()?;
        let tabs = self
            .touched
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        for tab in tabs {
            self.connection.settle(&tab).await?;
            self.touched
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&tab);
        }
        self.phase.store(SETTLED, Ordering::Release);
        Ok(())
    }

    async fn finish(&self) -> Result<(), Error> {
        let _operation = self.operation.lock().await;
        if self.phase.load(Ordering::Acquire) == FINISHED {
            return Ok(());
        }
        if self.phase.load(Ordering::Acquire) != SETTLED {
            return Err(Error::Busy);
        }
        let runtime = self.current_runtime()?;
        let mut state = runtime.state.lock().unwrap_or_else(|error| error.into_inner());
        state.claims.retain(|_, run_id| run_id != &self.id);
        state.runs.remove(&self.owner);
        self.phase.store(FINISHED, Ordering::Release);
        self.wake.notify_one();
        Ok(())
    }
}

#[async_trait]
impl AttachedBrowserTurn for Turn {
    fn cancel(&self) {
        self.0.cancel.cancel();
    }

    async fn invoke(&self, command: AttachedBrowserCommand) -> Result<Value, Error> {
        let run = self.0.clone();
        run.current_runtime()?;
        let operation = tokio::select! {
            biased;
            _ = run.cancel.cancelled() => return Err(Error::Cancelled),
            operation = run.operation.clone().lock_owned() => operation,
        };
        // Dropping the caller only detaches its wait. The original operation
        // keeps its Run lock and cleanup responsibility through native return.
        tokio::spawn(async move { run.invoke_owned(command, operation).await }).await
            .map_err(|_| Error::ExecutionFailed)?
    }

    async fn settle(&self) -> Result<(), Error> {
        self.0.cancel.cancel();
        let run = self.0.clone();
        tokio::spawn(async move { run.settle().await }).await
            .map_err(|_| Error::ExecutionFailed)?
    }

    async fn finish(&self) -> Result<(), Error> {
        let run = self.0.clone();
        tokio::spawn(async move { run.finish().await }).await
            .map_err(|_| Error::ExecutionFailed)?
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        self.0.abandon();
    }
}

async fn custodian(run: Arc<Run>, _runtime: Arc<ProviderRuntime>) {
    loop {
        if run.phase.load(Ordering::Acquire) == FINISHED {
            return;
        }
        if run.orphaned.load(Ordering::Acquire) {
            break;
        }
        run.wake.notified().await;
    }
    let mut retry_seconds = 1;
    loop {
        if run.settle().await.is_ok() && run.finish().await.is_ok() {
            return;
        }
        tracing::warn!("attached Chrome orphan cleanup retained for retry");
        tokio::time::sleep(std::time::Duration::from_secs(retry_seconds)).await;
        retry_seconds = (retry_seconds * 2).min(30);
    }
}

impl ProviderRuntime {
    fn cancel_runs(&self) {
        for run in self.state.lock().unwrap_or_else(|error| error.into_inner()).runs.values() {
            run.cancel.cancel();
        }
    }

    async fn drain_runs(&self) -> Result<(), AttachedProviderError> {
        let runs = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.runs.values().cloned().collect::<Vec<_>>()
        };
        for run in &runs {
            run.cancel.cancel();
        }
        for run in runs {
            run.settle()
                .await
                .map_err(|_| AttachedProviderError::CleanupFailed)?;
            run.finish()
                .await
                .map_err(|_| AttachedProviderError::CleanupFailed)?;
        }
        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    struct Factory {
        calls: AtomicUsize,
        connection: Arc<Connection>,
        block_connect: AtomicBool,
        connect_started: tokio::sync::Notify,
        connect_release: tokio::sync::Notify,
    }

    struct Connection {
        connected: AtomicBool,
        block_tabs: AtomicBool,
        tabs_started: tokio::sync::Notify,
        tabs_release: tokio::sync::Notify,
        block_invokes: AtomicBool,
        invoke_started: tokio::sync::Notify,
        invoke_cancelled: tokio::sync::Notify,
        invoke_release: tokio::sync::Semaphore,
        invokes: AtomicUsize,
        active_invokes: AtomicUsize,
        completed_invokes: AtomicUsize,
        invoke_order: Mutex<Vec<String>>,
        settles: AtomicUsize,
        disconnects: AtomicUsize,
    }

    #[async_trait]
    impl AttachedProviderConnectionFactory for Factory {
        async fn connect(&self) -> Result<Arc<dyn AttachedProviderConnection>, AttachError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.block_connect.load(Ordering::Acquire) {
                self.connect_started.notify_one();
                self.connect_release.notified().await;
            }
            Ok(self.connection.clone())
        }
    }

    #[async_trait]
    impl AttachedProviderConnection for Connection {
        fn is_connected(&self) -> bool {
            self.connected.load(Ordering::Acquire)
        }
        fn chromium_major(&self) -> u32 {
            152
        }
        fn request_disconnect(&self) {
            self.connected.store(false, Ordering::Release);
        }
        async fn tabs(&self) -> Result<Vec<AttachedTab>, AttachError> {
            if self.block_tabs.load(Ordering::Acquire) {
                self.tabs_started.notify_one();
                self.tabs_release.notified().await;
            }
            Ok(["tab", "other-tab"].into_iter().map(|tab_id| AttachedTab {
                tab_id: tab_id.into(), title: "Installed tab".into(), url: "https://example.test".into(),
            }).collect())
        }
        fn target_key(&self, tab_id: &str) -> Result<String, Error> {
            match tab_id {
                "tab" => Ok("a".repeat(64)),
                "other-tab" => Ok("b".repeat(64)),
                _ => Err(Error::TabDenied),
            }
        }
        async fn invoke(
            &self,
            tab_id: &str,
            _: AttachedBrowserCommand,
            cancel: &CancellationToken,
        ) -> Result<Value, Error> {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            self.invokes.fetch_add(1, Ordering::SeqCst);
            self.invoke_order.lock().unwrap().push(tab_id.to_owned());
            self.active_invokes.fetch_add(1, Ordering::SeqCst);
            struct Active<'a>(&'a AtomicUsize);
            impl Drop for Active<'_> {
                fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); }
            }
            let _active = Active(&self.active_invokes);
            if tab_id == "tab" && self.block_invokes.load(Ordering::Acquire) {
                self.invoke_started.notify_one();
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        self.invoke_cancelled.notify_one();
                        self.invoke_release.acquire().await.unwrap().forget();
                    },
                    permit = self.invoke_release.acquire() => permit.unwrap().forget(),
                }
            }
            self.completed_invokes.fetch_add(1, Ordering::SeqCst);
            if cancel.is_cancelled() { Err(Error::Cancelled) }
            else { Ok(json!({"observed": true})) }
        }
        async fn settle(&self, _: &str) -> Result<(), Error> {
            self.settles.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn disconnect(&self) -> Result<(), AttachError> {
            self.connected.store(false, Ordering::Release);
            self.disconnects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct Verifier;

    #[async_trait]
    impl AttachedBrowserSessionVerifier for Verifier {
        async fn verify(
            &self,
            principal_id: &str,
            agent_session_id: &str,
        ) -> Result<(), Error> {
            if principal_id == "alice" && agent_session_id.starts_with("session") {
                Ok(())
            } else {
                Err(Error::ActionDenied)
            }
        }
    }

    fn fixture() -> (
        Arc<AttachedChromeProviderService>,
        Arc<Factory>,
        Arc<Connection>,
    ) {
        let connection = Arc::new(Connection {
            connected: AtomicBool::new(true),
            block_tabs: AtomicBool::new(false),
            tabs_started: tokio::sync::Notify::new(),
            tabs_release: tokio::sync::Notify::new(),
            block_invokes: AtomicBool::new(false),
            invoke_started: tokio::sync::Notify::new(),
            invoke_cancelled: tokio::sync::Notify::new(),
            invoke_release: tokio::sync::Semaphore::new(0),
            invokes: AtomicUsize::new(0),
            active_invokes: AtomicUsize::new(0),
            completed_invokes: AtomicUsize::new(0),
            invoke_order: Mutex::new(Vec::new()),
            settles: AtomicUsize::new(0),
            disconnects: AtomicUsize::new(0),
        });
        let factory = Arc::new(Factory {
            calls: AtomicUsize::new(0),
            connection: connection.clone(),
            block_connect: AtomicBool::new(false),
            connect_started: tokio::sync::Notify::new(),
            connect_release: tokio::sync::Notify::new(),
        });
        let service = AttachedChromeProviderService::with_factory(factory.clone());
        service
            .install_session_verifier(Arc::new(Verifier))
            .unwrap();
        (service, factory, connection)
    }

    #[tokio::test]
    async fn one_installation_connection_is_reused_by_delegated_agent_sessions() {
        let (service, factory, connection) = fixture();
        let connected = service.connect("alice").await.unwrap();
        assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            service.connect("alice").await,
            Err(AttachedProviderError::Busy)
        );

        let first = service.resource("alice", "session-parent").await.unwrap();
        let second = service.resource("alice", "session-delegated").await.unwrap();
        let first = first.begin_run().await.unwrap();
        let second = second.begin_run().await.unwrap();
        first
            .invoke(AttachedBrowserCommand::Observe {
                tab_id: "tab".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            second
                .invoke(AttachedBrowserCommand::Observe {
                    tab_id: "tab".into(),
                })
                .await,
            Err(Error::Busy)
        );
        first.settle().await.unwrap();
        first.finish().await.unwrap();
        second
            .invoke(AttachedBrowserCommand::Observe {
                tab_id: "tab".into(),
            })
            .await
            .unwrap();
        second.settle().await.unwrap();
        second.finish().await.unwrap();
        assert_eq!(connection.invokes.load(Ordering::SeqCst), 2);
        assert_eq!(connection.settles.load(Ordering::SeqCst), 2);

        service
            .disconnect("alice", &connected.incarnation)
            .await
            .unwrap();
        assert_eq!(connection.disconnects.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn provider_fails_closed_for_an_unverified_session_or_owner() {
        let (service, _, _) = fixture();
        service.connect("alice").await.unwrap();
        assert!(matches!(
            service.resource("alice", "delegated-without-session-id").await,
            Err(Error::ActionDenied)
        ));
        assert_eq!(
            service.snapshot("mallory"),
            Err(AttachedProviderError::OwnerMismatch)
        );
    }

    #[tokio::test]
    async fn user_disconnect_revokes_every_session_and_drains_active_claims() {
        let (service, _, connection) = fixture();
        let connected = service.connect("alice").await.unwrap();
        let resource = service.resource("alice", "session-active").await.unwrap();
        let turn = resource.begin_run().await.unwrap();
        turn.invoke(AttachedBrowserCommand::Observe {
            tab_id: "tab".into(),
        })
        .await
        .unwrap();

        service
            .disconnect("alice", &connected.incarnation)
            .await
            .unwrap();
        assert_eq!(connection.settles.load(Ordering::SeqCst), 1);
        assert_eq!(connection.disconnects.load(Ordering::SeqCst), 1);
        assert_eq!(
            turn.invoke(AttachedBrowserCommand::Observe {
                tab_id: "tab".into(),
            })
            .await,
            Err(Error::StaleRun)
        );
        let fresh = service.resource("alice", "session-fresh").await.unwrap();
        assert!(matches!(fresh.begin_run().await, Err(Error::Unavailable)));
    }

    #[tokio::test]
    async fn disconnect_fences_pending_inventory_and_waits_its_registered_owner() {
        let (service, _, connection) = fixture();
        let connected = service.connect("alice").await.unwrap();
        let resource = service.resource("alice", "session-race").await.unwrap();
        connection.block_tabs.store(true, Ordering::Release);

        let begin = tokio::spawn(async move { resource.begin_run().await });
        connection.tabs_started.notified().await;
        let pending = service.runtime.state.lock().unwrap().runs.values().next().unwrap().clone();
        let disconnect = {
            let service = service.clone();
            tokio::spawn(async move {
                service
                    .disconnect("alice", &connected.incarnation)
                    .await
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(3), pending.cancel.cancelled()).await.unwrap();
        assert_eq!(service.snapshot("alice").unwrap().unwrap().state, AttachedProviderState::Disconnecting);
        assert!(!disconnect.is_finished(), "pending native inventory must retain its original owner");
        let late = service.resource("alice", "session-late").await.unwrap();
        assert!(matches!(late.begin_run().await, Err(Error::Unavailable)));

        connection.tabs_release.notify_one();
        assert!(matches!(begin.await.unwrap(), Err(Error::Disconnected | Error::Cancelled | Error::StaleRun)));
        disconnect.await.unwrap().unwrap();
        assert!(service.runtime.state.lock().unwrap().runs.is_empty());
        assert!(service.runtime.state.lock().unwrap().claims.is_empty());
        assert_eq!(service.snapshot("alice").unwrap(), None);
    }

    #[tokio::test]
    async fn independent_targets_progress_while_same_run_operations_keep_order_and_claims() {
        let (service, _, connection) = fixture();
        service.connect("alice").await.unwrap();
        let first = service.resource("alice", "session-first").await.unwrap().begin_run().await.unwrap();
        let second = service.resource("alice", "session-second").await.unwrap().begin_run().await.unwrap();
        connection.block_invokes.store(true, Ordering::Release);
        let busy = first.clone();
        let native = tokio::spawn(async move { busy.invoke(AttachedBrowserCommand::Observe { tab_id: "tab".into() }).await });
        connection.invoke_started.notified().await;
        let mut queued = Box::pin(first.invoke(AttachedBrowserCommand::Observe { tab_id: "tab".into() }));
        assert!(futures_util::poll!(&mut queued).is_pending());
        tokio::time::timeout(std::time::Duration::from_secs(3), second.invoke(
            AttachedBrowserCommand::Observe { tab_id: "other-tab".into() },
        )).await.expect("an unrelated target must not wait on the first Run").unwrap();
        assert_eq!(second.invoke(AttachedBrowserCommand::Observe { tab_id: "tab".into() }).await, Err(Error::Busy));
        assert_eq!(connection.invokes.load(Ordering::SeqCst), 2, "the queued same-Run command must not enter native work");
        connection.block_invokes.store(false, Ordering::Release);
        connection.invoke_release.add_permits(1);
        native.await.unwrap().unwrap();
        queued.await.unwrap();
        assert_eq!(*connection.invoke_order.lock().unwrap(), vec!["tab", "other-tab", "tab"]);
        first.settle().await.unwrap(); first.finish().await.unwrap();
        second.settle().await.unwrap(); second.finish().await.unwrap();
        service.shutdown().await.unwrap();
        assert!(service.runtime.state.lock().unwrap().claims.is_empty());
    }

    #[tokio::test]
    async fn disconnect_preempts_native_work_and_dropped_waiters_do_not_release_its_owner() {
        let (service, _, connection) = fixture();
        let connected = service.connect("alice").await.unwrap();
        let turn = service.resource("alice", "session-native").await.unwrap().begin_run().await.unwrap();
        connection.block_invokes.store(true, Ordering::Release);
        let busy = turn.clone();
        let caller = tokio::spawn(async move { busy.invoke(AttachedBrowserCommand::Observe { tab_id: "tab".into() }).await });
        connection.invoke_started.notified().await;
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        assert_eq!(connection.active_invokes.load(Ordering::SeqCst), 1,
            "dropping a waiter must not drop a dispatched native future");
        let mut queued = Box::pin(turn.invoke(AttachedBrowserCommand::Observe { tab_id: "other-tab".into() }));
        assert!(futures_util::poll!(&mut queued).is_pending());
        let closing = service.clone();
        let disconnect = tokio::spawn(async move { closing.disconnect("alice", &connected.incarnation).await });
        tokio::time::timeout(std::time::Duration::from_secs(3), connection.invoke_cancelled.notified()).await
            .expect("disconnect must signal the native owner without waiting for its operation lock");
        assert_eq!(queued.await, Err(Error::Cancelled));
        assert_eq!(service.snapshot("alice").unwrap().unwrap().state, AttachedProviderState::Disconnecting);
        assert_eq!(connection.invokes.load(Ordering::SeqCst), 1, "closing must reject the queued native dispatch");
        assert_eq!(connection.settles.load(Ordering::SeqCst), 0);
        assert_eq!(connection.disconnects.load(Ordering::SeqCst), 0);
        assert!(!disconnect.is_finished(), "a cancellation signal is not native cleanup proof");
        disconnect.abort();
        assert!(disconnect.await.unwrap_err().is_cancelled());
        connection.invoke_release.add_permits(1);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while service.snapshot("alice").unwrap().is_some() { tokio::task::yield_now().await; }
        }).await.expect("the retained disconnect owner must finish after its caller leaves");
        assert_eq!(connection.completed_invokes.load(Ordering::SeqCst), 1);
        assert_eq!(connection.active_invokes.load(Ordering::SeqCst), 0);
        assert_eq!(connection.settles.load(Ordering::SeqCst), 1);
        assert_eq!(connection.disconnects.load(Ordering::SeqCst), 1);
        assert!(service.runtime.state.lock().unwrap().runs.is_empty());
        assert!(service.runtime.state.lock().unwrap().claims.is_empty());
    }

    #[tokio::test]
    async fn shutdown_fences_and_disposes_a_retained_constructor_after_its_waiter_leaves() {
        let (service, factory, connection) = fixture();
        factory.block_connect.store(true, Ordering::Release);
        let connecting = service.clone();
        let caller = tokio::spawn(async move { connecting.connect("alice").await });
        factory.connect_started.notified().await;
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        let mut shutdown = Box::pin(service.shutdown());
        assert!(futures_util::poll!(&mut shutdown).is_pending());
        assert_eq!(service.connect("alice").await, Err(AttachedProviderError::Closed));
        factory.connect_release.notify_one();
        shutdown.await.unwrap();
        assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
        assert_eq!(connection.disconnects.load(Ordering::SeqCst), 1);
        assert!(service.snapshot("alice").unwrap().is_none());
    }

    #[tokio::test]
    async fn deleting_one_agent_session_keeps_the_installation_connection_and_other_sessions() {
        let (service, _, connection) = fixture();
        service.connect("alice").await.unwrap();
        let deleted = service.resource("alice", "session-deleted").await.unwrap();
        let turn = deleted.begin_run().await.unwrap();
        turn.invoke(AttachedBrowserCommand::Observe {
            tab_id: "tab".into(),
        })
        .await
        .unwrap();

        service
            .close_agent_session("alice", "session-deleted")
            .await
            .unwrap();
        assert!(service.snapshot("alice").unwrap().is_some());
        assert_eq!(connection.disconnects.load(Ordering::SeqCst), 0);
        assert!(matches!(deleted.begin_run().await, Err(Error::StaleRun)));

        let retained = service.resource("alice", "session-retained").await.unwrap();
        let retained = retained.begin_run().await.unwrap();
        retained
            .invoke(AttachedBrowserCommand::Observe {
                tab_id: "tab".into(),
            })
            .await
            .unwrap();
        retained.settle().await.unwrap();
        retained.finish().await.unwrap();
    }

    #[tokio::test]
    async fn dropped_run_retains_settle_and_claim_cleanup_authority() {
        let (service, _, connection) = fixture();
        service.connect("alice").await.unwrap();
        let resource = service.resource("alice", "session-drop").await.unwrap();
        let turn = resource.begin_run().await.unwrap();
        turn.invoke(AttachedBrowserCommand::Observe {
            tab_id: "tab".into(),
        })
        .await
        .unwrap();
        drop(turn);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if service
                    .runtime
                    .state
                    .lock()
                    .unwrap()
                    .runs
                    .is_empty()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(connection.settles.load(Ordering::SeqCst), 1);
        assert!(service
            .runtime
            .state
            .lock()
            .unwrap()
            .claims
            .is_empty());
    }
}
