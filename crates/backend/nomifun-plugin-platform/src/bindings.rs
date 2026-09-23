//! In-memory Action/Binding index for Unified Plugin Core.
//!
//! Binding point owners define their contracts and invocation adapters. Plugin
//! activation atomically replaces one Plugin's active publications without
//! creating another lifecycle model.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    DigestHex, PluginActionPublication, PluginBindingPoint, PluginId, StrictJsonValue,
};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::Notify;

const DEFAULT_ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_CALL_DEPTH: usize = 16;
const CANCELLATION_SETTLE_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingMultiplicity {
    Single,
    Multiple,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingFailureSemantics {
    FailClosed,
    Continue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindingPointContract {
    pub point: PluginBindingPoint,
    pub input_schema: StrictJsonValue,
    pub output_schema: StrictJsonValue,
    pub multiplicity: BindingMultiplicity,
    pub failure: BindingFailureSemantics,
    pub timeout: Duration,
}

impl BindingPointContract {
    pub fn validate(&self) -> PluginBindingResult<()> {
        if !self.input_schema.0.is_object() || !self.output_schema.0.is_object() {
            return Err(PluginBindingError::InvalidContract(
                "Binding input and output must be inline JSON Schema objects".into(),
            ));
        }
        if self.timeout.is_zero() {
            return Err(PluginBindingError::InvalidContract(
                "Binding timeout must be greater than zero".into(),
            ));
        }
        jsonschema::validator_for(&self.input_schema.0).map_err(|error| {
            PluginBindingError::InvalidContract(format!("invalid Binding input schema: {error}"))
        })?;
        jsonschema::validator_for(&self.output_schema.0).map_err(|error| {
            PluginBindingError::InvalidContract(format!("invalid Binding output schema: {error}"))
        })?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginActionRegistration {
    pub publication: PluginActionPublication,
    /// Optional points may remain dormant when their owner is not installed.
    /// Required points reject activation when unsupported.
    pub optional_bindings: BTreeSet<PluginBindingPoint>,
}

impl From<PluginActionPublication> for PluginActionRegistration {
    fn from(publication: PluginActionPublication) -> Self {
        Self {
            publication,
            optional_bindings: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PluginDispatchOptions {
    pub expected_artifact_digest: Option<DigestHex>,
    pub cancellation: PluginCancellation,
    pub call_chain: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PluginCancellation {
    state: Arc<PluginCancellationState>,
}

#[derive(Debug)]
struct PluginCancellationState {
    canceled: AtomicBool,
    notify: Notify,
}

impl Default for PluginCancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginCancellation {
    pub fn new() -> Self {
        Self {
            state: Arc::new(PluginCancellationState {
                canceled: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    pub fn cancel(&self) {
        if !self.state.canceled.swap(true, Ordering::AcqRel) {
            self.state.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.canceled.load(Ordering::Acquire)
    }

    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            let notified = self.state.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

impl PluginDispatchOptions {
    pub fn nested(invocation: &PluginActionInvocation) -> Self {
        Self {
            expected_artifact_digest: None,
            cancellation: invocation.cancellation.clone(),
            call_chain: invocation.call_chain.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PluginActionInvocation {
    pub stable_action_id: String,
    pub publication: PluginActionPublication,
    pub binding_point: Option<PluginBindingPoint>,
    pub input: StrictJsonValue,
    pub call_chain: Vec<String>,
    pub cancellation: PluginCancellation,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{code}: {message}")]
pub struct PluginActionCallError {
    pub code: String,
    pub message: String,
}

impl PluginActionCallError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait PluginActionRuntimePort: Send + Sync {
    async fn invoke(
        &self,
        invocation: PluginActionInvocation,
    ) -> Result<StrictJsonValue, PluginActionCallError>;
}

#[async_trait]
pub trait BindingInvocationAdapter: Send + Sync {
    async fn invoke(
        &self,
        runtime: Arc<dyn PluginActionRuntimePort>,
        invocation: PluginActionInvocation,
    ) -> Result<StrictJsonValue, PluginActionCallError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PassthroughBindingAdapter;

#[async_trait]
impl BindingInvocationAdapter for PassthroughBindingAdapter {
    async fn invoke(
        &self,
        runtime: Arc<dyn PluginActionRuntimePort>,
        invocation: PluginActionInvocation,
    ) -> Result<StrictJsonValue, PluginActionCallError> {
        runtime.invoke(invocation).await
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginActionUnavailableReason {
    Disabled,
    ActionRemoved,
    PluginRemoved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginActionAvailability {
    Available,
    Unavailable(PluginActionUnavailableReason),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoundPluginAction {
    pub stable_action_id: String,
    pub publication: PluginActionPublication,
    pub availability: PluginActionAvailability,
    pub order: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindingSequenceOutput {
    pub stable_action_id: String,
    pub output: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindingSequenceFailure {
    pub stable_action_id: String,
    pub error: PluginBindingError,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BindingSequenceReport {
    pub outputs: Vec<BindingSequenceOutput>,
    pub failures: Vec<BindingSequenceFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopCommandSelection {
    pub stable_action_id: String,
}

pub type PluginBindingResult<T> = Result<T, PluginBindingError>;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PluginBindingError {
    #[error("invalid Plugin Binding contract: {0}")]
    InvalidContract(String),
    #[error("Binding point owner is not registered: {0}")]
    UnsupportedBinding(String),
    #[error("Binding point owner is already registered: {0}")]
    DuplicateBindingOwner(String),
    #[error("Binding point permits only one active Action: {0}")]
    MultiplicityConflict(String),
    #[error("Plugin Action was not found: {0}")]
    ActionNotFound(String),
    #[error("Plugin Action is unavailable: {stable_action_id} ({reason:?})")]
    ActionUnavailable {
        stable_action_id: String,
        reason: PluginActionUnavailableReason,
    },
    #[error("Plugin Action is not bound to {point}: {stable_action_id}")]
    ActionNotBound {
        point: String,
        stable_action_id: String,
    },
    #[error("Plugin Action artifact fence rejected {0}")]
    ArtifactFence(String),
    #[error("Plugin Action call chain is recursive: {0}")]
    RecursiveCall(String),
    #[error("Plugin Action call chain exceeds its depth limit")]
    CallDepthExceeded,
    #[error("Plugin Action invocation was canceled")]
    Canceled,
    #[error("Plugin Action invocation timed out")]
    Timeout,
    #[error("Plugin Action input failed schema validation: {0}")]
    InputSchema(String),
    #[error("Plugin Action output failed schema validation: {0}")]
    OutputSchema(String),
    #[error("Plugin Action runtime failed: {code}: {message}")]
    Runtime { code: String, message: String },
    #[error("Plugin Action runtime task failed: {0}")]
    RuntimeTask(String),
    #[error("Plugin Binding registry lock was poisoned")]
    Poisoned,
}

#[derive(Clone)]
struct BindingOwner {
    contract: BindingPointContract,
    adapter: Arc<dyn BindingInvocationAdapter>,
}

#[derive(Clone)]
struct ActiveAction {
    registration: PluginActionRegistration,
    enabled: bool,
    order: u64,
}

#[derive(Clone)]
struct ActionTombstone {
    publication: PluginActionPublication,
    reason: PluginActionUnavailableReason,
    order: u64,
}

#[derive(Clone, Default)]
struct RegistryState {
    owners: BTreeMap<PluginBindingPoint, BindingOwner>,
    actions: BTreeMap<String, ActiveAction>,
    tombstones: BTreeMap<String, ActionTombstone>,
    bindings: BTreeMap<PluginBindingPoint, Vec<String>>,
    revision: u64,
    next_order: u64,
}

#[derive(Clone)]
pub struct InMemoryPluginBindingRegistry {
    state: Arc<RwLock<RegistryState>>,
    runtime: Arc<dyn PluginActionRuntimePort>,
    max_call_depth: usize,
    direct_timeout: Duration,
}

impl InMemoryPluginBindingRegistry {
    pub fn new(runtime: Arc<dyn PluginActionRuntimePort>) -> Self {
        Self {
            state: Arc::new(RwLock::new(RegistryState::default())),
            runtime,
            max_call_depth: DEFAULT_MAX_CALL_DEPTH,
            direct_timeout: DEFAULT_ACTION_TIMEOUT,
        }
    }

    pub fn with_limits(
        runtime: Arc<dyn PluginActionRuntimePort>,
        max_call_depth: usize,
        direct_timeout: Duration,
    ) -> PluginBindingResult<Self> {
        if max_call_depth == 0 || direct_timeout.is_zero() {
            return Err(PluginBindingError::InvalidContract(
                "dispatch limits must be greater than zero".into(),
            ));
        }
        Ok(Self {
            state: Arc::new(RwLock::new(RegistryState::default())),
            runtime,
            max_call_depth,
            direct_timeout,
        })
    }

    pub fn revision(&self) -> PluginBindingResult<u64> {
        Ok(self
            .state
            .read()
            .map_err(|_| PluginBindingError::Poisoned)?
            .revision)
    }

    pub fn register_binding_owner(
        &self,
        contract: BindingPointContract,
        adapter: Arc<dyn BindingInvocationAdapter>,
    ) -> PluginBindingResult<u64> {
        contract.validate()?;
        let mut state = self
            .state
            .write()
            .map_err(|_| PluginBindingError::Poisoned)?;
        if state.owners.contains_key(&contract.point) {
            return Err(PluginBindingError::DuplicateBindingOwner(
                contract.point.as_str().into(),
            ));
        }
        let mut candidate = state.clone();
        candidate
            .owners
            .insert(contract.point, BindingOwner { contract, adapter });
        rebuild_bindings(&mut candidate)?;
        candidate.revision = next_revision(candidate.revision)?;
        *state = candidate;
        Ok(state.revision)
    }

    pub fn replace_plugin(
        &self,
        plugin_id: PluginId,
        artifact_digest: DigestHex,
        enabled: bool,
        registrations: Vec<PluginActionRegistration>,
    ) -> PluginBindingResult<u64> {
        let mut state = self
            .state
            .write()
            .map_err(|_| PluginBindingError::Poisoned)?;
        let mut candidate = plugin_replacement_candidate(
            &state,
            &plugin_id,
            &artifact_digest,
            enabled,
            registrations,
        )?;
        candidate.revision = next_revision(candidate.revision)?;
        *state = candidate;
        Ok(state.revision)
    }

    pub fn validate_plugin_replacement(
        &self,
        plugin_id: &PluginId,
        artifact_digest: &DigestHex,
        enabled: bool,
        registrations: Vec<PluginActionRegistration>,
    ) -> PluginBindingResult<()> {
        let state = self
            .state
            .read()
            .map_err(|_| PluginBindingError::Poisoned)?;
        let candidate = plugin_replacement_candidate(
            &state,
            plugin_id,
            artifact_digest,
            enabled,
            registrations,
        )?;
        next_revision(candidate.revision)?;
        Ok(())
    }

    pub fn set_plugin_enabled(
        &self,
        plugin_id: &PluginId,
        enabled: bool,
    ) -> PluginBindingResult<u64> {
        let mut state = self
            .state
            .write()
            .map_err(|_| PluginBindingError::Poisoned)?;
        let mut candidate = state.clone();
        let mut found = false;
        for action in candidate.actions.values_mut() {
            if &action.registration.publication.plugin_id == plugin_id {
                action.enabled = enabled;
                found = true;
            }
        }
        if !found {
            return Err(PluginBindingError::ActionNotFound(format!(
                "plugin:{}",
                plugin_id.as_ref()
            )));
        }
        rebuild_bindings(&mut candidate)?;
        candidate.revision = next_revision(candidate.revision)?;
        *state = candidate;
        Ok(state.revision)
    }

    pub fn remove_plugin(&self, plugin_id: &PluginId) -> PluginBindingResult<u64> {
        let mut state = self
            .state
            .write()
            .map_err(|_| PluginBindingError::Poisoned)?;
        let mut candidate = state.clone();
        let removed = candidate
            .actions
            .iter()
            .filter(|(_, action)| &action.registration.publication.plugin_id == plugin_id)
            .map(|(stable, action)| (stable.clone(), action.clone()))
            .collect::<Vec<_>>();
        for (stable, action) in removed {
            candidate.actions.remove(&stable);
            candidate.tombstones.insert(
                stable,
                ActionTombstone {
                    publication: action.registration.publication,
                    reason: PluginActionUnavailableReason::PluginRemoved,
                    order: action.order,
                },
            );
        }
        rebuild_bindings(&mut candidate)?;
        candidate.revision = next_revision(candidate.revision)?;
        *state = candidate;
        Ok(state.revision)
    }

    pub fn resolve_action(&self, stable_action_id: &str) -> PluginBindingResult<BoundPluginAction> {
        let state = self
            .state
            .read()
            .map_err(|_| PluginBindingError::Poisoned)?;
        resolve_from_state(&state, stable_action_id)
    }

    pub fn list_binding(
        &self,
        point: PluginBindingPoint,
    ) -> PluginBindingResult<Vec<BoundPluginAction>> {
        let state = self
            .state
            .read()
            .map_err(|_| PluginBindingError::Poisoned)?;
        list_binding_from_state(&state, point)
    }

    pub async fn dispatch_action(
        &self,
        stable_action_id: &str,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<StrictJsonValue> {
        self.dispatch(None, stable_action_id, input, options).await
    }

    pub async fn dispatch_binding(
        &self,
        point: PluginBindingPoint,
        stable_action_id: &str,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<StrictJsonValue> {
        self.dispatch(Some(point), stable_action_id, input, options)
            .await
    }

    async fn dispatch(
        &self,
        point: Option<PluginBindingPoint>,
        stable_action_id: &str,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<StrictJsonValue> {
        if options.cancellation.is_cancelled() {
            return Err(PluginBindingError::Canceled);
        }
        if options.call_chain.len() >= self.max_call_depth {
            return Err(PluginBindingError::CallDepthExceeded);
        }
        if options.call_chain.iter().any(|entry| entry == stable_action_id) {
            return Err(PluginBindingError::RecursiveCall(stable_action_id.into()));
        }

        let (active, owner) = {
            let state = self
                .state
                .read()
                .map_err(|_| PluginBindingError::Poisoned)?;
            if let Some(tombstone) = state.tombstones.get(stable_action_id) {
                return Err(PluginBindingError::ActionUnavailable {
                    stable_action_id: stable_action_id.into(),
                    reason: tombstone.reason,
                });
            }
            let action = state.actions.get(stable_action_id).cloned().ok_or_else(|| {
                PluginBindingError::ActionNotFound(stable_action_id.into())
            })?;
            if !action.enabled {
                return Err(PluginBindingError::ActionUnavailable {
                    stable_action_id: stable_action_id.into(),
                    reason: PluginActionUnavailableReason::Disabled,
                });
            }
            let owner = match point {
                Some(point) => {
                    if !action.registration.publication.bindings.contains(&point) {
                        return Err(PluginBindingError::ActionNotBound {
                            point: point.as_str().into(),
                            stable_action_id: stable_action_id.into(),
                        });
                    }
                    Some(state.owners.get(&point).cloned().ok_or_else(|| {
                        PluginBindingError::UnsupportedBinding(point.as_str().into())
                    })?)
                }
                None => None,
            };
            (action, owner)
        };

        if options.expected_artifact_digest.as_ref().is_some_and(|expected| {
            expected != &active.registration.publication.artifact.digest
        }) {
            return Err(PluginBindingError::ArtifactFence(stable_action_id.into()));
        }
        validate_schema(
            &active.registration.publication.action.input.0,
            &input.0,
            true,
        )?;
        if let Some(owner) = &owner {
            validate_schema(&owner.contract.input_schema.0, &input.0, true)?;
        }

        let mut call_chain = options.call_chain;
        call_chain.push(stable_action_id.into());
        let child_cancellation = PluginCancellation::new();
        let invocation = PluginActionInvocation {
            stable_action_id: stable_action_id.into(),
            publication: active.registration.publication.clone(),
            binding_point: point,
            input,
            call_chain,
            cancellation: child_cancellation.clone(),
        };
        let timeout = owner
            .as_ref()
            .map(|owner| owner.contract.timeout)
            .unwrap_or(self.direct_timeout);
        let runtime = Arc::clone(&self.runtime);
        let adapter = owner.as_ref().map(|owner| Arc::clone(&owner.adapter));
        let task = tokio::spawn(async move {
            match adapter {
                Some(adapter) => adapter.invoke(runtime, invocation).await,
                None => runtime.invoke(invocation).await,
            }
        });
        tokio::pin!(task);
        let result = tokio::select! {
            result = &mut task => result.map_err(|error| PluginBindingError::RuntimeTask(error.to_string()))?
                .map_err(|error| PluginBindingError::Runtime { code: error.code, message: error.message }),
            () = options.cancellation.cancelled() => {
                child_cancellation.cancel();
                let _ = tokio::time::timeout(CANCELLATION_SETTLE_TIMEOUT, &mut task).await;
                return Err(PluginBindingError::Canceled);
            }
            () = tokio::time::sleep(timeout) => {
                child_cancellation.cancel();
                let _ = tokio::time::timeout(CANCELLATION_SETTLE_TIMEOUT, &mut task).await;
                return Err(PluginBindingError::Timeout);
            }
        }?;

        validate_schema(
            &active.registration.publication.action.output.0,
            &result.0,
            false,
        )?;
        if let Some(owner) = owner {
            validate_schema(&owner.contract.output_schema.0, &result.0, false)?;
        }
        Ok(result)
    }

    async fn dispatch_sequence(
        &self,
        point: PluginBindingPoint,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<BindingSequenceReport> {
        let (actions, failure) = {
            let state = self
                .state
                .read()
                .map_err(|_| PluginBindingError::Poisoned)?;
            let owner = state.owners.get(&point).ok_or_else(|| {
                PluginBindingError::UnsupportedBinding(point.as_str().into())
            })?;
            (
                state
                    .bindings
                    .get(&point)
                    .into_iter()
                    .flatten()
                    .filter_map(|stable| {
                        state
                            .actions
                            .get(stable)
                            .filter(|action| action.enabled)
                            .map(|_| stable.clone())
                    })
                    .collect::<Vec<_>>(),
                owner.contract.failure,
            )
        };
        let mut report = BindingSequenceReport::default();
        for stable_action_id in actions {
            match self
                .dispatch_binding(
                    point,
                    &stable_action_id,
                    input.clone(),
                    options.clone(),
                )
                .await
            {
                Ok(output) => report.outputs.push(BindingSequenceOutput {
                    stable_action_id,
                    output,
                }),
                Err(error) if failure == BindingFailureSemantics::Continue => {
                    report.failures.push(BindingSequenceFailure {
                        stable_action_id,
                        error,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        Ok(report)
    }
}

/// Application-service adapter: installation, recovery and lifecycle actions
/// update the same process-wide registry consumed by Agent/Desktop/Automation.
#[async_trait]
impl crate::PluginBindingPort for InMemoryPluginBindingRegistry {
    async fn validate(
        &self,
        owner_user_id: &str,
        plugin: &crate::PluginRecord,
        actions: &[PluginActionPublication],
    ) -> Result<(), String> {
        if plugin.owner_user_id != owner_user_id {
            return Err("Plugin Binding owner mismatch".into());
        }
        self.validate_plugin_replacement(
            &plugin.plugin_id,
            &plugin.active_artifact_digest,
            plugin.is_available(),
            actions
                .iter()
                .cloned()
                .map(PluginActionRegistration::from)
                .collect(),
        )
        .map_err(|error| error.to_string())
    }

    async fn replace(
        &self,
        owner_user_id: &str,
        plugin: &crate::PluginRecord,
        actions: &[PluginActionPublication],
    ) -> Result<(), String> {
        if plugin.owner_user_id != owner_user_id {
            return Err("Plugin Binding owner mismatch".into());
        }
        self.replace_plugin(
            plugin.plugin_id.clone(),
            plugin.active_artifact_digest.clone(),
            plugin.is_available(),
            actions
                .iter()
                .cloned()
                .map(PluginActionRegistration::from)
                .collect(),
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    async fn remove(
        &self,
        _owner_user_id: &str,
        plugin_id: &PluginId,
    ) -> Result<(), String> {
        self.remove_plugin(plugin_id)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone)]
pub struct DesktopPluginBindings {
    registry: InMemoryPluginBindingRegistry,
}

impl DesktopPluginBindings {
    pub fn new(registry: InMemoryPluginBindingRegistry) -> Self {
        Self { registry }
    }

    pub fn commands(&self) -> PluginBindingResult<Vec<BoundPluginAction>> {
        self.registry.list_binding(PluginBindingPoint::DesktopCommand)
    }

    pub fn select(&self, stable_action_id: &str) -> PluginBindingResult<DesktopCommandSelection> {
        let action = self
            .commands()?
            .into_iter()
            .find(|action| action.stable_action_id == stable_action_id)
            .ok_or_else(|| PluginBindingError::ActionNotFound(stable_action_id.into()))?;
        if let PluginActionAvailability::Unavailable(reason) = action.availability {
            return Err(PluginBindingError::ActionUnavailable {
                stable_action_id: stable_action_id.into(),
                reason,
            });
        }
        Ok(DesktopCommandSelection {
            stable_action_id: stable_action_id.into(),
        })
    }

    pub async fn trigger(
        &self,
        selection: &DesktopCommandSelection,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<StrictJsonValue> {
        self.registry
            .dispatch_binding(
                PluginBindingPoint::DesktopCommand,
                &selection.stable_action_id,
                input,
                options,
            )
            .await
    }

    pub async fn emit_event(
        &self,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<BindingSequenceReport> {
        self.registry
            .dispatch_sequence(PluginBindingPoint::DesktopEvent, input, options)
            .await
    }
}

#[derive(Clone)]
pub struct AutomationPluginBindings {
    registry: InMemoryPluginBindingRegistry,
}

impl AutomationPluginBindings {
    pub fn new(registry: InMemoryPluginBindingRegistry) -> Self {
        Self { registry }
    }

    pub fn actions(&self) -> PluginBindingResult<Vec<BoundPluginAction>> {
        self.registry
            .list_binding(PluginBindingPoint::AutomationAction)
    }

    pub async fn trigger(
        &self,
        stable_action_id: &str,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<StrictJsonValue> {
        self.registry
            .dispatch_binding(
                PluginBindingPoint::AutomationAction,
                stable_action_id,
                input,
                options,
            )
            .await
    }
}

#[derive(Clone)]
pub struct AgentPluginBindings {
    registry: InMemoryPluginBindingRegistry,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentPluginBindingSnapshot {
    pub revision: u64,
    pub tools: Vec<BoundPluginAction>,
    pub contexts: Vec<BoundPluginAction>,
    pub before_model: Vec<BoundPluginAction>,
    pub before_tool: Vec<BoundPluginAction>,
}

impl AgentPluginBindings {
    pub fn new(registry: InMemoryPluginBindingRegistry) -> Self {
        Self { registry }
    }

    pub fn tools(&self) -> PluginBindingResult<Vec<BoundPluginAction>> {
        self.registry.list_binding(PluginBindingPoint::AgentTool)
    }

    /// Freeze every Agent-owned Binding point under one registry read lock.
    /// Session assembly therefore cannot combine tools from one activation
    /// revision with hooks or context from another.
    pub fn snapshot(&self) -> PluginBindingResult<AgentPluginBindingSnapshot> {
        let state = self
            .registry
            .state
            .read()
            .map_err(|_| PluginBindingError::Poisoned)?;
        let list = |point| list_binding_from_state(&state, point);
        Ok(AgentPluginBindingSnapshot {
            revision: state.revision,
            tools: list(PluginBindingPoint::AgentTool)?,
            contexts: list(PluginBindingPoint::AgentContext)?,
            before_model: list(PluginBindingPoint::AgentBeforeModel)?,
            before_tool: list(PluginBindingPoint::AgentBeforeTool)?,
        })
    }

    pub fn contexts(&self) -> PluginBindingResult<Vec<BoundPluginAction>> {
        self.registry.list_binding(PluginBindingPoint::AgentContext)
    }

    pub fn before_model(&self) -> PluginBindingResult<Vec<BoundPluginAction>> {
        self.registry
            .list_binding(PluginBindingPoint::AgentBeforeModel)
    }

    pub fn before_tool(&self) -> PluginBindingResult<Vec<BoundPluginAction>> {
        self.registry
            .list_binding(PluginBindingPoint::AgentBeforeTool)
    }

    pub async fn invoke_tool(
        &self,
        stable_action_id: &str,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<StrictJsonValue> {
        self.registry
            .dispatch_binding(
                PluginBindingPoint::AgentTool,
                stable_action_id,
                input,
                options,
            )
            .await
    }

    pub async fn invoke_context(
        &self,
        stable_action_id: &str,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<StrictJsonValue> {
        self.registry
            .dispatch_binding(
                PluginBindingPoint::AgentContext,
                stable_action_id,
                input,
                options,
            )
            .await
    }

    pub async fn invoke_before_model(
        &self,
        stable_action_id: &str,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<StrictJsonValue> {
        self.registry
            .dispatch_binding(
                PluginBindingPoint::AgentBeforeModel,
                stable_action_id,
                input,
                options,
            )
            .await
    }

    pub async fn invoke_before_tool(
        &self,
        stable_action_id: &str,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<StrictJsonValue> {
        self.registry
            .dispatch_binding(
                PluginBindingPoint::AgentBeforeTool,
                stable_action_id,
                input,
                options,
            )
            .await
    }

    pub async fn contribute_context(
        &self,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<BindingSequenceReport> {
        self.registry
            .dispatch_sequence(PluginBindingPoint::AgentContext, input, options)
            .await
    }

    pub async fn run_before_model(
        &self,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<BindingSequenceReport> {
        self.registry
            .dispatch_sequence(PluginBindingPoint::AgentBeforeModel, input, options)
            .await
    }

    pub async fn run_before_tool(
        &self,
        input: StrictJsonValue,
        options: PluginDispatchOptions,
    ) -> PluginBindingResult<BindingSequenceReport> {
        self.registry
            .dispatch_sequence(PluginBindingPoint::AgentBeforeTool, input, options)
            .await
    }
}

fn plugin_replacement_candidate(
    state: &RegistryState,
    plugin_id: &PluginId,
    artifact_digest: &DigestHex,
    enabled: bool,
    registrations: Vec<PluginActionRegistration>,
) -> PluginBindingResult<RegistryState> {
    validate_plugin_id(plugin_id)?;
    validate_digest(artifact_digest)?;
    let mut candidate = state.clone();
    let mut incoming = Vec::new();
    let mut incoming_ids = BTreeSet::new();
    for registration in registrations {
        validate_registration(
            &registration,
            plugin_id,
            artifact_digest,
            &candidate.owners,
        )?;
        let stable_action_id = registration.publication.stable_id();
        if !incoming_ids.insert(stable_action_id.clone()) {
            return Err(PluginBindingError::InvalidContract(format!(
                "duplicate Action {stable_action_id}"
            )));
        }
        incoming.push((stable_action_id, registration));
    }

    let existing = candidate
        .actions
        .iter()
        .filter(|(_, action)| &action.registration.publication.plugin_id == plugin_id)
        .map(|(stable, action)| (stable.clone(), action.clone()))
        .collect::<BTreeMap<_, _>>();
    for (stable, action) in &existing {
        candidate.actions.remove(stable);
        if !incoming_ids.contains(stable) {
            candidate.tombstones.insert(
                stable.clone(),
                ActionTombstone {
                    publication: action.registration.publication.clone(),
                    reason: PluginActionUnavailableReason::ActionRemoved,
                    order: action.order,
                },
            );
        }
    }

    for (stable, registration) in incoming {
        let preserved_order = existing
            .get(&stable)
            .map(|action| action.order)
            .or_else(|| candidate.tombstones.get(&stable).map(|entry| entry.order));
        let order = match preserved_order {
            Some(order) => order,
            None => {
                let order = candidate.next_order;
                candidate.next_order = candidate.next_order.checked_add(1).ok_or_else(|| {
                    PluginBindingError::InvalidContract("Binding order overflow".into())
                })?;
                order
            }
        };
        candidate.tombstones.remove(&stable);
        candidate.actions.insert(
            stable,
            ActiveAction {
                registration,
                enabled,
                order,
            },
        );
    }
    rebuild_bindings(&mut candidate)?;
    Ok(candidate)
}

fn validate_registration(
    registration: &PluginActionRegistration,
    plugin_id: &PluginId,
    artifact_digest: &DigestHex,
    owners: &BTreeMap<PluginBindingPoint, BindingOwner>,
) -> PluginBindingResult<()> {
    let publication = &registration.publication;
    if &publication.plugin_id != plugin_id {
        return Err(PluginBindingError::InvalidContract(
            "Action belongs to another Plugin".into(),
        ));
    }
    if &publication.artifact.digest != artifact_digest {
        return Err(PluginBindingError::InvalidContract(
            "Action artifact differs from the active artifact fence".into(),
        ));
    }
    validate_digest(&publication.artifact.digest)?;
    if publication.action_id.is_empty()
        || publication.action_id.len() > 96
        || !publication
            .action_id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        || !publication.action_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err(PluginBindingError::InvalidContract(
            "Action id is invalid".into(),
        ));
    }
    if !registration
        .optional_bindings
        .is_subset(&publication.bindings)
    {
        return Err(PluginBindingError::InvalidContract(
            "optional Binding points must be declared by the Action".into(),
        ));
    }
    for point in &publication.bindings {
        if !owners.contains_key(point) && !registration.optional_bindings.contains(point) {
            return Err(PluginBindingError::UnsupportedBinding(
                point.as_str().into(),
            ));
        }
    }
    if !publication.action.input.0.is_object() || !publication.action.output.0.is_object() {
        return Err(PluginBindingError::InvalidContract(
            "Action schemas must be JSON objects".into(),
        ));
    }
    jsonschema::validator_for(&publication.action.input.0).map_err(|error| {
        PluginBindingError::InvalidContract(format!("invalid Action input schema: {error}"))
    })?;
    jsonschema::validator_for(&publication.action.output.0).map_err(|error| {
        PluginBindingError::InvalidContract(format!("invalid Action output schema: {error}"))
    })?;
    Ok(())
}

fn rebuild_bindings(state: &mut RegistryState) -> PluginBindingResult<()> {
    let mut bindings = BTreeMap::<PluginBindingPoint, Vec<(u64, String)>>::new();
    for (stable, action) in &state.actions {
        for point in &action.registration.publication.bindings {
            if state.owners.contains_key(point) {
                bindings
                    .entry(*point)
                    .or_default()
                    .push((action.order, stable.clone()));
            }
        }
    }
    for (stable, tombstone) in &state.tombstones {
        for point in &tombstone.publication.bindings {
            if state.owners.contains_key(point) {
                bindings
                    .entry(*point)
                    .or_default()
                    .push((tombstone.order, stable.clone()));
            }
        }
    }
    for values in bindings.values_mut() {
        values.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    }
    for (point, owner) in &state.owners {
        if owner.contract.multiplicity == BindingMultiplicity::Single {
            let active = bindings
                .get(point)
                .into_iter()
                .flatten()
                .filter(|(_, stable)| state.actions.get(stable).is_some_and(|action| action.enabled))
                .count();
            if active > 1 {
                return Err(PluginBindingError::MultiplicityConflict(
                    point.as_str().into(),
                ));
            }
        }
    }
    state.bindings = bindings
        .into_iter()
        .map(|(point, values)| {
            (
                point,
                values.into_iter().map(|(_, stable)| stable).collect(),
            )
        })
        .collect();
    Ok(())
}

fn resolve_from_state(
    state: &RegistryState,
    stable_action_id: &str,
) -> PluginBindingResult<BoundPluginAction> {
    if let Some(action) = state.actions.get(stable_action_id) {
        return Ok(BoundPluginAction {
            stable_action_id: stable_action_id.into(),
            publication: action.registration.publication.clone(),
            availability: if action.enabled {
                PluginActionAvailability::Available
            } else {
                PluginActionAvailability::Unavailable(
                    PluginActionUnavailableReason::Disabled,
                )
            },
            order: action.order,
        });
    }
    if let Some(tombstone) = state.tombstones.get(stable_action_id) {
        return Ok(BoundPluginAction {
            stable_action_id: stable_action_id.into(),
            publication: tombstone.publication.clone(),
            availability: PluginActionAvailability::Unavailable(tombstone.reason),
            order: tombstone.order,
        });
    }
    Err(PluginBindingError::ActionNotFound(stable_action_id.into()))
}

fn list_binding_from_state(
    state: &RegistryState,
    point: PluginBindingPoint,
) -> PluginBindingResult<Vec<BoundPluginAction>> {
    if !state.owners.contains_key(&point) {
        return Err(PluginBindingError::UnsupportedBinding(
            point.as_str().into(),
        ));
    }
    state
        .bindings
        .get(&point)
        .into_iter()
        .flatten()
        .map(|stable| resolve_from_state(state, stable))
        .collect()
}

fn validate_schema(
    schema: &Value,
    value: &Value,
    input: bool,
) -> PluginBindingResult<()> {
    let validator = jsonschema::validator_for(schema).map_err(|error| {
        PluginBindingError::InvalidContract(format!("invalid invocation schema: {error}"))
    })?;
    validator.validate(value).map_err(|error| {
        if input {
            PluginBindingError::InputSchema(error.to_string())
        } else {
            PluginBindingError::OutputSchema(error.to_string())
        }
    })
}

fn validate_plugin_id(plugin_id: &PluginId) -> PluginBindingResult<()> {
    let value = plugin_id.as_ref();
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        return Err(PluginBindingError::InvalidContract(
            "Plugin id is invalid".into(),
        ));
    }
    Ok(())
}

fn validate_digest(digest: &DigestHex) -> PluginBindingResult<()> {
    if digest.as_ref().len() != 64
        || !digest
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(PluginBindingError::InvalidContract(
            "artifact digest must be lowercase SHA-256".into(),
        ));
    }
    Ok(())
}

fn next_revision(revision: u64) -> PluginBindingResult<u64> {
    revision
        .checked_add(1)
        .ok_or_else(|| PluginBindingError::InvalidContract("registry revision overflow".into()))
}
