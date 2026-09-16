//! Explicit user connections to an already-running browser. No browser is
//! discovered by reads, and neither connection handles nor grants are persisted.
use async_trait::async_trait;
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use nomi_browser_engine::attached_browser::{
    AttachError, AttachedBrowser, GrantedTab, UserTabInventory,
};
use nomifun_browser_platform::system_browser::{SystemBrowserCommand, SystemBrowserRuntimeError};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

mod runtime;
pub use runtime::SystemBrowserOwnerVerifier;

const MAX_CONNECTIONS: usize = 128;
const MAX_GRANTED_TABS: usize = 32;
type Owner = (String, String);
type Retirement = Shared<BoxFuture<'static, Result<(), SystemBrowserError>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SystemBrowserError {
    #[error("System browser service is shutting down")]
    Closed,
    #[error("The browser connection changed; refresh before trying again")]
    StaleIncarnation,
    #[error("The browser connection is busy")]
    Busy,
    #[error("No system browser is connected")]
    NotConnected,
    #[error("Too many system browser connections")]
    Capacity,
    #[error("The connection request was cancelled")]
    Cancelled,
    #[error(transparent)]
    Connection(#[from] AttachError),
    #[error("Browser connection cleanup could not be confirmed; disconnect again")]
    CleanupFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemBrowserState {
    Connecting,
    Connected,
    ConnectionLost,
    Disconnecting,
    Disconnected,
    CleanupFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SystemBrowserTab {
    pub tab_id: String,
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SystemBrowserSnapshot {
    pub incarnation: String,
    pub state: SystemBrowserState,
    pub tabs: Vec<SystemBrowserTab>,
}

// This port is private to the host, never a model-supplied connection or token.
#[async_trait]
pub trait SystemBrowserConnection: Send + Sync {
    /// Memory-only liveness check; must not reconnect or perform browser I/O.
    fn is_connected(&self) -> bool;
    /// Immediately fence admission/pending I/O. Physical disposal is joined by disconnect.
    fn request_disconnect(&self);
    async fn choices(&self) -> Result<UserTabInventory, AttachError>;
    async fn grant(&self, choice_id: &str) -> Result<SystemBrowserTab, AttachError>;
    async fn disconnect(&self) -> Result<(), AttachError>;
    fn target_key(&self, _tab_id: &str) -> Result<String, SystemBrowserRuntimeError> {
        Err(SystemBrowserRuntimeError::Unavailable)
    }
    async fn invoke(
        &self,
        _tab_id: &str,
        _command: SystemBrowserCommand,
        _cancel: &CancellationToken,
    ) -> Result<serde_json::Value, SystemBrowserRuntimeError> {
        Err(SystemBrowserRuntimeError::Unavailable)
    }
    async fn settle(&self, _tab_id: &str) -> Result<(), SystemBrowserRuntimeError> {
        Ok(())
    }
}
#[async_trait]
pub trait SystemBrowserConnectionFactory: Send + Sync {
    async fn connect(&self) -> Result<Arc<dyn SystemBrowserConnection>, AttachError>;
}
struct RunningChromeFactory;
struct RunningChromeConnection {
    browser: AttachedBrowser,
    grants: Mutex<BTreeMap<String, GrantedTab>>,
}

/// Explicit harness-only discovery override. The connection, grants and Agent
/// runtime remain the production implementation. No HTTP/model entry uses it.
#[cfg(feature = "browser-conformance")]
pub fn conformance_connection_factory(port_file: std::path::PathBuf) -> Arc<dyn SystemBrowserConnectionFactory> {
    struct Factory(std::path::PathBuf);
    #[async_trait]
    impl SystemBrowserConnectionFactory for Factory {
        async fn connect(&self) -> Result<Arc<dyn SystemBrowserConnection>, AttachError> {
            Ok(Arc::new(RunningChromeConnection {
                browser: AttachedBrowser::connect_for_conformance(&self.0).await?,
                grants: Mutex::new(BTreeMap::new()),
            }))
        }
    }
    Arc::new(Factory(port_file))
}
#[async_trait]
impl SystemBrowserConnectionFactory for RunningChromeFactory {
    async fn connect(&self) -> Result<Arc<dyn SystemBrowserConnection>, AttachError> {
        Ok(Arc::new(RunningChromeConnection {
            browser: AttachedBrowser::connect_running_chrome().await?,
            grants: Mutex::new(BTreeMap::new()),
        }))
    }
}
#[async_trait]
impl SystemBrowserConnection for RunningChromeConnection {
    fn is_connected(&self) -> bool {
        self.browser.is_connected()
    }
    fn request_disconnect(&self) {
        self.browser.request_disconnect();
    }
    async fn choices(&self) -> Result<UserTabInventory, AttachError> {
        self.browser.tabs_for_user().await
    }
    async fn grant(&self, choice_id: &str) -> Result<SystemBrowserTab, AttachError> {
        let grant = self.browser.grant_tab(choice_id).await?;
        let info = self.browser.granted_tab_metadata(&grant).await?;
        let mut tab = SystemBrowserTab {
            tab_id: info.tab_id,
            title: info.title,
            url: info.url,
        };
        let mut grants = self.grants.lock().unwrap_or_else(|e| e.into_inner());
        if !self.browser.is_connected() {
            return Err(AttachError::ConnectionFailed);
        }
        if let Some(existing) = grants
            .values()
            .find(|existing| existing.same_target(&grant))
        {
            tab.tab_id = existing.id().to_owned();
        } else {
            if grants.len() >= MAX_GRANTED_TABS {
                return Err(AttachError::InventoryLimit);
            }
            grants.insert(grant.id().to_owned(), grant);
        }
        Ok(tab)
    }
    async fn disconnect(&self) -> Result<(), AttachError> {
        self.browser.disconnect().await?;
        self.grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        Ok(())
    }
    fn target_key(&self, tab_id: &str) -> Result<String, SystemBrowserRuntimeError> {
        self.grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(tab_id)
            .map(GrantedTab::target_key)
            .ok_or(SystemBrowserRuntimeError::TabDenied)
    }
    async fn invoke(
        &self,
        tab_id: &str,
        command: SystemBrowserCommand,
        cancel: &CancellationToken,
    ) -> Result<serde_json::Value, SystemBrowserRuntimeError> {
        let grant = self
            .grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(tab_id)
            .cloned()
            .ok_or(SystemBrowserRuntimeError::TabDenied)?;
        self.browser.execute_granted(&grant, command, cancel).await
    }
    async fn settle(&self, tab_id: &str) -> Result<(), SystemBrowserRuntimeError> {
        if !self.browser.is_connected() {
            return self
                .browser
                .disconnect()
                .await
                .map_err(|_| SystemBrowserRuntimeError::ExecutionFailed);
        }
        let grant = self
            .grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(tab_id)
            .cloned()
            .ok_or(SystemBrowserRuntimeError::TabDenied)?;
        self.browser.release_granted(&grant).await
    }
}

struct EntryData {
    state: SystemBrowserState,
    connection: Option<Arc<dyn SystemBrowserConnection>>,
    tabs: BTreeMap<String, SystemBrowserTab>,
    job: Option<Retirement>,
}
struct Entry {
    incarnation: String,
    data: Mutex<EntryData>,
    operation: tokio::sync::Mutex<()>,
    cancel_connect: CancellationToken,
}
impl Entry {
    fn snapshot(&self) -> SystemBrowserSnapshot {
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        if data.state == SystemBrowserState::Connected
            && data
                .connection
                .as_ref()
                .is_some_and(|connection| !connection.is_connected())
        {
            data.state = SystemBrowserState::ConnectionLost;
            data.tabs.clear();
        }
        SystemBrowserSnapshot {
            incarnation: self.incarnation.clone(),
            state: data.state,
            tabs: data.tabs.values().cloned().collect(),
        }
    }
}
struct ServiceState {
    entries: BTreeMap<Owner, Arc<Entry>>,
    closed: bool,
}

pub struct SystemBrowserService {
    factory: Arc<dyn SystemBrowserConnectionFactory>,
    state: Mutex<ServiceState>,
    closed: CancellationToken,
    weak: std::sync::Weak<Self>,
    owner_verifier: std::sync::OnceLock<Arc<dyn SystemBrowserOwnerVerifier>>,
    runtime: Arc<runtime::Runtime>,
}
impl SystemBrowserService {
    pub fn new() -> Arc<Self> {
        Self::with_factory(Arc::new(RunningChromeFactory))
    }
    pub fn with_factory(factory: Arc<dyn SystemBrowserConnectionFactory>) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            factory,
            state: Mutex::new(ServiceState {
                entries: BTreeMap::new(),
                closed: false,
            }),
            closed: CancellationToken::new(),
            weak: weak.clone(),
            owner_verifier: std::sync::OnceLock::new(),
            runtime: Arc::new(runtime::Runtime::default()),
        })
    }

    /// Cached metadata only: does not discover, reconnect, enumerate or refresh.
    pub fn snapshot(&self, user_id: &str, conversation_id: &str) -> Option<SystemBrowserSnapshot> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .get(&(user_id.to_owned(), conversation_id.to_owned()))
            .map(|entry| entry.snapshot())
    }

    pub async fn connect(
        self: &Arc<Self>,
        user_id: &str,
        conversation_id: &str,
        expected_incarnation: Option<&str>,
    ) -> Result<SystemBrowserSnapshot, SystemBrowserError> {
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let entry = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(SystemBrowserError::Closed);
            }
            let owner = (user_id.to_owned(), conversation_id.to_owned());
            match state.entries.get(&owner) {
                Some(entry) => {
                    if expected_incarnation != Some(entry.incarnation.as_str()) {
                        return Err(SystemBrowserError::StaleIncarnation);
                    }
                    if entry.snapshot().state != SystemBrowserState::Disconnected {
                        return Err(SystemBrowserError::Busy);
                    }
                }
                None if expected_incarnation.is_some() => {
                    return Err(SystemBrowserError::StaleIncarnation);
                }
                None => {}
            }
            if !state.entries.contains_key(&owner) && state.entries.len() >= MAX_CONNECTIONS {
                state
                    .entries
                    .retain(|_, entry| entry.snapshot().state != SystemBrowserState::Disconnected);
                if state.entries.len() >= MAX_CONNECTIONS {
                    return Err(SystemBrowserError::Capacity);
                }
            }
            let entry = Arc::new(Entry {
                incarnation: uuid::Uuid::now_v7().to_string(),
                data: Mutex::new(EntryData {
                    state: SystemBrowserState::Connecting,
                    connection: None,
                    tabs: BTreeMap::new(),
                    job: None,
                }),
                operation: tokio::sync::Mutex::new(()),
                cancel_connect: CancellationToken::new(),
            });
            let owned = entry.clone();
            let factory = self.factory.clone();
            let closed = self.closed.clone();
            // The owned worker survives a dropped HTTP future. Its acceptance
            // handshake also covers cancellation after the socket was created.
            let task = tokio::spawn(async move {
                let _operation = owned.operation.lock().await;
                let connection = match factory.connect().await {
                    Ok(connection) => connection,
                    Err(error) => {
                        let mut data = owned.data.lock().unwrap_or_else(|e| e.into_inner());
                        if data.state != SystemBrowserState::Disconnecting {
                            data.state = SystemBrowserState::Disconnected;
                        }
                        let _ = ready_tx.send(Err(SystemBrowserError::Connection(error)));
                        return Ok(());
                    }
                };
                {
                    let mut data = owned.data.lock().unwrap_or_else(|e| e.into_inner());
                    data.connection = Some(connection);
                    if closed.is_cancelled() || owned.cancel_connect.is_cancelled() {
                        data.state = SystemBrowserState::Disconnecting;
                    } else if data.state != SystemBrowserState::Disconnecting {
                        data.state = SystemBrowserState::Connected;
                    }
                }
                if !closed.is_cancelled()
                    && !owned.cancel_connect.is_cancelled()
                    && ready_tx.send(Ok(owned.snapshot())).is_ok()
                {
                    let accepted = tokio::select! { biased;
                        _ = closed.cancelled() => false,
                        _ = owned.cancel_connect.cancelled() => false,
                        result = accepted_rx => result.is_ok(),
                    };
                    if accepted {
                        return Ok(());
                    }
                }
                Self::close_entry(&owned).await
            });
            entry.data.lock().unwrap_or_else(|e| e.into_inner()).job = Some(
                async move { task.await.unwrap_or(Err(SystemBrowserError::CleanupFailed)) }
                    .boxed()
                    .shared(),
            );
            state.entries.insert(owner, entry.clone());
            entry
        };
        let result = tokio::select! { biased;
            _ = self.closed.cancelled() => return Err(SystemBrowserError::Closed),
            _ = entry.cancel_connect.cancelled() => return Err(SystemBrowserError::Cancelled),
            result = ready_rx => result,
        }
        .map_err(|_| SystemBrowserError::Cancelled)??;
        if self.closed.is_cancelled() {
            return Err(SystemBrowserError::Closed);
        }
        if entry.cancel_connect.is_cancelled() {
            return Err(SystemBrowserError::Cancelled);
        }
        accepted_tx
            .send(())
            .map_err(|_| SystemBrowserError::Cancelled)?;
        Ok(result)
    }

    fn entry(
        &self,
        user_id: &str,
        conversation_id: &str,
        incarnation: &str,
        allow_closed: bool,
    ) -> Result<Arc<Entry>, SystemBrowserError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed && !allow_closed {
            return Err(SystemBrowserError::Closed);
        }
        let entry = state
            .entries
            .get(&(user_id.to_owned(), conversation_id.to_owned()))
            .ok_or(SystemBrowserError::NotConnected)?;
        if entry.incarnation != incarnation {
            return Err(SystemBrowserError::StaleIncarnation);
        }
        Ok(entry.clone())
    }
    fn connected(
        &self,
        entry: &Entry,
    ) -> Result<Arc<dyn SystemBrowserConnection>, SystemBrowserError> {
        if self.closed.is_cancelled() {
            return Err(SystemBrowserError::Closed);
        }
        entry.snapshot();
        let data = entry.data.lock().unwrap_or_else(|e| e.into_inner());
        if data.state != SystemBrowserState::Connected {
            return Err(SystemBrowserError::NotConnected);
        }
        data.connection
            .clone()
            .ok_or(SystemBrowserError::NotConnected)
    }
    pub async fn choices(
        self: &Arc<Self>,
        user_id: &str,
        conversation_id: &str,
        incarnation: &str,
    ) -> Result<UserTabInventory, SystemBrowserError> {
        let entry = self.entry(user_id, conversation_id, incarnation, false)?;
        let _operation = entry.operation.lock().await;
        let inventory = self.connected(&entry)?.choices().await?;
        self.connected(&entry)?;
        Ok(inventory)
    }
    pub async fn grant(
        self: &Arc<Self>,
        user_id: &str,
        conversation_id: &str,
        incarnation: &str,
        choice_id: &str,
    ) -> Result<SystemBrowserSnapshot, SystemBrowserError> {
        let entry = self.entry(user_id, conversation_id, incarnation, false)?;
        let _operation = entry.operation.lock().await;
        if entry
            .data
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tabs
            .len()
            >= MAX_GRANTED_TABS
        {
            return Err(SystemBrowserError::Capacity);
        }
        let tab = self.connected(&entry)?.grant(choice_id).await?;
        self.connected(&entry)?;
        {
            let mut data = entry.data.lock().unwrap_or_else(|e| e.into_inner());
            if data.state != SystemBrowserState::Connected {
                return Err(SystemBrowserError::NotConnected);
            }
            data.tabs.insert(tab.tab_id.clone(), tab);
        }
        Ok(entry.snapshot())
    }

    async fn close_entry(entry: &Entry) -> Result<(), SystemBrowserError> {
        let connection = entry
            .data
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .connection
            .clone();
        let result = match connection {
            Some(connection) => connection
                .disconnect()
                .await
                .map_err(|_| SystemBrowserError::CleanupFailed),
            None => Ok(()),
        };
        let mut data = entry.data.lock().unwrap_or_else(|e| e.into_inner());
        data.tabs.clear();
        if result.is_ok() {
            data.connection = None;
            data.state = SystemBrowserState::Disconnected;
        } else {
            data.state = SystemBrowserState::CleanupFailed;
        }
        result
    }
    fn start_disconnect(entry: &Arc<Entry>) -> Retirement {
        let mut data = entry.data.lock().unwrap_or_else(|e| e.into_inner());
        if data.state == SystemBrowserState::Disconnecting {
            if let Some(job) = &data.job {
                return job.clone();
            }
        }
        let retry_failed_cleanup = data.state == SystemBrowserState::CleanupFailed;
        entry.cancel_connect.cancel();
        if let Some(connection) = &data.connection {
            connection.request_disconnect();
        }
        data.state = SystemBrowserState::Disconnecting;
        data.tabs.clear();
        let previous = data.job.take();
        let owned = entry.clone();
        let task = tokio::spawn(async move {
            // Never abandon a connecting socket or another cleanup attempt.
            if let Some(previous) = previous {
                if let Err(error) = previous.await {
                    // A cancelled connect may already have attempted cleanup.
                    // Report that exact failure once; only a later explicit
                    // disconnect/shutdown attempt may retry the retained owner.
                    if !retry_failed_cleanup {
                        owned.data.lock().unwrap_or_else(|e| e.into_inner()).state =
                            SystemBrowserState::CleanupFailed;
                        return Err(error);
                    }
                }
            }
            let _operation = owned.operation.lock().await;
            Self::close_entry(&owned).await
        });
        let job = async move { task.await.unwrap_or(Err(SystemBrowserError::CleanupFailed)) }
            .boxed()
            .shared();
        data.job = Some(job.clone());
        job
    }

    /// Allows the local user's disconnect request to cancel a pending connect
    /// before waiting on the conversation preparation gate. Never affects runs.
    pub fn cancel_pending_connect(
        &self,
        user_id: &str,
        conversation_id: &str,
        incarnation: &str,
    ) -> Result<(), SystemBrowserError> {
        let entry = self.entry(user_id, conversation_id, incarnation, true)?;
        if entry.snapshot().state == SystemBrowserState::Connecting {
            entry.cancel_connect.cancel();
        }
        Ok(())
    }
    pub async fn disconnect(
        self: &Arc<Self>,
        user_id: &str,
        conversation_id: &str,
        incarnation: &str,
    ) -> Result<SystemBrowserSnapshot, SystemBrowserError> {
        let entry = self.entry(user_id, conversation_id, incarnation, true)?;
        Self::start_disconnect(&entry).await?;
        Ok(entry.snapshot())
    }
    pub async fn shutdown(self: &Arc<Self>) -> Result<(), SystemBrowserError> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        self.closed.cancel();
        self.runtime
            .shutdown()
            .await
            .map_err(|_| SystemBrowserError::CleanupFailed)?;
        let jobs = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            self.closed.cancel();
            state
                .entries
                .values()
                .map(Self::start_disconnect)
                .collect::<Vec<_>>()
        };
        let mut failed = false;
        for job in jobs {
            failed |= job.await.is_err();
        }
        if failed {
            Err(SystemBrowserError::CleanupFailed)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
