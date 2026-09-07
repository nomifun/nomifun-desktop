use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    MiniAppBridgeCallId, MiniAppBridgeKvRequest, MiniAppBridgeRequest, MiniAppBridgeSession,
    MiniAppBridgeSessionId, MiniAppBridgeTarget, MiniAppId, MiniAppReleasePointerState,
    MiniAppServiceStorageDescriptor, MiniAppSurfaceSessionId, ResolvedMiniAppServiceSpec,
    StrictJsonValue,
};
use tokio::sync::Mutex;

use crate::{
    MiniAppCallCancellation, MiniAppPlatformError, MiniAppPlatformResult,
    MiniAppServiceHostPort,
};

/// A Host-owned MessageChannel endpoint. The surface cannot choose its owner,
/// release, epoch, or storage handles; those are fixed when this value is
/// created by the Host.
#[derive(Clone, Debug)]
pub struct MiniAppBridgePort {
    session: MiniAppBridgeSession,
    closed: Arc<std::sync::atomic::AtomicBool>,
}

impl MiniAppBridgePort {
    pub fn session(&self) -> &MiniAppBridgeSession {
        &self.session
    }

    pub fn close(&self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[derive(Clone, Debug)]
pub struct MiniAppBridgeBinding {
    pub miniapp_id: MiniAppId,
    pub surface_session_id: MiniAppSurfaceSessionId,
    pub active: MiniAppReleasePointerState,
    pub storage: MiniAppServiceStorageDescriptor,
    pub service_spec: Option<ResolvedMiniAppServiceSpec>,
}

impl MiniAppBridgeBinding {
    pub fn validate(&self) -> MiniAppPlatformResult<()> {
        self.active.validate()?;
        let active_release = self.active.active_release.as_ref().ok_or_else(|| {
            MiniAppPlatformError::ServiceUnavailable("Bridge requires an Active Release".into())
        })?;
        if self.active.miniapp_id != self.miniapp_id
            || self.storage.kv.miniapp_id != self.miniapp_id
            || self
                .storage
                .files_dir
                .as_ref()
                .is_some_and(|descriptor| descriptor.miniapp_id != self.miniapp_id)
            || self
                .storage
                .private_database
                .as_ref()
                .is_some_and(|descriptor| descriptor.miniapp_id != self.miniapp_id)
        {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        if self.service_spec.is_none()
            && (self.storage.files_dir.is_some() || self.storage.private_database.is_some())
        {
            return Err(MiniAppPlatformError::InvalidState(
                "UI-only MiniApp Bridge cannot bind Files or Private Database".into(),
            ));
        }
        if let Some(spec) = &self.service_spec
            && (spec.miniapp_id != self.miniapp_id
                || &spec.release != active_release
                || spec.active_release_epoch != self.active.active_release_epoch
                || spec.storage != self.storage)
        {
            return Err(MiniAppPlatformError::StaleServiceGeneration);
        }
        Ok(())
    }
}

#[async_trait]
pub trait MiniAppBridgeHost: Send + Sync {
    async fn open(
        &self,
        binding: MiniAppBridgeBinding,
    ) -> MiniAppPlatformResult<MiniAppBridgePort>;

    async fn request(
        &self,
        port: &MiniAppBridgePort,
        request: MiniAppBridgeRequest,
        cancellation: MiniAppCallCancellation,
        now_ms: i64,
    ) -> MiniAppPlatformResult<StrictJsonValue>;

    async fn close(&self, port: &MiniAppBridgePort);

    async fn cancel(&self, port: &MiniAppBridgePort, call_id: &MiniAppBridgeCallId);

    async fn invalidate_miniapp(&self, miniapp_id: &MiniAppId);
}

#[async_trait]
pub trait MiniAppHostKvPort: Send + Sync {
    async fn execute(
        &self,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        request: &MiniAppBridgeKvRequest,
    ) -> MiniAppPlatformResult<StrictJsonValue>;
}

struct BridgeEntry {
    port: MiniAppBridgePort,
    active_pointer: MiniAppReleasePointerState,
    storage: MiniAppServiceStorageDescriptor,
    service_spec: Option<ResolvedMiniAppServiceSpec>,
    in_flight: BTreeMap<MiniAppBridgeCallId, MiniAppCallCancellation>,
}

/// In-memory Host-owned MessageChannel coordinator. It models the ownership,
/// Active Release, epoch, and Surface Session fences without a socket.
pub struct InMemoryMiniAppBridgeHost<K> {
    kv: Arc<K>,
    service: Arc<dyn MiniAppServiceHostPort>,
    ports: Mutex<BTreeMap<MiniAppBridgeSessionId, BridgeEntry>>,
}

impl<K> InMemoryMiniAppBridgeHost<K>
where
    K: MiniAppHostKvPort + 'static,
{
    pub fn new(kv: Arc<K>, service: Arc<dyn MiniAppServiceHostPort>) -> Self {
        Self {
            kv,
            service,
            ports: Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn pointer_snapshot(
        &self,
        port: &MiniAppBridgePort,
    ) -> MiniAppPlatformResult<MiniAppReleasePointerState> {
        if port.is_closed() {
            return Err(MiniAppPlatformError::StaleBridgePort);
        }
        self.ports
            .lock()
            .await
            .get(&port.session.bridge_session_id)
            .filter(|entry| entry.port.session == port.session && !entry.port.is_closed())
            .map(|entry| entry.active_pointer.clone())
            .ok_or(MiniAppPlatformError::StaleBridgePort)
    }

    async fn remove(&self, port: &MiniAppBridgePort) -> Option<BridgeEntry> {
        let entry = self.ports.lock().await.remove(&port.session.bridge_session_id);
        if let Some(entry) = &entry {
            for cancellation in entry.in_flight.values() {
                cancellation.cancel();
            }
        }
        port.close();
        entry
    }

    async fn register_call(
        &self,
        port: &MiniAppBridgePort,
        call_id: &MiniAppBridgeCallId,
        cancellation: &MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<ResolvedBridgeEntry> {
        if port.is_closed() {
            return Err(MiniAppPlatformError::StaleBridgePort);
        }
        let mut ports = self.ports.lock().await;
        let entry = ports
            .get_mut(&port.session.bridge_session_id)
            .ok_or(MiniAppPlatformError::StaleBridgePort)?;
        if entry.port.session != port.session || entry.port.is_closed() {
            return Err(MiniAppPlatformError::StaleBridgePort);
        }
        if entry.in_flight.contains_key(call_id) {
            return Err(MiniAppPlatformError::DuplicateBridgeCall(
                call_id.as_ref().into(),
            ));
        }
        entry
            .in_flight
            .insert(call_id.clone(), cancellation.clone());
        Ok(ResolvedBridgeEntry {
            session: entry.port.session.clone(),
            active_pointer: entry.active_pointer.clone(),
            storage: entry.storage.clone(),
            service_spec: entry.service_spec.clone(),
        })
    }

    async fn finish_call(
        &self,
        port: &MiniAppBridgePort,
        call_id: &MiniAppBridgeCallId,
    ) -> bool {
        if port.is_closed() {
            return false;
        }
        let mut ports = self.ports.lock().await;
        ports
            .get_mut(&port.session.bridge_session_id)
            .is_some_and(|entry| {
                entry.port.session == port.session
                    && !entry.port.is_closed()
                    && entry.in_flight.remove(call_id).is_some()
            })
    }
}

#[derive(Clone)]
struct ResolvedBridgeEntry {
    session: MiniAppBridgeSession,
    active_pointer: MiniAppReleasePointerState,
    storage: MiniAppServiceStorageDescriptor,
    service_spec: Option<ResolvedMiniAppServiceSpec>,
}

#[async_trait]
impl<K> MiniAppBridgeHost for InMemoryMiniAppBridgeHost<K>
where
    K: MiniAppHostKvPort + 'static,
{
    async fn open(
        &self,
        binding: MiniAppBridgeBinding,
    ) -> MiniAppPlatformResult<MiniAppBridgePort> {
        binding.validate()?;
        let active_release = binding
            .active
            .active_release
            .clone()
            .expect("validated Active Release");
        let session = MiniAppBridgeSession {
            bridge_contract_version:
                nomifun_agent_contracts::MINIAPP_BRIDGE_CONTRACT_VERSION.into(),
            bridge_session_id: MiniAppBridgeSessionId::from(uuid::Uuid::now_v7().to_string()),
            surface_session_id: binding.surface_session_id.clone(),
            miniapp_id: binding.miniapp_id.clone(),
            active_release,
            active_release_epoch: binding.active.active_release_epoch,
            transport: nomifun_agent_contracts::MiniAppBridgeTransport::MessageChannelV1,
            service_run_key: binding
                .service_spec
                .as_ref()
                .map(|spec| spec.service_run_key.clone()),
        };
        session.validate()?;
        let port = MiniAppBridgePort {
            session: session.clone(),
            closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let mut ports = self.ports.lock().await;
        ports.retain(|_, entry| {
            let stale = entry.port.session.surface_session_id == binding.surface_session_id
                || (entry.port.session.miniapp_id == binding.miniapp_id
                    && (entry.port.session.active_release != session.active_release
                        || entry.port.session.active_release_epoch
                            != session.active_release_epoch));
            if stale {
                for cancellation in entry.in_flight.values() {
                    cancellation.cancel();
                }
                entry.port.close();
            }
            !stale
        });
        ports.insert(
            session.bridge_session_id.clone(),
            BridgeEntry {
                port: port.clone(),
                active_pointer: binding.active,
                storage: binding.storage,
                service_spec: binding.service_spec,
                in_flight: BTreeMap::new(),
            },
        );
        Ok(port)
    }

    async fn request(
        &self,
        port: &MiniAppBridgePort,
        request: MiniAppBridgeRequest,
        cancellation: MiniAppCallCancellation,
        now_ms: i64,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        let entry = self
            .register_call(port, &request.call_id, &cancellation)
            .await?;
        if let Err(error) = request.validate_for(&entry.session, &entry.active_pointer) {
            self.finish_call(port, &request.call_id).await;
            return Err(error.into());
        }
        let call_id = request.call_id.clone();
        let result = match request.target {
            MiniAppBridgeTarget::HostKv { request } => {
                self.kv
                    .execute(&entry.session.miniapp_id, &entry.storage, &request)
                    .await
            }
            MiniAppBridgeTarget::Service { method, payload } => {
                match entry.service_spec.as_ref() {
                    Some(spec) => {
                        self.service
                            .invoke(
                                spec,
                                request.call_id,
                                method,
                                payload,
                                cancellation.clone(),
                                now_ms,
                            )
                            .await
                    }
                    None => Err(MiniAppPlatformError::ServiceUnavailable(
                        "MiniApp has no Active Service".into(),
                    )),
                }
            }
        };
        let current = self.finish_call(port, &call_id).await;
        if !current {
            return Err(MiniAppPlatformError::StaleBridgePort);
        }
        if cancellation.is_canceled() {
            return Err(MiniAppPlatformError::Canceled);
        }
        result
    }

    async fn close(&self, port: &MiniAppBridgePort) {
        self.remove(port).await;
    }

    async fn cancel(&self, port: &MiniAppBridgePort, call_id: &MiniAppBridgeCallId) {
        let owner = {
            let ports = self.ports.lock().await;
            ports
                .get(&port.session.bridge_session_id)
                .filter(|entry| entry.port.session == port.session && !entry.port.is_closed())
                .map(|entry| {
                    if let Some(cancellation) = entry.in_flight.get(call_id) {
                        cancellation.cancel();
                    }
                    entry.port.session.miniapp_id.clone()
                })
        };
        if let Some(owner) = owner {
            self.service.cancel(&owner, call_id).await;
        }
    }

    async fn invalidate_miniapp(&self, miniapp_id: &MiniAppId) {
        let mut ports = self.ports.lock().await;
        ports.retain(|_, entry| {
            if &entry.port.session.miniapp_id != miniapp_id {
                return true;
            }
            for cancellation in entry.in_flight.values() {
                cancellation.cancel();
            }
            entry.port.close();
            false
        });
    }
}
