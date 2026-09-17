//! Application-owned Wave 2 capability host.
//!
//! Only operations with an existing typed, owner-scoped resource API are
//! configured here. Unsupported families fail closed instead of delegating to
//! the legacy Gateway or manufacturing an acknowledgement.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
#[cfg(test)]
use std::time::Duration;

use nomifun_agent_contracts::{
    DigestHex, StrictJsonValue, TypedResourceBinding, canonical_json_bytes, digest_payload,
};
use nomifun_agent_domain_wave2::{
    Wave2CapabilityOperation, Wave2HostContext, Wave2HostPort, Wave2HostPortError,
    Wave2HostRequest,
};
use nomifun_api_types::{TypedResourceBindingDto, WebSocketMessage};
use nomifun_common::AppError;
use nomifun_file::{
    AgentSessionPatchRequest, AgentSessionWorkspaceBinding, FileService,
    WorkspaceArtifactStore, WorkspaceVcsStageOwner,
    WORKSPACE_READ_OPERATION, WORKSPACE_RESOURCE_KIND, WORKSPACE_ROOT_PARAMETER,
    WORKSPACE_WRITE_OPERATION,
};
use nomifun_realtime::UserEventSink;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::agent_wave2_vcs_push::{
    VcsPushActionInput, VcsPushEffectDisposition, VcsPushError, VcsPushOwner,
    VcsPushRequest,
};
const MAX_DIFF_BYTES: usize = 1024 * 1024;
const MAX_WAVE2_IDEMPOTENCY_KEY_BYTES: usize = 128;

#[derive(Clone)]
pub(crate) struct Wave2ApplicationHost {
    files: Arc<FileService>,
    artifacts: Result<Arc<WorkspaceArtifactStore>, Arc<str>>,
    workspace_write_lock: Arc<tokio::sync::Mutex<()>>,
    git_mutation_lock: Arc<tokio::sync::Mutex<()>>,
    vcs_push_owner: Arc<OnceLock<Result<VcsPushOwner, VcsPushError>>>,
    vcs_stage_owner: Result<Arc<WorkspaceVcsStageOwner>, Arc<str>>,
    effect_store: Option<Arc<nomifun_agent_session::AgentSessionStore>>,
    configured_workspace_root: PathBuf,
    #[cfg(test)]
    git_mutation_hook: Option<Arc<GitMutationTestHook>>,
}

#[derive(Clone, Debug)]
struct Wave2EffectReservation {
    store: nomifun_agent_session::AgentSessionStore,
    request: nomifun_agent_session::EffectEventRequest,
}

#[derive(Debug)]
enum Wave2EffectAdmission {
    Replay(StrictJsonValue),
    Reserved(Wave2EffectReservation),
}

#[derive(Clone, Copy)]
enum Wave2EffectCompletion<'a> {
    Succeeded(&'a StrictJsonValue),
    Failed(&'a Wave2HostPortError),
    Uncertain(&'a Wave2HostPortError),
}

/// Compact structured observations survive the generic Kernel error channel
/// and its 2 KiB durable error projection. Never truncate the index groups.
fn patch_failure_error(
    code: &str,
    cause: &str,
    observation: &nomifun_file::AgentPatchFailureObservation,
    journal_settled: bool,
) -> Wave2HostPortError {
    let mut report = json!({
        "kind": "workspace_patch_failed", "version": 1,
        "journal_settlement": if journal_settled { "settled" } else { "unconfirmed" },
        "observation": observation,
        "cause": cause.chars().take(128).collect::<String>(),
        "recovery": "Indices refer to request.files (zero-based). Observations are historical, not current state. Re-read every target and replan; do not retry unchanged. Retained creations were not deleted."
    });
    if report.to_string().len() > 1536 {
        report["cause"] = json!("diagnostic omitted to preserve complete publication observations");
    }
    Wave2HostPortError::new(code, report.to_string())
}

impl Wave2ApplicationHost {
    pub(crate) fn new() -> Self {
        Self::for_workspace_root(std::env::temp_dir())
    }

    pub(crate) fn ensure_git_ready(&self) -> Result<(), AppError> {
        if let Some(Ok(owner)) = self.vcs_push_owner.get() {
            owner.ensure_idle().map_err(|error| AppError::Conflict(error.to_string()))?;
        }
        Ok(())
    }

    pub(crate) async fn settle_git(&self) -> Result<(), AppError> {
        if let Some(Ok(owner)) = self.vcs_push_owner.get() {
            owner.ensure_settled().await.map_err(|error| AppError::Conflict(error.to_string()))?;
        }
        Ok(())
    }

    pub(crate) fn for_workspace_root(workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        let vcs_stage_owner = WorkspaceVcsStageOwner::new(&workspace_root)
            .map(Arc::new)
            .map_err(|error| Arc::<str>::from(error.to_string()));
        let artifacts = WorkspaceArtifactStore::new(&workspace_root)
            .map(Arc::new)
            .map_err(|error| Arc::<str>::from(error.to_string()));
        Self {
            files: Arc::new(FileService::new(
                Arc::new(NullUserEvents),
                vec![workspace_root.clone()],
            )),
            artifacts,
            workspace_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            git_mutation_lock: Arc::new(tokio::sync::Mutex::new(())),
            vcs_push_owner: Arc::new(OnceLock::new()),
            vcs_stage_owner,
            effect_store: None,
            configured_workspace_root: workspace_root,
            #[cfg(test)]
            git_mutation_hook: None,
        }
    }

    pub(crate) fn with_effect_store(
        mut self,
        store: nomifun_agent_session::AgentSessionStore,
    ) -> Self {
        self.effect_store = Some(Arc::new(store));
        self
    }

    fn effect_store(
        &self,
    ) -> Result<&nomifun_agent_session::AgentSessionStore, Wave2HostPortError> {
        self.effect_store.as_deref().ok_or_else(|| {
            Wave2HostPortError::unavailable(
                "canonical Agent Session Effect store is not mounted for workspace effects",
            )
        })
    }

    /// Wrap a physical effect owned by another retained Domain adapter (the
    /// turn-scoped process owner) in the same canonical Agent Effect ledger as
    /// file, Git, and artifact mutations. The caller must finish all pure
    /// validation before entering this boundary.
    pub(crate) async fn invoke_managed_effect<F, Fut>(
        &self,
        context: &Wave2HostContext,
        binding: &TypedResourceBinding,
        input: &StrictJsonValue,
        invoke_owner: F,
    ) -> Result<StrictJsonValue, Wave2HostPortError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<StrictJsonValue, Wave2HostPortError>>,
    {
        let _effect_guard = self.workspace_write_lock.lock().await;
        match begin_wave2_effect(self.effect_store()?, context, binding, input).await? {
            Wave2EffectAdmission::Replay(output) => Ok(output),
            Wave2EffectAdmission::Reserved(reservation) => match invoke_owner().await {
                Ok(output) => {
                    finish_wave2_effect(
                        &reservation,
                        Wave2EffectCompletion::Succeeded(&output),
                    )
                    .await?;
                    Ok(output)
                }
                Err(error) if error.code == "EFFECT_OUTCOME_UNKNOWN" => {
                    // The process owner retains handles and cleanup authority,
                    // but until it can prove a terminal outcome this durable
                    // reservation must remain pending across restarts.
                    Err(error)
                }
                Err(error) => {
                    finish_wave2_effect(
                        &reservation,
                        Wave2EffectCompletion::Failed(&error),
                    )
                    .await?;
                    Err(error)
                }
            },
        }
    }

    #[cfg(test)]
    fn with_git_mutation_hook(mut self, hook: Arc<GitMutationTestHook>) -> Self {
        self.git_mutation_hook = Some(hook);
        self
    }

    async fn pause_after_git_admission_for_test(&self, action_id: &str) {
        #[cfg(test)]
        if let Some(hook) = &self.git_mutation_hook
            && hook.action_id == action_id
            && hook.armed.swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            hook.entered.notify_one();
            hook.release.notified().await;
        }
        #[cfg(not(test))]
        let _ = action_id;
    }
}

#[cfg(test)]
struct GitMutationTestHook {
    action_id: &'static str,
    armed: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[cfg(test)]
impl GitMutationTestHook {
    fn new(action_id: &'static str) -> Arc<Self> {
        Arc::new(Self {
            action_id,
            armed: std::sync::atomic::AtomicBool::new(true),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        })
    }
}

impl Default for Wave2ApplicationHost {
    fn default() -> Self {
        Self::new()
    }
}

fn wave2_effect_request_digest(
    context: &Wave2HostContext,
    binding: &TypedResourceBinding,
    input: &StrictJsonValue,
) -> Result<DigestHex, Wave2HostPortError> {
    let fingerprint = json!({
        "capability_module": context.capability_id.as_ref(),
        "action_id": context.action_id.as_ref(),
        "resource_binding": binding,
        "input": input.0,
    });
    digest_payload(&fingerprint).map_err(|error| {
        Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            format!("Wave 2 effect request could not be canonicalized: {error}"),
        )
    })
}

fn wave2_effect_id(context: &Wave2HostContext) -> Result<String, Wave2HostPortError> {
    let idempotency_key = context.idempotency_key.as_ref();
    if idempotency_key.is_empty() || idempotency_key != idempotency_key.trim() {
        return Err(Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            "Wave 2 effect action requires a canonical non-empty idempotency key without edge whitespace",
        ));
    }
    if idempotency_key.len() > MAX_WAVE2_IDEMPOTENCY_KEY_BYTES
        || !idempotency_key
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
    {
        return Err(Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            format!(
                "Wave 2 effect idempotency key must be 1..={MAX_WAVE2_IDEMPOTENCY_KEY_BYTES} visible ASCII bytes"
            ),
        ));
    }
    let identity = json!({
        "agent_session_id": context.agent_session_id.as_ref(),
        "turn_id": context.turn_id.as_ref(),
        "idempotency_key": idempotency_key,
    });
    let digest = digest_payload(&identity).map_err(|error| {
        Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            format!("Wave 2 effect identity could not be canonicalized: {error}"),
        )
    })?;
    Ok(format!("workspace:{}", digest.as_ref()))
}

fn wave2_effect_resource_key(
    context: &Wave2HostContext,
    binding: &TypedResourceBinding,
) -> Result<String, Wave2HostPortError> {
    if binding.resource_id.as_ref().trim().is_empty() {
        return Err(Wave2HostPortError::invalid_payload(
            "workspace effect requires a non-empty resource identity",
        ));
    }
    if binding.owner_id != context.principal.principal_id
        || context.principal.principal_kind.trim().is_empty()
    {
        return Err(Wave2HostPortError::invalid_payload(
            "workspace effect resource owner differs from authenticated authority",
        ));
    }
    let identity = json!({
        "principal_kind": context.principal.principal_kind.as_str(),
        "owner_id": binding.owner_id.as_str(),
        "resource_kind": binding.resource_kind.as_ref(),
        "resource_id": binding.resource_id.as_ref(),
    });
    let digest = digest_payload(&identity).map_err(|error| {
        Wave2HostPortError::invalid_payload(format!(
            "workspace effect resource identity could not be canonicalized: {error}"
        ))
    })?;
    Ok(format!("workspace:{}", digest.as_ref()))
}

fn effect_record_matches(
    record: &nomifun_agent_session::AgentEffectRecord,
    context: &Wave2HostContext,
    binding: &TypedResourceBinding,
    input_digest: &DigestHex,
    strategy: nomifun_agent_session::EffectStrategy,
    resource_key: &str,
) -> bool {
    record.agent_session_id == context.agent_session_id
        && record.turn_id == context.turn_id
        && record.owner_domain == "workspace"
        && record.capability_module == context.capability_id
        && record.action_id == context.action_id
        && record.resource_binding_id.as_ref() == Some(&binding.binding_id)
        && record.resource_key.as_deref() == Some(resource_key)
        && record.input_digest == *input_digest
        && record.strategy == strategy
}

fn observe_wave2_effect(
    record: nomifun_agent_session::AgentEffectRecord,
    context: &Wave2HostContext,
    binding: &TypedResourceBinding,
    input_digest: &DigestHex,
    strategy: nomifun_agent_session::EffectStrategy,
    resource_key: &str,
) -> Result<Wave2EffectAdmission, Wave2HostPortError> {
    if !effect_record_matches(
        &record,
        context,
        binding,
        input_digest,
        strategy,
        resource_key,
    ) {
        return Err(Wave2HostPortError::new(
            "IDEMPOTENCY_CONFLICT",
            "Wave 2 idempotency key was already used for a different turn, action, resource, or input",
        ));
    }
    match record.state {
        nomifun_agent_session::AgentEffectState::Returned => {
            let result = record
                .bounded_observation
                .as_ref()
                .and_then(|observation| observation.get("result"))
                .cloned()
                .ok_or_else(|| {
                    Wave2HostPortError::unavailable(
                        "Wave 2 effect already returned, but its bounded durable observation cannot reproduce the exact result; automatic execution remains disabled",
                    )
                })?;
            Ok(Wave2EffectAdmission::Replay(StrictJsonValue(result)))
        }
        nomifun_agent_session::AgentEffectState::Rejected => {
            let error = record
                .bounded_observation
                .as_ref()
                .and_then(|observation| observation.get("error"))
                .and_then(Value::as_object);
            let code = error
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                .unwrap_or("CAPABILITY_UNAVAILABLE");
            let message = error
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or(
                    "Wave 2 effect was durably rejected without a replayable bounded error",
                );
            Err(Wave2HostPortError::new(code, message))
        }
        nomifun_agent_session::AgentEffectState::Pending => {
            Err(Wave2HostPortError::unavailable(
                "Wave 2 effect has a durable pending outcome; restart or cancellation cannot authorize automatic retry",
            ))
        }
        nomifun_agent_session::AgentEffectState::Unknown => {
            Err(Wave2HostPortError::unavailable(
                "Wave 2 effect has a durable unknown outcome; explicit reconciliation is required before another physical effect",
            ))
        }
        nomifun_agent_session::AgentEffectState::Cancelled => {
            Err(Wave2HostPortError::unavailable(
                "Wave 2 effect was cancelled after admission; automatic retry is disabled",
            ))
        }
    }
}

fn new_wave2_effect_request(
    effect_id: &str,
    context: &Wave2HostContext,
    binding: &TypedResourceBinding,
    input_digest: DigestHex,
    resource_key: String,
    strategy: nomifun_agent_session::EffectStrategy,
    causation_event_id: nomifun_agent_contracts::EventId,
) -> nomifun_agent_session::EffectEventRequest {
    nomifun_agent_session::EffectEventRequest {
        agent_session_id: context.agent_session_id.clone(),
        effect_id: effect_id.to_owned(),
        turn_id: context.turn_id.clone(),
        operation_id: context.operation_id.clone(),
        owner_domain: "workspace".to_owned(),
        capability_module: context.capability_id.clone(),
        action_id: context.action_id.clone(),
        resource_binding_id: Some(binding.binding_id.clone()),
        resource_key: Some(resource_key),
        input_digest,
        recorded_at: nomifun_common::now_ms(),
        event_id: nomifun_agent_contracts::EventId::from(format!(
            "{effect_id}:started"
        )),
        producer_id: nomifun_agent_contracts::EventProducerId::from("capability_host"),
        // Session events scope idempotency by producer globally. Namespace the
        // lifecycle key with the already session+turn+caller-key-derived Effect
        // identity so equal caller keys in different AgentSessions cannot
        // collide while terminal events retain the started event's exact key.
        idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(format!(
            "{effect_id}:lifecycle"
        )),
        correlation_id: nomifun_agent_contracts::CorrelationId::from(effect_id.to_owned()),
        strategy,
        causation_event_id: Some(causation_event_id),
        payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
            StrictJsonValue(json!({})),
        ),
    }
}

async fn begin_wave2_effect(
    store: &nomifun_agent_session::AgentSessionStore,
    context: &Wave2HostContext,
    binding: &TypedResourceBinding,
    input: &StrictJsonValue,
) -> Result<Wave2EffectAdmission, Wave2HostPortError> {
    begin_wave2_effect_with_strategy(
        store,
        context,
        binding,
        input,
        nomifun_agent_session::EffectStrategy::ManagedEffect,
    )
    .await
}

async fn begin_wave2_exclusive_effect(
    store: &nomifun_agent_session::AgentSessionStore,
    context: &Wave2HostContext,
    binding: &TypedResourceBinding,
    input: &StrictJsonValue,
    strategy: nomifun_agent_session::EffectStrategy,
) -> Result<Wave2EffectAdmission, Wave2HostPortError> {
    // The Agent Store's unique unsettled-resource index is the cross-process
    // admission fence. It is intentionally used for every workspace physical
    // effect, so the "exclusive" name documents call-site intent rather than
    // selecting a weaker in-memory policy.
    begin_wave2_effect_with_strategy(store, context, binding, input, strategy).await
}

async fn begin_wave2_effect_with_strategy(
    store: &nomifun_agent_session::AgentSessionStore,
    context: &Wave2HostContext,
    binding: &TypedResourceBinding,
    input: &StrictJsonValue,
    strategy: nomifun_agent_session::EffectStrategy,
) -> Result<Wave2EffectAdmission, Wave2HostPortError> {
    let effect_id = wave2_effect_id(context)?;
    let input_digest = wave2_effect_request_digest(context, binding, input)?;
    let resource_key = wave2_effect_resource_key(context, binding)?;
    let read = || async {
        store
            .read_effect(&context.agent_session_id, &effect_id)
            .await
            .map_err(|error| {
                Wave2HostPortError::unavailable(format!(
                    "canonical Agent Effect ledger could not be read: {error}"
                ))
            })
    };
    if let Some(record) = read().await? {
        return observe_wave2_effect(
            record,
            context,
            binding,
            &input_digest,
            strategy,
            &resource_key,
        );
    }

    let causation_event_id = store
        .effect_causation_event_id(
            &context.agent_session_id,
            &context.turn_id,
            &context.operation_id,
            &context.capability_id,
            &context.action_id,
        )
        .await
        .map_err(|error| {
            Wave2HostPortError::unavailable(format!(
                "canonical Tool causation could not be proven for the workspace effect: {error}"
            ))
        })?;
    let request = new_wave2_effect_request(
        &effect_id,
        context,
        binding,
        input_digest.clone(),
        resource_key.clone(),
        strategy,
        causation_event_id,
    );
    match store.record_effect_started(request.clone()).await {
        Ok(_) => Ok(Wave2EffectAdmission::Reserved(Wave2EffectReservation {
            store: store.clone(),
            request,
        })),
        Err(error) => {
            // A concurrent identical invocation may have won after our read.
            // Re-read exactly once and either replay/fence that durable fact or
            // surface the unsettled-resource/store rejection. Never retry the
            // physical owner merely because admission raced.
            if let Some(record) = read().await? {
                return observe_wave2_effect(
                    record,
                    context,
                    binding,
                    &input_digest,
                    strategy,
                    &resource_key,
                );
            }
            Err(Wave2HostPortError::unavailable(format!(
                "canonical Agent Effect admission failed or another workspace effect is unsettled: {error}"
            )))
        }
    }
}

fn bounded_terminal_payload(completion: Wave2EffectCompletion<'_>) -> StrictJsonValue {
    const MAX_TERMINAL_OBSERVATION_BYTES: usize = 48 * 1024;
    let mut value = match completion {
        Wave2EffectCompletion::Succeeded(output) => json!({"result": output.0.clone()}),
        Wave2EffectCompletion::Failed(error) => json!({
            "error": {
                "code": error.code.as_str(),
                "message": error.message.chars().take(2048).collect::<String>(),
            }
        }),
        Wave2EffectCompletion::Uncertain(error) => json!({
            "outcome": "unknown",
            "error": {
                "code": error.code.as_str(),
                "message": error.message.chars().take(2048).collect::<String>(),
            }
        }),
    };
    let encoded = canonical_json_bytes(&value).unwrap_or_default();
    if encoded.len() > MAX_TERMINAL_OBSERVATION_BYTES {
        let digest = digest_payload(&value)
            .map(|digest| digest.as_ref().to_owned())
            .unwrap_or_else(|_| "unavailable".to_owned());
        value = json!({
            "observation_truncated": true,
            "observation_digest": digest,
            "serialized_preview": String::from_utf8_lossy(
                &encoded[..encoded.len().min(1024)]
            ),
        });
    }
    StrictJsonValue(value)
}

async fn finish_wave2_effect(
    reservation: &Wave2EffectReservation,
    completion: Wave2EffectCompletion<'_>,
) -> Result<(), Wave2HostPortError> {
    let (state, suffix) = match completion {
        Wave2EffectCompletion::Succeeded(_) => (
            nomifun_agent_session::EffectTerminalState::Succeeded,
            "succeeded",
        ),
        Wave2EffectCompletion::Failed(_) => (
            nomifun_agent_session::EffectTerminalState::Failed,
            "failed",
        ),
        Wave2EffectCompletion::Uncertain(_) => (
            nomifun_agent_session::EffectTerminalState::Uncertain,
            "uncertain",
        ),
    };
    let mut request = reservation.request.clone();
    request.recorded_at = nomifun_common::now_ms();
    request.event_id = nomifun_agent_contracts::EventId::from(format!(
        "{}:{suffix}",
        request.effect_id
    ));
    request.producer_id = nomifun_agent_contracts::EventProducerId::from("owning_plugin");
    request.causation_event_id = Some(reservation.request.event_id.clone());
    request.payload = nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
        bounded_terminal_payload(completion),
    );
    reservation
        .store
        .record_effect_terminal(request, state)
        .await
        .map_err(|error| {
            Wave2HostPortError::unavailable(format!(
                "canonical Agent Effect terminal observation could not be committed: {error}"
            ))
        })?;
    Ok(())
}

impl Wave2HostPort for Wave2ApplicationHost {
    fn invoke<'a>(
        &'a self,
        request: Wave2HostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let capability_id = request.context.capability_id.as_ref().to_owned();
            if !nomifun_agent_domain_wave2::action_ids(&capability_id)
                .contains(&request.context.action_id)
            {
                return Err(Wave2HostPortError::new(
                    "ACTION_NOT_DECLARED",
                    format!(
                        "{capability_id} does not declare action {}",
                        request.context.action_id.as_ref()
                    ),
                ));
            }
            match request.operation {
                Wave2CapabilityOperation::WorkspaceExecution { input } => {
                    self.ensure_git_ready().map_err(|error| Wave2HostPortError::unavailable(error.to_string()))?;
                    self.invoke_workspace(&request.context, &capability_id, input)
                        .await
                }
                Wave2CapabilityOperation::Ssh { .. }
                | Wave2CapabilityOperation::Browser { .. }
                | Wave2CapabilityOperation::ComputerA11y { .. } => {
                    Err(unavailable(&capability_id))
                }
            }
        })
    }
}

impl Wave2ApplicationHost {
    async fn invoke_workspace(
        &self,
        context: &Wave2HostContext,
        capability_id: &str,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let action_id = context.action_id.as_ref();
        let scope = self.workspace_scope(context)?;
        match action_id {
            "workspace.files/read" => {
                let result = super::workspace_file_read::read(&self.files, &scope, input)
                    .await
                    .map_err(|error| operation_error(capability_id, error))?
                    .ok_or_else(|| {
                        Wave2HostPortError::new(
                            "RESOURCE_NOT_FOUND",
                            "workspace file was not found",
                        )
                    })?;
                Ok(result)
            }
            "workspace.files/write" => {
                let params: WriteParams = decode(input)?;
                if params.content.len() > 8 * 1024 * 1024 {
                    return Err(Wave2HostPortError::invalid_payload(
                        "workspace.files/write content exceeds the 8 MiB UTF-8 byte limit",
                    ));
                }
                let binding = workspace_typed_binding(context)?;
                let effect_input = StrictJsonValue(serde_json::to_value(&params).map_err(
                    |error| {
                        Wave2HostPortError::new(
                            "INVALID_PAYLOAD",
                            format!("{capability_id} input could not be encoded: {error}"),
                        )
                    },
                )?);
                let _write_guard = self.workspace_write_lock.lock().await;
                match begin_wave2_effect(self.effect_store()?, context, binding, &effect_input).await? {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        let result = self
                            .files
                            .write_file_for_agent_session(
                                &scope,
                                &params.path,
                                params.content.as_bytes(),
                            )
                            .await;
                        match result {
                            Ok(created) => {
                                let output = StrictJsonValue(json!({
                                    "path": params.path,
                                    "written": true,
                                    "created": created
                                }));
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await?;
                                Ok(output)
                            }
                            Err(error) => {
                                let owner_error = operation_error(capability_id, error);
                                let _ = finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&owner_error),
                                )
                                .await;
                                Err(owner_error)
                            }
                        }
                    }
                }
            }
            "workspace.files/patch" => {
                let request: AgentSessionPatchRequest = decode(input)?;
                let binding = workspace_typed_binding(context)?;
                let effect_input = StrictJsonValue(
                    serde_json::to_value(&request).map_err(|error| {
                        Wave2HostPortError::new(
                            "INVALID_PAYLOAD",
                            format!("{capability_id} input could not be encoded: {error}"),
                        )
                    })?,
                );
                // A lost settlement must also fence a new idempotency key;
                // otherwise re-submitting could duplicate a published patch.
                let _write_guard = self.workspace_write_lock.lock().await;
                match begin_wave2_exclusive_effect(
                    self.effect_store()?,
                    context,
                    binding,
                    &effect_input,
                    nomifun_agent_session::EffectStrategy::ManagedEffect,
                )
                .await?
                {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        let result = self.files.apply_patch_with_observation_for_agent_session(&scope, request).await;
                        match result {
                            Ok(result) => {
                                let output = StrictJsonValue(
                                    serde_json::to_value(result).map_err(|error| {
                                        Wave2HostPortError::new(
                                            "CAPABILITY_UNAVAILABLE",
                                            format!(
                                                "{capability_id} result could not be encoded: {error}"
                                            ),
                                        )
                                    })?,
                                );
                                if finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await.is_err() {
                                    return Err(Wave2HostPortError::unavailable(
                                        "workspace.files/patch published all request files, but journal settlement is unconfirmed. No automatic retry; re-read every target. Publication is not a current-state lock or task success."
                                    ));
                                }
                                Ok(output)
                            }
                            Err(failure) => {
                                let cause = operation_error(capability_id, failure.error);
                                let owner_error = patch_failure_error(&cause.code, &cause.message, &failure.observation, true);
                                if finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&owner_error),
                                )
                                .await.is_err() {
                                    return Err(patch_failure_error(&cause.code, &cause.message, &failure.observation, false));
                                }
                                Err(owner_error)
                            }
                        }
                    }
                }
            }
            "workspace.files/delete" => {
                let params: PathParams = decode(input)?;
                let binding = workspace_typed_binding(context)?;
                let effect_input = StrictJsonValue(serde_json::to_value(&params).map_err(
                    |error| {
                        Wave2HostPortError::new(
                            "INVALID_PAYLOAD",
                            format!("{capability_id} input could not be encoded: {error}"),
                        )
                    },
                )?);
                let _write_guard = self.workspace_write_lock.lock().await;
                match begin_wave2_effect(self.effect_store()?, context, binding, &effect_input).await? {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        let result = self
                            .files
                            .remove_entry_for_agent_session(&scope, &params.path)
                            .await;
                        match result {
                            Ok(()) => {
                                let output = StrictJsonValue(json!({
                                    "path": params.path,
                                    "deleted": true
                                }));
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await?;
                                Ok(output)
                            }
                            Err(error) => {
                                let owner_error = operation_error(capability_id, error);
                                let _ = finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&owner_error),
                                )
                                .await;
                                Err(owner_error)
                            }
                        }
                    }
                }
            }
            "workspace.files/search" => {
                let params: nomifun_file::AgentTextSearchRequest = decode(input)?;
                let result = self
                    .files
                    .search_text_for_agent_session(&scope, params)
                    .await
                    .map_err(|error| operation_error(capability_id, error))?;
                Ok(StrictJsonValue(serde_json::to_value(result)
                    .map_err(|error| operation_error(capability_id, AppError::Internal(error.to_string())))?))
            }
            "workspace.artifacts/read" => {
                scope
                    .require_operation(WORKSPACE_READ_OPERATION)
                    .map_err(|error| operation_error(action_id, error))?;
                let params: ArtifactReadParams = decode(input)?;
                let store = self.artifacts.as_ref().map_err(|error| {
                    Wave2HostPortError::unavailable(format!("artifact owner unavailable: {error}"))
                })?.clone();
                let result = tokio::task::spawn_blocking(move || {
                    store.read(
                        &params.artifact_id,
                        params.offset.unwrap_or(0),
                        params.limit.unwrap_or(16 * 1024),
                    )
                })
                .await
                .map_err(|error| {
                    Wave2HostPortError::unavailable(format!(
                        "workspace artifact read owner stopped unexpectedly: {error}"
                    ))
                })?
                .map_err(|error| operation_error(action_id, error))?;
                Ok(StrictJsonValue(serde_json::to_value(result).map_err(
                    |error| {
                        Wave2HostPortError::unavailable(format!(
                            "workspace artifact read receipt could not be encoded: {error}"
                        ))
                    },
                )?))
            }
            "workspace.artifacts/publish" => {
                scope
                    .require_operation(WORKSPACE_WRITE_OPERATION)
                    .map_err(|error| operation_error(action_id, error))?;
                let params: ArtifactPublishParams = decode(input)?;
                let binding = workspace_typed_binding(context)?;
                let store = self.artifacts.as_ref().map_err(|error| {
                    Wave2HostPortError::unavailable(format!("artifact owner unavailable: {error}"))
                })?.clone();
                let effect_input = StrictJsonValue(serde_json::to_value(&params).map_err(
                    |error| {
                        Wave2HostPortError::invalid_payload(format!(
                            "workspace artifact publication input could not be encoded: {error}"
                        ))
                    },
                )?);
                let _write_guard = self.workspace_write_lock.lock().await;
                match begin_wave2_exclusive_effect(
                    self.effect_store()?,
                    context,
                    binding,
                    &effect_input,
                    nomifun_agent_session::EffectStrategy::ManagedEffect,
                )
                .await?
                {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        let path = params.path.clone();
                        let expected_sha256 = params.expected_sha256.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            store.publish(&path, expected_sha256.as_deref())
                        })
                        .await
                        .map_err(|error| {
                            Wave2HostPortError::unavailable(format!(
                                "workspace artifact publication owner stopped unexpectedly: {error}"
                            ))
                        })?
                        .map_err(|error| operation_error(action_id, error));
                        match result {
                            Ok(result) => {
                                let output = StrictJsonValue(
                                    serde_json::to_value(result).map_err(|error| {
                                        Wave2HostPortError::unavailable(format!(
                                            "workspace artifact publication receipt could not be encoded: {error}"
                                        ))
                                    })?,
                                );
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await?;
                                Ok(output)
                            }
                            Err(error) if error.code == "EFFECT_OUTCOME_UNKNOWN" => {
                                // The hard-link may exist without a durable
                                // directory observation. Keep the Agent Effect
                                // pending so neither restart nor a new caller
                                // key can publish again automatically.
                                Err(error)
                            }
                            Err(error) => {
                                let _ = finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&error),
                                )
                                .await;
                                Err(error)
                            }
                        }
                    }
                }
            }
            "workspace.vcs/status" => {
                scope
                    .require_operation(WORKSPACE_READ_OPERATION)
                    .map_err(|error| operation_error(capability_id, error))?;
                self.invoke_vcs_status(&scope, capability_id).await
            }
            "workspace.vcs/diff" => {
                scope
                    .require_operation(WORKSPACE_READ_OPERATION)
                    .map_err(|error| operation_error(capability_id, error))?;
                let params: VcsPathParams = decode(input)?;
                self.invoke_vcs_diff(&scope, capability_id, params.path.as_deref())
                    .await
            }
            "workspace.vcs/stage" => {
                scope
                    .require_operation(WORKSPACE_WRITE_OPERATION)
                    .map_err(|error| operation_error(capability_id, error))?;
                let params: PathParams = decode(input)?;
                let binding = workspace_typed_binding(context)?;
                let effect_input = StrictJsonValue(serde_json::to_value(&params).map_err(
                    |error| {
                        Wave2HostPortError::new(
                            "INVALID_PAYLOAD",
                            format!("{capability_id} input could not be encoded: {error}"),
                        )
                    },
                )?);
                let _effect_guard = self.workspace_write_lock.lock().await;
                let _git_guard = self.git_mutation_lock.lock().await;
                self.pause_after_git_admission_for_test(action_id).await;
                match begin_wave2_effect(self.effect_store()?, context, binding, &effect_input).await? {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        match self
                            .invoke_vcs_stage(&scope, capability_id, &params.path)
                            .await
                        {
                            Ok(output) => {
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await?;
                                Ok(output)
                            }
                            Err(owner_error)
                                if owner_error.code == "EFFECT_OUTCOME_UNKNOWN" =>
                            {
                                // The index may already contain the staged
                                // mutation. Preserve the durable pending fence
                                // until an explicit index observation settles it.
                                Err(owner_error)
                            }
                            Err(owner_error) => {
                                let _ = finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&owner_error),
                                )
                                .await;
                                Err(owner_error)
                            }
                        }
                    }
                }
            }
            "workspace.vcs/commit" => {
                scope
                    .require_operation(WORKSPACE_WRITE_OPERATION)
                    .map_err(|error| operation_error(capability_id, error))?;
                let params: VcsCommitParams = decode(input)?;
                let binding = workspace_typed_binding(context)?;
                let effect_input = StrictJsonValue(
                    serde_json::to_value(&params).map_err(|error| {
                        Wave2HostPortError::new(
                            "INVALID_PAYLOAD",
                            format!("{capability_id} input could not be encoded: {error}"),
                        )
                    })?,
                );
                let _effect_guard = self.workspace_write_lock.lock().await;
                let _git_guard = self.git_mutation_lock.lock().await;
                self.pause_after_git_admission_for_test(action_id).await;
                match begin_wave2_effect(self.effect_store()?, context, binding, &effect_input).await? {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        match self
                            .invoke_vcs_commit(&scope, capability_id, &params.message)
                            .await
                        {
                            Ok(output) => {
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await?;
                                Ok(output)
                            }
                            Err(owner_error) => {
                                let _ = finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&owner_error),
                                )
                                .await;
                                Err(owner_error)
                            }
                        }
                    }
                }
            }
            "workspace.vcs/push" => {
                scope
                    .require_operation(WORKSPACE_WRITE_OPERATION)
                    .map_err(|error| operation_error(capability_id, error))?;
                let params: VcsPushActionInput = decode(input)?;
                let binding = workspace_typed_binding(context)?.clone();
                let owner = self
                    .vcs_push_owner
                    .get_or_init(|| VcsPushOwner::new(&self.configured_workspace_root))
                    .as_ref()
                    .map_err(|error| vcs_push_error(error.clone()))?;
                let effect_input = StrictJsonValue(
                    serde_json::to_value(&params).map_err(|error| {
                        Wave2HostPortError::new(
                            "INVALID_PAYLOAD",
                            format!("{capability_id} input could not be encoded: {error}"),
                        )
                    })?,
                );
                let _effect_guard = self.workspace_write_lock.lock().await;
                let _git_guard = self.git_mutation_lock.lock().await;
                self.pause_after_git_admission_for_test(action_id).await;
                let admission = match begin_wave2_exclusive_effect(
                    self.effect_store()?,
                    context,
                    &binding,
                    &effect_input,
                    nomifun_agent_session::EffectStrategy::ExternalUncertainEffect,
                )
                .await
                {
                    Ok(admission) => admission,
                    Err(error) => return Err(error),
                };
                match admission {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        let mut settlement = owner.settlement_guard();
                        let request = VcsPushRequest::from_action_input(
                            context.principal.principal_id.clone(),
                            scope.workspace_root().to_path_buf(),
                            binding,
                            params,
                        );
                        match owner.push(request).await {
                            Ok(receipt) => {
                                let output = StrictJsonValue(
                                    serde_json::to_value(receipt).map_err(|error| {
                                        Wave2HostPortError::new(
                                            "CAPABILITY_UNAVAILABLE",
                                            format!(
                                                "{capability_id} result could not be encoded: {error}"
                                            ),
                                        )
                                    })?,
                                );
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await?;
                                settlement.confirm();
                                Ok(output)
                            }
                            Err(error)
                                if error.disposition
                                    == VcsPushEffectDisposition::OutcomeUnknown =>
                            {
                                let owner_error = vcs_push_error(error);
                                // Persist the owner's explicit uncertainty. The Agent
                                // Store retains the unsettled resource fence across
                                // process restarts; the local settlement guard remains
                                // unconfirmed as a second current-process fence.
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Uncertain(&owner_error),
                                )
                                .await?;
                                Err(owner_error)
                            }
                            Err(error) => {
                                let owner_error = vcs_push_error(error);
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&owner_error),
                                )
                                .await?;
                                settlement.confirm();
                                Err(owner_error)
                            }
                        }
                    }
                }
            }
            _ => Err(unavailable(capability_id)),
        }
    }

    async fn invoke_vcs_status(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let workspace = scope.workspace_root().to_path_buf();
        let capability_id = capability_id.to_owned();
        let worker_capability_id = capability_id.clone();
        let status = tokio::task::spawn_blocking(move || {
            let (repository, workspace_prefix) = scoped_repository(&workspace)?;
            let mut options = git2::StatusOptions::new();
            options
                .include_untracked(true)
                .recurse_untracked_dirs(true)
                .include_ignored(false);
            let statuses = repository.statuses(Some(&mut options)).map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!(
                        "{worker_capability_id} could not read Git status: {error}"
                    ),
                )
            })?;
            let mut entries = Vec::new();
            for entry in statuses.iter() {
                let Some(path) = entry.path() else {
                    continue;
                };
                let Some(path) = path_relative_to_workspace(path, &workspace_prefix) else {
                    continue;
                };
                if is_workspace_owner_relative(&path) {
                    continue;
                }
                entries.push(json!({
                    "path": path,
                    "status": git_status_name(entry.status())
                }));
            }
            Ok::<_, Wave2HostPortError>(StrictJsonValue(json!({
                "repository": "workspace",
                "entries": entries
            })))
        })
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("{capability_id} status worker failed: {error}"),
            )
        })??;
        Ok(status)
    }

    async fn invoke_vcs_diff(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
        path: Option<&str>,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let workspace = scope.workspace_root().to_path_buf();
        let capability_id = capability_id.to_owned();
        let worker_capability_id = capability_id.clone();
        let path = match path.filter(|path| !path.is_empty()) {
            Some(path) => {
                let resolved = scope
                    .resolve_relative_path(path)
                    .map_err(|error| operation_error(&capability_id, error))?;
                let relative = resolved.strip_prefix(scope.workspace_root()).map_err(|_| {
                    Wave2HostPortError::new(
                        "INVALID_PAYLOAD",
                        "workspace.vcs/diff path is outside the workspace",
                    )
                })?;
                Some(git_path_to_string(relative)?)
            }
            None => None,
        };
        let repository_prefix = scoped_repository(&workspace)
            .map(|(_, prefix)| prefix)
            .map_err(|error| error)?;
        let pathspec = path
            .as_deref()
            .map(|relative| join_repo_path(&repository_prefix, relative));
        tokio::task::spawn_blocking(move || {
            let (repository, actual_prefix) = scoped_repository(&workspace)?;
            debug_assert_eq!(actual_prefix, repository_prefix);
            let scope_pathspec = pathspec
                .as_deref()
                .or_else(|| (!repository_prefix.is_empty()).then_some(repository_prefix.as_str()));
            let head_tree = repository
                .head()
                .ok()
                .and_then(|head| head.peel_to_tree().ok());
            let mut staged_options = git2::DiffOptions::new();
            if let Some(pathspec) = scope_pathspec {
                staged_options.pathspec(pathspec);
            }
            let staged = repository
                .diff_tree_to_index(
                    head_tree.as_ref(),
                    None,
                    Some(&mut staged_options),
                )
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!(
                            "{worker_capability_id} could not read staged Git diff: {error}"
                        ),
                    )
                })?;
            let mut unstaged_options = git2::DiffOptions::new();
            if let Some(pathspec) = scope_pathspec {
                unstaged_options.pathspec(pathspec);
            }
            let unstaged = repository
                .diff_index_to_workdir(None, Some(&mut unstaged_options))
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!(
                            "{worker_capability_id} could not read unstaged Git diff: {error}"
                        ),
                    )
                })?;
            let mut staged_patch = String::new();
            let mut truncated = false;
            append_diff_patch(
                &staged,
                &mut staged_patch,
                &mut truncated,
                &worker_capability_id,
                &repository_prefix,
            )?;
            let mut unstaged_patch = String::new();
            append_diff_patch(
                &unstaged,
                &mut unstaged_patch,
                &mut truncated,
                &worker_capability_id,
                &repository_prefix,
            )?;
            let patch = format!("{staged_patch}{unstaged_patch}");
            Ok::<_, Wave2HostPortError>(StrictJsonValue(json!({
                "path": path,
                "patch": patch,
                "staged_patch": staged_patch,
                "unstaged_patch": unstaged_patch,
                "truncated": truncated
            })))
        })
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("{capability_id} diff worker failed: {error}"),
            )
        })?
    }

    async fn invoke_vcs_stage(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
        path: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        if path.is_empty() {
            return Err(Wave2HostPortError::invalid_payload(
                "workspace.vcs/stage path must not be empty",
            ));
        }
        let resolved = scope
            .resolve_relative_path(path)
            .map_err(|error| operation_error(capability_id, error))?;
        let relative = resolved
            .strip_prefix(scope.workspace_root())
            .map(|relative| relative.to_path_buf())
            .map_err(|_| {
                operation_error(
                    capability_id,
                    AppError::BadRequest(
                        "workspace.vcs/stage path is outside the workspace".to_owned(),
                    ),
                )
            })?;
        let workspace = scope.workspace_root().to_path_buf();
        let path_label = path.to_owned();
        let owner = self.vcs_stage_owner.as_ref().map_err(|error| {
            Wave2HostPortError::unavailable(format!("VCS stage owner is unavailable: {error}"))
        })?.clone();
        tokio::task::spawn_blocking(move || {
            let (repository, workspace_prefix) = scoped_repository(&workspace)?;
            let staged_paths = owner.stage_and_write(&repository, &workspace_prefix, &relative)
                .map_err(|error| operation_error("workspace.vcs/stage", error))?;
            Ok::<_, Wave2HostPortError>(StrictJsonValue(json!({
                "path": path_label,
                "paths": staged_paths,
                "staged": true
            })))
        })
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("{capability_id} stage worker failed: {error}"),
            )
        })?
    }

    async fn invoke_vcs_commit(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
        message: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let message = message.trim();
        if message.is_empty() {
            return Err(Wave2HostPortError::invalid_payload(
                "workspace.vcs/commit message must not be empty",
            ));
        }
        if message.chars().count() > 512 {
            return Err(Wave2HostPortError::invalid_payload(
                "workspace.vcs/commit message must not exceed 512 characters",
            ));
        }
        let workspace = scope.workspace_root().to_path_buf();
        let capability_id = capability_id.to_owned();
        let worker_capability_id = capability_id.clone();
        let message = message.to_owned();
        tokio::task::spawn_blocking(move || {
            let (repository, workspace_prefix) = scoped_repository(&workspace)?;
            let mut index = repository.index().map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("workspace.vcs/commit could not open the Git index: {error}"),
                )
            })?;

            let parent = match repository.head() {
                Ok(head) if head.target().is_none() => {
                    if repository.is_empty().map_err(|error| {
                        Wave2HostPortError::new(
                            "CAPABILITY_UNAVAILABLE",
                            format!("workspace.vcs/commit could not inspect repository emptiness: {error}"),
                        )
                    })? {
                        None
                    } else {
                        return Err(Wave2HostPortError::new(
                            "CAPABILITY_UNAVAILABLE",
                            "workspace.vcs/commit found an unborn HEAD in a non-empty repository",
                        ));
                    }
                }
                Ok(head) => Some(head.peel_to_commit().map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!("workspace.vcs/commit could not peel the repository HEAD: {error}"),
                    )
                })?),
                Err(error)
                    if matches!(
                        error.code(),
                        git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
                    ) && repository.is_empty().map_err(|inspect_error| {
                        Wave2HostPortError::new(
                            "CAPABILITY_UNAVAILABLE",
                            format!(
                                "workspace.vcs/commit could not inspect repository emptiness: {inspect_error}"
                            ),
                        )
                    })? =>
                {
                    None
                }
                Err(error) => {
                    return Err(Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!("workspace.vcs/commit could not read the repository HEAD: {error}"),
                    ));
                }
            };
            let parent_tree = parent.as_ref().map(|commit| commit.tree()).transpose().map_err(
                |error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!("workspace.vcs/commit could not load the parent tree: {error}"),
                    )
                },
            )?;
            let staged_diff = repository
                .diff_tree_to_index(parent_tree.as_ref(), Some(&index), None)
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!("workspace.vcs/commit could not inspect staged changes: {error}"),
                    )
                })?;
            let mut scoped_paths = Vec::new();
            for delta in staged_diff.deltas() {
                let old_path = delta.old_file().path().map(git_path_to_string).transpose()?;
                let new_path = delta.new_file().path().map(git_path_to_string).transpose()?;
                let paths = [old_path.as_deref(), new_path.as_deref()];
                if paths.iter().all(Option::is_none) {
                    return Err(Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        "workspace.vcs/commit encountered a staged change without a path",
                    ));
                }
                for path in paths.into_iter().flatten() {
                    let Some(relative) = path_relative_to_workspace(path, &workspace_prefix) else {
                        return Err(Wave2HostPortError::new(
                            "PRESET_RESOURCE_NOT_BOUND",
                            "workspace.vcs/commit refuses to commit staged paths outside the bound workspace",
                        ));
                    };
                    if is_workspace_owner_relative(&relative) {
                        return Err(Wave2HostPortError::new(
                            "PRESET_RESOURCE_NOT_BOUND",
                            "workspace.vcs/commit refuses to commit the workspace owner directory",
                        ));
                    }
                    scoped_paths.push(relative);
                }
            }
            if scoped_paths.is_empty() {
                return Err(Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    "workspace.vcs/commit has no staged changes in the bound workspace",
                ));
            }
            scoped_paths.sort();
            scoped_paths.dedup();

            let tree_id = index.write_tree().map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("workspace.vcs/commit could not write the Git tree: {error}"),
                )
            })?;
            let tree = repository.find_tree(tree_id).map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("workspace.vcs/commit could not load the Git tree: {error}"),
                )
            })?;
            let signature = repository.signature().map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!(
                        "workspace.vcs/commit requires configured Git user.name/user.email: {error}"
                    ),
                )
            })?;
            let parents = parent.iter().collect::<Vec<_>>();
            let commit_id = repository
                .commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    &message,
                    &tree,
                    &parents,
                )
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!(
                            "{worker_capability_id} could not create the commit: {error}"
                        ),
                    )
                })?;
            Ok::<_, Wave2HostPortError>(StrictJsonValue(json!({
                "committed": true,
                "commit_id": commit_id.to_string(),
                "message": message,
                "paths": scoped_paths
            })))
        })
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("{capability_id} commit worker failed: {error}"),
            )
        })?
    }
}

fn append_diff_patch(
    diff: &git2::Diff<'_>,
    patch: &mut String,
    truncated: &mut bool,
    capability_id: &str,
    workspace_prefix: &str,
) -> Result<(), Wave2HostPortError> {
    if *truncated {
        return Ok(());
    }
    diff.print(git2::DiffFormat::Patch, |delta, _hunk, line| {
        let owner_delta = [delta.old_file().path(), delta.new_file().path()]
            .into_iter()
            .flatten()
            .filter_map(|path| path.to_str())
            .filter_map(|path| path_relative_to_workspace(path, workspace_prefix))
            .any(|path| is_workspace_owner_relative(&path));
        if owner_delta {
            return true;
        }
        if line.origin() != '\0' {
            patch.push(line.origin());
        }
        patch.push_str(&String::from_utf8_lossy(line.content()));
        if patch.len() > MAX_DIFF_BYTES {
            let mut end = MAX_DIFF_BYTES;
            while end > 0 && !patch.is_char_boundary(end) {
                end -= 1;
            }
            patch.truncate(end);
            *truncated = true;
            false
        } else {
            true
        }
    })
    .map_err(|error| {
        Wave2HostPortError::new(
            "CAPABILITY_UNAVAILABLE",
            format!("{capability_id} could not render Git diff: {error}"),
        )
    })
}

fn repo_path_component_matches(left: &str, right: &str) -> bool {
    #[cfg(windows)]
    {
        left.eq_ignore_ascii_case(right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn is_workspace_owner_relative(path: &str) -> bool {
    path.split('/')
        .next()
        .is_some_and(|component| {
            repo_path_component_matches(
                component,
                nomifun_file::WORKSPACE_OWNER_DIRECTORY,
            )
        })
}

fn path_relative_to_workspace(path: &str, prefix: &str) -> Option<String> {
    // Git index paths are canonical forward-slash paths on every platform.
    // Treating a literal backslash as a separator on Unix can turn a sibling
    // filename such as `nested\\outside.txt` into an apparently in-scope path.
    if path.contains('\\') {
        return None;
    }
    let path = path.replace('\\', "/");
    let prefix = prefix.replace('\\', "/");
    if prefix.is_empty() {
        return Some(path);
    }
    let mut path_components = path.split('/');
    for expected in prefix.split('/') {
        let actual = path_components.next()?;
        if !repo_path_component_matches(actual, expected) {
            return None;
        }
    }
    Some(path_components.collect::<Vec<_>>().join("/"))
}

/// Open the repository containing the bound workspace and return the
/// repository-relative prefix of that workspace. Git status/index APIs operate
/// from the repository root, so every result and mutation must be projected
/// back into the exact typed workspace scope.
fn scoped_repository(
    workspace: &Path,
) -> Result<(git2::Repository, String), Wave2HostPortError> {
    let repository = git2::Repository::discover(workspace).map_err(|error| {
        Wave2HostPortError::new(
            "RESOURCE_NOT_FOUND",
            format!("workspace is not a Git repository: {error}"),
        )
    })?;
    let repository_root = repository.workdir().ok_or_else(|| {
        Wave2HostPortError::new(
            "RESOURCE_NOT_FOUND",
            "Git repository has no working directory",
        )
    })?;
    let repository_root = std::fs::canonicalize(repository_root).map_err(|error| {
        Wave2HostPortError::new(
            "RESOURCE_NOT_FOUND",
            format!("Git repository working directory is unavailable: {error}"),
        )
    })?;
    let workspace = std::fs::canonicalize(workspace).map_err(|error| {
        Wave2HostPortError::new(
            "RESOURCE_NOT_FOUND",
            format!("workspace is unavailable: {error}"),
        )
    })?;
    let workspace_relative = workspace.strip_prefix(&repository_root).map_err(|_| {
        Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "workspace is outside the discovered Git repository",
        )
    })?;
    let prefix = git_path_to_string(workspace_relative)?
        .trim_matches('/')
        .to_owned();
    Ok((repository, prefix))
}

fn git_path_to_string(path: &Path) -> Result<String, Wave2HostPortError> {
    path.to_str()
        .map(|path| normalize_git_path(path.to_owned()))
        .ok_or_else(|| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                "Git path is not valid UTF-8 and cannot be projected safely",
            )
        })
}

fn normalize_git_path(path: String) -> String {
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path
    }
}

fn join_repo_path(prefix: &str, relative: &str) -> String {
    let relative = relative.trim_matches('/');
    if prefix.is_empty() {
        relative.to_owned()
    } else if relative.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}/{relative}")
    }
}

fn git_status_name(status: git2::Status) -> Vec<&'static str> {
    let mut names = Vec::new();
    for (flag, name) in [
        (git2::Status::INDEX_NEW, "index_new"),
        (git2::Status::INDEX_MODIFIED, "index_modified"),
        (git2::Status::INDEX_DELETED, "index_deleted"),
        (git2::Status::INDEX_RENAMED, "index_renamed"),
        (git2::Status::INDEX_TYPECHANGE, "index_typechange"),
        (git2::Status::WT_NEW, "worktree_new"),
        (git2::Status::WT_MODIFIED, "worktree_modified"),
        (git2::Status::WT_DELETED, "worktree_deleted"),
        (git2::Status::WT_RENAMED, "worktree_renamed"),
        (git2::Status::WT_TYPECHANGE, "worktree_typechange"),
        (git2::Status::CONFLICTED, "conflicted"),
        (git2::Status::IGNORED, "ignored"),
    ] {
        if status.contains(flag) {
            names.push(name);
        }
    }
    names
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WriteParams {
    path: String,
    content: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VcsPathParams {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VcsCommitParams {
    message: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArtifactPublishParams {
    path: String,
    #[serde(default)]
    expected_sha256: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactReadParams {
    artifact_id: String,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    limit: Option<usize>,
}

fn decode<T: for<'de> Deserialize<'de>>(
    input: StrictJsonValue,
) -> Result<T, Wave2HostPortError> {
    serde_json::from_value(input.0).map_err(|error| {
        Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            format!("Wave 2 filesystem input is invalid: {error}"),
        )
    })
}

impl Wave2ApplicationHost {
    fn workspace_scope(
        &self,
        context: &Wave2HostContext,
    ) -> Result<AgentSessionWorkspaceBinding, Wave2HostPortError> {
        if context
            .resource_bindings
            .iter()
            .any(|binding| binding.resource_kind.as_ref() != WORKSPACE_RESOURCE_KIND)
        {
            return Err(Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "Wave 2 filesystem action received a non-workspace resource binding",
            ));
        }
        let mut bindings = context
            .resource_bindings
            .iter()
            .filter(|binding| binding.resource_kind.as_ref() == WORKSPACE_RESOURCE_KIND);
        let binding = bindings.next().ok_or_else(|| {
            Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "Wave 2 filesystem action requires one workspace resource binding",
            )
        })?;
        if bindings.next().is_some() {
            return Err(Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "Wave 2 filesystem action received more than one workspace resource binding",
            ));
        }
        if binding.owner_id != context.principal.principal_id {
            return Err(Wave2HostPortError::new(
                "RESOURCE_OWNER_MISMATCH",
                format!(
                    "workspace binding {} belongs to a different principal",
                    binding.binding_id.as_ref()
                ),
            ));
        }
        let requested_root = binding
            .typed_parameters
            .get(WORKSPACE_ROOT_PARAMETER)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                Wave2HostPortError::new(
                    "PRESET_RESOURCE_NOT_BOUND",
                    format!(
                        "workspace binding {} has no host-resolved {} parameter",
                        binding.binding_id.as_ref(),
                        WORKSPACE_ROOT_PARAMETER
                    ),
                )
            })?;
        let workspace_root = resolve_allowed_workspace_root(
            &self.configured_workspace_root,
            requested_root,
        )?;
        AgentSessionWorkspaceBinding::new(
            context.agent_session_id.as_ref(),
            binding_dto(binding),
            workspace_root,
        )
        .map_err(|error| operation_error(context.capability_id.as_ref(), error))
    }
}

fn resolve_allowed_workspace_root(
    configured_root: &Path,
    requested_root: &str,
) -> Result<PathBuf, Wave2HostPortError> {
    let configured_root = std::fs::canonicalize(configured_root).map_err(|error| {
        Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!(
                "configured host workspace root '{}' is unavailable: {error}",
                configured_root.display()
            ),
        )
    })?;
    let requested_root = PathBuf::from(requested_root.trim());
    if !requested_root.is_absolute() {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "workspace_root must be an absolute host-resolved path",
        ));
    }
    let requested_root = std::fs::canonicalize(&requested_root).map_err(|error| {
        Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!("workspace_root is unavailable: {error}"),
        )
    })?;
    if !requested_root.starts_with(&configured_root) {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!(
                "workspace_root '{}' is outside the configured host workspace root '{}'",
                requested_root.display(),
                configured_root.display()
            ),
        ));
    }
    Ok(requested_root)
}

fn binding_dto(binding: &TypedResourceBinding) -> TypedResourceBindingDto {
    TypedResourceBindingDto {
        binding_id: binding.binding_id.as_ref().to_owned(),
        resource_kind: binding.resource_kind.as_ref().to_owned(),
        resource_id: binding.resource_id.as_ref().to_owned(),
        owner_id: binding.owner_id.clone(),
        operations: binding.operations.clone(),
        connection_config_ref: binding
            .connection_config_ref
            .as_ref()
            .map(|reference| reference.as_ref().to_owned()),
        typed_parameters: binding.typed_parameters.clone(),
    }
}

fn workspace_typed_binding<'a>(
    context: &'a Wave2HostContext,
) -> Result<&'a TypedResourceBinding, Wave2HostPortError> {
    let mut bindings = context
        .resource_bindings
        .iter()
        .filter(|binding| binding.resource_kind.as_ref() == WORKSPACE_RESOURCE_KIND);
    let binding = bindings.next().ok_or_else(|| {
        Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "Wave 2 effect action requires one workspace resource binding",
        )
    })?;
    if bindings.next().is_some() {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "Wave 2 effect action received more than one workspace resource binding",
        ));
    }
    Ok(binding)
}

fn operation_error(capability_id: &str, error: AppError) -> Wave2HostPortError {
    let code = match &error {
        error
            if nomifun_file::artifact_publication_outcome_unknown(error)
                || nomifun_file::vcs_stage_outcome_unknown(error) =>
        {
            "EFFECT_OUTCOME_UNKNOWN"
        }
        AppError::BadRequest(_) => "INVALID_PAYLOAD",
        AppError::Forbidden(_) => "PRESET_RESOURCE_NOT_BOUND",
        AppError::NotFound(_) => "RESOURCE_NOT_FOUND",
        AppError::Conflict(_) | AppError::RevisionConflict(_) => "CAPABILITY_UNAVAILABLE",
        _ => "CAPABILITY_UNAVAILABLE_ON_PLATFORM",
    };
    Wave2HostPortError::new(code, format!("{capability_id} failed: {error}"))
}

fn vcs_push_error(error: VcsPushError) -> Wave2HostPortError {
    Wave2HostPortError::new(error.code, error.message)
}

fn unavailable(capability_id: &str) -> Wave2HostPortError {
    Wave2HostPortError::unavailable(format!(
        "no canonical application owner is wired for {capability_id}"
    ))
}

struct NullUserEvents;

impl UserEventSink for NullUserEvents {
    fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<Value>) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Mutex, OnceLock};
    use std::task::{Context, Poll, Waker};

    use nomifun_agent_contracts::{
        ActionId, AgentBindingValue, AgentPresetId, AgentPresetRevision,
        AgentPresetRevisionPayload, AgentSessionId, AgentSessionLiveRecord,
        AgentSessionMetadata, CapabilityId, CapabilityRef, CapabilitySelection, CorrelationId,
        DigestHex, EventId, EventProducerId, IdempotencyKey, OperationId, PresetRevisionRef,
        PrincipalRef, ResolvedSnapshotRef, ResourceBindingId, ResourceId, ResourceKind,
        RuntimeProfileKind, RuntimeTarget, ScopeKey, SemanticSessionEventDraft,
        SessionEventAppend, SessionEventKind, SessionEventPayloadRef, StrictJsonValue,
        TypedResourceBinding, UserId, VersionString,
    };
    use nomifun_agent_domain_wave2::Wave2StateHandle;
    use nomifun_agent_kernel::{
        AgentPresetCompiler, CapabilityInvocationRequest, CompileRequest,
        CompilerEnvironment, InMemoryPluginStatePersistence, KernelRegistry,
        MaterializationPolicy, SessionCapabilityState,
    };

    struct StateCaptureHostPort {
        captured: Arc<Mutex<Option<Wave2StateHandle>>>,
    }

    impl Wave2HostPort for StateCaptureHostPort {
        fn invoke<'a>(
            &'a self,
            request: Wave2HostRequest,
        ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>
        {
            let captured = Arc::clone(&self.captured);
            let state = request.context.state;
            Box::pin(std::future::ready({
                *captured.lock().expect("state capture mutex") = Some(state);
                Ok(StrictJsonValue(json!({})))
            }))
        }
    }

    fn poll_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("state capture future must complete synchronously"),
        }
    }

    fn test_state_handle() -> Wave2StateHandle {
        static HANDLE: OnceLock<Wave2StateHandle> = OnceLock::new();
        HANDLE.get_or_init(capture_state_handle).clone()
    }

    fn capture_state_handle() -> Wave2StateHandle {
        let captured = Arc::new(Mutex::new(None));
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(nomifun_agent_domain_wave2::CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("kernel registry");
        let materialized = registry
            .replace_all(
                nomifun_agent_domain_wave2::registrations_with_host_port(Arc::new(
                    StateCaptureHostPort {
                        captured: Arc::clone(&captured),
                    },
                ))
                .expect("Wave 2 registrations"),
            )
            .expect("publish Wave 2 registrations");
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "wave2-host-owner".to_owned(),
        };
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("wave2-host-workspace"),
            resource_kind: ResourceKind::from(WORKSPACE_RESOURCE_KIND),
            resource_id: ResourceId::from("wave2-host-resource"),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from([WORKSPACE_READ_OPERATION.to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let action = ActionId::from("workspace.files/read");
        let payload = AgentPresetRevisionPayload {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: VersionString::from(nomifun_agent_domain_wave2::CONTRACT_VERSION),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![CapabilitySelection {
                capability: CapabilityRef {
                    id: CapabilityId::from("workspace.files"),
                    version: VersionString::from(nomifun_agent_domain_wave2::CONTRACT_VERSION),
                },
                action_allowlist: BTreeSet::from([action.clone()]),
            }],

            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "Wave 2 host test".to_owned(),
            instructions: "Invoke the selected capability.".to_owned(),
            starter_prompts: Vec::new(),
        };
        let contribution_locks = vec![materialized
            .capability(&CapabilityId::from("workspace.files"))
            .expect("materialized workspace.files Module")
            .contribution_lock
            .clone()];
        let mut revision = AgentPresetRevision {
            reference: PresetRevisionRef {
                preset_id: AgentPresetId::from("wave2-host-test"),
                revision: 1,
                revision_digest: DigestHex::from(""),
            },
            payload,
            contribution_locks,
            created_by: UserId::from(principal.principal_id.clone()),
            created_at_ms: 1,
            reason: None,
        };
        revision.reference.revision_digest =
            revision.revision_digest().expect("revision digest");
        let snapshot = AgentPresetCompiler::compile(
            &materialized,
            &CompilerEnvironment {
                resolver_version: VersionString::from(nomifun_agent_domain_wave2::CONTRACT_VERSION),
                required_runtime_protocol_version: VersionString::from(nomifun_agent_domain_wave2::CONTRACT_VERSION),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: DigestHex::from("runtime"),
                available_runtime_features: BTreeSet::new(),
                installation_role_bindings: BTreeMap::new(),
                canonical_schema_manifest_digest: DigestHex::from("schema"),
                target_contribution_manifest_digest: DigestHex::from("target"),
                host_target: RuntimeTarget::from("test-target"),
                host_surface: "desktop".to_owned(),
                availability_evidence_revision: "wave2-host-test".to_owned(),
            },
            CompileRequest {
                plugin_product_capabilities: Vec::new(),
                revision,
                principal: principal.clone(),
                scene: "wave2-host-test".to_owned(),
                surface: "desktop".to_owned(),
                audience: "test".to_owned(),
                created_at_ms: 2,
                resolver_run_id: OperationId::from("wave2-host-resolve"),
            },
        )
        .expect("compile selected capability")
        .with_target_resource_bindings(&principal, vec![binding])
        .expect("bind selected target resource");
        let active = SessionCapabilityState::new(&snapshot)
            .snapshot()
            .expect("initial active set");
        poll_ready(registry.invoke(
            &snapshot,
            &active,
            CapabilityInvocationRequest {
                principal: principal.clone(),
                session_owner: principal,
                agent_session_id: AgentSessionId::from("wave2-host-session"),
                turn_id: OperationId::from("wave2-host-turn"),
                operation_id: OperationId::from("wave2-host-operation"),
                idempotency_key: IdempotencyKey::from("wave2-host-idempotency"),
                correlation_id: CorrelationId::from("wave2-host-correlation"),
                resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from("workspace.files"),
                action_id: action,
                resource_binding_ids: BTreeSet::from([ResourceBindingId::from(
                    "wave2-host-workspace",
                )]),
                state_scope_key: ScopeKey::from("session:wave2-host"),
                input: StrictJsonValue(json!({"path": "fixture.txt"})),
            },
        ))
        .expect("state projection invocation");
        captured
            .lock()
            .expect("state capture mutex")
            .take()
            .expect("host received the state handle")
    }

    fn context(root: &std::path::Path) -> Wave2HostContext {
        let agent_session_id = AgentSessionId::from(nomifun_common::generate_id());
        Wave2HostContext {
            principal: PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: "owner-1".to_owned(),
            },
            agent_session_id: agent_session_id.clone(),
            turn_id: OperationId::from("turn-1"),
            operation_id: OperationId::from("operation-1"),
            idempotency_key: IdempotencyKey::from("idempotency-1"),
            correlation_id: CorrelationId::from("correlation-1"),
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: "snapshot-1".into(),
                snapshot_digest: "a".repeat(64).into(),
            },
            registry_generation: 1,
            capability_id: CapabilityId::from("workspace.files"),
            action_id: ActionId::from("workspace.files/write"),
            role_provider: None,
            state: test_state_handle(),
            resource_bindings: vec![TypedResourceBinding {
                binding_id: ResourceBindingId::from(format!(
                    "workspace-binding:{}",
                    agent_session_id.as_ref()
                )),
                resource_kind: ResourceKind::from(WORKSPACE_RESOURCE_KIND),
                resource_id: ResourceId::from(format!(
                    "workspace-resource:{}",
                    root.to_string_lossy()
                )),
                owner_id: "owner-1".to_owned(),
                operations: BTreeSet::from([
                    "read".to_owned(),
                    "write".to_owned(),
                    "delete".to_owned(),
                ]),
                connection_config_ref: None,
                typed_parameters: BTreeMap::from([(
                    WORKSPACE_ROOT_PARAMETER.to_owned(),
                    root.to_string_lossy().into_owned(),
                )]),
            }],
        }
    }

    async fn test_effect_store() -> nomifun_agent_session::AgentSessionStore {
        let database = nomifun_db::init_database_memory()
            .await
            .expect("in-memory Agent Store database");
        nomifun_agent_session::AgentSessionStore::from_pool(database.pool().clone())
            .await
            .expect("canonical Agent Session Store")
    }

    async fn test_host(root: &Path) -> Wave2ApplicationHost {
        Wave2ApplicationHost::for_workspace_root(root)
            .with_effect_store(test_effect_store().await)
    }

    fn effectful_workspace_action(action_id: &str) -> bool {
        matches!(
            action_id,
            "workspace.files/write"
                | "workspace.files/patch"
                | "workspace.files/delete"
                | "workspace.artifacts/publish"
                | "workspace.vcs/stage"
                | "workspace.vcs/commit"
                | "workspace.vcs/push"
        )
    }

    async fn ensure_test_effect_context(
        store: &nomifun_agent_session::AgentSessionStore,
        context: &Wave2HostContext,
    ) {
        let session_exists = store
            .get_live_session(&context.agent_session_id)
            .await
            .is_ok();
        let preset_ref = PresetRevisionRef {
            preset_id: AgentPresetId::from("wave2-workspace-test"),
            revision: 1,
            revision_digest: DigestHex::from("b".repeat(64)),
        };
        let session = AgentSessionLiveRecord {
            agent_session_id: context.agent_session_id.clone(),
            owner_ref: context.principal.clone(),
            metadata: AgentSessionMetadata {
                title: Some("Wave 2 workspace test".to_owned()),
                archived: false,
                pinned: false,
            },
            agent_binding: AgentBindingValue {
                preset_revision_ref: preset_ref,
                resolved_snapshot_ref: context.resolved_snapshot_ref.clone(),
                typed_resource_bindings: context.resource_bindings.clone(),
                binding_version: 1,
            },
            remote_binding_provenance: None,
            parent_session_id: None,
            fork_base_payload_id: None,
            next_seq: 1,
        };
        let session_key = format!("workspace-test-session:{}", context.agent_session_id.as_ref());
        if !session_exists {
            let created = store
                .create_session(nomifun_agent_session::CreateSessionRequest::new(
                    session,
                    1,
                    OperationId::from(format!("{session_key}:create")),
                    EventProducerId::from("session_api"),
                    IdempotencyKey::from(format!("{session_key}:create")),
                    CorrelationId::from(format!("{session_key}:create")),
                ))
                .await
                .expect("create durable effect test Session");
            store
                .append_event(&SessionEventAppend {
                    agent_session_id: context.agent_session_id.clone(),
                    event_id: EventId::from(format!("{session_key}:ready")),
                    producer_id: EventProducerId::from("runtime_supervisor"),
                    idempotency_key: IdempotencyKey::from(format!("{session_key}:ready")),
                    runtime_binding_id: None,
                    runtime_producer_seq: None,
                    semantic_event: SemanticSessionEventDraft {
                        kind: SessionEventKind("session/ready".to_owned()),
                        kind_version: 1,
                        correlation_id: CorrelationId::from(format!("{session_key}:ready")),
                        causation_event_id: Some(created.opening_ack.event_id),
                        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
                    },
                })
                .await
                .expect("mark durable effect test Session ready");
        }
        let receipt = store
            .read_turn_receipt(&context.agent_session_id, &context.turn_id)
            .await
            .expect("read durable effect test Turn");
        let turn_event_id = match receipt.started_event {
            Some(started) => started.event_id,
            None => store
                .start_turn(
                    &context.agent_session_id,
                    EventProducerId::from("session_api"),
                    IdempotencyKey::from(format!(
                        "{session_key}:turn:{}",
                        context.turn_id.as_ref()
                    )),
                    context.turn_id.clone(),
                    StrictJsonValue(json!({"content": "exercise a workspace effect"})),
                )
                .await
                .expect("start durable effect test Turn")
                .1
                .ack
                .expect("turn start acknowledgement")
                .event_id,
        };
        let tool_key = format!(
            "{session_key}:tool:{}:{}:{}",
            context.operation_id.as_ref(),
            context.capability_id.as_ref(),
            context.action_id.as_ref()
        );
        let tool = SessionEventAppend {
            agent_session_id: context.agent_session_id.clone(),
            event_id: EventId::from(tool_key.clone()),
            producer_id: EventProducerId::from("capability_host"),
            idempotency_key: IdempotencyKey::from(tool_key.clone()),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("tool/call-started".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(tool_key),
                causation_event_id: Some(turn_event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "operation_id": context.operation_id.as_ref(),
                    "capability_id": context.capability_id.as_ref(),
                    "action_id": context.action_id.as_ref(),
                }))),
            },
        };
        store
            .append_event(&tool)
            .await
            .expect("append durable effect test Tool fact");
    }

    async fn invoke(
        host: &Wave2ApplicationHost,
        mut context: Wave2HostContext,
        action_id: &str,
        input: Value,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let capability_id = action_id.split('/').next().ok_or_else(|| {
            Wave2HostPortError::invalid_payload("test Action ID has no Module prefix")
        })?;
        let action_identity_changed = context.action_id.as_ref() != action_id;
        context.capability_id = CapabilityId::from(capability_id.to_owned());
        context.action_id = ActionId::from(action_id.to_owned());
        if action_identity_changed && context.idempotency_key.as_ref() == "idempotency-1" {
            context.operation_id = OperationId::from(format!(
                "{}:{action_id}",
                context.operation_id.as_ref()
            ));
            context.idempotency_key = IdempotencyKey::from(format!(
                "{}:{action_id}",
                context.idempotency_key.as_ref()
            ));
            context.correlation_id = CorrelationId::from(format!(
                "{}:{action_id}",
                context.correlation_id.as_ref()
            ));
        }
        if effectful_workspace_action(action_id) {
            let store = host
                .effect_store
                .as_deref()
                .expect("effectful test host must mount the canonical Agent Store");
            ensure_test_effect_context(store, &context).await;
        }
        host.invoke(Wave2HostRequest {
            context,
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: StrictJsonValue(input),
            },
        })
        .await
    }

    #[tokio::test]
    async fn workspace_file_actions_use_the_typed_binding_root() {
        let directory = tempfile::tempdir().unwrap();
        let host = test_host(directory.path()).await;
        let context = context(directory.path());

        let written = invoke(
            &host,
            context.clone(),
            "workspace.files/write",
            json!({"path": "test.txt", "content": "hello"}),
        )
        .await
        .unwrap();
        assert_eq!(written.0["written"], true);

        let read = invoke(
            &host,
            context.clone(),
            "workspace.files/read",
            json!({"path": "test.txt"}),
        )
        .await
        .unwrap();
        assert_eq!(read.0["content"], "hello");

        let deleted = invoke(
            &host,
            context,
            "workspace.files/delete",
            json!({"path": "test.txt"}),
        )
        .await
        .unwrap();
        assert_eq!(deleted.0["deleted"], true);
        assert!(!directory.path().join("test.txt").exists());
    }

    #[tokio::test]
    async fn workspace_patch_replaces_exact_content_and_rejects_stale_context() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("patch.txt"),
            "before\nold value\nafter\n",
        )
        .unwrap();
        let host = test_host(directory.path()).await;
        let context = context(directory.path());

        let patched = invoke(
            &host,
            context.clone(),
            "workspace.files/patch",
            json!({
                "files": [{
                    "path": "patch.txt",
                    "hunks": [{
                        "old_start": 2,
                        "old_lines": 1,
                        "new_start": 2,
                        "new_lines": 1,
                        "lines": [
                            {"kind": "remove", "text": "old value"},
                            {"kind": "add", "text": "new value"}
                        ]
                    }]
                }]
            }),
        )
        .await
        .unwrap();
        assert_eq!(patched.0["file_count"], 1);
        assert_eq!(patched.0["files"][0]["path"], "patch.txt");
        assert_eq!(patched.0["files"][0]["hunks_applied"], 1);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("patch.txt")).unwrap(),
            "before\nnew value\nafter\n"
        );

        let mut stale_context = context;
        stale_context.idempotency_key = IdempotencyKey::from("patch-stale");
        let stale = invoke(
            &host,
            stale_context,
            "workspace.files/patch",
            json!({
                "files": [{
                    "path": "patch.txt",
                    "hunks": [{
                        "old_start": 2,
                        "old_lines": 1,
                        "new_start": 2,
                        "new_lines": 1,
                        "lines": [
                            {"kind": "remove", "text": "old value"},
                            {"kind": "add", "text": "another value"}
                        ]
                    }]
                }]
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(stale.code, "INVALID_PAYLOAD");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("patch.txt")).unwrap(),
            "before\nnew value\nafter\n"
        );
    }

    #[tokio::test]
    async fn concurrent_workspace_patches_do_not_overwrite_each_other() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("first.txt"), "alpha\n").unwrap();
        std::fs::write(directory.path().join("second.txt"), "beta\n").unwrap();
        let host = test_host(directory.path()).await;
        let context = context(directory.path());

        let mut first_context = context.clone();
        first_context.idempotency_key = IdempotencyKey::from("patch-alpha");
        let first = invoke(
            &host,
            first_context,
            "workspace.files/patch",
            json!({
                "files": [{
                    "path": "first.txt",
                    "hunks": [{
                        "old_start": 1,
                        "old_lines": 1,
                        "new_start": 1,
                        "new_lines": 1,
                        "lines": [
                            {"kind": "remove", "text": "alpha"},
                            {"kind": "add", "text": "ALPHA"}
                        ]
                    }]
                }]
            }),
        );
        let mut second_context = context;
        second_context.idempotency_key = IdempotencyKey::from("patch-beta");
        let second = invoke(
            &host,
            second_context,
            "workspace.files/patch",
            json!({
                "files": [{
                    "path": "second.txt",
                    "hunks": [{
                        "old_start": 1,
                        "old_lines": 1,
                        "new_start": 1,
                        "new_lines": 1,
                        "lines": [
                            {"kind": "remove", "text": "beta"},
                            {"kind": "add", "text": "BETA"}
                        ]
                    }]
                }]
            }),
        );
        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        second.unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("first.txt")).unwrap(),
            "ALPHA\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("second.txt")).unwrap(),
            "BETA\n"
        );
    }

    #[tokio::test]
    async fn workspace_artifact_publish_and_read_use_the_file_domain_owner() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("result.txt"), "artifact payload").unwrap();
        let host = test_host(directory.path()).await;
        let context = context(directory.path());

        let published = invoke(
            &host,
            context.clone(),
            "workspace.artifacts/publish",
            json!({"path": "result.txt"}),
        )
        .await
        .unwrap();
        let artifact_id = published.0["artifact_id"].as_str().unwrap();
        assert_eq!(published.0["sha256"], artifact_id);
        assert!(directory
            .path()
            .join(".nomifun")
            .join("artifacts")
            .join(artifact_id)
            .is_file());

        let read = invoke(
            &host,
            context,
            "workspace.artifacts/read",
            json!({"artifact_id": artifact_id, "limit": 1024}),
        )
        .await
        .unwrap();
        assert_eq!(read.0["complete"], true);
        assert_eq!(read.0["sha256"], artifact_id);
    }

    #[tokio::test]
    async fn workspace_artifact_owner_directory_is_absent_from_vcs_status() {
        let directory = tempfile::tempdir().unwrap();
        let _repository = initialize_git_repository(directory.path());
        std::fs::write(directory.path().join("result.txt"), "artifact payload").unwrap();
        let host = test_host(directory.path()).await;
        let context = context(directory.path());
        invoke(
            &host,
            context.clone(),
            "workspace.artifacts/publish",
            json!({"path": "result.txt"}),
        )
        .await
        .unwrap();

        let status = invoke(&host, context.clone(), "workspace.vcs/status", json!({}))
            .await
            .unwrap();
        assert!(status.0["entries"].as_array().unwrap().iter().all(|entry| {
            !entry["path"]
                .as_str()
                .is_some_and(|path| path == ".nomifun" || path.starts_with(".nomifun/"))
        }));
        let denied = invoke(
            &host,
            context,
            "workspace.vcs/stage",
            json!({"path": ".nomifun"}),
        )
        .await
        .unwrap_err();
        assert_eq!(denied.code, "RESOURCE_NOT_FOUND");
    }

    #[tokio::test]
    async fn missing_workspace_root_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let host = test_host(directory.path()).await;
        let mut context = context(directory.path());
        context.resource_bindings[0].typed_parameters.clear();
        let error = invoke(&host, context, "workspace.files/read", json!({"path": "x"}))
            .await
            .unwrap_err();
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
    }

    #[tokio::test]
    async fn host_rejects_action_identity_and_extra_resource_bindings() {
        let directory = tempfile::tempdir().unwrap();
        let host = test_host(directory.path()).await;
        let mut wrong_action = context(directory.path());
        wrong_action.capability_id = CapabilityId::from("workspace.files");
        wrong_action.action_id = ActionId::from("workspace.files/unknown");
        let error = host
            .invoke(Wave2HostRequest {
                context: wrong_action,
                operation: Wave2CapabilityOperation::WorkspaceExecution {
                    input: StrictJsonValue(json!({"path": "x.txt"})),
                },
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "ACTION_NOT_DECLARED");

        let mut extra_binding = context(directory.path());
        extra_binding.resource_bindings.push(TypedResourceBinding {
            binding_id: ResourceBindingId::from("process-binding"),
            resource_kind: ResourceKind::from("process_session"),
            resource_id: ResourceId::from("process-resource"),
            owner_id: "owner-1".to_owned(),
            operations: BTreeSet::from(["execute".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        });
        let error = invoke(
            &host,
            extra_binding,
            "workspace.files/read",
            json!({"path": "x.txt"}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
    }

    #[tokio::test]
    async fn workspace_search_returns_real_content_matches() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("needle.txt"), "before\nneedle line\nafter\n")
            .unwrap();
        let host = test_host(directory.path()).await;
        let result = invoke(
            &host,
            context(directory.path()),
            "workspace.files/search",
            json!({"query": "needle"}),
        )
        .await
        .unwrap();
        assert_eq!(result.0["matches"][0]["path"], "needle.txt");
        assert_eq!(result.0["matches"][0]["line"], 2);
        assert_eq!(result.0["truncated"], false);
    }

    #[tokio::test]
    async fn workspace_patch_uses_the_bound_file_owner_and_is_all_or_nothing() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("a.txt"), "alpha\n").unwrap();
        std::fs::write(directory.path().join("b.txt"), "bravo\n").unwrap();
        let host = test_host(directory.path()).await;
        let patch = |path: &str, old: &str, new: &str| {
            json!({
                "path": path,
                "hunks": [{
                    "old_start": 1,
                    "old_lines": 1,
                    "new_start": 1,
                    "new_lines": 1,
                    "lines": [
                        {"kind": "remove", "text": old},
                        {"kind": "add", "text": new}
                    ]
                }]
            })
        };

        let result = invoke(
            &host,
            context(directory.path()),
            "workspace.files/patch",
            json!({
                "files": [
                    patch("a.txt", "alpha", "ALPHA"),
                    patch("b.txt", "bravo", "BRAVO")
                ]
            }),
        )
        .await
        .unwrap();
        assert_eq!(result.0["file_count"], 2);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("a.txt")).unwrap(),
            "ALPHA\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("b.txt")).unwrap(),
            "BRAVO\n"
        );

        let mut invalid_context = context(directory.path());
        invalid_context.idempotency_key = IdempotencyKey::from("patch-invalid-hunk-key");
        let error = invoke(
            &host,
            invalid_context,
            "workspace.files/patch",
            json!({
                "files": [
                    patch("a.txt", "ALPHA", "again"),
                    patch("b.txt", "not-present", "must-not-write")
                ]
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "INVALID_PAYLOAD");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("a.txt")).unwrap(),
            "ALPHA\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("b.txt")).unwrap(),
            "BRAVO\n"
        );
    }

    #[tokio::test]
    async fn workspace_patch_rejects_traversal_and_read_only_bindings() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret\n").unwrap();
        let host = test_host(directory.path()).await;
        let mut read_only = context(directory.path());
        read_only.resource_bindings[0].operations =
            BTreeSet::from([WORKSPACE_READ_OPERATION.to_owned()]);

        let read_only_error = invoke(
            &host,
            read_only,
            "workspace.files/patch",
            json!({
                "files": [{
                    "path": "new.txt",
                    "hunks": [{
                        "old_start": 0,
                        "old_lines": 0,
                        "new_start": 1,
                        "new_lines": 1,
                        "lines": [{"kind": "add", "text": "nope"}]
                    }]
                }]
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(read_only_error.code, "PRESET_RESOURCE_NOT_BOUND");

        let mut traversal_context = context(directory.path());
        traversal_context.idempotency_key = IdempotencyKey::from("patch-traversal-key");
        let traversal_error = invoke(
            &host,
            traversal_context,
            "workspace.files/patch",
            json!({
                "files": [{
                    "path": format!("../{}", outside.path().join("secret.txt").file_name().unwrap().to_string_lossy()),
                    "hunks": [{
                        "old_start": 1,
                        "old_lines": 1,
                        "new_start": 1,
                        "new_lines": 1,
                        "lines": [
                            {"kind": "remove", "text": "secret"},
                            {"kind": "add", "text": "escaped"}
                        ]
                    }]
                }]
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(traversal_error.code, "INVALID_PAYLOAD");
        assert_eq!(
            std::fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
            "secret\n"
        );
    }

    #[tokio::test]
    async fn effectful_workspace_actions_replay_and_conflict_by_idempotency_key() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("entry.txt"), "before\n").unwrap();
        let host = test_host(directory.path()).await;
        let mut replay_context = context(directory.path());
        replay_context.capability_id = CapabilityId::from("workspace.files");
        replay_context.action_id = ActionId::from("workspace.files/patch");
        replay_context.idempotency_key = IdempotencyKey::from("patch-replay-key");
        let patch = json!({
            "files": [{
                "path": "entry.txt",
                "hunks": [{
                    "old_start": 1,
                    "old_lines": 1,
                    "new_start": 1,
                    "new_lines": 1,
                    "lines": [
                        {"kind": "remove", "text": "before"},
                        {"kind": "add", "text": "after"}
                    ]
                }]
            }]
        });
        let first = invoke(&host, replay_context.clone(), "workspace.files/patch", patch.clone())
            .await
            .unwrap();
        let replay = invoke(&host, replay_context.clone(), "workspace.files/patch", patch)
            .await
            .unwrap();
        assert_eq!(replay, first);
        let durable = host
            .effect_store()
            .unwrap()
            .read_effect(
                &replay_context.agent_session_id,
                &wave2_effect_id(&replay_context).unwrap(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(durable.turn_id, replay_context.turn_id);
        assert_eq!(durable.capability_module.as_ref(), "workspace.files");
        assert_eq!(durable.action_id.as_ref(), "workspace.files/patch");
        assert_eq!(
            durable.resource_binding_id.as_ref(),
            Some(&replay_context.resource_bindings[0].binding_id)
        );
        assert_eq!(durable.state, nomifun_agent_session::AgentEffectState::Returned);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("entry.txt")).unwrap(),
            "after\n"
        );

        let conflict = invoke(
            &host,
            replay_context,
            "workspace.files/patch",
            json!({
                "files": [{
                    "path": "entry.txt",
                    "hunks": [{
                        "old_start": 1,
                        "old_lines": 1,
                        "new_start": 1,
                        "new_lines": 1,
                        "lines": [
                            {"kind": "remove", "text": "after"},
                            {"kind": "add", "text": "different"}
                        ]
                    }]
                }]
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(conflict.code, "IDEMPOTENCY_CONFLICT");

        let mut action_context = context(directory.path());
        action_context.idempotency_key = IdempotencyKey::from("action-mismatch-key");
        let action_conflict = invoke(
            &host,
            action_context.clone(),
            "workspace.files/write",
            json!({"path": "entry.txt", "content": "replacement"}),
        )
        .await
        .unwrap();
        assert_eq!(action_conflict.0["written"], true);
        let mismatch = invoke(
            &host,
            action_context,
            "workspace.files/delete",
            json!({"path": "entry.txt"}),
        )
        .await
        .unwrap_err();
        assert_eq!(mismatch.code, "IDEMPOTENCY_CONFLICT");
        assert!(directory.path().join("entry.txt").exists());
    }

    #[tokio::test]
    async fn durable_effect_ledger_has_no_128_record_lockout_and_namespaces_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let host = test_host(directory.path()).await;
        let base = context(directory.path());
        for index in 0..129 {
            let mut invocation = base.clone();
            invocation.idempotency_key = IdempotencyKey::from(format!("write-{index}"));
            invocation.operation_id = OperationId::from(format!("write-operation-{index}"));
            invoke(
                &host,
                invocation,
                "workspace.files/write",
                json!({"path": format!("record-{index}.txt"), "content": index.to_string()}),
            )
            .await
            .unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(directory.path().join("record-128.txt")).unwrap(),
            "128"
        );

        let mut first_session = context(directory.path());
        first_session.idempotency_key = IdempotencyKey::from("same-caller-key");
        let mut second_session = context(directory.path());
        second_session.idempotency_key = IdempotencyKey::from("same-caller-key");
        invoke(
            &host,
            first_session,
            "workspace.files/write",
            json!({"path": "first-session.txt", "content": "first"}),
        )
        .await
        .unwrap();
        invoke(
            &host,
            second_session,
            "workspace.files/write",
            json!({"path": "second-session.txt", "content": "second"}),
        )
        .await
        .unwrap();
    }

    #[test]
    fn durable_effect_identity_rejects_edge_whitespace_in_wire_keys() {
        let directory = tempfile::tempdir().unwrap();
        for key in [" key", "key ", "\tkey"] {
            let mut value = context(directory.path());
            value.idempotency_key = IdempotencyKey::from(key);
            let error = wave2_effect_id(&value).unwrap_err();
            assert_eq!(error.code, "INVALID_PAYLOAD");
        }
        let mut value = context(directory.path());
        value.idempotency_key = IdempotencyKey::from("key value");
        assert!(wave2_effect_id(&value).is_ok());
    }

    #[test]
    fn truncated_terminal_observation_is_bounded_and_never_decoded_as_original_result() {
        let directory = tempfile::tempdir().unwrap();
        let mut context = context(directory.path());
        context.capability_id = CapabilityId::from("workspace.files");
        context.action_id = ActionId::from("workspace.files/write");
        let binding = context.resource_bindings[0].clone();
        let input = StrictJsonValue(json!({"path": "large", "content": "value"}));
        let input_digest = wave2_effect_request_digest(&context, &binding, &input).unwrap();
        let resource_key = wave2_effect_resource_key(&context, &binding).unwrap();
        let output = StrictJsonValue(json!({"blob": "x".repeat(96 * 1024)}));
        let bounded = bounded_terminal_payload(Wave2EffectCompletion::Succeeded(&output));
        assert!(canonical_json_bytes(&bounded.0).unwrap().len() < 64 * 1024);
        assert_eq!(bounded.0["observation_truncated"], true);
        assert!(bounded.0.get("result").is_none());
        let record = nomifun_agent_session::AgentEffectRecord {
            effect_id: wave2_effect_id(&context).unwrap(),
            agent_session_id: context.agent_session_id.clone(),
            turn_id: context.turn_id.clone(),
            operation_id: context.operation_id.clone(),
            owner_domain: "workspace".to_owned(),
            capability_module: context.capability_id.clone(),
            action_id: context.action_id.clone(),
            resource_binding_id: Some(binding.binding_id.clone()),
            resource_key: Some(resource_key.clone()),
            input_digest: input_digest.clone(),
            strategy: nomifun_agent_session::EffectStrategy::ManagedEffect,
            state: nomifun_agent_session::AgentEffectState::Returned,
            bounded_observation: Some(bounded.0),
            started_event_id: EventId::from("started"),
            terminal_event_id: Some(EventId::from("terminal")),
            created_at: 1,
            settled_at: Some(2),
        };
        let error = observe_wave2_effect(
            record,
            &context,
            &binding,
            &input_digest,
            nomifun_agent_session::EffectStrategy::ManagedEffect,
            &resource_key,
        )
        .unwrap_err();
        assert_eq!(error.code, "CAPABILITY_UNAVAILABLE");
        assert!(error.message.contains("cannot reproduce the exact result"));
    }

    #[tokio::test]
    async fn durable_effect_resource_fence_is_owner_scoped_but_physical_resource_stable() {
        let directory = tempfile::tempdir().unwrap();
        let store = test_effect_store().await;
        let host = Wave2ApplicationHost::for_workspace_root(directory.path())
            .with_effect_store(store.clone());
        let input = StrictJsonValue(json!({"path": "owned.txt", "content": "value"}));

        let mut first = context(directory.path());
        first.capability_id = CapabilityId::from("workspace.files");
        first.action_id = ActionId::from("workspace.files/write");
        first.idempotency_key = IdempotencyKey::from("first-owner-pending");
        first.resource_bindings[0].resource_id = ResourceId::from("workspace");
        ensure_test_effect_context(&store, &first).await;
        let first_binding = workspace_typed_binding(&first).unwrap();
        assert!(matches!(
            begin_wave2_effect(host.effect_store().unwrap(), &first, first_binding, &input)
                .await
                .unwrap(),
            Wave2EffectAdmission::Reserved(_)
        ));

        let mut second_owner = context(directory.path());
        second_owner.principal.principal_id = "owner-2".to_owned();
        second_owner.resource_bindings[0].owner_id = "owner-2".to_owned();
        second_owner.resource_bindings[0].resource_id = ResourceId::from("workspace");
        second_owner.capability_id = CapabilityId::from("workspace.files");
        second_owner.action_id = ActionId::from("workspace.files/write");
        second_owner.idempotency_key = IdempotencyKey::from("second-owner-pending");
        ensure_test_effect_context(&store, &second_owner).await;
        assert!(matches!(
            begin_wave2_effect(
                host.effect_store().unwrap(),
                &second_owner,
                workspace_typed_binding(&second_owner).unwrap(),
                &input,
            )
            .await
            .unwrap(),
            Wave2EffectAdmission::Reserved(_)
        ));

        let mut same_owner = context(directory.path());
        same_owner.resource_bindings[0].resource_id = ResourceId::from("workspace");
        same_owner.capability_id = CapabilityId::from("workspace.files");
        same_owner.action_id = ActionId::from("workspace.files/write");
        same_owner.idempotency_key = IdempotencyKey::from("same-owner-competing");
        ensure_test_effect_context(&store, &same_owner).await;
        let blocked = begin_wave2_effect(
            host.effect_store().unwrap(),
            &same_owner,
            workspace_typed_binding(&same_owner).unwrap(),
            &input,
        )
        .await
        .unwrap_err();
        assert_eq!(blocked.code, "CAPABILITY_UNAVAILABLE");
        assert!(blocked.message.contains("unsettled"));
    }

    #[tokio::test]
    async fn durable_effect_pending_and_unknown_fences_survive_host_restart() {
        let pending_root = tempfile::tempdir().unwrap();
        let store = test_effect_store().await;
        let first_host = Wave2ApplicationHost::for_workspace_root(pending_root.path())
            .with_effect_store(store.clone());
        let mut pending_context = context(pending_root.path());
        pending_context.capability_id = CapabilityId::from("workspace.files");
        pending_context.action_id = ActionId::from("workspace.files/write");
        pending_context.idempotency_key = IdempotencyKey::from("restart-pending");
        ensure_test_effect_context(&store, &pending_context).await;
        let pending_input = StrictJsonValue(json!({
            "path": "pending.txt",
            "content": "must-not-run"
        }));
        let pending_binding = workspace_typed_binding(&pending_context).unwrap();
        let admission = begin_wave2_effect(
            first_host.effect_store().unwrap(),
            &pending_context,
            pending_binding,
            &pending_input,
        )
        .await
        .unwrap();
        assert!(matches!(admission, Wave2EffectAdmission::Reserved(_)));
        drop(first_host);

        let restarted = Wave2ApplicationHost::for_workspace_root(pending_root.path())
            .with_effect_store(store.clone());
        let pending = invoke(
            &restarted,
            pending_context,
            "workspace.files/write",
            pending_input.0,
        )
        .await
        .unwrap_err();
        assert_eq!(pending.code, "CAPABILITY_UNAVAILABLE");
        assert!(pending.message.contains("durable pending"));
        assert!(!pending_root.path().join("pending.txt").exists());

        let unknown_root = tempfile::tempdir().unwrap();
        let first_host = Wave2ApplicationHost::for_workspace_root(unknown_root.path())
            .with_effect_store(store.clone());
        let mut unknown_context = context(unknown_root.path());
        unknown_context.capability_id = CapabilityId::from("workspace.vcs");
        unknown_context.action_id = ActionId::from("workspace.vcs/push");
        unknown_context.idempotency_key = IdempotencyKey::from("restart-unknown");
        ensure_test_effect_context(&store, &unknown_context).await;
        let unknown_input = StrictJsonValue(json!({
            "remote": "origin",
            "refspec": "HEAD:refs/heads/main"
        }));
        let unknown_binding = workspace_typed_binding(&unknown_context).unwrap();
        let Wave2EffectAdmission::Reserved(reservation) = begin_wave2_exclusive_effect(
            first_host.effect_store().unwrap(),
            &unknown_context,
            unknown_binding,
            &unknown_input,
            nomifun_agent_session::EffectStrategy::ExternalUncertainEffect,
        )
        .await
        .unwrap()
        else {
            panic!("fresh external effect must reserve")
        };
        let uncertain = Wave2HostPortError::unavailable("transport outcome is unknown");
        finish_wave2_effect(
            &reservation,
            Wave2EffectCompletion::Uncertain(&uncertain),
        )
        .await
        .unwrap();
        drop(first_host);

        let restarted = Wave2ApplicationHost::for_workspace_root(unknown_root.path())
            .with_effect_store(store);
        let unknown = begin_wave2_exclusive_effect(
            restarted.effect_store().unwrap(),
            &unknown_context,
            workspace_typed_binding(&unknown_context).unwrap(),
            &unknown_input,
            nomifun_agent_session::EffectStrategy::ExternalUncertainEffect,
        )
        .await
        .unwrap_err();
        assert_eq!(unknown.code, "CAPABILITY_UNAVAILABLE");
        assert!(unknown.message.contains("durable unknown"));
    }

    fn initialize_git_repository(root: &Path) -> git2::Repository {
        let repository = git2::Repository::init(root).unwrap();
        std::fs::write(root.join("tracked.txt"), "base\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repository.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("NomiFun test", "test@nomifun.invalid").unwrap();
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "initial",
                &tree,
                &[],
            )
            .unwrap();
        drop(tree);
        repository
    }

    #[tokio::test]
    async fn vcs_status_diff_and_stage_use_the_bound_repository() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "base\nchanged\n").unwrap();
        let host = test_host(directory.path()).await;
        let base_context = context(directory.path());

        let status = invoke(&host, base_context.clone(), "workspace.vcs/status", json!({}))
            .await
            .unwrap();
        assert_eq!(status.0["entries"][0]["path"], "tracked.txt");
        assert!(
            status.0["entries"][0]["status"]
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value == "worktree_modified"))
        );

        let diff = invoke(&host, base_context.clone(), "workspace.vcs/diff", json!({}))
            .await
            .unwrap();
        assert!(diff.0["patch"].as_str().unwrap().contains("changed"));

        let staged = invoke(
            &host,
            base_context.clone(),
            "workspace.vcs/stage",
            json!({"path": "tracked.txt"}),
        )
        .await
        .unwrap();
        assert_eq!(staged.0["staged"], true);
        let status_after = repository.statuses(None).unwrap();
        assert!(
            status_after
                .iter()
                .any(|entry| entry.status().contains(git2::Status::INDEX_MODIFIED))
        );
        let staged_diff = invoke(&host, base_context, "workspace.vcs/diff", json!({}))
            .await
            .unwrap();
        assert!(
            staged_diff.0["staged_patch"]
                .as_str()
                .unwrap()
                .contains("changed")
        );
        assert_eq!(staged_diff.0["unstaged_patch"], "");
    }

    #[tokio::test]
    async fn vcs_stage_recurses_directories_and_records_deletions() {
        let directory = tempfile::tempdir().unwrap();
        let _repository = initialize_git_repository(directory.path());
        let batch = directory.path().join("batch");
        std::fs::create_dir(&batch).unwrap();
        std::fs::write(batch.join("keep.txt"), "keep\n").unwrap();
        std::fs::write(batch.join("remove.txt"), "remove\n").unwrap();
        let host = test_host(directory.path()).await;
        let context = context(directory.path());

        invoke(
            &host,
            context.clone(),
            "workspace.vcs/stage",
            json!({"path": "batch"}),
        )
        .await
        .unwrap();
        let index = git2::Repository::open(directory.path())
            .unwrap()
            .index()
            .unwrap();
        assert!(index.get_path(Path::new("batch/keep.txt"), 0).is_some());
        assert!(index.get_path(Path::new("batch/remove.txt"), 0).is_some());
        drop(index);

        std::fs::remove_file(batch.join("remove.txt")).unwrap();
        let mut second_context = context;
        second_context.idempotency_key = IdempotencyKey::from("stage-after-delete");
        invoke(&host, second_context, "workspace.vcs/stage", json!({"path": "batch"}))
            .await
            .unwrap();
        let index = git2::Repository::open(directory.path())
            .unwrap()
            .index()
            .unwrap();
        assert!(index.get_path(Path::new("batch/keep.txt"), 0).is_some());
        assert!(index.get_path(Path::new("batch/remove.txt"), 0).is_none());
    }

    #[test]
    fn vcs_workspace_prefix_projection_matches_host_path_semantics() {
        assert_eq!(
            path_relative_to_workspace("nested/file.txt", "nested"),
            Some("file.txt".to_owned())
        );
        #[cfg(windows)]
        assert_eq!(
            path_relative_to_workspace("Nested/File.txt", "nested"),
            Some("File.txt".to_owned())
        );
        #[cfg(not(windows))]
        assert_eq!(
            path_relative_to_workspace("Nested/File.txt", "nested"),
            None
        );
    }

    #[tokio::test]
    async fn vcs_operations_remain_confined_to_a_repository_subdirectory() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        let nested = directory.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("inside.txt"), "inside\n").unwrap();
        std::fs::write(directory.path().join("tracked.txt"), "root changed\n").unwrap();

        let host = test_host(&nested).await;
        let context = {
            let mut context = context(&nested);
            context.resource_bindings[0]
                .typed_parameters
                .insert(
                    WORKSPACE_ROOT_PARAMETER.to_owned(),
                    nested.to_string_lossy().into_owned(),
                );
            context
        };

        let status = invoke(&host, context.clone(), "workspace.vcs/status", json!({}))
            .await
            .unwrap();
        let status_paths = status.0["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["path"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(status_paths, vec!["inside.txt"]);

        let diff = invoke(&host, context.clone(), "workspace.vcs/diff", json!({}))
            .await
            .unwrap();
        assert!(!diff.0["patch"].as_str().unwrap().contains("root changed"));

        invoke(
            &host,
            context,
            "workspace.vcs/stage",
            json!({"path": "inside.txt"}),
        )
        .await
        .unwrap();
        let status_after = repository.statuses(None).unwrap();
        assert!(
            status_after.iter().any(|entry| {
                entry.path() == Some("nested/inside.txt")
                    && entry.status().contains(git2::Status::INDEX_NEW)
            })
        );
        assert!(
            status_after.iter().any(|entry| {
                entry.path() == Some("tracked.txt")
                    && entry.status().contains(git2::Status::WT_MODIFIED)
            })
        );
    }

    #[tokio::test]
    async fn vcs_commit_commits_only_staged_changes_in_the_bound_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "base\nchanged\n").unwrap();
        {
            let mut config = repository.config().unwrap();
            config.set_str("user.name", "NomiFun Test").unwrap();
            config.set_str("user.email", "nomifun-test@nomifun.invalid").unwrap();
        }
        let host = test_host(directory.path()).await;
        let context = context(directory.path());

        invoke(
            &host,
            context.clone(),
            "workspace.vcs/stage",
            json!({"path": "tracked.txt"}),
        )
        .await
        .unwrap();
        let committed = invoke(
            &host,
            context.clone(),
            "workspace.vcs/commit",
            json!({"message": "record workspace change"}),
        )
        .await
        .unwrap();
        assert_eq!(committed.0["committed"], true);
        assert_eq!(committed.0["message"], "record workspace change");
        assert_eq!(
            committed.0["paths"],
            json!(["tracked.txt"])
        );

        let head = repository.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message(), Some("record workspace change"));
        assert!(repository.statuses(None).unwrap().is_empty());

        let mut retry_context = context;
        retry_context.idempotency_key = IdempotencyKey::from("vcs-commit-empty-retry");
        let error = invoke(&host, retry_context, "workspace.vcs/commit", json!({"message": "empty"}))
            .await
            .unwrap_err();
        assert_eq!(error.code, "CAPABILITY_UNAVAILABLE");
    }

    #[tokio::test]
    async fn vcs_commit_replays_the_original_commit_result_for_the_same_key() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "new\n").unwrap();
        {
            let mut config = repository.config().unwrap();
            config.set_str("user.name", "NomiFun Test").unwrap();
            config.set_str("user.email", "nomifun-test@nomifun.invalid").unwrap();
        }
        let host = test_host(directory.path()).await;
        let context = context(directory.path());
        invoke(
            &host,
            context.clone(),
            "workspace.vcs/stage",
            json!({"path": "tracked.txt"}),
        )
        .await
        .unwrap();
        let first = invoke(
            &host,
            context.clone(),
            "workspace.vcs/commit",
            json!({"message": "replayable commit"}),
        )
        .await
        .unwrap();
        let replay = invoke(
            &host,
            context,
            "workspace.vcs/commit",
            json!({"message": "replayable commit"}),
        )
        .await
        .unwrap();
        assert_eq!(replay, first);
        assert_eq!(repository.head().unwrap().target(), first.0["commit_id"].as_str().and_then(|id| git2::Oid::from_str(id).ok()));
    }

    #[tokio::test]
    async fn vcs_push_updates_a_real_bare_remote_and_replays_the_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let worktree = directory.path().join("worktree");
        let remote_path = directory.path().join("remote.git");
        std::fs::create_dir(&worktree).unwrap();
        let repository = initialize_git_repository(&worktree);
        git2::Repository::init_bare(&remote_path).unwrap();
        repository
            .remote("origin", remote_path.to_str().unwrap())
            .unwrap();
        let expected = repository.head().unwrap().target().unwrap();
        let host = test_host(&worktree).await;
        let context = context(&worktree);
        let input = json!({
            "remote": "origin",
            "refspec": "HEAD:refs/heads/main"
        });

        let first = invoke(
            &host,
            context.clone(),
            "workspace.vcs/push",
            input.clone(),
        )
        .await
        .unwrap();
        let replay = invoke(&host, context, "workspace.vcs/push", input).await.unwrap();

        assert_eq!(replay, first);
        assert_eq!(first.0["remote"], "origin");
        assert_eq!(first.0["remote_commit_after"], expected.to_string());
        assert_eq!(
            git2::Repository::open_bare(&remote_path)
                .unwrap()
                .find_reference("refs/heads/main")
                .unwrap()
                .target(),
            Some(expected)
        );
    }

    #[tokio::test]
    async fn vcs_push_replays_a_not_applied_failure_without_late_execution() {
        let directory = tempfile::tempdir().unwrap();
        let worktree = directory.path().join("worktree");
        let remote_path = directory.path().join("remote.git");
        std::fs::create_dir(&worktree).unwrap();
        let repository = initialize_git_repository(&worktree);
        git2::Repository::init_bare(&remote_path).unwrap();
        let host = test_host(&worktree).await;
        let context = context(&worktree);
        let input = json!({
            "remote": "origin",
            "refspec": "HEAD:refs/heads/main"
        });

        let first = invoke(
            &host,
            context.clone(),
            "workspace.vcs/push",
            input.clone(),
        )
        .await
        .unwrap_err();
        assert_eq!(first.code, "RESOURCE_NOT_FOUND");
        repository
            .remote("origin", remote_path.to_str().unwrap())
            .unwrap();

        let replay = invoke(&host, context, "workspace.vcs/push", input)
            .await
            .unwrap_err();
        assert_eq!(replay.code, first.code);
        assert_eq!(replay.message, first.message);
        assert!(
            git2::Repository::open_bare(&remote_path)
                .unwrap()
                .find_reference("refs/heads/main")
                .is_err()
        );
    }

    #[tokio::test]
    async fn vcs_commit_rejects_staged_paths_outside_a_nested_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        let nested = directory.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(directory.path().join("tracked.txt"), "root changed\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.write().unwrap();

        let host = test_host(&nested).await;
        let context = {
            let mut context = context(&nested);
            context.resource_bindings[0]
                .typed_parameters
                .insert(
                    WORKSPACE_ROOT_PARAMETER.to_owned(),
                    nested.to_string_lossy().into_owned(),
                );
            context
        };
        let error = invoke(
            &host,
            context,
            "workspace.vcs/commit",
            json!({"message": "must stay scoped"}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
        assert!(repository.head().unwrap().peel_to_commit().unwrap().message() == Some("initial"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn vcs_commit_rejects_literal_backslash_paths_outside_a_nested_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        let nested = directory.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let literal = directory.path().join("nested\\outside.txt");
        std::fs::write(&literal, "outside\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("nested\\outside.txt")).unwrap();
        index.write().unwrap();

        let host = test_host(&nested).await;
        let mut context = context(&nested);
        context.resource_bindings[0]
            .typed_parameters
            .insert(
                WORKSPACE_ROOT_PARAMETER.to_owned(),
                nested.to_string_lossy().into_owned(),
            );
        let error = invoke(
            &host,
            context,
            "workspace.vcs/commit",
            json!({"message": "reject literal separator"}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
        assert_eq!(
            repository.head().unwrap().peel_to_commit().unwrap().message(),
            Some("initial")
        );
    }

    #[tokio::test]
    async fn vcs_read_actions_do_not_allow_stage_or_commit_without_write_grant() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "changed\n").unwrap();
        let host = test_host(directory.path()).await;
        let mut read_only = context(directory.path());
        read_only.resource_bindings[0].operations =
            BTreeSet::from(["read".to_owned()]);

        let status = invoke(&host, read_only.clone(), "workspace.vcs/status", json!({}))
            .await
            .unwrap();
        assert_eq!(status.0["entries"][0]["path"], "tracked.txt");

        let stage_error = invoke(
            &host,
            read_only.clone(),
            "workspace.vcs/stage",
            json!({"path": "tracked.txt"}),
        )
        .await
        .unwrap_err();
        assert_eq!(stage_error.code, "PRESET_RESOURCE_NOT_BOUND");

        let commit_error = invoke(
            &host,
            read_only,
            "workspace.vcs/commit",
            json!({"message": "must be denied"}),
        )
        .await
        .unwrap_err();
        assert_eq!(commit_error.code, "PRESET_RESOURCE_NOT_BOUND");
        assert_eq!(
            repository.head().unwrap().peel_to_commit().unwrap().message(),
            Some("initial")
        );
    }

    fn configure_git_identity(repository: &git2::Repository) {
        let mut config = repository.config().unwrap();
        config.set_str("user.name", "NomiFun Test").unwrap();
        config
            .set_str("user.email", "nomifun-test@nomifun.invalid")
            .unwrap();
    }

    fn distinct_context(root: &Path, suffix: &str) -> Wave2HostContext {
        let mut value = context(root);
        value.operation_id = OperationId::from(format!("operation-{suffix}"));
        value.idempotency_key = IdempotencyKey::from(format!("idempotency-{suffix}"));
        value.correlation_id = CorrelationId::from(format!("correlation-{suffix}"));
        value
    }

    #[tokio::test]
    async fn stage_and_commit_share_one_workspace_git_mutation_gate() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        configure_git_identity(&repository);
        std::fs::write(directory.path().join("tracked.txt"), "changed\n").unwrap();
        let hook = GitMutationTestHook::new("workspace.vcs/stage");
        let host = Arc::new(
            test_host(directory.path())
                .await
                .with_git_mutation_hook(Arc::clone(&hook)),
        );
        let stage_host = Arc::clone(&host);
        let stage_root = directory.path().to_path_buf();
        let stage = tokio::spawn(async move {
            invoke(
                &stage_host,
                distinct_context(&stage_root, "stage-gate"),
                "workspace.vcs/stage",
                json!({"path": "tracked.txt"}),
            )
            .await
        });
        hook.entered.notified().await;
        let commit_host = Arc::clone(&host);
        let commit_root = directory.path().to_path_buf();
        let mut commit = tokio::spawn(async move {
            invoke(
                &commit_host,
                distinct_context(&commit_root, "commit-after-stage"),
                "workspace.vcs/commit",
                json!({"message": "serialized stage"}),
            )
            .await
        });
        assert!(tokio::time::timeout(Duration::from_millis(100), &mut commit).await.is_err());
        hook.release.notify_one();
        stage.await.unwrap().unwrap();
        commit.await.unwrap().unwrap();
        assert_eq!(repository.head().unwrap().peel_to_commit().unwrap().message(), Some("serialized stage"));
    }

    #[tokio::test]
    async fn concurrent_commits_cannot_both_commit_one_index_state() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        configure_git_identity(&repository);
        std::fs::write(directory.path().join("tracked.txt"), "changed\n").unwrap();
        let hook = GitMutationTestHook::new("workspace.vcs/commit");
        let host = Arc::new(
            test_host(directory.path())
                .await
                .with_git_mutation_hook(Arc::clone(&hook)),
        );
        invoke(
            &host,
            distinct_context(directory.path(), "stage-before-commits"),
            "workspace.vcs/stage",
            json!({"path": "tracked.txt"}),
        )
        .await
        .unwrap();
        let first_host = Arc::clone(&host);
        let first_root = directory.path().to_path_buf();
        let first = tokio::spawn(async move {
            invoke(&first_host, distinct_context(&first_root, "commit-one"), "workspace.vcs/commit", json!({"message": "first"})).await
        });
        hook.entered.notified().await;
        let second_host = Arc::clone(&host);
        let second_root = directory.path().to_path_buf();
        let mut second = tokio::spawn(async move {
            invoke(&second_host, distinct_context(&second_root, "commit-two"), "workspace.vcs/commit", json!({"message": "second"})).await
        });
        assert!(tokio::time::timeout(Duration::from_millis(100), &mut second).await.is_err());
        hook.release.notify_one();
        first.await.unwrap().unwrap();
        assert!(second.await.unwrap().is_err());
        let head = repository.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message(), Some("first"));
        assert_eq!(head.parent_count(), 1);
        assert_eq!(head.parent(0).unwrap().message(), Some("initial"));
    }

    #[tokio::test]
    async fn push_waits_for_commit_and_observes_the_new_head() {
        let directory = tempfile::tempdir().unwrap();
        let worktree = directory.path().join("worktree");
        let remote = directory.path().join("remote.git");
        std::fs::create_dir(&worktree).unwrap();
        let repository = initialize_git_repository(&worktree);
        configure_git_identity(&repository);
        git2::Repository::init_bare(&remote).unwrap();
        repository.remote("origin", remote.to_str().unwrap()).unwrap();
        std::fs::write(worktree.join("tracked.txt"), "changed\n").unwrap();
        let hook = GitMutationTestHook::new("workspace.vcs/commit");
        let host = Arc::new(
            test_host(&worktree)
                .await
                .with_git_mutation_hook(Arc::clone(&hook)),
        );
        invoke(&host, distinct_context(&worktree, "stage-before-push"), "workspace.vcs/stage", json!({"path":"tracked.txt"})).await.unwrap();
        let commit_host = Arc::clone(&host);
        let commit_root = worktree.clone();
        let commit = tokio::spawn(async move {
            invoke(&commit_host, distinct_context(&commit_root, "commit-before-push"), "workspace.vcs/commit", json!({"message":"before push"})).await
        });
        hook.entered.notified().await;
        let push_host = Arc::clone(&host);
        let push_root = worktree.clone();
        let mut push = tokio::spawn(async move {
            invoke(&push_host, distinct_context(&push_root, "push-after-commit"), "workspace.vcs/push", json!({"remote":"origin","refspec":"HEAD:refs/heads/main"})).await
        });
        assert!(tokio::time::timeout(Duration::from_millis(100), &mut push).await.is_err());
        hook.release.notify_one();
        commit.await.unwrap().unwrap();
        push.await.unwrap().unwrap();
        let local = repository.head().unwrap().target().unwrap();
        let remote_head = git2::Repository::open_bare(&remote).unwrap().find_reference("refs/heads/main").unwrap().target().unwrap();
        assert_eq!(remote_head, local);
    }
}
