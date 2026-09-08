use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    DigestHex, MiniAppBridgeCallId, MiniAppId, MiniAppReleaseRef, MiniAppServiceLifecycle,
    ResolvedMiniAppServiceSpec, StrictJsonValue,
};
use tokio::sync::Mutex;

use crate::{MiniAppPlatformError, MiniAppPlatformResult};

pub const DEFAULT_MAX_ACTIVE_SERVICE_HOSTS: usize = 4;
pub const MINIAPP_CONTINUOUS_CRASH_FAILURE_THRESHOLD: u32 = 3;
pub const MINIAPP_CONTINUOUS_CRASH_BACKOFF_MS: [i64; 2] = [1_000, 5_000];

#[derive(Clone, Debug, Default)]
pub struct MiniAppCallCancellation {
    canceled: Arc<AtomicBool>,
}

impl MiniAppCallCancellation {
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
    }

    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppServiceGenerationFence {
    pub miniapp_id: MiniAppId,
    pub release: MiniAppReleaseRef,
    pub active_release_epoch: u64,
    pub service_run_key: DigestHex,
    pub host_generation: u64,
}

impl MiniAppServiceGenerationFence {
    fn matches_spec(&self, spec: &ResolvedMiniAppServiceSpec) -> bool {
        self.miniapp_id == spec.miniapp_id
            && self.release == spec.release
            && self.active_release_epoch == spec.active_release_epoch
            && self.service_run_key == spec.service_run_key
    }
}

#[derive(Clone, Debug)]
pub struct MiniAppServiceLaunch {
    pub spec: ResolvedMiniAppServiceSpec,
    pub host_generation: u64,
}

#[derive(Clone, Debug)]
pub struct MiniAppServiceInvocation {
    pub fence: MiniAppServiceGenerationFence,
    pub call_id: MiniAppBridgeCallId,
    pub method: String,
    pub payload: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MiniAppServiceProcessError {
    Rejected(String),
    Crashed(String),
}

#[async_trait]
pub trait MiniAppServiceProcess: Send + Sync {
    async fn invoke(
        &self,
        invocation: MiniAppServiceInvocation,
        cancellation: MiniAppCallCancellation,
    ) -> Result<StrictJsonValue, MiniAppServiceProcessError>;

    async fn stop(&self);
}

#[async_trait]
pub trait MiniAppServiceProcessFactory: Send + Sync {
    async fn start(
        &self,
        launch: MiniAppServiceLaunch,
    ) -> MiniAppPlatformResult<Arc<dyn MiniAppServiceProcess>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiniAppServiceHostState {
    Stopped,
    Starting {
        host_generation: u64,
    },
    Running {
        fence: MiniAppServiceGenerationFence,
    },
    Backoff {
        host_generation: u64,
        consecutive_failures: u32,
        retry_at_ms: i64,
        error: String,
    },
    Error {
        host_generation: u64,
        consecutive_failures: u32,
        error: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppServiceCapacitySnapshot {
    pub max_active_service_hosts: usize,
    pub active_miniapps: Vec<MiniAppId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppContinuousReconcileResult {
    pub restarted: Vec<MiniAppId>,
    pub blocked: Vec<(MiniAppId, String)>,
}

#[async_trait]
pub trait MiniAppServiceHostPort: Send + Sync {
    async fn bind_active(
        &self,
        spec: ResolvedMiniAppServiceSpec,
        enabled: bool,
    ) -> MiniAppPlatformResult<()>;

    async fn invoke(
        &self,
        spec: &ResolvedMiniAppServiceSpec,
        call_id: MiniAppBridgeCallId,
        method: String,
        payload: StrictJsonValue,
        cancellation: MiniAppCallCancellation,
        now_ms: i64,
    ) -> MiniAppPlatformResult<StrictJsonValue>;

    async fn cancel(&self, miniapp_id: &MiniAppId, call_id: &MiniAppBridgeCallId);

    async fn stop(&self, miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()>;

    async fn retry(&self, miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()>;

    async fn reap_idle(
        &self,
        now_ms: i64,
        idle_window_ms: i64,
    ) -> MiniAppPlatformResult<Vec<MiniAppId>>;

    async fn report_crash(
        &self,
        fence: &MiniAppServiceGenerationFence,
        reason: String,
        now_ms: i64,
    ) -> MiniAppPlatformResult<bool>;

    async fn reconcile_continuous(
        &self,
        now_ms: i64,
    ) -> MiniAppPlatformResult<MiniAppContinuousReconcileResult>;

    async fn state(&self, miniapp_id: &MiniAppId) -> Option<MiniAppServiceHostState>;
}

struct ServiceSlot {
    spec: ResolvedMiniAppServiceSpec,
    enabled: bool,
    next_generation: u64,
    process: Option<Arc<dyn MiniAppServiceProcess>>,
    state: MiniAppServiceHostState,
    in_flight: BTreeMap<MiniAppBridgeCallId, MiniAppCallCancellation>,
    last_activity_ms: i64,
    consecutive_failures: u32,
}

impl ServiceSlot {
    fn new(spec: ResolvedMiniAppServiceSpec, enabled: bool) -> Self {
        Self {
            spec,
            enabled,
            next_generation: 1,
            process: None,
            state: MiniAppServiceHostState::Stopped,
            in_flight: BTreeMap::new(),
            last_activity_ms: 0,
            consecutive_failures: 0,
        }
    }

    fn cancel_all(&mut self) {
        for cancellation in self.in_flight.values() {
            cancellation.cancel();
        }
        self.in_flight.clear();
    }

    fn active_generation(&self) -> Option<u64> {
        match &self.state {
            MiniAppServiceHostState::Starting { host_generation }
            | MiniAppServiceHostState::Error {
                host_generation, ..
            }
            | MiniAppServiceHostState::Backoff {
                host_generation, ..
            } => Some(*host_generation),
            MiniAppServiceHostState::Running { fence } => Some(fence.host_generation),
            MiniAppServiceHostState::Stopped => None,
        }
    }
}

struct ServiceCapacityState {
    max_active_service_hosts: usize,
    active: BTreeMap<MiniAppId, u64>,
}

pub struct InMemoryMiniAppServiceHost {
    factory: Arc<dyn MiniAppServiceProcessFactory>,
    slots: Mutex<BTreeMap<MiniAppId, Arc<Mutex<ServiceSlot>>>>,
    capacity: Mutex<ServiceCapacityState>,
}

impl InMemoryMiniAppServiceHost {
    pub fn new(factory: Arc<dyn MiniAppServiceProcessFactory>) -> Self {
        Self::with_capacity(factory, DEFAULT_MAX_ACTIVE_SERVICE_HOSTS)
            .expect("default MiniApp Service Host capacity is positive")
    }

    pub fn with_capacity(
        factory: Arc<dyn MiniAppServiceProcessFactory>,
        max_active_service_hosts: usize,
    ) -> MiniAppPlatformResult<Self> {
        if max_active_service_hosts == 0 {
            return Err(MiniAppPlatformError::InvalidState(
                "max_active_service_hosts must be a positive integer".into(),
            ));
        }
        Ok(Self {
            factory,
            slots: Mutex::new(BTreeMap::new()),
            capacity: Mutex::new(ServiceCapacityState {
                max_active_service_hosts,
                active: BTreeMap::new(),
            }),
        })
    }

    pub async fn update_capacity(
        &self,
        max_active_service_hosts: usize,
    ) -> MiniAppPlatformResult<MiniAppServiceCapacitySnapshot> {
        if max_active_service_hosts == 0 {
            return Err(MiniAppPlatformError::InvalidState(
                "max_active_service_hosts must be a positive integer".into(),
            ));
        }
        let mut capacity = self.capacity.lock().await;
        capacity.max_active_service_hosts = max_active_service_hosts;
        Ok(capacity_snapshot(&capacity))
    }

    pub async fn capacity_snapshot(&self) -> MiniAppServiceCapacitySnapshot {
        let capacity = self.capacity.lock().await;
        capacity_snapshot(&capacity)
    }

    async fn slot(&self, miniapp_id: &MiniAppId) -> Option<Arc<Mutex<ServiceSlot>>> {
        self.slots.lock().await.get(miniapp_id).cloned()
    }

    async fn reserve_capacity(
        &self,
        miniapp_id: &MiniAppId,
        host_generation: u64,
    ) -> MiniAppPlatformResult<()> {
        let mut capacity = self.capacity.lock().await;
        if capacity.active.contains_key(miniapp_id) {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp already owns an active Service Host capacity slot".into(),
            ));
        }
        if capacity.active.len() >= capacity.max_active_service_hosts {
            return Err(MiniAppPlatformError::ServiceCapacityExhausted {
                max_active: capacity.max_active_service_hosts,
                active_miniapps: capacity
                    .active
                    .keys()
                    .map(|id| id.as_ref().to_owned())
                    .collect(),
            });
        }
        capacity
            .active
            .insert(miniapp_id.clone(), host_generation);
        Ok(())
    }

    async fn release_capacity(&self, miniapp_id: &MiniAppId, host_generation: u64) {
        let mut capacity = self.capacity.lock().await;
        if capacity.active.get(miniapp_id) == Some(&host_generation) {
            capacity.active.remove(miniapp_id);
        }
    }

    async fn ensure_started(
        &self,
        slot: &mut ServiceSlot,
        now_ms: i64,
    ) -> MiniAppPlatformResult<(
        Arc<dyn MiniAppServiceProcess>,
        MiniAppServiceGenerationFence,
    )> {
        if !slot.enabled {
            return Err(MiniAppPlatformError::ServiceUnavailable(
                "MiniApp is not enabled".into(),
            ));
        }
        if let (Some(process), MiniAppServiceHostState::Running { fence }) =
            (&slot.process, &slot.state)
        {
            return Ok((process.clone(), fence.clone()));
        }

        let generation = slot.next_generation;
        let next_generation = slot
            .next_generation
            .checked_add(1)
            .ok_or_else(|| MiniAppPlatformError::Runtime("Service generation overflow".into()))?;
        self.reserve_capacity(&slot.spec.miniapp_id, generation)
            .await?;
        slot.next_generation = next_generation;
        slot.state = MiniAppServiceHostState::Starting {
            host_generation: generation,
        };
        let launch = MiniAppServiceLaunch {
            spec: slot.spec.clone(),
            host_generation: generation,
        };
        match self.factory.start(launch).await {
            Ok(process) => {
                let fence = MiniAppServiceGenerationFence {
                    miniapp_id: slot.spec.miniapp_id.clone(),
                    release: slot.spec.release.clone(),
                    active_release_epoch: slot.spec.active_release_epoch,
                    service_run_key: slot.spec.service_run_key.clone(),
                    host_generation: generation,
                };
                slot.process = Some(process.clone());
                slot.state = MiniAppServiceHostState::Running {
                    fence: fence.clone(),
                };
                slot.last_activity_ms = now_ms;
                Ok((process, fence))
            }
            Err(error) => {
                self.release_capacity(&slot.spec.miniapp_id, generation)
                    .await;
                slot.process = None;
                slot.consecutive_failures = slot.consecutive_failures.saturating_add(1);
                let error_text = error.to_string();
                if slot.spec.lifecycle == MiniAppServiceLifecycle::Continuous
                    && slot.consecutive_failures < MINIAPP_CONTINUOUS_CRASH_FAILURE_THRESHOLD
                {
                    let backoff_index = slot.consecutive_failures.saturating_sub(1) as usize;
                    let backoff_ms = MINIAPP_CONTINUOUS_CRASH_BACKOFF_MS[backoff_index
                        .min(MINIAPP_CONTINUOUS_CRASH_BACKOFF_MS.len().saturating_sub(1))];
                    slot.state = MiniAppServiceHostState::Backoff {
                        host_generation: generation,
                        consecutive_failures: slot.consecutive_failures,
                        retry_at_ms: now_ms.saturating_add(backoff_ms),
                        error: error_text.clone(),
                    };
                } else {
                    slot.state = MiniAppServiceHostState::Error {
                        host_generation: generation,
                        consecutive_failures: slot.consecutive_failures,
                        error: error_text,
                    };
                }
                Err(error)
            }
        }
    }

    async fn stop_slot(&self, slot: &mut ServiceSlot) {
        let generation = slot.active_generation();
        slot.cancel_all();
        if let Some(process) = slot.process.take() {
            process.stop().await;
        }
        if let Some(generation) = generation {
            self.release_capacity(&slot.spec.miniapp_id, generation)
                .await;
        }
        slot.state = MiniAppServiceHostState::Stopped;
    }

    async fn transition_crash(&self, slot: &mut ServiceSlot, reason: String, now_ms: i64) {
        let generation = slot.active_generation().unwrap_or(0);
        slot.cancel_all();
        if let Some(process) = slot.process.take() {
            process.stop().await;
        }
        if generation > 0 {
            self.release_capacity(&slot.spec.miniapp_id, generation)
                .await;
        }
        slot.consecutive_failures = slot.consecutive_failures.saturating_add(1);
        if slot.spec.lifecycle != MiniAppServiceLifecycle::Continuous
            || slot.consecutive_failures >= MINIAPP_CONTINUOUS_CRASH_FAILURE_THRESHOLD
        {
            slot.state = MiniAppServiceHostState::Error {
                host_generation: generation,
                consecutive_failures: slot.consecutive_failures,
                error: reason,
            };
            return;
        }
        let backoff_index = slot.consecutive_failures.saturating_sub(1) as usize;
        let backoff_ms = MINIAPP_CONTINUOUS_CRASH_BACKOFF_MS[backoff_index
            .min(MINIAPP_CONTINUOUS_CRASH_BACKOFF_MS.len().saturating_sub(1))];
        slot.state = MiniAppServiceHostState::Backoff {
            host_generation: generation,
            consecutive_failures: slot.consecutive_failures,
            retry_at_ms: now_ms.saturating_add(backoff_ms),
            error: reason,
        };
    }
}

#[async_trait]
impl MiniAppServiceHostPort for InMemoryMiniAppServiceHost {
    async fn bind_active(
        &self,
        spec: ResolvedMiniAppServiceSpec,
        enabled: bool,
    ) -> MiniAppPlatformResult<()> {
        if spec.active_release_epoch == 0 {
            return Err(MiniAppPlatformError::InvalidState(
                "Service Host requires a positive Active Release epoch".into(),
            ));
        }
        let miniapp_id = spec.miniapp_id.clone();
        let slot = {
            let mut slots = self.slots.lock().await;
            slots
                .entry(miniapp_id)
                .or_insert_with(|| Arc::new(Mutex::new(ServiceSlot::new(spec.clone(), enabled))))
                .clone()
        };
        let mut slot = slot.lock().await;
        let changed = slot.spec != spec;
        if changed || !enabled {
            self.stop_slot(&mut slot).await;
        }
        if changed {
            slot.consecutive_failures = 0;
        }
        slot.spec = spec;
        slot.enabled = enabled;
        if enabled
            && slot.spec.lifecycle == MiniAppServiceLifecycle::Continuous
            && matches!(slot.state, MiniAppServiceHostState::Stopped)
        {
            self.ensure_started(&mut slot, nomifun_common::now_ms()).await?;
        }
        Ok(())
    }

    async fn invoke(
        &self,
        spec: &ResolvedMiniAppServiceSpec,
        call_id: MiniAppBridgeCallId,
        method: String,
        payload: StrictJsonValue,
        cancellation: MiniAppCallCancellation,
        now_ms: i64,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        let slot = self.slot(&spec.miniapp_id).await.ok_or_else(|| {
            MiniAppPlatformError::ServiceUnavailable("Active Service is not bound".into())
        })?;
        let (process, fence) = {
            let mut slot = slot.lock().await;
            if slot.spec != *spec {
                return Err(MiniAppPlatformError::StaleServiceGeneration);
            }
            if matches!(
                slot.state,
                MiniAppServiceHostState::Backoff { .. } | MiniAppServiceHostState::Error { .. }
            ) {
                return Err(MiniAppPlatformError::ServiceUnavailable(
                    "Service is waiting for Retry or continuous reconciliation".into(),
                ));
            }
            if slot.in_flight.contains_key(&call_id) {
                return Err(MiniAppPlatformError::DuplicateBridgeCall(
                    call_id.as_ref().into(),
                ));
            }
            let (process, fence) = self.ensure_started(&mut slot, now_ms).await?;
            slot.in_flight.insert(call_id.clone(), cancellation.clone());
            slot.last_activity_ms = now_ms;
            (process, fence)
        };

        if cancellation.is_canceled() {
            let mut slot = slot.lock().await;
            slot.in_flight.remove(&call_id);
            return Err(MiniAppPlatformError::Canceled);
        }

        let result = process
            .invoke(
                MiniAppServiceInvocation {
                    fence: fence.clone(),
                    call_id: call_id.clone(),
                    method,
                    payload,
                },
                cancellation.clone(),
            )
            .await;

        let mut slot = slot.lock().await;
        let call_is_current = slot
            .in_flight
            .remove(&call_id)
            .is_some_and(|registered| Arc::ptr_eq(&registered.canceled, &cancellation.canceled));
        let generation_is_current = matches!(
            &slot.state,
            MiniAppServiceHostState::Running { fence: current } if current == &fence
        ) && fence.matches_spec(&slot.spec);
        slot.last_activity_ms = now_ms;

        if !call_is_current || !generation_is_current {
            return Err(MiniAppPlatformError::StaleServiceGeneration);
        }
        if cancellation.is_canceled() {
            return Err(MiniAppPlatformError::Canceled);
        }

        match result {
            Ok(value) => Ok(value),
            Err(MiniAppServiceProcessError::Rejected(message)) => {
                Err(MiniAppPlatformError::Runtime(message))
            }
            Err(MiniAppServiceProcessError::Crashed(message)) => {
                self.transition_crash(&mut slot, message.clone(), now_ms)
                    .await;
                Err(MiniAppPlatformError::ServiceCrashed(message))
            }
        }
    }

    async fn cancel(&self, miniapp_id: &MiniAppId, call_id: &MiniAppBridgeCallId) {
        if let Some(slot) = self.slot(miniapp_id).await
            && let Some(cancellation) = slot.lock().await.in_flight.get(call_id)
        {
            cancellation.cancel();
        }
    }

    async fn stop(&self, miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()> {
        if let Some(slot) = self.slot(miniapp_id).await {
            let mut slot = slot.lock().await;
            self.stop_slot(&mut slot).await;
        }
        Ok(())
    }

    async fn retry(&self, miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()> {
        let slot = self.slot(miniapp_id).await.ok_or_else(|| {
            MiniAppPlatformError::ServiceUnavailable("Active Service is not bound".into())
        })?;
        let mut slot = slot.lock().await;
        self.stop_slot(&mut slot).await;
        if !slot.enabled {
            return Err(MiniAppPlatformError::ServiceUnavailable(
                "MiniApp is not enabled".into(),
            ));
        }
        slot.consecutive_failures = 0;
        self.ensure_started(&mut slot, nomifun_common::now_ms()).await?;
        Ok(())
    }

    async fn reap_idle(
        &self,
        now_ms: i64,
        idle_window_ms: i64,
    ) -> MiniAppPlatformResult<Vec<MiniAppId>> {
        if idle_window_ms <= 0 {
            return Err(MiniAppPlatformError::InvalidState(
                "Service idle window must be positive".into(),
            ));
        }
        let slots = self.slots.lock().await.values().cloned().collect::<Vec<_>>();
        let mut reaped = Vec::new();
        for slot in slots {
            let mut slot = slot.lock().await;
            if slot.spec.lifecycle == MiniAppServiceLifecycle::OnDemand
                && slot.in_flight.is_empty()
                && matches!(slot.state, MiniAppServiceHostState::Running { .. })
                && now_ms.saturating_sub(slot.last_activity_ms) >= idle_window_ms
            {
                reaped.push(slot.spec.miniapp_id.clone());
                self.stop_slot(&mut slot).await;
            }
        }
        Ok(reaped)
    }

    async fn report_crash(
        &self,
        fence: &MiniAppServiceGenerationFence,
        reason: String,
        now_ms: i64,
    ) -> MiniAppPlatformResult<bool> {
        let Some(slot) = self.slot(&fence.miniapp_id).await else {
            return Ok(false);
        };
        let mut slot = slot.lock().await;
        let current = matches!(
            &slot.state,
            MiniAppServiceHostState::Running { fence: current } if current == fence
        );
        if !current {
            return Ok(false);
        }
        self.transition_crash(&mut slot, reason, now_ms).await;
        Ok(true)
    }

    async fn reconcile_continuous(
        &self,
        now_ms: i64,
    ) -> MiniAppPlatformResult<MiniAppContinuousReconcileResult> {
        let slots = self.slots.lock().await.values().cloned().collect::<Vec<_>>();
        let mut result = MiniAppContinuousReconcileResult {
            restarted: Vec::new(),
            blocked: Vec::new(),
        };
        for slot in slots {
            let mut slot = slot.lock().await;
            let due = matches!(
                slot.state,
                MiniAppServiceHostState::Backoff { retry_at_ms, .. } if retry_at_ms <= now_ms
            );
            if !slot.enabled
                || slot.spec.lifecycle != MiniAppServiceLifecycle::Continuous
                || !due
            {
                continue;
            }
            match self.ensure_started(&mut slot, now_ms).await {
                Ok(_) => result.restarted.push(slot.spec.miniapp_id.clone()),
                Err(error @ MiniAppPlatformError::ServiceCapacityExhausted { .. }) => {
                    result
                        .blocked
                        .push((slot.spec.miniapp_id.clone(), error.to_string()));
                }
                Err(error) => {
                    if matches!(slot.state, MiniAppServiceHostState::Stopped) {
                        self.transition_crash(&mut slot, error.to_string(), now_ms)
                            .await;
                    }
                    result
                        .blocked
                        .push((slot.spec.miniapp_id.clone(), error.to_string()));
                }
            }
        }
        Ok(result)
    }

    async fn state(&self, miniapp_id: &MiniAppId) -> Option<MiniAppServiceHostState> {
        let slot = self.slot(miniapp_id).await?;
        Some(slot.lock().await.state.clone())
    }
}

fn capacity_snapshot(capacity: &ServiceCapacityState) -> MiniAppServiceCapacitySnapshot {
    MiniAppServiceCapacitySnapshot {
        max_active_service_hosts: capacity.max_active_service_hosts,
        active_miniapps: capacity.active.keys().cloned().collect(),
    }
}
