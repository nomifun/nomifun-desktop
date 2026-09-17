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
    operation: tokio::sync::Mutex<()>,
    verifier: OnceLock<Arc<dyn AttachedBrowserSessionVerifier>>,
    runtime: Arc<ProviderRuntime>,
    weak: Weak<Self>,
}

impl AttachedChromeProviderService {
    pub fn new() -> Arc<Self> {
        Self::with_factory(Arc::new(RunningChromeFactory))
    }

    pub(crate) fn with_factory(factory: Arc<dyn AttachedProviderConnectionFactory>) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            factory,
            state: Mutex::new(ServiceState::default()),
            operation: tokio::sync::Mutex::new(()),
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
        let _operation = self.operation.lock().await;
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
            owner_id: owner_id.to_owned(),
            incarnation: nomifun_common::generate_id(),
            connection,
            state: AttachedProviderState::Connected,
        };
        let snapshot = AttachedProviderSnapshot {
            incarnation: entry.incarnation.clone(),
            state: entry.state,
            chromium_major: entry.connection.chromium_major(),
        };
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .connection = Some(entry);
        unpublished.0 = None;
        Ok(snapshot)
    }

    pub async fn disconnect(
        &self,
        owner_id: &str,
        incarnation: &str,
    ) -> Result<(), AttachedProviderError> {
        let _operation = self.operation.lock().await;
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

    /// Freeze one Connected incarnation and its tab inventory while the caller
    /// owns `operation`. Disconnect uses that same barrier, so the inventory
    /// and subsequent run publication belong to one provider epoch.
    async fn frozen_connection(
        &self,
        principal_id: &str,
    ) -> Result<(String, Arc<dyn AttachedProviderConnection>, BTreeMap<String, AttachedTab>), Error> {
        let connection = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let entry = state.connection.as_ref().ok_or(Error::Unavailable)?;
            if entry.owner_id != principal_id
                || entry.state != AttachedProviderState::Connected
                || !entry.connection.is_connected()
            {
                return Err(Error::Unavailable);
            }
            (entry.incarnation.clone(), entry.connection.clone())
        };
        let tabs = connection
            .1
            .tabs()
            .await
            .map_err(|_| Error::Disconnected)?
            .into_iter()
            .map(|tab| (tab.tab_id.clone(), tab))
            .collect();
        Ok((connection.0, connection.1, tabs))
    }

    pub async fn close_agent_session(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<(), AttachedProviderError> {
        let owner = (principal_id.to_owned(), agent_session_id.to_owned());
        let _operation = self.operation.lock().await;
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
            run.settle()
                .await
                .map_err(|_| AttachedProviderError::CleanupFailed)?;
            run.finish()
                .await
                .map_err(|_| AttachedProviderError::CleanupFailed)?;
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), AttachedProviderError> {
        let _operation = self.operation.lock().await;
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            if let Some(entry) = state.connection.as_mut() {
                entry.state = AttachedProviderState::Disconnecting;
            }
        }
        self.runtime.shutdown().await?;
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
    tabs: BTreeMap<String, AttachedTab>,
    phase: AtomicU8,
    cancel: CancellationToken,
    operation: tokio::sync::Mutex<()>,
    touched: Mutex<BTreeSet<String>>,
    orphaned: AtomicBool,
    wake: tokio::sync::Notify,
}

struct Turn(Arc<Run>);

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
        // Admission, inventory and run publication share the same epoch as
        // disconnect. Revalidate after acquiring it: canonical deletion can
        // retire this Session while a caller is waiting here.
        let _provider_operation = self.service.operation.lock().await;
        self.service.verify_session(&self.owner).await?;
        if self
            .service
            .runtime
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retired_sessions
            .contains(&self.owner)
        {
            return Err(Error::StaleRun);
        }
        let (incarnation, connection, tabs) = self
            .service
            .frozen_connection(&self.owner.0)
            .await?;
        let runtime = self.service.runtime.clone();
        let run = {
            let mut state = runtime.state.lock().unwrap_or_else(|error| error.into_inner());
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
                incarnation,
                connection,
                tabs,
                phase: AtomicU8::new(ACTIVE),
                cancel: CancellationToken::new(),
                operation: tokio::sync::Mutex::new(()),
                touched: Mutex::new(BTreeSet::new()),
                orphaned: AtomicBool::new(false),
                wake: tokio::sync::Notify::new(),
            });
            state.runs.insert(self.owner.clone(), run.clone());
            run
        };
        tokio::spawn(custodian(run.clone(), runtime));
        Ok(Arc::new(Turn(run)))
    }
}

impl Run {
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
        let entry = state.connection.as_ref().ok_or(Error::Disconnected)?;
        if entry.state != AttachedProviderState::Connected
            || entry.incarnation != self.incarnation
            || !Arc::ptr_eq(&entry.connection, &self.connection)
            || !entry.connection.is_connected()
        {
            return Err(Error::Disconnected);
        }
        Ok(())
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
        let run = &self.0;
        // Take the provider epoch before the per-run operation lock. Disconnect
        // takes the same order before draining runs, avoiding both an admission
        // gap and a lock inversion. Once Disconnecting is published, no later
        // operation or target claim can enter.
        let service = run.service.upgrade().ok_or(Error::Disconnected)?;
        let _provider_operation = service.operation.lock().await;
        let _operation = run.operation.lock().await;
        let runtime = run.current_runtime()?;
        if run.phase.load(Ordering::Acquire) != ACTIVE || run.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        run.current_connection()?;
        let tab_id = match &command {
            AttachedBrowserCommand::Tabs {} => {
                let tabs = run
                    .tabs
                    .values()
                    .map(|tab| {
                        json!({
                            "tab_id": tab.tab_id,
                            "title": tab.title,
                            "url": nomifun_browser_platform::url_projection::project_metadata_url(&tab.url),
                        })
                    })
                    .collect::<Vec<_>>();
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
        if !run.tabs.contains_key(&tab_id) {
            return Err(Error::TabDenied);
        }
        let target_key = run.connection.target_key(&tab_id)?;
        {
            let mut state = runtime.state.lock().unwrap_or_else(|error| error.into_inner());
            if state
                .claims
                .get(&target_key)
                .is_some_and(|run_id| run_id != &run.id)
            {
                return Err(Error::Busy);
            }
            state.claims.insert(target_key, run.id.clone());
        }
        run.touched
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(tab_id.clone());
        run.connection.invoke(&tab_id, command, &run.cancel).await
    }

    async fn settle(&self) -> Result<(), Error> {
        self.0.settle().await
    }

    async fn finish(&self) -> Result<(), Error> {
        self.0.finish().await
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        if self.0.phase.load(Ordering::Acquire) != FINISHED {
            self.0.orphaned.store(true, Ordering::Release);
            self.0.cancel.cancel();
            self.0.wake.notify_one();
        }
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

    async fn shutdown(&self) -> Result<(), AttachedProviderError> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed = true;
        self.drain_runs().await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    struct Factory {
        calls: AtomicUsize,
        connection: Arc<Connection>,
    }

    struct Connection {
        connected: AtomicBool,
        block_tabs: AtomicBool,
        tabs_started: tokio::sync::Notify,
        tabs_release: tokio::sync::Notify,
        invokes: AtomicUsize,
        settles: AtomicUsize,
        disconnects: AtomicUsize,
    }

    #[async_trait]
    impl AttachedProviderConnectionFactory for Factory {
        async fn connect(&self) -> Result<Arc<dyn AttachedProviderConnection>, AttachError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
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
            Ok(vec![AttachedTab {
                tab_id: "tab".into(),
                title: "Installed tab".into(),
                url: "https://example.test".into(),
            }])
        }
        fn target_key(&self, tab_id: &str) -> Result<String, Error> {
            (tab_id == "tab")
                .then(|| "a".repeat(64))
                .ok_or(Error::TabDenied)
        }
        async fn invoke(
            &self,
            _: &str,
            _: AttachedBrowserCommand,
            cancel: &CancellationToken,
        ) -> Result<Value, Error> {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            self.invokes.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"observed": true}))
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
            invokes: AtomicUsize::new(0),
            settles: AtomicUsize::new(0),
            disconnects: AtomicUsize::new(0),
        });
        let factory = Arc::new(Factory {
            calls: AtomicUsize::new(0),
            connection: connection.clone(),
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
    async fn disconnect_cannot_pass_a_frozen_inventory_before_run_publication() {
        let (service, _, connection) = fixture();
        let connected = service.connect("alice").await.unwrap();
        let resource = service.resource("alice", "session-race").await.unwrap();
        connection.block_tabs.store(true, Ordering::Release);

        let begin = tokio::spawn(async move { resource.begin_run().await });
        connection.tabs_started.notified().await;
        let disconnect = {
            let service = service.clone();
            tokio::spawn(async move {
                service
                    .disconnect("alice", &connected.incarnation)
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert!(!disconnect.is_finished(), "disconnect must wait for admission publication");

        connection.tabs_release.notify_one();
        let turn = begin.await.unwrap().unwrap();
        disconnect.await.unwrap().unwrap();
        assert!(service.runtime.state.lock().unwrap().runs.is_empty());
        assert!(service.runtime.state.lock().unwrap().claims.is_empty());
        assert_eq!(service.snapshot("alice").unwrap(), None);
        assert!(matches!(
            turn.invoke(AttachedBrowserCommand::Tabs {}).await,
            Err(Error::StaleRun | Error::Disconnected | Error::Cancelled)
        ));
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
