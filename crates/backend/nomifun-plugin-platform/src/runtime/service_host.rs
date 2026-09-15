use std::{
    collections::BTreeMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    DigestHex, PluginBridgeCallId, PluginProductId, PluginReleaseRef, PluginServiceLifecycle,
    ResolvedPluginServiceSpec, StrictJsonValue,
};
use tokio::sync::Mutex;

use crate::runtime::{PluginRuntimePlatformError, PluginRuntimePlatformResult};

pub const DEFAULT_MAX_ACTIVE_SERVICE_HOSTS: usize = 4;
pub const PLUGIN_CONTINUOUS_CRASH_FAILURE_THRESHOLD: u32 = 3;
pub const PLUGIN_CONTINUOUS_CRASH_BACKOFF_MS: [i64; 2] = [1_000, 5_000];

#[derive(Clone, Debug, Default)]
pub struct PluginRuntimeCallCancellation {
    canceled: Arc<AtomicBool>,
}

impl PluginRuntimeCallCancellation {
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
    }

    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeServiceGenerationFence {
    pub plugin_product_id: PluginProductId,
    pub release: PluginReleaseRef,
    pub active_release_epoch: u64,
    pub service_run_key: DigestHex,
    pub host_generation: u64,
}

impl PluginRuntimeServiceGenerationFence {
    fn matches_spec(&self, spec: &ResolvedPluginServiceSpec) -> bool {
        self.plugin_product_id == spec.plugin_product_id
            && self.release == spec.release
            && self.active_release_epoch == spec.active_release_epoch
            && self.service_run_key == spec.service_run_key
    }
}

#[derive(Clone, Debug)]
pub struct PluginRuntimeServiceLaunch {
    pub spec: ResolvedPluginServiceSpec,
    pub host_generation: u64,
}

#[derive(Clone, Debug)]
pub struct PluginRuntimeServiceInvocation {
    pub fence: PluginRuntimeServiceGenerationFence,
    pub call_id: PluginBridgeCallId,
    pub method: String,
    pub payload: StrictJsonValue,
    /// Optional, bounded host-side incremental delivery. The invocation future
    /// still owns completion and cancellation; events alone are not success.
    /// Dropping the receiver cancels this request, not the whole Service.
    pub events: Option<tokio::sync::mpsc::Sender<StrictJsonValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginRuntimeServiceProcessError {
    Rejected(String),
    Crashed(String),
}

#[async_trait]
pub trait PluginRuntimeServiceProcess: Send + Sync {
    async fn invoke(
        &self,
        invocation: PluginRuntimeServiceInvocation,
        cancellation: PluginRuntimeCallCancellation,
    ) -> Result<StrictJsonValue, PluginRuntimeServiceProcessError>;

    async fn stop(&self);

    /// Returns a terminal process result without waiting. Implementations that
    /// cannot observe passive exits may keep the default `None`.
    fn terminal_result(&self) -> Option<Result<(), String>> {
        None
    }
}

#[async_trait]
pub trait PluginRuntimeServiceProcessFactory: Send + Sync {
    async fn start(
        &self,
        launch: PluginRuntimeServiceLaunch,
    ) -> PluginRuntimePlatformResult<Arc<dyn PluginRuntimeServiceProcess>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginRuntimeServiceHostState {
    Stopped,
    Starting {
        host_generation: u64,
    },
    Running {
        fence: PluginRuntimeServiceGenerationFence,
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
pub struct PluginRuntimeServiceCapacitySnapshot {
    pub max_active_service_hosts: usize,
    pub active_plugins: Vec<PluginProductId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeContinuousReconcileResult {
    pub restarted: Vec<PluginProductId>,
    pub blocked: Vec<(PluginProductId, String)>,
}

#[async_trait]
pub trait PluginRuntimeServiceHostPort: Send + Sync {
    async fn bind_active(
        &self,
        spec: ResolvedPluginServiceSpec,
        enabled: bool,
    ) -> PluginRuntimePlatformResult<()>;

    async fn invoke(
        &self,
        spec: &ResolvedPluginServiceSpec,
        call_id: PluginBridgeCallId,
        method: String,
        payload: StrictJsonValue,
        cancellation: PluginRuntimeCallCancellation,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> {
        self.invoke_with_events(spec, call_id, method, payload, cancellation, now_ms, None).await
    }

    /// Same invocation ownership and generation checks as a unary call. The
    /// caller must poll completion while consuming this bounded channel; its
    /// items are provisional until the original invocation returns success.
    async fn invoke_with_events(
        &self,
        spec: &ResolvedPluginServiceSpec,
        call_id: PluginBridgeCallId,
        method: String,
        payload: StrictJsonValue,
        cancellation: PluginRuntimeCallCancellation,
        now_ms: i64,
        events: Option<tokio::sync::mpsc::Sender<StrictJsonValue>>,
    ) -> PluginRuntimePlatformResult<StrictJsonValue>;

    async fn cancel(&self, plugin_product_id: &PluginProductId, call_id: &PluginBridgeCallId);

    async fn stop(&self, plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()>;

    async fn retry(&self, plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()>;

    async fn reap_idle(
        &self,
        now_ms: i64,
        idle_window_ms: i64,
    ) -> PluginRuntimePlatformResult<Vec<PluginProductId>>;

    async fn report_crash(
        &self,
        fence: &PluginRuntimeServiceGenerationFence,
        reason: String,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<bool>;

    async fn reconcile_continuous(
        &self,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<PluginRuntimeContinuousReconcileResult>;

    async fn state(&self, plugin_product_id: &PluginProductId) -> Option<PluginRuntimeServiceHostState>;
}

struct ServiceSlot {
    spec: ResolvedPluginServiceSpec,
    enabled: bool,
    next_generation: u64,
    process: Option<Arc<dyn PluginRuntimeServiceProcess>>,
    state: PluginRuntimeServiceHostState,
    // The waiting invocation owns the registration. Dropping its future must
    // not keep a Service permanently busy or require a detached cleanup task.
    in_flight: BTreeMap<PluginBridgeCallId, Weak<PluginRuntimeCallCancellation>>,
    last_activity_ms: i64,
    consecutive_failures: u32,
}

impl ServiceSlot {
    fn new(spec: ResolvedPluginServiceSpec, enabled: bool) -> Self {
        Self {
            spec,
            enabled,
            next_generation: 1,
            process: None,
            state: PluginRuntimeServiceHostState::Stopped,
            in_flight: BTreeMap::new(),
            last_activity_ms: 0,
            consecutive_failures: 0,
        }
    }

    fn cancel_all(&mut self) {
        for cancellation in self.in_flight.values().filter_map(Weak::upgrade) {
            cancellation.cancel();
        }
        self.in_flight.clear();
    }

    fn prune_finished_calls(&mut self) {
        self.in_flight.retain(|_, call| call.strong_count() != 0);
    }

    fn active_generation(&self) -> Option<u64> {
        match &self.state {
            PluginRuntimeServiceHostState::Starting { host_generation }
            | PluginRuntimeServiceHostState::Error {
                host_generation, ..
            }
            | PluginRuntimeServiceHostState::Backoff {
                host_generation, ..
            } => Some(*host_generation),
            PluginRuntimeServiceHostState::Running { fence } => Some(fence.host_generation),
            PluginRuntimeServiceHostState::Stopped => None,
        }
    }
}

struct ActiveServiceCall {
    cancellation: Arc<PluginRuntimeCallCancellation>,
    completed: bool,
}

impl Drop for ActiveServiceCall {
    fn drop(&mut self) {
        if !self.completed {
            self.cancellation.cancel();
        }
    }
}

struct ServiceCapacityState {
    max_active_service_hosts: usize,
    active: BTreeMap<PluginProductId, u64>,
}

pub struct InMemoryPluginRuntimeServiceHost {
    factory: Arc<dyn PluginRuntimeServiceProcessFactory>,
    slots: Mutex<BTreeMap<PluginProductId, Arc<Mutex<ServiceSlot>>>>,
    capacity: Mutex<ServiceCapacityState>,
}

impl InMemoryPluginRuntimeServiceHost {
    pub fn new(factory: Arc<dyn PluginRuntimeServiceProcessFactory>) -> Self {
        Self::with_capacity(factory, DEFAULT_MAX_ACTIVE_SERVICE_HOSTS)
            .expect("default Plugin Service Host capacity is positive")
    }

    pub fn with_capacity(
        factory: Arc<dyn PluginRuntimeServiceProcessFactory>,
        max_active_service_hosts: usize,
    ) -> PluginRuntimePlatformResult<Self> {
        if max_active_service_hosts == 0 {
            return Err(PluginRuntimePlatformError::InvalidState(
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
    ) -> PluginRuntimePlatformResult<PluginRuntimeServiceCapacitySnapshot> {
        if max_active_service_hosts == 0 {
            return Err(PluginRuntimePlatformError::InvalidState(
                "max_active_service_hosts must be a positive integer".into(),
            ));
        }
        let mut capacity = self.capacity.lock().await;
        capacity.max_active_service_hosts = max_active_service_hosts;
        Ok(capacity_snapshot(&capacity))
    }

    pub async fn capacity_snapshot(&self) -> PluginRuntimeServiceCapacitySnapshot {
        let capacity = self.capacity.lock().await;
        capacity_snapshot(&capacity)
    }

    async fn slot(&self, plugin_product_id: &PluginProductId) -> Option<Arc<Mutex<ServiceSlot>>> {
        self.slots.lock().await.get(plugin_product_id).cloned()
    }

    async fn reserve_capacity(
        &self,
        plugin_product_id: &PluginProductId,
        host_generation: u64,
    ) -> PluginRuntimePlatformResult<()> {
        let mut capacity = self.capacity.lock().await;
        if capacity.active.contains_key(plugin_product_id) {
            return Err(PluginRuntimePlatformError::InvalidState(
                "Plugin already owns an active Service Host capacity slot".into(),
            ));
        }
        if capacity.active.len() >= capacity.max_active_service_hosts {
            return Err(PluginRuntimePlatformError::ServiceCapacityExhausted {
                max_active: capacity.max_active_service_hosts,
                active_plugins: capacity
                    .active
                    .keys()
                    .map(|id| id.as_ref().to_owned())
                    .collect(),
            });
        }
        capacity
            .active
            .insert(plugin_product_id.clone(), host_generation);
        Ok(())
    }

    async fn release_capacity(&self, plugin_product_id: &PluginProductId, host_generation: u64) {
        let mut capacity = self.capacity.lock().await;
        if capacity.active.get(plugin_product_id) == Some(&host_generation) {
            capacity.active.remove(plugin_product_id);
        }
    }

    async fn ensure_started(
        &self,
        slot: &mut ServiceSlot,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<(
        Arc<dyn PluginRuntimeServiceProcess>,
        PluginRuntimeServiceGenerationFence,
    )> {
        if !slot.enabled {
            return Err(PluginRuntimePlatformError::ServiceUnavailable(
                "Plugin is not enabled".into(),
            ));
        }
        if let (Some(process), PluginRuntimeServiceHostState::Running { fence }) =
            (&slot.process, &slot.state)
        {
            return Ok((process.clone(), fence.clone()));
        }

        let generation = slot.next_generation;
        let next_generation = slot
            .next_generation
            .checked_add(1)
            .ok_or_else(|| PluginRuntimePlatformError::Runtime("Service generation overflow".into()))?;
        self.reserve_capacity(&slot.spec.plugin_product_id, generation)
            .await?;
        slot.next_generation = next_generation;
        slot.state = PluginRuntimeServiceHostState::Starting {
            host_generation: generation,
        };
        let launch = PluginRuntimeServiceLaunch {
            spec: slot.spec.clone(),
            host_generation: generation,
        };
        match self.factory.start(launch).await {
            Ok(process) => {
                let fence = PluginRuntimeServiceGenerationFence {
                    plugin_product_id: slot.spec.plugin_product_id.clone(),
                    release: slot.spec.release.clone(),
                    active_release_epoch: slot.spec.active_release_epoch,
                    service_run_key: slot.spec.service_run_key.clone(),
                    host_generation: generation,
                };
                slot.process = Some(process.clone());
                slot.state = PluginRuntimeServiceHostState::Running {
                    fence: fence.clone(),
                };
                slot.last_activity_ms = now_ms;
                Ok((process, fence))
            }
            Err(error) => {
                self.release_capacity(&slot.spec.plugin_product_id, generation)
                    .await;
                slot.process = None;
                slot.consecutive_failures = slot.consecutive_failures.saturating_add(1);
                let error_text = error.to_string();
                if slot.spec.lifecycle == PluginServiceLifecycle::Continuous
                    && slot.consecutive_failures < PLUGIN_CONTINUOUS_CRASH_FAILURE_THRESHOLD
                {
                    let backoff_index = slot.consecutive_failures.saturating_sub(1) as usize;
                    let backoff_ms = PLUGIN_CONTINUOUS_CRASH_BACKOFF_MS[backoff_index
                        .min(PLUGIN_CONTINUOUS_CRASH_BACKOFF_MS.len().saturating_sub(1))];
                    slot.state = PluginRuntimeServiceHostState::Backoff {
                        host_generation: generation,
                        consecutive_failures: slot.consecutive_failures,
                        retry_at_ms: now_ms.saturating_add(backoff_ms),
                        error: error_text.clone(),
                    };
                } else {
                    slot.state = PluginRuntimeServiceHostState::Error {
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
            self.release_capacity(&slot.spec.plugin_product_id, generation)
                .await;
        }
        slot.state = PluginRuntimeServiceHostState::Stopped;
    }

    async fn transition_crash(&self, slot: &mut ServiceSlot, reason: String, now_ms: i64) {
        let generation = slot.active_generation().unwrap_or(0);
        slot.cancel_all();
        if let Some(process) = slot.process.take() {
            process.stop().await;
        }
        if generation > 0 {
            self.release_capacity(&slot.spec.plugin_product_id, generation)
                .await;
        }
        slot.consecutive_failures = slot.consecutive_failures.saturating_add(1);
        if slot.spec.lifecycle != PluginServiceLifecycle::Continuous
            || slot.consecutive_failures >= PLUGIN_CONTINUOUS_CRASH_FAILURE_THRESHOLD
        {
            slot.state = PluginRuntimeServiceHostState::Error {
                host_generation: generation,
                consecutive_failures: slot.consecutive_failures,
                error: reason,
            };
            return;
        }
        let backoff_index = slot.consecutive_failures.saturating_sub(1) as usize;
        let backoff_ms = PLUGIN_CONTINUOUS_CRASH_BACKOFF_MS[backoff_index
            .min(PLUGIN_CONTINUOUS_CRASH_BACKOFF_MS.len().saturating_sub(1))];
        slot.state = PluginRuntimeServiceHostState::Backoff {
            host_generation: generation,
            consecutive_failures: slot.consecutive_failures,
            retry_at_ms: now_ms.saturating_add(backoff_ms),
            error: reason,
        };
    }

    /// Returns the exact specs for enabled Service bindings.
    ///
    /// Runtime candidate validation uses these immutable inputs after the
    /// current processes have been quiesced. Disabled slots are deliberately
    /// omitted because they do not participate in the active Runtime switch.
    pub async fn enabled_service_specs(&self) -> Vec<ResolvedPluginServiceSpec> {
        let slots = self.slots.lock().await.values().cloned().collect::<Vec<_>>();
        let mut specs = Vec::new();
        for slot in slots {
            let slot = slot.lock().await;
            if slot.enabled {
                specs.push(slot.spec.clone());
            }
        }
        specs.sort_by(|left, right| left.plugin_product_id.cmp(&right.plugin_product_id));
        specs
    }

    /// Detect processes that exited without an invocation being in flight.
    /// Runtime maintenance uses this so an externally terminated on-demand
    /// Service cannot remain projected as Running.
    pub async fn observe_process_exits(&self, now_ms: i64) -> Vec<PluginProductId> {
        let slots = self.slots.lock().await.values().cloned().collect::<Vec<_>>();
        let mut exited = Vec::new();
        for slot in slots {
            let mut slot = slot.lock().await;
            let terminal = match (&slot.process, &slot.state) {
                (Some(process), PluginRuntimeServiceHostState::Running { .. }) => {
                    process.terminal_result()
                }
                _ => None,
            };
            let Some(terminal) = terminal else {
                continue;
            };
            let reason = terminal.err().unwrap_or_else(|| {
                "Plugin Service process exited without a Host stop request".into()
            });
            exited.push(slot.spec.plugin_product_id.clone());
            self.transition_crash(&mut slot, reason, now_ms).await;
        }
        exited.sort();
        exited
    }
}

#[async_trait]
impl PluginRuntimeServiceHostPort for InMemoryPluginRuntimeServiceHost {
    async fn bind_active(
        &self,
        spec: ResolvedPluginServiceSpec,
        enabled: bool,
    ) -> PluginRuntimePlatformResult<()> {
        if spec.active_release_epoch == 0 {
            return Err(PluginRuntimePlatformError::InvalidState(
                "Service Host requires a positive Active Release epoch".into(),
            ));
        }
        let plugin_product_id = spec.plugin_product_id.clone();
        let slot = {
            let mut slots = self.slots.lock().await;
            slots
                .entry(plugin_product_id)
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
            && slot.spec.lifecycle == PluginServiceLifecycle::Continuous
            && matches!(slot.state, PluginRuntimeServiceHostState::Stopped)
        {
            self.ensure_started(&mut slot, nomifun_common::now_ms()).await?;
        }
        Ok(())
    }

    async fn invoke_with_events(
        &self,
        spec: &ResolvedPluginServiceSpec,
        call_id: PluginBridgeCallId,
        method: String,
        payload: StrictJsonValue,
        cancellation: PluginRuntimeCallCancellation,
        now_ms: i64,
        events: Option<tokio::sync::mpsc::Sender<StrictJsonValue>>,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> {
        let slot = self.slot(&spec.plugin_product_id).await.ok_or_else(|| {
            PluginRuntimePlatformError::ServiceUnavailable("Active Service is not bound".into())
        })?;
        let (process, fence, mut call) = {
            let mut slot = slot.lock().await;
            if slot.spec != *spec {
                return Err(PluginRuntimePlatformError::StaleServiceGeneration);
            }
            if matches!(
                slot.state,
                PluginRuntimeServiceHostState::Backoff { .. } | PluginRuntimeServiceHostState::Error { .. }
            ) {
                return Err(PluginRuntimePlatformError::ServiceUnavailable(
                    "Service is waiting for Retry or continuous reconciliation".into(),
                ));
            }
            slot.prune_finished_calls();
            if slot.in_flight.contains_key(&call_id) {
                return Err(PluginRuntimePlatformError::DuplicateBridgeCall(
                    call_id.as_ref().into(),
                ));
            }
            let (process, fence) = self.ensure_started(&mut slot, now_ms).await?;
            let call = ActiveServiceCall {
                cancellation: Arc::new(cancellation.clone()),
                completed: false,
            };
            slot.in_flight.insert(call_id.clone(), Arc::downgrade(&call.cancellation));
            slot.last_activity_ms = now_ms;
            (process, fence, call)
        };

        if cancellation.is_canceled() {
            return Err(PluginRuntimePlatformError::Canceled);
        }

        let result = process
            .invoke(
                PluginRuntimeServiceInvocation {
                    fence: fence.clone(),
                    call_id: call_id.clone(),
                    method,
                    payload,
                    events,
                },
                cancellation.clone(),
            )
            .await;

        let mut slot = slot.lock().await;
        let call_is_current = slot.in_flight.get(&call_id)
            .is_some_and(|registered| Weak::ptr_eq(registered, &Arc::downgrade(&call.cancellation)));
        if call_is_current {
            slot.in_flight.remove(&call_id);
        }
        call.completed = true;
        let generation_is_current = matches!(
            &slot.state,
            PluginRuntimeServiceHostState::Running { fence: current } if current == &fence
        ) && fence.matches_spec(&slot.spec);
        slot.last_activity_ms = now_ms;

        if !call_is_current || !generation_is_current {
            return Err(PluginRuntimePlatformError::StaleServiceGeneration);
        }
        if cancellation.is_canceled() {
            return Err(PluginRuntimePlatformError::Canceled);
        }

        match result {
            Ok(value) => Ok(value),
            Err(PluginRuntimeServiceProcessError::Rejected(message)) => {
                Err(PluginRuntimePlatformError::Runtime(message))
            }
            Err(PluginRuntimeServiceProcessError::Crashed(message)) => {
                self.transition_crash(&mut slot, message.clone(), now_ms)
                    .await;
                Err(PluginRuntimePlatformError::ServiceCrashed(message))
            }
        }
    }

    async fn cancel(&self, plugin_product_id: &PluginProductId, call_id: &PluginBridgeCallId) {
        if let Some(slot) = self.slot(plugin_product_id).await
            && let Some(cancellation) = slot.lock().await.in_flight.get(call_id).and_then(Weak::upgrade)
        {
            cancellation.cancel();
        }
    }

    async fn stop(&self, plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()> {
        if let Some(slot) = self.slot(plugin_product_id).await {
            let mut slot = slot.lock().await;
            self.stop_slot(&mut slot).await;
        }
        Ok(())
    }

    async fn retry(&self, plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()> {
        let slot = self.slot(plugin_product_id).await.ok_or_else(|| {
            PluginRuntimePlatformError::ServiceUnavailable("Active Service is not bound".into())
        })?;
        let mut slot = slot.lock().await;
        self.stop_slot(&mut slot).await;
        if !slot.enabled {
            return Err(PluginRuntimePlatformError::ServiceUnavailable(
                "Plugin is not enabled".into(),
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
    ) -> PluginRuntimePlatformResult<Vec<PluginProductId>> {
        if idle_window_ms <= 0 {
            return Err(PluginRuntimePlatformError::InvalidState(
                "Service idle window must be positive".into(),
            ));
        }
        let slots = self.slots.lock().await.values().cloned().collect::<Vec<_>>();
        let mut reaped = Vec::new();
        for slot in slots {
            let mut slot = slot.lock().await;
            slot.prune_finished_calls();
            if slot.spec.lifecycle == PluginServiceLifecycle::OnDemand
                && slot.in_flight.is_empty()
                && matches!(slot.state, PluginRuntimeServiceHostState::Running { .. })
                && now_ms.saturating_sub(slot.last_activity_ms) >= idle_window_ms
            {
                reaped.push(slot.spec.plugin_product_id.clone());
                self.stop_slot(&mut slot).await;
            }
        }
        Ok(reaped)
    }

    async fn report_crash(
        &self,
        fence: &PluginRuntimeServiceGenerationFence,
        reason: String,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<bool> {
        let Some(slot) = self.slot(&fence.plugin_product_id).await else {
            return Ok(false);
        };
        let mut slot = slot.lock().await;
        let current = matches!(
            &slot.state,
            PluginRuntimeServiceHostState::Running { fence: current } if current == fence
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
    ) -> PluginRuntimePlatformResult<PluginRuntimeContinuousReconcileResult> {
        let slots = self.slots.lock().await.values().cloned().collect::<Vec<_>>();
        let mut result = PluginRuntimeContinuousReconcileResult {
            restarted: Vec::new(),
            blocked: Vec::new(),
        };
        for slot in slots {
            let mut slot = slot.lock().await;
            let due = matches!(
                slot.state,
                PluginRuntimeServiceHostState::Backoff { retry_at_ms, .. } if retry_at_ms <= now_ms
            );
            if !slot.enabled
                || slot.spec.lifecycle != PluginServiceLifecycle::Continuous
                || !due
            {
                continue;
            }
            match self.ensure_started(&mut slot, now_ms).await {
                Ok(_) => result.restarted.push(slot.spec.plugin_product_id.clone()),
                Err(error @ PluginRuntimePlatformError::ServiceCapacityExhausted { .. }) => {
                    result
                        .blocked
                        .push((slot.spec.plugin_product_id.clone(), error.to_string()));
                }
                Err(error) => {
                    if matches!(slot.state, PluginRuntimeServiceHostState::Stopped) {
                        self.transition_crash(&mut slot, error.to_string(), now_ms)
                            .await;
                    }
                    result
                        .blocked
                        .push((slot.spec.plugin_product_id.clone(), error.to_string()));
                }
            }
        }
        Ok(result)
    }

    async fn state(&self, plugin_product_id: &PluginProductId) -> Option<PluginRuntimeServiceHostState> {
        let slot = self.slot(plugin_product_id).await?;
        Some(slot.lock().await.state.clone())
    }

}

fn capacity_snapshot(capacity: &ServiceCapacityState) -> PluginRuntimeServiceCapacitySnapshot {
    PluginRuntimeServiceCapacitySnapshot {
        max_active_service_hosts: capacity.max_active_service_hosts,
        active_plugins: capacity.active.keys().cloned().collect(),
    }
}
