//! Remote MCP session authority and lifecycle cleanup.
//!
//! rmcp owns the transport worker, but the Remote front door owns browser
//! attachment policy. This wrapper injects the server-generated logical
//! session id into every MCP request and revokes that exact browser owner when
//! rmcp closes the session (DELETE, idle timeout, or worker exit).

use std::{collections::HashMap, sync::Arc};
#[cfg(feature = "browser-use")]
use std::{collections::HashSet, time::Duration};

use futures::Stream;
use nomifun_common::CompanionId;
use nomifun_gateway::GatewayDeps;
use rmcp::model::{ClientJsonRpcMessage, GetExtensions, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_server::session::{
    RestoreOutcome, ServerSseMessage, SessionId, SessionManager,
    local::{LocalSessionManager, LocalSessionManagerError, LocalSessionWorker},
};
use rmcp::transport::WorkerTransport;
use thiserror::Error;

/// Trusted marker copied into the request context by the session manager.
///
/// Its value is generated and validated by rmcp's server-side session map; a
/// client-supplied `Mcp-Session-Id` is accepted only after `has_session`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteMcpSessionId(pub SessionId);

/// The server-pinned identity for one logical Remote MCP session.
///
/// This is deliberately copied into every message by [`RemoteSessionManager`].
/// The HTTP bearer token is checked on every request, but a later request must
/// still resolve to the same companion that authenticated `initialize`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteMcpSessionIdentity {
    pub session_id: SessionId,
    pub companion_id: CompanionId,
    /// The capability-domain scope selected during `initialize`.
    ///
    /// `None` means the full Remote catalog. This is server-pinned and is
    /// never re-derived from a later request URI.
    pub scope: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RemoteMcpSessionBinding {
    companion_id: CompanionId,
    scope: Option<Vec<String>>,
}

fn canonical_scope<'a>(domains: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut scope: Vec<String> = domains.into_iter().map(ToOwned::to_owned).collect();
    scope.sort_unstable();
    scope.dedup();
    scope
}

fn binding_accepts_request(
    binding: &RemoteMcpSessionBinding,
    companion_id: &CompanionId,
    requested_scope: Option<&[String]>,
    explicit_scope: bool,
) -> bool {
    binding.companion_id == *companion_id
        && (!explicit_scope || binding.scope.as_deref() == requested_scope)
}

#[cfg(feature = "browser-use")]
const REMOTE_BROWSER_CLEANUP_INTERVAL: Duration = Duration::from_millis(250);
#[cfg(feature = "browser-use")]
const REMOTE_BROWSER_FINAL_RETRY_ATTEMPTS: usize = 4;
#[cfg(feature = "browser-use")]
const REMOTE_BROWSER_FINAL_RETRY_WAIT: Duration = Duration::from_millis(50);

#[cfg(feature = "browser-use")]
trait RemoteBrowserCleanupRegistry: Send + Sync {
    fn revoke_trusted_identity<'a>(
        &'a self,
        runtime_instance_id: &'a str,
    ) -> futures::future::BoxFuture<
        'a,
        Result<(), nomifun_browser_platform::BrowserPlatformError>,
    >;

    fn retry_pending_browser_cleanups(
        &self,
    ) -> futures::future::BoxFuture<'_, ()>;
}

#[cfg(feature = "browser-use")]
impl RemoteBrowserCleanupRegistry
    for nomifun_gateway::browser_registry::BrowserRegistry
{
    fn revoke_trusted_identity<'a>(
        &'a self,
        runtime_instance_id: &'a str,
    ) -> futures::future::BoxFuture<
        'a,
        Result<(), nomifun_browser_platform::BrowserPlatformError>,
    > {
        Box::pin(async move {
            nomifun_gateway::browser_registry::BrowserRegistry::revoke_trusted_identity(
                self,
                runtime_instance_id,
            )
            .await
            .map(|_| ())
        })
    }

    fn retry_pending_browser_cleanups(
        &self,
    ) -> futures::future::BoxFuture<'_, ()> {
        Box::pin(async move {
            nomifun_gateway::browser_registry::BrowserRegistry::retry_pending_browser_cleanups(
                self,
            )
            .await;
        })
    }
}

/// Durable retry authority for Remote MCP browser attachments.
///
/// `BrowserRegistry::revoke_trusted_identity` retains failed exact-owner
/// cleanup as `revocation_pending`; this process-lifetime worker keeps retrying
/// those records after DELETE/idle/worker-exit callbacks have returned. The
/// local `pending_sessions` set additionally preserves lifecycle attribution
/// until the registry revoke itself reports success.
#[cfg(feature = "browser-use")]
struct RemoteBrowserCleanupState {
    registry: Arc<dyn RemoteBrowserCleanupRegistry>,
    bindings: Arc<tokio::sync::RwLock<HashMap<SessionId, RemoteMcpSessionBinding>>>,
    pending_sessions: tokio::sync::Mutex<HashSet<String>>,
}

#[cfg(feature = "browser-use")]
#[derive(Clone)]
struct RemoteBrowserCleanupAuthority {
    state: Arc<RemoteBrowserCleanupState>,
    shutdown: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

#[cfg(feature = "browser-use")]
impl RemoteBrowserCleanupAuthority {
    fn new(
        registry: nomifun_gateway::browser_registry::BrowserRegistry,
        bindings: Arc<tokio::sync::RwLock<HashMap<SessionId, RemoteMcpSessionBinding>>>,
    ) -> Self {
        Self::with_registry(Arc::new(registry), bindings)
    }

    fn with_registry(
        registry: Arc<dyn RemoteBrowserCleanupRegistry>,
        bindings: Arc<tokio::sync::RwLock<HashMap<SessionId, RemoteMcpSessionBinding>>>,
    ) -> Self {
        let state = Arc::new(RemoteBrowserCleanupState {
            registry,
            bindings,
            pending_sessions: tokio::sync::Mutex::new(HashSet::new()),
        });
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let worker_state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(REMOTE_BROWSER_CLEANUP_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        retry_remote_browser_cleanups(&worker_state).await;
                    }
                    _ = &mut shutdown_rx => {
                        drain_remote_browser_cleanups(&worker_state).await;
                        break;
                    }
                }
            }
        });
        Self {
            state,
            shutdown: Arc::new(std::sync::Mutex::new(Some(shutdown_tx))),
        }
    }

    async fn revoke_or_queue(&self, runtime_instance_id: &str) {
        self.state
            .pending_sessions
            .lock()
            .await
            .insert(runtime_instance_id.to_owned());
        try_revoke_remote_browser_session(&self.state, runtime_instance_id).await;
    }

    #[cfg(test)]
    async fn pending_count(&self) -> usize {
        self.state.pending_sessions.lock().await.len()
    }
}

#[cfg(feature = "browser-use")]
impl Drop for RemoteBrowserCleanupAuthority {
    fn drop(&mut self) {
        if Arc::strong_count(&self.shutdown) != 1 {
            return;
        }
        if let Some(shutdown) = self
            .shutdown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            // The worker owns its state independently. Dropping its JoinHandle
            // never cancels the final drain.
            let _ = shutdown.send(());
        }
    }
}

#[cfg(feature = "browser-use")]
async fn try_revoke_remote_browser_session(
    state: &RemoteBrowserCleanupState,
    runtime_instance_id: &str,
) {
    match state
        .registry
        .revoke_trusted_identity(runtime_instance_id)
        .await
    {
        Ok(_) => {
            state
                .pending_sessions
                .lock()
                .await
                .remove(runtime_instance_id);
        }
        Err(error) => {
            tracing::warn!(
                session_id = runtime_instance_id,
                code = ?error.code,
                "Remote MCP browser attachment revoke failed; retained for retry"
            );
        }
    }
}

#[cfg(feature = "browser-use")]
async fn retry_remote_browser_cleanups(state: &RemoteBrowserCleanupState) {
    let pending = state
        .pending_sessions
        .lock()
        .await
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    for runtime_instance_id in pending {
        try_revoke_remote_browser_session(state, &runtime_instance_id).await;
    }
    // Also covers a registry-side pending record left after a caller was
    // cancelled between the registry transition and local bookkeeping.
    state.registry.retry_pending_browser_cleanups().await;
}

#[cfg(feature = "browser-use")]
async fn drain_remote_browser_cleanups(state: &RemoteBrowserCleanupState) {
    // Any still-bound session is ending with the manager/router itself. Queue
    // it before the final retry rounds so process shutdown cannot strand it.
    let bound = state
        .bindings
        .read()
        .await
        .keys()
        .map(|id| id.as_ref().to_owned())
        .collect::<Vec<_>>();
    state.pending_sessions.lock().await.extend(bound);
    for attempt in 0..REMOTE_BROWSER_FINAL_RETRY_ATTEMPTS {
        retry_remote_browser_cleanups(state).await;
        if state.pending_sessions.lock().await.is_empty() {
            break;
        }
        if attempt + 1 < REMOTE_BROWSER_FINAL_RETRY_ATTEMPTS {
            tokio::time::sleep(REMOTE_BROWSER_FINAL_RETRY_WAIT).await;
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum RemoteSessionManagerError {
    #[error(transparent)]
    Local(#[from] LocalSessionManagerError),
    #[error("{0}")]
    Transport(#[from] std::io::Error),
}

pub(crate) struct RemoteSessionManager {
    inner: LocalSessionManager,
    domains: Option<Vec<String>>,
    bindings: Arc<tokio::sync::RwLock<HashMap<SessionId, RemoteMcpSessionBinding>>>,
    #[cfg(feature = "browser-use")]
    browser_cleanup: Option<RemoteBrowserCleanupAuthority>,
}

impl RemoteSessionManager {
    pub(crate) fn new(
        deps: Arc<GatewayDeps>,
        domains: Option<&'static [&'static str]>,
    ) -> Self {
        #[cfg(not(feature = "browser-use"))]
        let _ = deps;
        let bindings = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        #[cfg(feature = "browser-use")]
        let browser_cleanup = deps
            .browser_registry
            .clone()
            .map(|registry| {
                RemoteBrowserCleanupAuthority::new(
                    registry,
                    Arc::clone(&bindings),
                )
            });
        Self {
            inner: LocalSessionManager::default(),
            domains: domains.map(|domains| canonical_scope(domains.iter().copied())),
            bindings,
            #[cfg(feature = "browser-use")]
            browser_cleanup,
        }
    }

    fn scope_from_message(
        &self,
        message: &ClientJsonRpcMessage,
    ) -> Result<(Option<Vec<String>>, bool), std::io::Error> {
        if let Some(domains) = &self.domains {
            return Ok((Some(domains.clone()), true));
        }
        let parts = match message {
            ClientJsonRpcMessage::Request(request) => request
                .request
                .extensions()
                .get::<axum::http::request::Parts>(),
            ClientJsonRpcMessage::Notification(notification) => notification
                .notification
                .extensions()
                .get::<axum::http::request::Parts>(),
            ClientJsonRpcMessage::Response(_) | ClientJsonRpcMessage::Error(_) => None,
        }
        .ok_or_else(|| {
            std::io::Error::other(
                "authenticated Remote MCP request has no HTTP request parts",
            )
        })?;
        let explicit = parts.uri.query().is_some_and(|query| {
            query.split('&').any(|pair| {
                let key = pair.split_once('=').map_or(pair, |(key, _)| key);
                key == "domains" || key == "profile"
            })
        });
        Ok((
            crate::handler::domain_scope_from_query(parts.uri.query())
                .map(|scope| canonical_scope(scope.iter().map(String::as_str))),
            explicit,
        ))
    }

    fn companion_from_message(
        message: &ClientJsonRpcMessage,
    ) -> Result<CompanionId, std::io::Error> {
        let parts = match message {
            ClientJsonRpcMessage::Request(request) => request
                .request
                .extensions()
                .get::<axum::http::request::Parts>(),
            ClientJsonRpcMessage::Notification(notification) => notification
                .notification
                .extensions()
                .get::<axum::http::request::Parts>(),
            ClientJsonRpcMessage::Response(_) | ClientJsonRpcMessage::Error(_) => None,
        }
            .ok_or_else(|| {
                std::io::Error::other(
                    "authenticated Remote MCP request has no HTTP request parts",
                )
            })?;
        parts
            .extensions
            .get::<crate::router::RemoteCompanion>()
            .map(|companion| companion.0.clone())
            .ok_or_else(|| {
                std::io::Error::other(
                    "authenticated Remote MCP request has no canonical companion identity",
                )
            })
    }

    async fn inject_pinned_identity(
        &self,
        id: &SessionId,
        message: &mut ClientJsonRpcMessage,
        pin_if_missing: bool,
    ) -> Result<(), std::io::Error> {
        let companion = Self::companion_from_message(message)?;
        let (requested_scope, explicit_scope) = self.scope_from_message(message)?;
        let mut bindings = self.bindings.write().await;
        match bindings.get(id) {
            Some(binding)
                if !binding_accepts_request(
                    binding,
                    &companion,
                    requested_scope.as_deref(),
                    explicit_scope,
                ) =>
            {
                let message = if binding.companion_id != companion {
                    "Remote MCP session is bound to a different companion"
                } else {
                    "Remote MCP session is bound to a different capability scope"
                };
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    message,
                ));
            }
            None if pin_if_missing => {
                bindings.insert(
                    id.clone(),
                    RemoteMcpSessionBinding {
                        companion_id: companion.clone(),
                        scope: requested_scope.clone(),
                    },
                );
            }
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Remote MCP session has no pinned identity",
                ));
            }
            Some(_) => {}
        }
        let binding = bindings
            .get(id)
            .expect("session binding inserted or already present")
            .clone();
        drop(bindings);
        message.insert_extension(RemoteMcpSessionIdentity {
            session_id: id.clone(),
            companion_id: binding.companion_id,
            scope: binding.scope,
        });
        // Keep the narrower marker for code/tests that only need the logical
        // session id. The identity above is the authoritative companion pin.
        message.insert_extension(RemoteMcpSessionId(id.clone()));
        Ok(())
    }

    async fn revoke_browser_attachment(&self, id: &SessionId) {
        #[cfg(feature = "browser-use")]
        if let Some(cleanup) = self.browser_cleanup.as_ref() {
            cleanup.revoke_or_queue(id.as_ref()).await;
        }
        #[cfg(not(feature = "browser-use"))]
        let _ = id;
    }

    async fn discard_failed_initialization(&self, id: &SessionId) {
        // `StreamableHttpService` allocates the LocalSession worker before it
        // forwards initialize.  If initialize fails, rmcp does not get as far
        // as its normal worker-exit callback, so close the worker here instead
        // of leaving it alive until the init timeout.
        if let Err(error) = self.inner.close_session(id).await {
            tracing::debug!(
                session_id = id.as_ref(),
                error = %error,
                "Remote MCP initialize cleanup found the session worker already closed"
            );
        }
        self.bindings.write().await.remove(id);
        self.revoke_browser_attachment(id).await;
    }

    pub(crate) async fn companion_matches(
        &self,
        id: &SessionId,
        companion_id: &CompanionId,
    ) -> bool {
        self.bindings
            .read()
            .await
            .get(id)
            .is_some_and(|binding| binding.companion_id == *companion_id)
    }
}

impl SessionManager for RemoteSessionManager {
    type Error = RemoteSessionManagerError;
    type Transport = WorkerTransport<LocalSessionWorker>;

    async fn create_session(
        &self,
    ) -> Result<(SessionId, Self::Transport), Self::Error> {
        Ok(self.inner.create_session().await?)
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        mut message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        if let Err(error) = self.inject_pinned_identity(id, &mut message, true).await {
            self.discard_failed_initialization(id).await;
            return Err(error.into());
        }
        match self.inner.initialize_session(id, message).await {
            Ok(response) => Ok(response),
            Err(error) => {
                self.discard_failed_initialization(id).await;
                Err(RemoteSessionManagerError::Local(error))
            }
        }
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        Ok(self.inner.has_session(id).await?)
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        // Remove/close the transport worker first so no later request can race
        // with attachment revocation. Both layers are independently idempotent.
        let result = self.inner.close_session(id).await;
        self.bindings.write().await.remove(id);
        self.revoke_browser_attachment(id).await;
        Ok(result?)
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        mut message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error>
    {
        self.inject_pinned_identity(id, &mut message, false).await?;
        Ok(self.inner.create_stream(id, message).await?)
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        mut message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        match &message {
            ClientJsonRpcMessage::Response(_) | ClientJsonRpcMessage::Error(_) => {
                if !self.bindings.read().await.contains_key(id) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "Remote MCP session has no pinned identity",
                    )
                    .into());
                }
            }
            _ => self.inject_pinned_identity(id, &mut message, false).await?,
        }
        Ok(self.inner.accept_message(id, message).await?)
    }

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error>
    {
        if !self.bindings.read().await.contains_key(id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Remote MCP session has no pinned identity",
            )
            .into());
        }
        Ok(self.inner.create_standalone_stream(id).await?)
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error>
    {
        if !self.bindings.read().await.contains_key(id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Remote MCP session has no pinned identity",
            )
            .into());
        }
        Ok(self.inner.resume(id, last_event_id).await?)
    }

    async fn restore_session(
        &self,
        _id: SessionId,
    ) -> Result<RestoreOutcome<Self::Transport>, Self::Error> {
        // Do not delegate rmcp's restore implementation: LocalSessionManager
        // accepts the caller-supplied id when restoring, which would weaken
        // the invariant that session ids are server-generated and validated by
        // `has_session`.  This front door has no persisted, authenticated
        // companion/scope binding to restore, so fail closed instead.
        Ok(RestoreOutcome::NotSupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "browser-use")]
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(feature = "browser-use")]
    use futures::FutureExt;
    #[cfg(feature = "browser-use")]
    use nomifun_browser_platform::{
        BrowserErrorCode, BrowserPlatformError,
    };

    fn companion() -> CompanionId {
        CompanionId::new()
    }

    #[cfg(feature = "browser-use")]
    fn session_id(value: &str) -> SessionId {
        Arc::<str>::from(value)
    }

    #[cfg(feature = "browser-use")]
    fn binding() -> RemoteMcpSessionBinding {
        RemoteMcpSessionBinding {
            companion_id: companion(),
            scope: Some(canonical_scope(["browser"])),
        }
    }

    #[cfg(feature = "browser-use")]
    fn cleanup_error() -> BrowserPlatformError {
        BrowserPlatformError::new(
            BrowserErrorCode::BrowserUnavailable,
            "Synthetic Remote browser cleanup failure.",
            true,
            "Retry the authoritative cleanup.",
        )
    }

    #[cfg(feature = "browser-use")]
    #[derive(Default)]
    struct CleanupRegistryProbe {
        failures_remaining: AtomicUsize,
        retry_calls: AtomicUsize,
        revocations: tokio::sync::Mutex<Vec<String>>,
        notify: tokio::sync::Notify,
    }

    #[cfg(feature = "browser-use")]
    impl CleanupRegistryProbe {
        fn fail_next(count: usize) -> Arc<Self> {
            Arc::new(Self {
                failures_remaining: AtomicUsize::new(count),
                ..Default::default()
            })
        }

        async fn revocations(&self) -> Vec<String> {
            self.revocations.lock().await.clone()
        }

        async fn wait_for_revocations(&self, expected: usize) {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if self.revocations.lock().await.len() >= expected {
                        break;
                    }
                    self.notify.notified().await;
                }
            })
            .await
            .expect("Remote browser cleanup did not reach the registry");
        }
    }

    #[cfg(feature = "browser-use")]
    impl RemoteBrowserCleanupRegistry for CleanupRegistryProbe {
        fn revoke_trusted_identity<'a>(
            &'a self,
            runtime_instance_id: &'a str,
        ) -> futures::future::BoxFuture<
            'a,
            Result<(), BrowserPlatformError>,
        > {
            async move {
                self.revocations
                    .lock()
                    .await
                    .push(runtime_instance_id.to_owned());
                self.notify.notify_waiters();
                if self
                    .failures_remaining
                    .fetch_update(
                        Ordering::AcqRel,
                        Ordering::Acquire,
                        |remaining| {
                            (remaining > 0).then(|| remaining - 1)
                        },
                    )
                    .is_ok()
                {
                    Err(cleanup_error())
                } else {
                    Ok(())
                }
            }
            .boxed()
        }

        fn retry_pending_browser_cleanups(
            &self,
        ) -> futures::future::BoxFuture<'_, ()> {
            async move {
                self.retry_calls.fetch_add(1, Ordering::AcqRel);
                self.notify.notify_waiters();
            }
            .boxed()
        }
    }

    #[cfg(feature = "browser-use")]
    fn cleanup_authority(
        registry: Arc<CleanupRegistryProbe>,
        bindings: Arc<
            tokio::sync::RwLock<HashMap<SessionId, RemoteMcpSessionBinding>>,
        >,
    ) -> RemoteBrowserCleanupAuthority {
        RemoteBrowserCleanupAuthority::with_registry(registry, bindings)
    }

    #[cfg(feature = "browser-use")]
    fn manager_with_cleanup(
        registry: Arc<CleanupRegistryProbe>,
    ) -> RemoteSessionManager {
        let bindings = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        RemoteSessionManager {
            inner: LocalSessionManager::default(),
            domains: None,
            bindings: Arc::clone(&bindings),
            browser_cleanup: Some(cleanup_authority(registry, bindings)),
        }
    }

    #[cfg(feature = "browser-use")]
    fn manager_with_cleanup_and_keep_alive(
        registry: Arc<CleanupRegistryProbe>,
        keep_alive: Duration,
    ) -> Arc<RemoteSessionManager> {
        let bindings = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let mut inner = LocalSessionManager::default();
        inner.session_config.keep_alive = Some(keep_alive);
        Arc::new(RemoteSessionManager {
            inner,
            domains: None,
            bindings: Arc::clone(&bindings),
            browser_cleanup: Some(cleanup_authority(registry, bindings)),
        })
    }

    #[cfg(feature = "browser-use")]
    fn initialize_request() -> ClientJsonRpcMessage {
        use rmcp::model::{
            ClientCapabilities, ClientRequest, Implementation,
            InitializeRequest, InitializeRequestParams, RequestId,
        };

        ClientJsonRpcMessage::request(
            ClientRequest::InitializeRequest(InitializeRequest::new(
                InitializeRequestParams::new(
                    ClientCapabilities::default(),
                    Implementation::new("nomifun-public-session-test", "1"),
                ),
            )),
            RequestId::Number(1),
        )
    }

    #[cfg(feature = "browser-use")]
    async fn create_and_bind_session(
        manager: &RemoteSessionManager,
    ) -> (SessionId, RemoteBrowserCleanupAuthority) {
        let (id, transport) = manager.create_session().await.unwrap();
        drop(transport);
        manager.bindings.write().await.insert(id.clone(), binding());
        let cleanup = manager
            .browser_cleanup
            .as_ref()
            .expect("test manager has browser cleanup")
            .clone();
        (id, cleanup)
    }

    #[cfg(feature = "browser-use")]
    async fn wait_until(
        description: &str,
        mut predicate: impl AsyncFnMut() -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if predicate().await {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {description}"));
    }

    #[test]
    fn session_binding_pins_companion_and_scope() {
        let first = companion();
        let second = companion();
        let binding = RemoteMcpSessionBinding {
            companion_id: first.clone(),
            scope: Some(canonical_scope(["files", "agent"])),
        };
        assert!(binding_accepts_request(
            &binding,
            &first,
            Some(&canonical_scope(["agent", "files"])),
            true
        ));
        assert!(!binding_accepts_request(
            &binding,
            &first,
            Some(&canonical_scope(["agent", "files", "browser"])),
            true
        ));
        assert!(!binding_accepts_request(
            &binding,
            &second,
            Some(&canonical_scope(["agent", "files"])),
            true
        ));
        // A later request without a query cannot widen the initialize scope;
        // the handler receives the pinned scope from the identity marker.
        assert!(binding_accepts_request(&binding, &first, None, false));
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn close_session_revokes_only_the_exact_remote_session() {
        let registry = CleanupRegistryProbe::fail_next(0);
        let manager = manager_with_cleanup(Arc::clone(&registry));
        let (first, cleanup) = create_and_bind_session(&manager).await;
        let (second, _) = create_and_bind_session(&manager).await;

        manager.close_session(&first).await.unwrap();

        assert_eq!(
            registry.revocations().await,
            vec![first.as_ref().to_owned()]
        );
        assert!(!manager.bindings.read().await.contains_key(&first));
        assert!(manager.bindings.read().await.contains_key(&second));
        assert!(manager.has_session(&second).await.unwrap());
        assert_eq!(cleanup.pending_count().await, 0);

        manager.close_session(&second).await.unwrap();
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn worker_idle_exit_flows_through_close_session_and_revoke() {
        use rmcp::ServerHandler;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService,
        };

        #[derive(Clone, Copy)]
        struct TestHandler;
        impl ServerHandler for TestHandler {}

        let registry = CleanupRegistryProbe::fail_next(0);
        let manager = manager_with_cleanup_and_keep_alive(
            Arc::clone(&registry),
            Duration::from_millis(40),
        );
        let service = StreamableHttpService::new(
            || Ok(TestHandler),
            Arc::clone(&manager),
            StreamableHttpServerConfig::default()
                .disable_allowed_hosts()
                .with_sse_keep_alive(None),
        );
        let app = axum::Router::new().nest_service("/mcp", service);

        let response = tower::ServiceExt::oneshot(
            app,
            axum::http::Request::post("/mcp")
                .header("host", "localhost")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .extension(crate::router::RemoteCompanion(companion()))
                .body(axum::body::Body::from(
                    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
        if response.status() != axum::http::StatusCode::OK {
            let status = response.status();
            let body =
                axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("read failed initialize response");
            panic!(
                "initialize fixture returned {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let id = session_id(
            response.headers()["mcp-session-id"]
                .to_str()
                .expect("session id header"),
        );

        wait_until("idle worker session removal", || async {
            !manager.has_session(&id).await.unwrap()
        })
        .await;
        registry.wait_for_revocations(1).await;
        assert_eq!(
            registry.revocations().await,
            vec![id.as_ref().to_owned()]
        );
        assert!(!manager.bindings.read().await.contains_key(&id));
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn failed_initialization_revokes_and_discards_the_session() {
        let registry = CleanupRegistryProbe::fail_next(0);
        let manager = manager_with_cleanup(Arc::clone(&registry));
        let (id, transport) = manager.create_session().await.unwrap();
        drop(transport);

        let mut message = initialize_request();
        let ClientJsonRpcMessage::Request(request) = &mut message else {
            unreachable!("initialize helper returns a request")
        };
        let (mut parts, _) = axum::http::Request::builder()
            .uri("/mcp")
            .body(())
            .unwrap()
            .into_parts();
        parts
            .extensions
            .insert(crate::router::RemoteCompanion(companion()));
        request.request.extensions_mut().insert(parts);

        let result = manager.initialize_session(&id, message).await;
        assert!(result.is_err(), "the detached worker must fail initialize");
        assert!(!manager.has_session(&id).await.unwrap());
        assert!(!manager.bindings.read().await.contains_key(&id));
        assert_eq!(
            registry.revocations().await,
            vec![id.as_ref().to_owned()]
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn failed_revoke_is_retried_by_the_durable_worker() {
        let registry = CleanupRegistryProbe::fail_next(1);
        let manager = manager_with_cleanup(Arc::clone(&registry));
        let (id, cleanup) = create_and_bind_session(&manager).await;

        manager.close_session(&id).await.unwrap();
        assert_eq!(cleanup.pending_count().await, 1);

        registry.wait_for_revocations(2).await;
        wait_until("durable cleanup pending set to drain", || async {
            cleanup.pending_count().await == 0
        })
        .await;
        assert_eq!(
            registry.revocations().await,
            vec![id.as_ref().to_owned(), id.as_ref().to_owned()]
        );
        assert!(registry.retry_calls.load(Ordering::Acquire) >= 1);
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn cleanup_authority_drop_drains_still_bound_sessions() {
        let registry = CleanupRegistryProbe::fail_next(0);
        let bindings = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        let first = session_id("remote-drop-first");
        let second = session_id("remote-drop-second");
        bindings.write().await.insert(first.clone(), binding());
        bindings.write().await.insert(second.clone(), binding());
        let cleanup =
            cleanup_authority(Arc::clone(&registry), Arc::clone(&bindings));

        drop(cleanup);
        registry.wait_for_revocations(2).await;

        let mut revoked = registry.revocations().await;
        revoked.sort();
        assert_eq!(
            revoked,
            vec![first.as_ref().to_owned(), second.as_ref().to_owned()]
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    async fn stale_session_cleanup_cannot_revoke_a_replacement_session() {
        let registry = CleanupRegistryProbe::fail_next(0);
        let manager = manager_with_cleanup(Arc::clone(&registry));
        let (stale, _) = create_and_bind_session(&manager).await;
        let (replacement, _) = create_and_bind_session(&manager).await;

        manager.close_session(&stale).await.unwrap();

        assert_eq!(
            registry.revocations().await,
            vec![stale.as_ref().to_owned()]
        );
        assert!(manager.bindings.read().await.contains_key(&replacement));
        assert!(manager.has_session(&replacement).await.unwrap());

        manager.close_session(&replacement).await.unwrap();
    }
}
