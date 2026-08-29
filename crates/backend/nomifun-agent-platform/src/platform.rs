//! Shared production composition for the Fresh-v4 Agent platform.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::http::StatusCode;
use nomifun_agent_contracts::{
    AgentBindingValue, AgentPreset, AgentPresetId, AgentPresetRevision, AgentPresetSource,
    AgentSessionId, AgentSessionLiveRecord, AgentSessionMetadata, CanonicalDigestError,
    CapabilityId, CorrelationId, DeleteAgentSessionCommand, DigestHex, EffectClass, EventId,
    EventProducerId, FullAutoExecutionWire, IdempotencyKey, NativeActionStart,
    NativeActionStartAck, OperationId, PluginStateEntry, PresetRevisionRef, PrincipalRef,
    RemoteBinding, RemoteBindingId, RemoteBindingProvenance, ResolvedSnapshotEnvelope,
    RuntimeBindingContract, RuntimeBindingId, RuntimeCancelParams, RuntimeCommand,
    RuntimeCommandContext, RuntimeCreateParams, RuntimeEventEnvelope, RuntimeEventWireAck,
    RuntimeEventWireEnvelope, RuntimeProfileKind, RuntimeSessionDisposeParams,
    RuntimeStartTurnParams, ScopeKey, SemanticSessionEventDraft, SessionEventAppend,
    SessionEventCursor, SessionEventKind, SessionEventPayloadRef, StrictJsonValue,
    TypedResourceBindings, UserId, VersionString, canonical_json_bytes, digest_payload,
};
use nomifun_agent_control_plane::{
    AgentBindingTarget, AgentControlPlane, CatalogProvider, CatalogSnapshot, CompilerReleaseInputs,
    ControlPlaneError, ControlPlaneStore, OfficialTemplateCatalog, PresetPreviewCompiler,
    StoredAgentBinding, StoredPreset,
};
use nomifun_agent_kernel::{
    ActivationOutcome, ActiveCapabilitySetSnapshot, AgentPresetCompiler,
    CapabilityInvocationRequest, CompileRequest, CompiledSnapshot, CompilerEnvironment,
    CompletedTurnBoundary, KernelError, KernelRegistry, MaterializationPolicy, PluginRegistration,
    PluginStateError, PluginStatePersistence, PluginStateSnapshot, SessionCapabilityState,
    StateIdentity, ThinAuthority,
};
use nomifun_agent_session::{
    AgentSessionStore, CreateSessionRequest, DeleteResult, EffectEventRequest, ForkRequest,
    EffectTerminalState, ForkResult, RuntimeAppendContext, SessionCreateResult,
    SessionEventAppendResult, SessionEventPage, SessionHeadProjection, SessionObservation,
    SessionRehydrationInput, SessionStoreError, ZeroOutstandingProof,
};
use nomifun_api_types::{
    CreateAgentSessionRequestDto, CreateAgentSessionResponseDto, CreateAgentSessionTurnRequestDto,
    CreateAgentSessionTurnResponseDto, SessionCursorDto,
};
use nomifun_chat_model_broker::{
    ChatBrokerPort, ChatModelError, ChatModelRequest, ChatModelStream,
};
use nomifun_codex_runtime::{
    CodexRuntimeSupervisor, ManagedRuntimeSession, RuntimeDisposeReport, RuntimeError,
    RuntimeIngressPort, RuntimeLaunchRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Sqlite, SqlitePool, Transaction};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AgentPlatformError {
    #[error("control-plane error: {0}")]
    ControlPlane(#[from] ControlPlaneError),
    #[error("AgentSession error: {0}")]
    Session(#[from] SessionStoreError),
    #[error("Kernel error: {0}")]
    Kernel(#[from] KernelError),
    #[error("Codex Runtime error: {0}")]
    Runtime(#[from] RuntimeError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] sqlx::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("canonical digest error: {0}")]
    Digest(#[from] CanonicalDigestError),
    #[error("PluginState error: {0}")]
    PluginState(String),
    #[error("ChatModelBroker error: {0}")]
    Model(String),
    #[error("Agent platform contract error: {0}")]
    Contract(String),
}

impl From<PluginStateError> for AgentPlatformError {
    fn from(error: PluginStateError) -> Self {
        Self::PluginState(error.to_string())
    }
}

impl From<ChatModelError> for AgentPlatformError {
    fn from(error: ChatModelError) -> Self {
        Self::Model(format!("{:?}: {}", error.code, error.message))
    }
}

#[async_trait]
pub trait CodexRuntimePort: Send + Sync {
    async fn launch(
        &self,
        request: RuntimeLaunchRequest,
    ) -> Result<Arc<ManagedRuntimeSession>, RuntimeError>;

    async fn binding(
        &self,
        runtime_binding_id: &RuntimeBindingId,
    ) -> Option<RuntimeBindingContract>;

    async fn command(
        &self,
        runtime_binding_id: &RuntimeBindingId,
        command: &RuntimeCommand,
    ) -> Result<Value, RuntimeError>;

    async fn dispose(
        &self,
        params: RuntimeSessionDisposeParams,
    ) -> Result<RuntimeDisposeReport, RuntimeError>;
}

pub struct SupervisedCodexRuntimePort {
    supervisor: Arc<CodexRuntimeSupervisor>,
}

impl SupervisedCodexRuntimePort {
    pub fn new(supervisor: Arc<CodexRuntimeSupervisor>) -> Self {
        Self { supervisor }
    }

    pub fn supervisor(&self) -> &Arc<CodexRuntimeSupervisor> {
        &self.supervisor
    }
}

#[async_trait]
impl CodexRuntimePort for SupervisedCodexRuntimePort {
    async fn launch(
        &self,
        request: RuntimeLaunchRequest,
    ) -> Result<Arc<ManagedRuntimeSession>, RuntimeError> {
        self.supervisor.launch(request).await
    }

    async fn binding(
        &self,
        runtime_binding_id: &RuntimeBindingId,
    ) -> Option<RuntimeBindingContract> {
        self.supervisor
            .session(runtime_binding_id)
            .await
            .map(|session| session.binding().clone())
    }

    async fn command(
        &self,
        runtime_binding_id: &RuntimeBindingId,
        command: &RuntimeCommand,
    ) -> Result<Value, RuntimeError> {
        let session = self
            .supervisor
            .session(runtime_binding_id)
            .await
            .ok_or(RuntimeError::SessionNotFound)?;
        session.client().command(command).await
    }

    async fn dispose(
        &self,
        params: RuntimeSessionDisposeParams,
    ) -> Result<RuntimeDisposeReport, RuntimeError> {
        let runtime_binding_id = params.runtime_binding_id.clone();
        let report = self.supervisor.dispose(params).await?;
        self.supervisor.evict_disposed(&runtime_binding_id).await;
        Ok(report)
    }
}

pub struct AgentPlatformConfig {
    pub pool: SqlitePool,
    pub materialization_policy: MaterializationPolicy,
    pub control_plane_release: CompilerReleaseInputs,
    pub kernel_environment: CompilerEnvironment,
    pub runtime: Arc<dyn CodexRuntimePort>,
    pub broker: Arc<dyn ChatBrokerPort>,
    pub initial_plugins: Vec<PluginRegistration>,
}

impl AgentPlatformConfig {
    pub fn with_supervisor(
        pool: SqlitePool,
        materialization_policy: MaterializationPolicy,
        control_plane_release: CompilerReleaseInputs,
        kernel_environment: CompilerEnvironment,
        supervisor: Arc<CodexRuntimeSupervisor>,
        broker: Arc<dyn ChatBrokerPort>,
    ) -> Self {
        Self {
            pool,
            materialization_policy,
            control_plane_release,
            kernel_environment,
            runtime: Arc::new(SupervisedCodexRuntimePort::new(supervisor)),
            broker,
            initial_plugins: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct OpenAgentSessionRequest {
    pub requested_session_id: Option<AgentSessionId>,
    pub owner_ref: PrincipalRef,
    pub agent_binding: AgentBindingValue,
    pub metadata: AgentSessionMetadata,
    pub remote_binding_provenance: Option<RemoteBindingProvenance>,
    pub operation_id: OperationId,
    pub producer_id: EventProducerId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub created_at: i64,
    pub scene: String,
    pub surface: String,
    pub audience: String,
}

impl OpenAgentSessionRequest {
    pub fn user(
        owner: &UserId,
        agent_binding: AgentBindingValue,
        idempotency_key: impl Into<IdempotencyKey>,
    ) -> Self {
        let idempotency_key = idempotency_key.into();
        Self {
            requested_session_id: None,
            owner_ref: user_principal(owner),
            agent_binding,
            metadata: AgentSessionMetadata {
                title: None,
                archived: false,
                pinned: false,
            },
            remote_binding_provenance: None,
            operation_id: OperationId::from(format!(
                "session-open:{}",
                idempotency_key.as_ref()
            )),
            producer_id: EventProducerId::from(format!("session_api:{}", owner.as_ref())),
            correlation_id: CorrelationId::from(format!(
                "session-open:{}",
                idempotency_key.as_ref()
            )),
            idempotency_key,
            created_at: now_ms(),
            scene: "chat".to_owned(),
            surface: "desktop".to_owned(),
            audience: "owner".to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StartAgentTurnRequest {
    pub agent_session_id: AgentSessionId,
    pub principal: PrincipalRef,
    pub input: StrictJsonValue,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Clone, Debug)]
pub struct AgentTurnDispatch {
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub input_event: SessionEventAppendResult,
    pub turn_event: SessionEventAppendResult,
    pub runtime_response: Value,
}

#[derive(Clone, Debug)]
pub struct ActivateCapabilityRequest {
    pub agent_session_id: AgentSessionId,
    pub principal: PrincipalRef,
    pub capability_id: CapabilityId,
    pub expected_generation: u64,
    pub completed_turn_operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Clone, Debug)]
pub struct InvokeCapabilityCommand {
    pub agent_session_id: AgentSessionId,
    pub invocation: CapabilityInvocationRequest,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
}

#[async_trait]
pub trait AgentSessionCommandPort: Send + Sync {
    async fn open_session(
        &self,
        request: OpenAgentSessionRequest,
    ) -> Result<SessionCreateResult, AgentPlatformError>;

    async fn append_event(
        &self,
        append: &SessionEventAppend,
    ) -> Result<SessionEventAppendResult, AgentPlatformError>;

    async fn start_turn(
        &self,
        request: StartAgentTurnRequest,
    ) -> Result<AgentTurnDispatch, AgentPlatformError>;

    async fn activate_capability(
        &self,
        request: ActivateCapabilityRequest,
    ) -> Result<nomifun_agent_kernel::ActivationOutcome, AgentPlatformError>;

    async fn invoke_capability(
        &self,
        command: InvokeCapabilityCommand,
    ) -> Result<StrictJsonValue, AgentPlatformError>;

    async fn fork_session(
        &self,
        parent_session_id: &AgentSessionId,
        request: ForkRequest,
    ) -> Result<ForkResult, AgentPlatformError>;
}

#[async_trait]
pub trait AgentSessionQueryPort: Send + Sync {
    async fn observe_session(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
        after: Option<&SessionEventCursor>,
        limit: u32,
    ) -> Result<SessionObservation, AgentPlatformError>;

    async fn session_head(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
    ) -> Result<SessionHeadProjection, AgentPlatformError>;

    async fn session_events(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
        after: Option<&SessionEventCursor>,
        limit: u32,
    ) -> Result<SessionEventPage, AgentPlatformError>;

    async fn rehydration_input(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
    ) -> Result<SessionRehydrationInput, AgentPlatformError>;
}

#[async_trait]
pub trait AgentSessionDeletePort: Send + Sync {
    async fn delete_session(
        &self,
        command: DeleteAgentSessionCommand,
        deleted_at: i64,
    ) -> Result<DeleteResult, AgentPlatformError>;
}

struct SessionExecutionState {
    compiled: Arc<CompiledSnapshot>,
    capabilities: Arc<SessionCapabilityState>,
    runtime_binding: RwLock<Option<RuntimeBindingContract>>,
}

pub struct AgentPlatform {
    pool: SqlitePool,
    control_store: Arc<SqliteControlPlaneStore>,
    control_plane: Arc<AgentControlPlane>,
    sessions: Arc<AgentSessionStore>,
    kernel: Arc<KernelRegistry>,
    kernel_environment: CompilerEnvironment,
    runtime: Arc<dyn CodexRuntimePort>,
    broker: Arc<dyn ChatBrokerPort>,
    executions: RwLock<BTreeMap<AgentSessionId, Arc<SessionExecutionState>>>,
    opening_bindings: RwLock<BTreeMap<RuntimeBindingId, AgentSessionId>>,
    registrations: RwLock<Vec<PluginRegistration>>,
    publish_lock: Mutex<()>,
}

pub struct TriadHarness {
    platform: Arc<AgentPlatform>,
    owner: UserId,
}

impl TriadHarness {
    pub fn new(platform: Arc<AgentPlatform>, owner: UserId) -> Self {
        Self { platform, owner }
    }

    pub fn platform(&self) -> &Arc<AgentPlatform> {
        &self.platform
    }

    pub fn owner(&self) -> &UserId {
        &self.owner
    }

    pub async fn publish_plugins(
        &self,
        registrations: Vec<PluginRegistration>,
    ) -> Result<Arc<nomifun_agent_kernel::MaterializedRegistry>, AgentPlatformError> {
        self.platform.publish_plugins(registrations).await
    }

    pub async fn open_session(
        &self,
        request: CreateAgentSessionRequestDto,
        idempotency_key: impl Into<IdempotencyKey>,
    ) -> Result<CreateAgentSessionResponseDto, AgentPlatformError> {
        self.platform
            .create_session_from_dto(&self.owner, request, idempotency_key)
            .await
    }

    pub async fn start_turn(
        &self,
        session_id: &str,
        request: CreateAgentSessionTurnRequestDto,
    ) -> Result<CreateAgentSessionTurnResponseDto, AgentPlatformError> {
        self.platform
            .start_turn_from_dto(&self.owner, session_id, request)
            .await
    }

    pub async fn observe(
        &self,
        session_id: &str,
        after: Option<SessionCursorDto>,
        limit: u32,
    ) -> Result<SessionObservation, AgentPlatformError> {
        self.platform
            .observe_from_cursor(&self.owner, session_id, after, limit)
            .await
    }

    pub async fn delete_session(
        &self,
        session_id: &str,
    ) -> Result<DeleteResult, AgentPlatformError> {
        let requested_at = now_ms();
        AgentSessionDeletePort::delete_session(
            self.platform.as_ref(),
            DeleteAgentSessionCommand {
                operation_id: OperationId::from(format!(
                    "triad-delete:{}",
                    session_id
                )),
                agent_session_id: AgentSessionId::from(session_id.to_owned()),
                owner_ref: user_principal(&self.owner),
                requested_at,
            },
            requested_at.saturating_add(1),
        )
        .await
    }
}

fn user_principal(owner: &UserId) -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: owner.as_ref().to_owned(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn platform_cast<T: Serialize, U: for<'de> Deserialize<'de>>(
    value: &T,
) -> Result<U, AgentPlatformError> {
    Ok(serde_json::from_value(serde_json::to_value(value)?)?)
}

fn cursor_dto(cursor: &SessionEventCursor) -> SessionCursorDto {
    SessionCursorDto {
        agent_session_id: cursor.agent_session_id.as_ref().to_owned(),
        seq: cursor.seq,
    }
}

// The persistent adapters and orchestration implementations continue below.

#[derive(Clone)]
pub struct SqliteControlPlaneStore {
    pool: SqlitePool,
}

impl SqliteControlPlaneStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedPresetDisplay {
    display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedRuntimeProfile {
    profile_kind: RuntimeProfileKind,
    runtime_protocol_version: VersionString,
    enabled_runtime_features: BTreeSet<nomifun_agent_contracts::RuntimeFeatureId>,
    initial_capabilities: BTreeSet<CapabilityId>,
    on_demand_capabilities: BTreeSet<CapabilityId>,
    typed_resource_bindings: TypedResourceBindings,
}

#[async_trait]
impl ControlPlaneStore for SqliteControlPlaneStore {
    async fn list_presets(
        &self,
        owner: &UserId,
    ) -> Result<Vec<StoredPreset>, ControlPlaneError> {
        let rows: Vec<(String, String, String, String, Option<i64>)> = sqlx::query_as(
            "SELECT preset_id, owner_ref_json, source_json, display_json, \
                    current_stable_revision \
             FROM agent_presets ORDER BY preset_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(control_sql)?;
        let mut presets = Vec::new();
        for row in rows {
            let preset = preset_from_row(&self.pool, row).await?;
            if preset.preset.owner_user_id.as_ref() == Some(owner) {
                presets.push(preset);
            }
        }
        Ok(presets)
    }

    async fn get_preset(
        &self,
        preset_id: &AgentPresetId,
    ) -> Result<Option<StoredPreset>, ControlPlaneError> {
        let row: Option<(String, String, String, String, Option<i64>)> = sqlx::query_as(
            "SELECT preset_id, owner_ref_json, source_json, display_json, \
                    current_stable_revision \
             FROM agent_presets WHERE preset_id = ?",
        )
        .bind(preset_id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(control_sql)?;
        match row {
            Some(row) => preset_from_row(&self.pool, row).await.map(Some),
            None => Ok(None),
        }
    }

    async fn insert_preset(&self, preset: StoredPreset) -> Result<(), ControlPlaneError> {
        let mut tx = self.pool.begin().await.map_err(control_sql)?;
        insert_preset_tx(&mut tx, &preset, now_ms()).await?;
        tx.commit().await.map_err(control_sql)?;
        Ok(())
    }

    async fn insert_preset_with_revision(
        &self,
        preset: StoredPreset,
        revision: AgentPresetRevision,
        snapshot: ResolvedSnapshotEnvelope,
    ) -> Result<StoredPreset, ControlPlaneError> {
        validate_revision_snapshot(&preset, &revision, &snapshot)?;
        let mut tx = self.pool.begin().await.map_err(control_sql)?;
        insert_preset_tx(&mut tx, &preset, revision.created_at_ms).await?;
        insert_revision_snapshot_tx(&mut tx, &revision, &snapshot).await?;
        tx.commit().await.map_err(control_sql)?;
        Ok(preset)
    }

    async fn update_preset(&self, preset: StoredPreset) -> Result<(), ControlPlaneError> {
        let owner = encode_control_json(&preset_owner_ref(&preset.preset))?;
        let source = encode_control_json(&preset.preset.source)?;
        let display = encode_control_json(&PersistedPresetDisplay {
            display_name: preset.preset.display_name.clone(),
            description: preset.preset.description.clone(),
        })?;
        let current = preset
            .preset
            .current_stable_revision
            .as_ref()
            .map(|reference| i64_from_u64(reference.revision, "revision"))
            .transpose()?;
        let changed = sqlx::query(
            "UPDATE agent_presets SET owner_ref_json = ?, source_json = ?, display_json = ?, \
                    current_stable_revision = ? WHERE preset_id = ?",
        )
        .bind(owner)
        .bind(source)
        .bind(display)
        .bind(current)
        .bind(preset.preset.preset_id.as_ref())
        .execute(&self.pool)
        .await
        .map_err(control_sql)?;
        if changed.rows_affected() != 1 {
            return Err(control_not_found("AgentPreset"));
        }
        Ok(())
    }

    async fn get_revision(
        &self,
        reference: &PresetRevisionRef,
    ) -> Result<Option<AgentPresetRevision>, ControlPlaneError> {
        let revision = load_revision(&self.pool, &reference.preset_id, reference.revision).await?;
        Ok(revision.filter(|revision| {
            revision.reference.revision_digest == reference.revision_digest
        }))
    }

    async fn get_revision_number(
        &self,
        preset_id: &AgentPresetId,
        revision: u64,
    ) -> Result<Option<AgentPresetRevision>, ControlPlaneError> {
        load_revision(&self.pool, preset_id, revision).await
    }

    async fn append_revision(
        &self,
        expected_current: Option<&PresetRevisionRef>,
        revision: AgentPresetRevision,
        snapshot: ResolvedSnapshotEnvelope,
        display_name: String,
        description: Option<String>,
    ) -> Result<StoredPreset, ControlPlaneError> {
        revision
            .validate()
            .map_err(|violation| {
                ControlPlaneError::canonical(
                    violation.code,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    violation.message,
                )
            })?;
        snapshot.validate().map_err(|violation| {
            ControlPlaneError::canonical(
                violation.code,
                StatusCode::UNPROCESSABLE_ENTITY,
                violation.message,
            )
        })?;
        if snapshot.content.preset_revision_ref != revision.reference {
            return Err(control_conflict(
                "ResolvedSnapshot does not bind the appended Preset revision",
            ));
        }

        let mut tx = self.pool.begin().await.map_err(control_sql)?;
        let current = current_revision_ref_tx(&mut tx, &revision.reference.preset_id).await?;
        if current.as_ref() != expected_current {
            return Err(control_conflict(
                "expected_current_revision does not match the persisted Preset",
            ));
        }
        insert_revision_snapshot_tx(&mut tx, &revision, &snapshot).await?;
        let display = encode_control_json(&PersistedPresetDisplay {
            display_name,
            description,
        })?;
        let changed = sqlx::query(
            "UPDATE agent_presets SET display_json = ?, current_stable_revision = ? \
             WHERE preset_id = ?",
        )
        .bind(display)
        .bind(i64_from_u64(revision.reference.revision, "revision")?)
        .bind(revision.reference.preset_id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(control_sql)?;
        if changed.rows_affected() != 1 {
            return Err(control_not_found("AgentPreset"));
        }
        tx.commit().await.map_err(control_sql)?;
        self.get_preset(&revision.reference.preset_id)
            .await?
            .ok_or_else(|| control_not_found("AgentPreset"))
    }

    async fn get_snapshot(
        &self,
        reference: &PresetRevisionRef,
    ) -> Result<Option<ResolvedSnapshotEnvelope>, ControlPlaneError> {
        let row: Option<String> = sqlx::query_scalar(
            "SELECT envelope_json FROM agent_runtime_snapshots \
             WHERE preset_id = ? AND revision_no = ?",
        )
        .bind(reference.preset_id.as_ref())
        .bind(i64_from_u64(reference.revision, "revision")?)
        .fetch_optional(&self.pool)
        .await
        .map_err(control_sql)?;
        let Some(envelope_json) = row else {
            return Ok(None);
        };
        let snapshot: ResolvedSnapshotEnvelope =
            serde_json::from_str(&envelope_json).map_err(ControlPlaneError::from)?;
        snapshot.validate().map_err(|violation| {
            ControlPlaneError::canonical(
                violation.code,
                StatusCode::INTERNAL_SERVER_ERROR,
                violation.message,
            )
        })?;
        if snapshot.content.preset_revision_ref != *reference {
            return Err(ControlPlaneError::Wire(
                "persisted ResolvedSnapshot references a different Preset revision".to_owned(),
            ));
        }
        Ok(Some(snapshot))
    }

    async fn list_agent_bindings(
        &self,
        owner: &UserId,
    ) -> Result<Vec<StoredAgentBinding>, ControlPlaneError> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT target_kind, target_id, agent_binding_json \
             FROM agent_bindings ORDER BY target_kind, target_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(control_sql)?;
        let mut bindings = Vec::new();
        for (target_kind, target_id, value_json) in rows {
            let value: AgentBindingValue =
                serde_json::from_str(&value_json).map_err(ControlPlaneError::from)?;
            let Some(binding_owner) =
                preset_owner(&self.pool, &value.preset_revision_ref.preset_id).await?
            else {
                continue;
            };
            if &binding_owner == owner {
                bindings.push(StoredAgentBinding {
                    target: AgentBindingTarget {
                        target_kind,
                        target_id,
                    },
                    owner_user_id: binding_owner,
                    value,
                });
            }
        }
        Ok(bindings)
    }

    async fn get_agent_binding(
        &self,
        target: &AgentBindingTarget,
    ) -> Result<Option<StoredAgentBinding>, ControlPlaneError> {
        let value_json: Option<String> = sqlx::query_scalar(
            "SELECT agent_binding_json FROM agent_bindings \
             WHERE target_kind = ? AND target_id = ?",
        )
        .bind(&target.target_kind)
        .bind(&target.target_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(control_sql)?;
        let Some(value_json) = value_json else {
            return Ok(None);
        };
        let value: AgentBindingValue =
            serde_json::from_str(&value_json).map_err(ControlPlaneError::from)?;
        let owner_user_id = preset_owner(&self.pool, &value.preset_revision_ref.preset_id)
            .await?
            .ok_or_else(|| ControlPlaneError::Wire("binding Preset owner is missing".to_owned()))?;
        Ok(Some(StoredAgentBinding {
            target: target.clone(),
            owner_user_id,
            value,
        }))
    }

    async fn put_agent_binding(
        &self,
        binding: StoredAgentBinding,
        expected_binding_version: Option<u64>,
    ) -> Result<StoredAgentBinding, ControlPlaneError> {
        let mut tx = self.pool.begin().await.map_err(control_sql)?;
        let existing_json: Option<String> = sqlx::query_scalar(
            "SELECT agent_binding_json FROM agent_bindings \
             WHERE target_kind = ? AND target_id = ?",
        )
        .bind(&binding.target.target_kind)
        .bind(&binding.target.target_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(control_sql)?;
        match existing_json {
            Some(existing_json) => {
                let existing: AgentBindingValue =
                    serde_json::from_str(&existing_json).map_err(ControlPlaneError::from)?;
                if expected_binding_version != Some(existing.binding_version) {
                    return Err(control_conflict("agent binding version changed"));
                }
                let existing_owner =
                    preset_owner_tx(&mut tx, &existing.preset_revision_ref.preset_id)
                        .await?
                        .ok_or_else(|| {
                            ControlPlaneError::Wire(
                                "existing binding Preset owner is missing".to_owned(),
                            )
                        })?;
                if existing_owner != binding.owner_user_id {
                    return Err(control_not_found("AgentBinding"));
                }
            }
            None if expected_binding_version.is_some() => {
                return Err(control_conflict(
                    "agent binding does not exist at the expected version",
                ));
            }
            None => {}
        }
        let owner = preset_owner_tx(
            &mut tx,
            &binding.value.preset_revision_ref.preset_id,
        )
        .await?
        .ok_or_else(|| control_not_found("AgentPreset"))?;
        if owner != binding.owner_user_id {
            return Err(control_not_found("AgentPreset"));
        }
        sqlx::query(
            "INSERT INTO agent_bindings (target_kind, target_id, agent_binding_json) \
             VALUES (?, ?, ?) \
             ON CONFLICT (target_kind, target_id) DO UPDATE SET \
                 agent_binding_json = excluded.agent_binding_json",
        )
        .bind(&binding.target.target_kind)
        .bind(&binding.target.target_id)
        .bind(encode_control_json(&binding.value)?)
        .execute(&mut *tx)
        .await
        .map_err(control_sql)?;
        tx.commit().await.map_err(control_sql)?;
        Ok(binding)
    }

    async fn list_remote_bindings(
        &self,
        owner: &UserId,
    ) -> Result<Vec<RemoteBinding>, ControlPlaneError> {
        let rows: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT remote_binding_id, owner_user_id, name, agent_binding_json \
             FROM remote_bindings WHERE owner_user_id = ? ORDER BY remote_binding_id",
        )
        .bind(owner.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(control_sql)?;
        rows.into_iter()
            .map(remote_binding_from_row)
            .collect()
    }

    async fn get_remote_binding(
        &self,
        binding_id: &RemoteBindingId,
    ) -> Result<Option<RemoteBinding>, ControlPlaneError> {
        let row: Option<(String, String, String, String)> = sqlx::query_as(
            "SELECT remote_binding_id, owner_user_id, name, agent_binding_json \
             FROM remote_bindings WHERE remote_binding_id = ?",
        )
        .bind(binding_id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(control_sql)?;
        row.map(remote_binding_from_row).transpose()
    }

    async fn insert_remote_binding(
        &self,
        binding: RemoteBinding,
    ) -> Result<RemoteBinding, ControlPlaneError> {
        sqlx::query(
            "INSERT INTO remote_bindings \
             (remote_binding_id, owner_user_id, name, agent_binding_json) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(binding.remote_binding_id.as_ref())
        .bind(binding.owner_user_id.as_ref())
        .bind(&binding.name)
        .bind(encode_control_json(&binding.agent_binding)?)
        .execute(&self.pool)
        .await
        .map_err(control_sql)?;
        Ok(binding)
    }

    async fn update_remote_binding(
        &self,
        binding: RemoteBinding,
        expected_binding_version: u64,
        expected_agent_binding_digest: &str,
    ) -> Result<RemoteBinding, ControlPlaneError> {
        let mut tx = self.pool.begin().await.map_err(control_sql)?;
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT owner_user_id, agent_binding_json FROM remote_bindings \
             WHERE remote_binding_id = ?",
        )
        .bind(binding.remote_binding_id.as_ref())
        .fetch_optional(&mut *tx)
        .await
        .map_err(control_sql)?;
        let Some((owner_user_id, existing_json)) = row else {
            return Err(control_not_found("RemoteBinding"));
        };
        if owner_user_id != binding.owner_user_id.as_ref() {
            return Err(control_not_found("RemoteBinding"));
        }
        let existing: AgentBindingValue =
            serde_json::from_str(&existing_json).map_err(ControlPlaneError::from)?;
        let existing_digest = digest_payload(&existing)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        if existing.binding_version != expected_binding_version
            || existing_digest.as_ref() != expected_agent_binding_digest
        {
            return Err(ControlPlaneError::canonical(
                "REMOTE_BINDING_VERSION_CONFLICT",
                StatusCode::CONFLICT,
                "RemoteBinding version or digest changed",
            ));
        }
        sqlx::query(
            "UPDATE remote_bindings SET name = ?, agent_binding_json = ? \
             WHERE remote_binding_id = ? AND owner_user_id = ?",
        )
        .bind(&binding.name)
        .bind(encode_control_json(&binding.agent_binding)?)
        .bind(binding.remote_binding_id.as_ref())
        .bind(binding.owner_user_id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(control_sql)?;
        tx.commit().await.map_err(control_sql)?;
        Ok(binding)
    }

    async fn delete_remote_binding(
        &self,
        owner: &UserId,
        binding_id: &RemoteBindingId,
    ) -> Result<(), ControlPlaneError> {
        let result = sqlx::query(
            "DELETE FROM remote_bindings \
             WHERE remote_binding_id = ? AND owner_user_id = ?",
        )
        .bind(binding_id.as_ref())
        .bind(owner.as_ref())
        .execute(&self.pool)
        .await
        .map_err(control_sql)?;
        if result.rows_affected() != 1 {
            return Err(control_not_found("RemoteBinding"));
        }
        Ok(())
    }
}

fn control_sql(error: sqlx::Error) -> ControlPlaneError {
    ControlPlaneError::Wire(error.to_string())
}

fn control_conflict(message: impl Into<String>) -> ControlPlaneError {
    ControlPlaneError::canonical(
        "PRESET_REVISION_DIGEST_MISMATCH",
        StatusCode::CONFLICT,
        message,
    )
}

fn control_not_found(subject: &str) -> ControlPlaneError {
    ControlPlaneError::canonical(
        "PRESET_REVISION_DIGEST_MISMATCH",
        StatusCode::NOT_FOUND,
        format!("{subject} was not found"),
    )
}

fn encode_control_json<T: Serialize>(value: &T) -> Result<String, ControlPlaneError> {
    String::from_utf8(
        canonical_json_bytes(value)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?,
    )
    .map_err(|error| ControlPlaneError::Wire(error.to_string()))
}

fn wire_string<T: Serialize>(value: &T) -> Result<String, ControlPlaneError> {
    match serde_json::to_value(value).map_err(ControlPlaneError::from)? {
        Value::String(value) => Ok(value),
        _ => Err(ControlPlaneError::Wire(
            "wire enum did not serialize to a string".to_owned(),
        )),
    }
}

fn i64_from_u64(value: u64, field: &str) -> Result<i64, ControlPlaneError> {
    i64::try_from(value).map_err(|_| {
        ControlPlaneError::Wire(format!("{field} exceeds the SQLite i64 range"))
    })
}

fn u64_from_i64(value: i64, field: &str) -> Result<u64, ControlPlaneError> {
    u64::try_from(value).map_err(|_| {
        ControlPlaneError::Wire(format!("{field} is negative in SQLite"))
    })
}

fn preset_owner_ref(preset: &AgentPreset) -> PrincipalRef {
    match &preset.owner_user_id {
        Some(owner) => user_principal(owner),
        None => PrincipalRef {
            principal_kind: "system".to_owned(),
            principal_id: "official".to_owned(),
        },
    }
}

fn owner_from_json(value: &str) -> Result<Option<UserId>, ControlPlaneError> {
    if let Ok(principal) = serde_json::from_str::<PrincipalRef>(value) {
        return Ok((principal.principal_kind == "user")
            .then(|| UserId::from(principal.principal_id)));
    }
    if let Ok(owner) = serde_json::from_str::<Option<UserId>>(value) {
        return Ok(owner);
    }
    Err(ControlPlaneError::Wire(
        "agent_presets.owner_ref_json is not a canonical PrincipalRef".to_owned(),
    ))
}

fn source_from_json(value: &str) -> Result<AgentPresetSource, ControlPlaneError> {
    if let Ok(source) = serde_json::from_str::<AgentPresetSource>(value) {
        return Ok(source);
    }
    let value: Value = serde_json::from_str(value).map_err(ControlPlaneError::from)?;
    let source = value
        .get("source")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ControlPlaneError::Wire(
                "agent_presets.source_json has no canonical source".to_owned(),
            )
        })?;
    serde_json::from_value(Value::String(source.to_owned())).map_err(ControlPlaneError::from)
}

fn display_from_json(value: &str) -> Result<PersistedPresetDisplay, ControlPlaneError> {
    if let Ok(display) = serde_json::from_str::<PersistedPresetDisplay>(value) {
        return Ok(display);
    }
    let value: Value = serde_json::from_str(value).map_err(ControlPlaneError::from)?;
    let display_name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ControlPlaneError::Wire(
                "agent_presets.display_json has no display name".to_owned(),
            )
        })?
        .to_owned();
    let description = value
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(PersistedPresetDisplay {
        display_name,
        description,
    })
}

async fn preset_from_row(
    pool: &SqlitePool,
    row: (String, String, String, String, Option<i64>),
) -> Result<StoredPreset, ControlPlaneError> {
    let (preset_id, owner_json, source_json, display_json, current_revision) = row;
    let preset_id = AgentPresetId::from(preset_id);
    let current_stable_revision = match current_revision {
        Some(revision) => {
            let revision = u64_from_i64(revision, "current_stable_revision")?;
            let digest: Option<String> = sqlx::query_scalar(
                "SELECT revision_digest FROM agent_preset_revisions \
                 WHERE preset_id = ? AND revision_no = ?",
            )
            .bind(preset_id.as_ref())
            .bind(i64_from_u64(revision, "revision")?)
            .fetch_optional(pool)
            .await
            .map_err(control_sql)?;
            Some(PresetRevisionRef {
                preset_id: preset_id.clone(),
                revision,
                revision_digest: DigestHex::from(digest.ok_or_else(|| {
                    ControlPlaneError::Wire(
                        "current_stable_revision has no persisted revision".to_owned(),
                    )
                })?),
            })
        }
        None => None,
    };
    let display = display_from_json(&display_json)?;
    Ok(StoredPreset {
        preset: AgentPreset {
            preset_id,
            owner_user_id: owner_from_json(&owner_json)?,
            source: source_from_json(&source_json)?,
            display_name: display.display_name,
            description: display.description,
            current_stable_revision,
        },
    })
}

async fn insert_preset_tx(
    tx: &mut Transaction<'_, Sqlite>,
    preset: &StoredPreset,
    created_at: i64,
) -> Result<(), ControlPlaneError> {
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_presets WHERE preset_id = ?")
            .bind(preset.preset.preset_id.as_ref())
            .fetch_one(&mut **tx)
            .await
            .map_err(control_sql)?;
    if exists != 0 {
        return Err(control_conflict("AgentPreset already exists"));
    }
    let current = preset
        .preset
        .current_stable_revision
        .as_ref()
        .map(|reference| i64_from_u64(reference.revision, "revision"))
        .transpose()?;
    sqlx::query(
        "INSERT INTO agent_presets \
         (preset_id, owner_ref_json, source_json, display_json, \
          current_stable_revision, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(preset.preset.preset_id.as_ref())
    .bind(encode_control_json(&preset_owner_ref(&preset.preset))?)
    .bind(encode_control_json(&preset.preset.source)?)
    .bind(encode_control_json(&PersistedPresetDisplay {
        display_name: preset.preset.display_name.clone(),
        description: preset.preset.description.clone(),
    })?)
    .bind(current)
    .bind(created_at.max(0))
    .execute(&mut **tx)
    .await
    .map_err(control_sql)?;
    Ok(())
}

fn validate_revision_snapshot(
    preset: &StoredPreset,
    revision: &AgentPresetRevision,
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<(), ControlPlaneError> {
    revision.validate().map_err(|violation| {
        ControlPlaneError::canonical(
            violation.code,
            StatusCode::UNPROCESSABLE_ENTITY,
            violation.message,
        )
    })?;
    snapshot.validate().map_err(|violation| {
        ControlPlaneError::canonical(
            violation.code,
            StatusCode::UNPROCESSABLE_ENTITY,
            violation.message,
        )
    })?;
    if revision.reference.preset_id != preset.preset.preset_id
        || snapshot.content.preset_revision_ref != revision.reference
        || preset.preset.current_stable_revision.as_ref() != Some(&revision.reference)
    {
        return Err(control_conflict(
            "atomic Preset/Revision/Snapshot identities do not match",
        ));
    }
    Ok(())
}

async fn insert_revision_snapshot_tx(
    tx: &mut Transaction<'_, Sqlite>,
    revision: &AgentPresetRevision,
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<(), ControlPlaneError> {
    if snapshot.content.preset_revision_ref != revision.reference {
        return Err(control_conflict(
            "ResolvedSnapshot does not bind the persisted Revision",
        ));
    }
    let revision_id = format!(
        "{}@{}",
        revision.reference.preset_id.as_ref(),
        revision.reference.revision
    );
    sqlx::query(
        "INSERT INTO agent_preset_revisions \
         (revision_id, preset_id, revision_no, schema_version, editor_document_json, \
          revision_digest, created_by, created_at, reason) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&revision_id)
    .bind(revision.reference.preset_id.as_ref())
    .bind(i64_from_u64(revision.reference.revision, "revision")?)
    .bind(revision.payload.schema_version.as_ref())
    .bind(encode_control_json(&revision.payload)?)
    .bind(revision.reference.revision_digest.as_ref())
    .bind(revision.created_by.as_ref())
    .bind(revision.created_at_ms)
    .bind(revision.reason.as_deref().unwrap_or_default())
    .execute(&mut **tx)
    .await
    .map_err(control_sql)?;

    for (task, route) in &revision.payload.model_route_refs {
        sqlx::query(
            "INSERT INTO agent_preset_model_routes \
             (revision_id, model_task, route_json) VALUES (?, ?, ?)",
        )
        .bind(&revision_id)
        .bind(task)
        .bind(encode_control_json(route)?)
        .execute(&mut **tx)
        .await
        .map_err(control_sql)?;
    }
    for selection in &revision.payload.initial_capabilities {
        sqlx::query(
            "INSERT INTO preset_initial_capabilities \
             (revision_id, capability_id, capability_version, selection_json) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(&revision_id)
        .bind(selection.capability.id.as_ref())
        .bind(selection.capability.version.as_ref())
        .bind(encode_control_json(selection)?)
        .execute(&mut **tx)
        .await
        .map_err(control_sql)?;
    }
    for selection in &revision.payload.on_demand_capabilities {
        sqlx::query(
            "INSERT INTO preset_on_demand_capabilities \
             (revision_id, capability_id, capability_version, selection_json) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(&revision_id)
        .bind(selection.capability.id.as_ref())
        .bind(selection.capability.version.as_ref())
        .bind(encode_control_json(selection)?)
        .execute(&mut **tx)
        .await
        .map_err(control_sql)?;
    }
    for skill in &revision.payload.skill_bindings {
        sqlx::query(
            "INSERT INTO preset_skill_bindings \
             (revision_id, skill_id, skill_version) VALUES (?, ?, ?)",
        )
        .bind(&revision_id)
        .bind(skill.id.as_ref())
        .bind(skill.version.as_ref())
        .execute(&mut **tx)
        .await
        .map_err(control_sql)?;
    }
    for binding in &revision.payload.resource_bindings {
        sqlx::query(
            "INSERT INTO preset_resource_bindings \
             (revision_id, resource_binding_id, binding_json) VALUES (?, ?, ?)",
        )
        .bind(&revision_id)
        .bind(binding.binding_id.as_ref())
        .bind(encode_control_json(binding)?)
        .execute(&mut **tx)
        .await
        .map_err(control_sql)?;
    }

    sqlx::query(
        "INSERT INTO agent_runtime_snapshots \
         (snapshot_id, snapshot_digest, preset_id, revision_no, revision_digest, \
          content_json, envelope_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(snapshot.snapshot_ref.snapshot_id.as_ref())
    .bind(snapshot.snapshot_ref.snapshot_digest.as_ref())
    .bind(revision.reference.preset_id.as_ref())
    .bind(i64_from_u64(revision.reference.revision, "revision")?)
    .bind(revision.reference.revision_digest.as_ref())
    .bind(encode_control_json(&snapshot.content)?)
    .bind(encode_control_json(snapshot)?)
    .bind(snapshot.created_at_ms)
    .execute(&mut **tx)
    .await
    .map_err(control_sql)?;

    for capability in &snapshot.content.initial_capabilities {
        insert_snapshot_capability_tx(
            tx,
            snapshot,
            capability,
            "initial",
        )
        .await?;
    }
    for capability in &snapshot.content.on_demand_capabilities {
        insert_snapshot_capability_tx(
            tx,
            snapshot,
            capability,
            "on_demand",
        )
        .await?;
    }
    let profile = PersistedRuntimeProfile {
        profile_kind: snapshot.content.required_runtime_profile,
        runtime_protocol_version: snapshot.content.required_runtime_protocol_version.clone(),
        enabled_runtime_features: snapshot.content.required_runtime_features.clone(),
        initial_capabilities: snapshot
            .content
            .initial_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect(),
        on_demand_capabilities: snapshot
            .content
            .on_demand_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect(),
        typed_resource_bindings: snapshot.content.typed_resource_bindings.clone(),
    };
    sqlx::query(
        "INSERT INTO agent_runtime_profiles \
         (snapshot_id, profile_kind, profile_json, profile_digest) VALUES (?, ?, ?, ?)",
    )
    .bind(snapshot.snapshot_ref.snapshot_id.as_ref())
    .bind(wire_string(&snapshot.content.required_runtime_profile)?)
    .bind(encode_control_json(&profile)?)
    .bind(snapshot.content.compiled_runtime_profile_digest.as_ref())
    .execute(&mut **tx)
    .await
    .map_err(control_sql)?;
    Ok(())
}

async fn insert_snapshot_capability_tx(
    tx: &mut Transaction<'_, Sqlite>,
    snapshot: &ResolvedSnapshotEnvelope,
    capability: &nomifun_agent_contracts::ResolvedCapability,
    set_kind: &str,
) -> Result<(), ControlPlaneError> {
    let activation_plan_json = snapshot
        .content
        .on_demand_activation_plans
        .get(&capability.capability.id)
        .map(encode_control_json)
        .transpose()?;
    sqlx::query(
        "INSERT INTO agent_runtime_snapshot_capabilities \
         (snapshot_id, capability_id, capability_version, set_kind, activation_plan_json) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(snapshot.snapshot_ref.snapshot_id.as_ref())
    .bind(capability.capability.id.as_ref())
    .bind(capability.capability.version.as_ref())
    .bind(set_kind)
    .bind(activation_plan_json)
    .execute(&mut **tx)
    .await
    .map_err(control_sql)?;
    Ok(())
}

async fn load_revision(
    pool: &SqlitePool,
    preset_id: &AgentPresetId,
    revision: u64,
) -> Result<Option<AgentPresetRevision>, ControlPlaneError> {
    let row: Option<(i64, String, String, String, i64, String)> = sqlx::query_as(
        "SELECT revision_no, revision_digest, editor_document_json, created_by, \
                created_at, reason \
         FROM agent_preset_revisions WHERE preset_id = ? AND revision_no = ?",
    )
    .bind(preset_id.as_ref())
    .bind(i64_from_u64(revision, "revision")?)
    .fetch_optional(pool)
    .await
    .map_err(control_sql)?;
    let Some((revision_no, digest, document, created_by, created_at, reason)) = row else {
        return Ok(None);
    };
    let revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: preset_id.clone(),
            revision: u64_from_i64(revision_no, "revision_no")?,
            revision_digest: DigestHex::from(digest),
        },
        payload: serde_json::from_str(&document).map_err(ControlPlaneError::from)?,
        created_by: UserId::from(created_by),
        created_at_ms: created_at,
        reason: (!reason.is_empty()).then_some(reason),
    };
    revision.validate().map_err(|violation| {
        ControlPlaneError::canonical(
            violation.code,
            StatusCode::INTERNAL_SERVER_ERROR,
            violation.message,
        )
    })?;
    Ok(Some(revision))
}

async fn current_revision_ref_tx(
    tx: &mut Transaction<'_, Sqlite>,
    preset_id: &AgentPresetId,
) -> Result<Option<PresetRevisionRef>, ControlPlaneError> {
    let revision: Option<i64> = sqlx::query_scalar(
        "SELECT current_stable_revision FROM agent_presets WHERE preset_id = ?",
    )
    .bind(preset_id.as_ref())
    .fetch_optional(&mut **tx)
    .await
    .map_err(control_sql)?
    .flatten();
    let Some(revision) = revision else {
        return Ok(None);
    };
    let digest: String = sqlx::query_scalar(
        "SELECT revision_digest FROM agent_preset_revisions \
         WHERE preset_id = ? AND revision_no = ?",
    )
    .bind(preset_id.as_ref())
    .bind(revision)
    .fetch_one(&mut **tx)
    .await
    .map_err(control_sql)?;
    Ok(Some(PresetRevisionRef {
        preset_id: preset_id.clone(),
        revision: u64_from_i64(revision, "current_stable_revision")?,
        revision_digest: DigestHex::from(digest),
    }))
}

async fn preset_owner(
    pool: &SqlitePool,
    preset_id: &AgentPresetId,
) -> Result<Option<UserId>, ControlPlaneError> {
    let owner: Option<String> =
        sqlx::query_scalar("SELECT owner_ref_json FROM agent_presets WHERE preset_id = ?")
            .bind(preset_id.as_ref())
            .fetch_optional(pool)
            .await
            .map_err(control_sql)?;
    owner
        .map(|owner| {
            owner_from_json(&owner)?.ok_or_else(|| control_not_found("AgentPreset"))
        })
        .transpose()
}

async fn preset_owner_tx(
    tx: &mut Transaction<'_, Sqlite>,
    preset_id: &AgentPresetId,
) -> Result<Option<UserId>, ControlPlaneError> {
    let owner: Option<String> =
        sqlx::query_scalar("SELECT owner_ref_json FROM agent_presets WHERE preset_id = ?")
            .bind(preset_id.as_ref())
            .fetch_optional(&mut **tx)
            .await
            .map_err(control_sql)?;
    owner
        .map(|owner| {
            owner_from_json(&owner)?.ok_or_else(|| control_not_found("AgentPreset"))
        })
        .transpose()
}

fn remote_binding_from_row(
    row: (String, String, String, String),
) -> Result<RemoteBinding, ControlPlaneError> {
    Ok(RemoteBinding {
        remote_binding_id: RemoteBindingId::from(row.0),
        owner_user_id: UserId::from(row.1),
        name: row.2,
        agent_binding: serde_json::from_str(&row.3).map_err(ControlPlaneError::from)?,
    })
}

pub struct SqlitePluginStatePersistence {
    pool: SqlitePool,
    snapshot: StdMutex<PluginStateSnapshot>,
}

impl SqlitePluginStatePersistence {
    pub async fn from_pool(pool: SqlitePool) -> Result<Self, AgentPlatformError> {
        let snapshot = load_plugin_state_snapshot(&pool).await?;
        Ok(Self {
            pool,
            snapshot: StdMutex::new(snapshot),
        })
    }
}

impl PluginStatePersistence for SqlitePluginStatePersistence {
    fn load(&self) -> Result<PluginStateSnapshot, PluginStateError> {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| PluginStateError::LockPoisoned)
    }

    fn save(&self, snapshot: &PluginStateSnapshot) -> Result<(), PluginStateError> {
        let pool = self.pool.clone();
        let next = snapshot.clone();
        run_async_on_thread("nomifun-plugin-state-save", async move {
            persist_plugin_state_snapshot(&pool, &next).await
        })
        .map_err(PluginStateError::Persistence)?;
        *self
            .snapshot
            .lock()
            .map_err(|_| PluginStateError::LockPoisoned)? = snapshot.clone();
        Ok(())
    }
}

async fn load_plugin_state_snapshot(
    pool: &SqlitePool,
) -> Result<PluginStateSnapshot, AgentPlatformError> {
    let rows: Vec<(String, String, String, String, Option<String>, i64, String, String)> =
        sqlx::query_as(
            "SELECT package_id, mount_id, scope_key, state_key, value_json, cas_revision, \
                    state_format_version, writer_package_version \
             FROM plugin_states ORDER BY package_id, mount_id, scope_key, state_key",
        )
        .fetch_all(pool)
        .await?;
    let mut entries = BTreeMap::new();
    let mut revisions = BTreeMap::new();
    for (
        package_id,
        mount_id,
        scope_key,
        state_key,
        value_json,
        revision,
        state_format_version,
        writer_package_version,
    ) in rows
    {
        let identity = StateIdentity {
            package_id: package_id.into(),
            mount_id: mount_id.into(),
            scope_key: scope_key.into(),
            state_key: state_key.into(),
        };
        let revision = u64::try_from(revision).map_err(|_| {
            AgentPlatformError::Contract(
                "plugin_states.cas_revision is negative".to_owned(),
            )
        })?;
        revisions.insert(identity.clone(), revision);
        if let Some(value_json) = value_json {
            entries.insert(
                identity.clone(),
                PluginStateEntry {
                    namespace: identity.namespace(),
                    revision,
                    state_format_version: state_format_version.into(),
                    writer_package_version: writer_package_version.into(),
                    value: StrictJsonValue(serde_json::from_str(&value_json)?),
                },
            );
        }
    }
    Ok(PluginStateSnapshot::from_parts(entries, revisions)?)
}

async fn persist_plugin_state_snapshot(
    pool: &SqlitePool,
    snapshot: &PluginStateSnapshot,
) -> Result<(), String> {
    let existing_rows: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
        "SELECT package_id, mount_id, scope_key, state_key, \
                state_format_version, writer_package_version \
         FROM plugin_states",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    let existing = existing_rows
        .into_iter()
        .map(
            |(package_id, mount_id, scope_key, state_key, format, writer)| {
                (
                    StateIdentity {
                        package_id: package_id.into(),
                        mount_id: mount_id.into(),
                        scope_key: scope_key.into(),
                        state_key: state_key.into(),
                    },
                    (format, writer),
                )
            },
        )
        .collect::<BTreeMap<_, _>>();

    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    sqlx::query("DELETE FROM plugin_states")
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    for (identity, revision) in snapshot.revisions() {
        let entry = snapshot.entry(
            &identity.package_id,
            &identity.mount_id,
            &identity.scope_key,
            &identity.state_key,
        );
        let (state_format_version, writer_package_version) = match entry {
            Some(entry) => (
                entry.state_format_version.as_ref().to_owned(),
                entry.writer_package_version.as_ref().to_owned(),
            ),
            None => existing
                .get(identity)
                .cloned()
                .unwrap_or_else(|| ("1.0.0".to_owned(), "1.0.0".to_owned())),
        };
        let value_json = entry
            .map(|entry| {
                String::from_utf8(
                    canonical_json_bytes(&entry.value.0)
                        .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())
            })
            .transpose()?;
        sqlx::query(
            "INSERT INTO plugin_states \
             (package_id, mount_id, scope_key, state_key, value_json, cas_revision, \
              state_format_version, writer_package_version) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(identity.package_id.as_ref())
        .bind(identity.mount_id.as_ref())
        .bind(identity.scope_key.as_ref())
        .bind(identity.state_key.as_ref())
        .bind(value_json)
        .bind(
            i64::try_from(*revision)
                .map_err(|_| "plugin state revision exceeds SQLite i64".to_owned())?,
        )
        .bind(state_format_version)
        .bind(writer_package_version)
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    }
    tx.commit().await.map_err(|error| error.to_string())
}

fn run_async_on_thread<T, F>(name: &str, future: F) -> Result<T, String>
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?
                .block_on(future)
        })
        .map_err(|error| error.to_string())?
        .join()
        .map_err(|_| format!("{name} thread panicked"))?
}

pub struct KernelCatalogProvider {
    registry: Arc<KernelRegistry>,
}

impl KernelCatalogProvider {
    pub fn new(registry: Arc<KernelRegistry>) -> Self {
        Self { registry }
    }
}

impl CatalogProvider for KernelCatalogProvider {
    fn snapshot(&self) -> Result<Arc<CatalogSnapshot>, ControlPlaneError> {
        let registry = self
            .registry
            .snapshot()
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        let capabilities = registry
            .capabilities
            .values()
            .map(|capability| capability.manifest.clone())
            .collect();
        let skills = registry
            .skills
            .values()
            .map(|skill| skill.definition.clone())
            .collect();
        let mcp_tools = registry
            .mcp_tools
            .values()
            .map(|mapping| mapping.mapping.clone())
            .collect();
        let package_sources = registry
            .packages
            .values()
            .map(|package| {
                (
                    nomifun_agent_contracts::PackageRef {
                        id: package.manifest.package_id.clone(),
                        version: package.manifest.package_version.clone(),
                    },
                    package.source.source_kind,
                )
            })
            .collect();
        Ok(Arc::new(CatalogSnapshot {
            capabilities,
            skills,
            mcp_tools,
            package_sources,
            unavailable_capabilities: BTreeMap::new(),
            service_key_diagnostics: Vec::new(),
        }))
    }
}

impl AgentPlatform {
    pub async fn from_pool(
        config: AgentPlatformConfig,
    ) -> Result<Arc<Self>, AgentPlatformError> {
        let state_persistence = Arc::new(
            SqlitePluginStatePersistence::from_pool(config.pool.clone()).await?,
        );
        let kernel = Arc::new(KernelRegistry::new(
            config.materialization_policy,
            state_persistence,
        )?);
        let control_store = Arc::new(SqliteControlPlaneStore::new(config.pool.clone()));
        let templates = OfficialTemplateCatalog::load()?;
        let catalog = Arc::new(KernelCatalogProvider::new(Arc::clone(&kernel)));
        let control_plane = Arc::new(AgentControlPlane::new(
            Arc::clone(&control_store) as Arc<dyn ControlPlaneStore>,
            catalog,
            templates.clone(),
            PresetPreviewCompiler::new(config.control_plane_release, templates),
        ));
        let sessions = Arc::new(AgentSessionStore::from_pool(config.pool.clone()).await?);
        let initial_plugins = config.initial_plugins;
        let platform = Arc::new(Self {
            pool: config.pool,
            control_store,
            control_plane,
            sessions,
            kernel,
            kernel_environment: config.kernel_environment,
            runtime: config.runtime,
            broker: config.broker,
            executions: RwLock::new(BTreeMap::new()),
            opening_bindings: RwLock::new(BTreeMap::new()),
            registrations: RwLock::new(Vec::new()),
            publish_lock: Mutex::new(()),
        });
        if !initial_plugins.is_empty() {
            platform.publish_plugins(initial_plugins).await?;
        }
        Ok(platform)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn control_plane(&self) -> &Arc<AgentControlPlane> {
        &self.control_plane
    }

    pub fn control_store(&self) -> &Arc<SqliteControlPlaneStore> {
        &self.control_store
    }

    pub fn session_store(&self) -> &Arc<AgentSessionStore> {
        &self.sessions
    }

    pub fn kernel_registry(&self) -> &Arc<KernelRegistry> {
        &self.kernel
    }

    pub fn runtime_port(&self) -> &Arc<dyn CodexRuntimePort> {
        &self.runtime
    }

    pub fn broker_port(&self) -> &Arc<dyn ChatBrokerPort> {
        &self.broker
    }

    pub fn materialized_registry(
        &self,
    ) -> Result<Arc<nomifun_agent_kernel::MaterializedRegistry>, AgentPlatformError> {
        Ok(self.kernel.snapshot()?)
    }

    pub async fn publish_plugins(
        &self,
        registrations: Vec<PluginRegistration>,
    ) -> Result<Arc<nomifun_agent_kernel::MaterializedRegistry>, AgentPlatformError> {
        let _guard = self.publish_lock.lock().await;
        let previous = self.registrations.read().await.clone();
        let mut tx = self.pool.begin().await?;
        persist_plugin_registrations_tx(&mut tx, &registrations).await?;
        let published = match self.kernel.replace_all(registrations.clone()) {
            Ok(published) => published,
            Err(error) => return Err(error.into()),
        };
        if let Err(error) = tx.commit().await {
            let _ = self.kernel.replace_all(previous);
            return Err(error.into());
        }
        *self.registrations.write().await = registrations;
        self.executions.write().await.clear();
        Ok(published)
    }

    pub fn pinned_runtime_profile(
        &self,
        compiled: &CompiledSnapshot,
    ) -> nomifun_codex_runtime::PinnedRuntimeProfile {
        nomifun_codex_runtime::PinnedRuntimeProfile {
            kind: compiled.content().required_runtime_profile,
            runtime_protocol_version: compiled
                .content()
                .required_runtime_protocol_version
                .clone(),
            profile_digest: compiled
                .content()
                .compiled_runtime_profile_digest
                .clone(),
            enabled_runtime_features: compiled.content().required_runtime_features.clone(),
            initial_capabilities: compiled
                .content()
                .initial_capabilities
                .iter()
                .map(|capability| capability.capability.id.clone())
                .collect(),
            on_demand_capabilities: compiled
                .content()
                .on_demand_capabilities
                .iter()
                .map(|capability| capability.capability.id.clone())
                .collect(),
            typed_resource_bindings: compiled.content().typed_resource_bindings.clone(),
        }
    }

    pub fn runtime_create_command(
        &self,
        agent_session_id: AgentSessionId,
        runtime_binding_id: RuntimeBindingId,
        operation_id: OperationId,
        compiled: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
    ) -> RuntimeCommand {
        RuntimeCommand::Create(RuntimeCreateParams {
            context: RuntimeCommandContext {
                agent_session_id,
                runtime_binding_id,
                operation_id,
                resolved_snapshot_ref: compiled.snapshot_ref().clone(),
                runtime_profile_digest: compiled
                    .content()
                    .compiled_runtime_profile_digest
                    .clone(),
                active_set_generation: active.generation,
            },
            profile_kind: compiled.content().required_runtime_profile,
            full_auto: FullAutoExecutionWire::fixed(),
            initial_capabilities: compiled
                .content()
                .initial_capabilities
                .iter()
                .map(|capability| capability.capability.id.clone())
                .collect(),
            on_demand_capabilities: compiled
                .content()
                .on_demand_capabilities
                .iter()
                .map(|capability| capability.capability.id.clone())
                .collect(),
            typed_resource_bindings: compiled.content().typed_resource_bindings.clone(),
        })
    }

    pub async fn open_model_stream(
        &self,
        request: ChatModelRequest,
    ) -> Result<ChatModelStream, AgentPlatformError> {
        Ok(self.broker.open_chat_stream(request).await?)
    }

    pub async fn create_session_from_dto(
        &self,
        owner: &UserId,
        request: CreateAgentSessionRequestDto,
        idempotency_key: impl Into<IdempotencyKey>,
    ) -> Result<CreateAgentSessionResponseDto, AgentPlatformError> {
        let mut open = OpenAgentSessionRequest::user(
            owner,
            platform_cast::<_, AgentBindingValue>(&request.agent_binding)?,
            idempotency_key,
        );
        open.metadata.title = request.title;
        let result = AgentSessionCommandPort::open_session(self, open).await?;
        Ok(CreateAgentSessionResponseDto {
            agent_session_id: result.session.agent_session_id.as_ref().to_owned(),
            agent_binding: platform_cast(&result.session.agent_binding)?,
            state: "opening".to_owned(),
            cursor: cursor_dto(&result.activation_ack.cursor),
        })
    }

    pub async fn start_turn_from_dto(
        &self,
        owner: &UserId,
        session_id: &str,
        request: CreateAgentSessionTurnRequestDto,
    ) -> Result<CreateAgentSessionTurnResponseDto, AgentPlatformError> {
        let session_id = AgentSessionId::from(session_id.to_owned());
        let dispatch = AgentSessionCommandPort::start_turn(
            self,
            StartAgentTurnRequest {
                agent_session_id: session_id.clone(),
                principal: user_principal(owner),
                input: StrictJsonValue(request.input),
                idempotency_key: request.idempotency_key.into(),
            },
        )
        .await?;
        let cursor = self.sessions.current_cursor(&session_id).await?;
        Ok(CreateAgentSessionTurnResponseDto {
            agent_session_id: session_id.as_ref().to_owned(),
            operation_id: dispatch.operation_id.as_ref().to_owned(),
            cursor: cursor_dto(&cursor),
            status: "running".to_owned(),
        })
    }

    pub async fn observe_from_cursor(
        &self,
        owner: &UserId,
        session_id: &str,
        after: Option<SessionCursorDto>,
        limit: u32,
    ) -> Result<SessionObservation, AgentPlatformError> {
        let session_id = AgentSessionId::from(session_id.to_owned());
        let after = after
            .map(|cursor| SessionEventCursor {
                agent_session_id: AgentSessionId::from(cursor.agent_session_id),
                seq: cursor.seq,
            });
        AgentSessionQueryPort::observe_session(
            self,
            &user_principal(owner),
            &session_id,
            after.as_ref(),
            limit,
        )
        .await
    }

    pub async fn cancel_turn(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
        target_operation_id: OperationId,
        idempotency_key: IdempotencyKey,
    ) -> Result<SessionEventAppendResult, AgentPlatformError> {
        self.require_owned_session(principal, session_id).await?;
        let execution = self.execution_for(session_id).await?;
        let binding = execution
            .runtime_binding
            .read()
            .await
            .clone()
            .ok_or(RuntimeError::SessionNotFound)?;
        let runtime_binding_id = binding.runtime_binding_id.clone();
        let active = execution.capabilities.snapshot()?;
        self.runtime
            .command(
                &runtime_binding_id,
                &RuntimeCommand::Cancel(RuntimeCancelParams {
                    context: RuntimeCommandContext {
                        agent_session_id: session_id.clone(),
                        runtime_binding_id: runtime_binding_id.clone(),
                        operation_id: OperationId::from(format!(
                            "cancel:{}",
                            idempotency_key.as_ref()
                        )),
                        resolved_snapshot_ref: execution.compiled.snapshot_ref().clone(),
                        runtime_profile_digest: execution
                            .compiled
                            .content()
                            .compiled_runtime_profile_digest
                            .clone(),
                        active_set_generation: active.generation,
                    },
                    target_operation_id: target_operation_id.clone(),
                }),
            )
            .await?;
        Ok(self
            .sessions
            .append_event(&SessionEventAppend {
                agent_session_id: session_id.clone(),
                event_id: stable_event_id(
                    "turn-cancelled",
                    session_id,
                    idempotency_key.as_ref(),
                ),
                producer_id: EventProducerId::from("session_api"),
                idempotency_key,
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("turn/cancelled".to_owned()),
                    kind_version: 1,
                    correlation_id: CorrelationId::from(
                        target_operation_id.as_ref().to_owned(),
                    ),
                    causation_event_id: None,
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                        "target_operation_id": target_operation_id
                    }))),
                },
            })
            .await?)
    }

    pub async fn compile_saved_binding(
        &self,
        principal: &PrincipalRef,
        binding: &AgentBindingValue,
        scene: impl Into<String>,
        surface: impl Into<String>,
        audience: impl Into<String>,
    ) -> Result<Arc<CompiledSnapshot>, AgentPlatformError> {
        if binding
            .typed_resource_bindings
            .iter()
            .any(|resource| resource.owner_id != principal.principal_id)
        {
            return Err(AgentPlatformError::Contract(
                "typed resource binding owner differs from the Session principal".to_owned(),
            ));
        }
        let preset = self
            .control_store
            .get_preset(&binding.preset_revision_ref.preset_id)
            .await?
            .ok_or_else(|| {
                AgentPlatformError::Contract(
                    "AgentBinding references a missing AgentPreset".to_owned(),
                )
            })?;
        if preset
            .preset
            .owner_user_id
            .as_ref()
            .map(|owner| owner.as_ref())
            != Some(principal.principal_id.as_str())
        {
            return Err(AgentPlatformError::Contract(
                "AgentBinding Preset owner differs from the Session principal".to_owned(),
            ));
        }
        let revision = self
            .control_store
            .get_revision(&binding.preset_revision_ref)
            .await?
            .ok_or_else(|| {
                AgentPlatformError::Contract(
                    "AgentBinding references a missing Preset revision".to_owned(),
                )
            })?;
        let persisted = self
            .control_store
            .get_snapshot(&binding.preset_revision_ref)
            .await?
            .ok_or_else(|| {
                AgentPlatformError::Contract(
                    "AgentBinding references a missing persisted Snapshot".to_owned(),
                )
            })?;
        if persisted.snapshot_ref != binding.resolved_snapshot_ref {
            return Err(AgentPlatformError::Contract(
                "AgentBinding ResolvedSnapshotRef differs from the persisted revision Snapshot"
                    .to_owned(),
            ));
        }

        let surface = surface.into();
        let registry = self.kernel.snapshot()?;
        let mut environment = self.kernel_environment.clone();
        environment.required_runtime_protocol_version =
            persisted.content.required_runtime_protocol_version.clone();
        environment.required_runtime_profile = persisted.content.required_runtime_profile;
        environment.runtime_feature_inventory_digest =
            persisted.content.runtime_feature_inventory_digest.clone();
        environment.canonical_schema_manifest_digest =
            persisted.content.canonical_schema_manifest_digest.clone();
        environment.target_contribution_manifest_digest =
            persisted.content.target_contribution_manifest_digest.clone();
        environment.host_surface = surface.clone();
        let compiled = AgentPresetCompiler::compile(
            &registry,
            &environment,
            CompileRequest {
                revision,
                principal: principal.clone(),
                scene: scene.into(),
                surface,
                audience: audience.into(),
                created_at_ms: persisted.created_at_ms,
                resolver_run_id: persisted.resolver_run_id.clone(),
            },
        )?;
        validate_compiler_convergence(&persisted, &compiled)?;
        Ok(Arc::new(CompiledSnapshot {
            envelope: persisted,
            authority_policies: compiled.authority_policies,
            registry_generation: compiled.registry_generation,
            registry_digest: compiled.registry_digest,
        }))
    }

    pub async fn execution_snapshot(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<Arc<CompiledSnapshot>, AgentPlatformError> {
        Ok(Arc::clone(&self.execution_for(session_id).await?.compiled))
    }

    pub async fn active_capabilities(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<ActiveCapabilitySetSnapshot, AgentPlatformError> {
        Ok(self
            .execution_for(session_id)
            .await?
            .capabilities
            .snapshot()?)
    }

    pub async fn runtime_binding(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<Option<RuntimeBindingContract>, AgentPlatformError> {
        Ok(self
            .execution_for(session_id)
            .await?
            .runtime_binding
            .read()
            .await
            .clone())
    }

    pub async fn launch_runtime(
        self: &Arc<Self>,
        mut request: RuntimeLaunchRequest,
    ) -> Result<Arc<ManagedRuntimeSession>, AgentPlatformError> {
        let context = runtime_open_context(&request.open_command)?.clone();
        let execution = self.execution_for(&context.agent_session_id).await?;
        let active = execution.capabilities.snapshot()?;
        if context.resolved_snapshot_ref != *execution.compiled.snapshot_ref()
            || context.runtime_profile_digest
                != execution
                    .compiled
                    .content()
                    .compiled_runtime_profile_digest
            || context.active_set_generation != active.generation
            || request.profile != self.pinned_runtime_profile(&execution.compiled)
        {
            return Err(AgentPlatformError::Contract(
                "Runtime launch request differs from the persisted Session execution state"
                    .to_owned(),
            ));
        }
        request.ingress = Arc::clone(self) as Arc<dyn RuntimeIngressPort>;
        {
            let mut opening = self.opening_bindings.write().await;
            if opening
                .insert(
                    context.runtime_binding_id.clone(),
                    context.agent_session_id.clone(),
                )
                .is_some()
            {
                return Err(RuntimeError::SessionAlreadyExists.into());
            }
        }
        let launched = self.runtime.launch(request).await;
        self.opening_bindings
            .write()
            .await
            .remove(&context.runtime_binding_id);
        let managed = launched?;
        if let Err(error) = self
            .commit_runtime_binding(managed.binding().clone())
            .await
        {
            let _ = self
                .runtime
                .dispose(RuntimeSessionDisposeParams {
                    agent_session_id: managed.binding().agent_session_id.clone(),
                    runtime_binding_id: managed.binding().runtime_binding_id.clone(),
                    operation_id: OperationId::from(format!(
                        "dispose-after-bind-failure:{}",
                        managed.binding().runtime_binding_id.as_ref()
                    )),
                    reason: nomifun_agent_contracts::CanonicalErrorCode::from(
                        "RUNTIME_BINDING_COMMIT_FAILED",
                    ),
                })
                .await;
            return Err(error);
        }
        Ok(managed)
    }

    async fn commit_runtime_binding(
        &self,
        binding: RuntimeBindingContract,
    ) -> Result<(), AgentPlatformError> {
        let envelope = RuntimeEventEnvelope {
            runtime_binding_id: binding.runtime_binding_id.clone(),
            producer_seq: 1,
            event_id: binding.runtime_bound_event_id.clone(),
            idempotency_key: IdempotencyKey::from(format!(
                "runtime-bound:{}",
                binding.runtime_binding_id.as_ref()
            )),
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("runtime/bound".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(
                    binding.runtime_binding_id.as_ref().to_owned(),
                ),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(
                    serde_json::to_value(&binding)?,
                )),
            },
        };
        self.sessions
            .append_runtime_event(RuntimeAppendContext {
                agent_session_id: binding.agent_session_id.clone(),
                envelope,
            })
            .await?;
        let ready = SessionEventAppend {
            agent_session_id: binding.agent_session_id.clone(),
            event_id: stable_event_id(
                "session-ready",
                &binding.agent_session_id,
                binding.runtime_binding_id.as_ref(),
            ),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(format!(
                "session-ready:{}",
                binding.runtime_binding_id.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("session/ready".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(
                    binding.agent_session_id.as_ref().to_owned(),
                ),
                causation_event_id: Some(binding.runtime_bound_event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "runtime_binding_id": binding.runtime_binding_id,
                    "resolved_snapshot_ref": binding.resolved_snapshot_ref,
                    "runtime_release_digest": binding.runtime_release_digest,
                    "runtime_build_digest": binding.runtime_build_digest,
                    "protocol_version": binding.protocol_version
                }))),
            },
        };
        self.sessions.append_event(&ready).await?;
        let execution = self.execution_for(&binding.agent_session_id).await?;
        *execution.runtime_binding.write().await = Some(binding);
        Ok(())
    }

    async fn execution_for(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<Arc<SessionExecutionState>, AgentPlatformError> {
        if let Some(execution) = self.executions.read().await.get(session_id).cloned() {
            return Ok(execution);
        }
        let session = self.sessions.get_live_session(session_id).await?;
        let compiled = self
            .compile_saved_binding(
                &session.owner_ref,
                &session.agent_binding,
                "agent_session",
                "desktop",
                "owner",
            )
            .await?;
        let capabilities = Arc::new(SessionCapabilityState::new(&compiled));
        replay_capability_state(&self.sessions, session_id, &capabilities).await?;
        let head = self.sessions.head(session_id).await?;
        let runtime_binding = match (
            head.runtime_bound_event_id,
            head.runtime_protocol_version,
            head.snapshot_digest,
        ) {
            (Some(_), Some(_), Some(_)) => {
                find_supervised_binding(&self.runtime, session_id).await
            }
            _ => None,
        };
        let execution = Arc::new(SessionExecutionState {
            compiled,
            capabilities,
            runtime_binding: RwLock::new(runtime_binding),
        });
        let mut executions = self.executions.write().await;
        Ok(executions
            .entry(session_id.clone())
            .or_insert_with(|| Arc::clone(&execution))
            .clone())
    }

    async fn require_owned_session(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
    ) -> Result<AgentSessionLiveRecord, AgentPlatformError> {
        let session = self.sessions.get_live_session(session_id).await?;
        if &session.owner_ref != principal {
            return Err(AgentPlatformError::Contract(
                "AgentSession principal ownership check failed".to_owned(),
            ));
        }
        Ok(session)
    }
}

#[async_trait]
impl AgentSessionCommandPort for AgentPlatform {
    async fn open_session(
        &self,
        request: OpenAgentSessionRequest,
    ) -> Result<SessionCreateResult, AgentPlatformError> {
        let compiled = self
            .compile_saved_binding(
                &request.owner_ref,
                &request.agent_binding,
                request.scene.clone(),
                request.surface.clone(),
                request.audience.clone(),
            )
            .await?;
        let agent_session_id = request
            .requested_session_id
            .unwrap_or_else(|| AgentSessionId::from(Uuid::now_v7().to_string()));
        let session = AgentSessionLiveRecord {
            agent_session_id: agent_session_id.clone(),
            owner_ref: request.owner_ref,
            metadata: request.metadata,
            agent_binding: request.agent_binding,
            remote_binding_provenance: request.remote_binding_provenance,
            parent_session_id: None,
            fork_base_payload_id: None,
            next_seq: 1,
        };
        let mut create = CreateSessionRequest::new(
            session,
            request.created_at,
            request.operation_id,
            request.producer_id,
            request.idempotency_key.clone(),
            request.correlation_id,
        );
        create.opening_event_id = Some(stable_event_id(
            "session-opening",
            &agent_session_id,
            request.idempotency_key.as_ref(),
        ));
        create.activation_event_id = Some(stable_event_id(
            "active-set-0",
            &agent_session_id,
            request.idempotency_key.as_ref(),
        ));
        create.initial_active_capability_ids = compiled
            .content()
            .initial_capabilities
            .iter()
            .map(|capability| capability.capability.id.as_ref().to_owned())
            .collect();
        let result = self.sessions.create_session(create).await?;
        let execution = Arc::new(SessionExecutionState {
            capabilities: Arc::new(SessionCapabilityState::new(&compiled)),
            compiled,
            runtime_binding: RwLock::new(None),
        });
        self.executions
            .write()
            .await
            .entry(result.session.agent_session_id.clone())
            .or_insert(execution);
        Ok(result)
    }

    async fn append_event(
        &self,
        append: &SessionEventAppend,
    ) -> Result<SessionEventAppendResult, AgentPlatformError> {
        Ok(self.sessions.append_event(append).await?)
    }

    async fn start_turn(
        &self,
        request: StartAgentTurnRequest,
    ) -> Result<AgentTurnDispatch, AgentPlatformError> {
        self.require_owned_session(&request.principal, &request.agent_session_id)
            .await?;
        let execution = self.execution_for(&request.agent_session_id).await?;
        let binding = execution
            .runtime_binding
            .read()
            .await
            .clone()
            .ok_or(RuntimeError::SessionNotFound)?;
        let head = self.sessions.head(&request.agent_session_id).await?;
        if head.status != "ready" || head.active_turn_id.is_some() {
            return Err(AgentPlatformError::Contract(
                "AgentSession is not at a completed-turn boundary".to_owned(),
            ));
        }
        let operation_id = OperationId::from(format!(
            "turn:{}:{}",
            request.agent_session_id.as_ref(),
            request.idempotency_key.as_ref()
        ));
        let input_event_id = stable_event_id(
            "message-user",
            &request.agent_session_id,
            request.idempotency_key.as_ref(),
        );
        let input_event = self
            .sessions
            .append_event(&SessionEventAppend {
                agent_session_id: request.agent_session_id.clone(),
                event_id: input_event_id.clone(),
                producer_id: EventProducerId::from("session_api"),
                idempotency_key: request.idempotency_key.clone(),
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("message/user-accepted".to_owned()),
                    kind_version: 1,
                    correlation_id: CorrelationId::from(operation_id.as_ref().to_owned()),
                    causation_event_id: None,
                    payload: SessionEventPayloadRef::InlineJson(request.input.clone()),
                },
            })
            .await?;
        let turn_event_id = stable_event_id(
            "turn-started",
            &request.agent_session_id,
            request.idempotency_key.as_ref(),
        );
        let turn_event = self
            .sessions
            .append_event(&SessionEventAppend {
                agent_session_id: request.agent_session_id.clone(),
                event_id: turn_event_id.clone(),
                producer_id: EventProducerId::from("session_api"),
                idempotency_key: IdempotencyKey::from(format!(
                    "{}:turn-started",
                    request.idempotency_key.as_ref()
                )),
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("turn/started".to_owned()),
                    kind_version: 1,
                    correlation_id: CorrelationId::from(operation_id.as_ref().to_owned()),
                    causation_event_id: Some(input_event_id.clone()),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                        "operation_id": &operation_id,
                        "input_event_id": &input_event_id
                    }))),
                },
            })
            .await?;
        let active = execution.capabilities.snapshot()?;
        let command = RuntimeCommand::StartTurn(RuntimeStartTurnParams {
            context: RuntimeCommandContext {
                agent_session_id: request.agent_session_id.clone(),
                runtime_binding_id: binding.runtime_binding_id.clone(),
                operation_id: operation_id.clone(),
                resolved_snapshot_ref: execution.compiled.snapshot_ref().clone(),
                runtime_profile_digest: execution
                    .compiled
                    .content()
                    .compiled_runtime_profile_digest
                    .clone(),
                active_set_generation: active.generation,
            },
            idempotency_key: request.idempotency_key,
            input_event_id,
        });
        let runtime_response = match self
            .runtime
            .command(&binding.runtime_binding_id, &command)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                let _ = append_turn_failure(
                    &self.sessions,
                    &request.agent_session_id,
                    &operation_id,
                    &turn_event_id,
                    &error,
                )
                .await;
                return Err(error.into());
            }
        };
        Ok(AgentTurnDispatch {
            agent_session_id: request.agent_session_id,
            operation_id,
            input_event,
            turn_event,
            runtime_response,
        })
    }

    async fn activate_capability(
        &self,
        request: ActivateCapabilityRequest,
    ) -> Result<ActivationOutcome, AgentPlatformError> {
        self.require_owned_session(&request.principal, &request.agent_session_id)
            .await?;
        let execution = self.execution_for(&request.agent_session_id).await?;
        let current = execution.capabilities.snapshot()?;
        if current.active.contains(&request.capability_id) {
            return Ok(ActivationOutcome::AlreadyActive {
                generation: current.generation,
            });
        }
        if current.generation != request.expected_generation {
            return Err(KernelError::ActivationGenerationConflict {
                expected: request.expected_generation,
                current: current.generation,
            }
            .into());
        }
        let plan = execution
            .compiled
            .content()
            .on_demand_activation_plans
            .get(&request.capability_id)
            .ok_or_else(|| KernelError::CapabilityNotInPreset {
                capability_id: request.capability_id.clone(),
            })?
            .clone();
        let generation = current
            .generation
            .checked_add(1)
            .ok_or(KernelError::ActivationGenerationExhausted)?;
        let mut active = current.active.clone();
        active.extend(plan.capability_bundle.iter().cloned());
        let active_ids = active
            .iter()
            .map(|capability| capability.as_ref().to_owned())
            .collect::<Vec<_>>();
        let delta = plan
            .capability_bundle
            .iter()
            .map(|capability| capability.as_ref().to_owned())
            .collect::<Vec<_>>();
        self.sessions
            .append_event(&SessionEventAppend {
                agent_session_id: request.agent_session_id.clone(),
                event_id: stable_event_id(
                    "active-set",
                    &request.agent_session_id,
                    request.idempotency_key.as_ref(),
                ),
                producer_id: EventProducerId::from("capability_host"),
                idempotency_key: request.idempotency_key,
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("capability/active-set-committed".to_owned()),
                    kind_version: 1,
                    correlation_id: CorrelationId::from(
                        request.agent_session_id.as_ref().to_owned(),
                    ),
                    causation_event_id: None,
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                        "generation": generation,
                        "active_capability_ids": active_ids,
                        "active_set_digest": digest_payload(&active_ids)?,
                        "delta": delta,
                        "requested_capability_id": &request.capability_id
                    }))),
                },
            })
            .await?;
        match execution.capabilities.activate_at_boundary(
            request.expected_generation,
            &request.capability_id,
            CompletedTurnBoundary::committed(request.completed_turn_operation_id),
        ) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.executions
                    .write()
                    .await
                    .remove(&request.agent_session_id);
                Err(error.into())
            }
        }
    }

    async fn invoke_capability(
        &self,
        command: InvokeCapabilityCommand,
    ) -> Result<StrictJsonValue, AgentPlatformError> {
        let session = self
            .require_owned_session(
                &command.invocation.principal,
                &command.agent_session_id,
            )
            .await?;
        if command.invocation.session_owner != session.owner_ref {
            return Err(AgentPlatformError::Contract(
                "Capability invocation session_owner differs from the persisted Session"
                    .to_owned(),
            ));
        }
        let execution = self.execution_for(&command.agent_session_id).await?;
        let active = execution.capabilities.snapshot()?;
        ThinAuthority::enforce(&execution.compiled, &active, &command.invocation)?;
        let registry = self.kernel.snapshot()?;
        let materialized = registry
            .capability(&command.invocation.capability_id)
            .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                capability_id: command.invocation.capability_id.clone(),
                version: VersionString::from("unknown"),
            })?;
        let effect_class = materialized
            .manifest
            .contributions
            .actions
            .iter()
            .find(|action| action.action_id == command.invocation.action_id)
            .map(|action| action.effect_class)
            .ok_or_else(|| KernelError::ActionNotDeclared {
                capability_id: command.invocation.capability_id.clone(),
                action_id: command.invocation.action_id.clone(),
            })?;
        let tool_event_id = stable_event_id(
            "tool-call",
            &command.agent_session_id,
            command.idempotency_key.as_ref(),
        );
        self.sessions
            .append_event(&SessionEventAppend {
                agent_session_id: command.agent_session_id.clone(),
                event_id: tool_event_id.clone(),
                producer_id: EventProducerId::from("capability_host"),
                idempotency_key: IdempotencyKey::from(format!(
                    "{}:tool-started",
                    command.idempotency_key.as_ref()
                )),
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: SemanticSessionEventDraft {
                    kind: SessionEventKind("tool/call-started".to_owned()),
                    kind_version: 1,
                    correlation_id: command.correlation_id.clone(),
                    causation_event_id: None,
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                        "operation_id": &command.operation_id,
                        "capability_id": &command.invocation.capability_id,
                        "action_id": &command.invocation.action_id,
                        "input": &command.invocation.input
                    }))),
                },
            })
            .await?;
        let effectful = is_effectful(effect_class);
        let effect_started = if effectful {
            Some(
                self.sessions
                    .record_effect_started(EffectEventRequest {
                        agent_session_id: command.agent_session_id.clone(),
                        event_id: stable_event_id(
                            "effect-started",
                            &command.agent_session_id,
                            command.idempotency_key.as_ref(),
                        ),
                        producer_id: EventProducerId::from("capability_host"),
                        idempotency_key: IdempotencyKey::from(format!(
                            "{}:effect-started",
                            command.idempotency_key.as_ref()
                        )),
                        correlation_id: command.correlation_id.clone(),
                        causation_event_id: Some(tool_event_id.clone()),
                        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                            "operation_id": &command.operation_id,
                            "capability_id": &command.invocation.capability_id,
                            "action_id": &command.invocation.action_id
                        }))),
                    })
                    .await?,
            )
        } else {
            None
        };
        let output = match self
            .kernel
            .invoke(&execution.compiled, &active, command.invocation.clone())
            .await
        {
            Ok(output) => output,
            Err(error) => {
                if effect_started.is_some() {
                    let _ = self
                        .sessions
                        .record_effect_terminal(
                            EffectEventRequest {
                                agent_session_id: command.agent_session_id.clone(),
                                event_id: stable_event_id(
                                    "effect-failed",
                                    &command.agent_session_id,
                                    command.idempotency_key.as_ref(),
                                ),
                                producer_id: EventProducerId::from("capability_host"),
                                idempotency_key: IdempotencyKey::from(format!(
                                    "{}:effect-failed",
                                    command.idempotency_key.as_ref()
                                )),
                                correlation_id: command.correlation_id.clone(),
                                causation_event_id: effect_started
                                    .as_ref()
                                    .and_then(|event| event.ack.as_ref())
                                    .map(|ack| ack.event_id.clone()),
                                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(
                                    json!({"error": error.to_string()}),
                                )),
                            },
                            EffectTerminalState::Failed,
                        )
                        .await;
                }
                let _ = append_tool_result(
                    &self.sessions,
                    &command,
                    &tool_event_id,
                    None,
                    Some(error.to_string()),
                )
                .await;
                return Err(error.into());
            }
        };
        let terminal_cause = if effectful {
            let terminal = self
                .sessions
                .record_effect_terminal(
                    EffectEventRequest {
                        agent_session_id: command.agent_session_id.clone(),
                        event_id: stable_event_id(
                            "effect-succeeded",
                            &command.agent_session_id,
                            command.idempotency_key.as_ref(),
                        ),
                        producer_id: EventProducerId::from("capability_host"),
                        idempotency_key: IdempotencyKey::from(format!(
                            "{}:effect-succeeded",
                            command.idempotency_key.as_ref()
                        )),
                        correlation_id: command.correlation_id.clone(),
                        causation_event_id: effect_started
                            .as_ref()
                            .and_then(|event| event.ack.as_ref())
                            .map(|ack| ack.event_id.clone()),
                        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                            "output": &output
                        }))),
                    },
                    EffectTerminalState::Succeeded,
                )
                .await;
            match terminal {
                Ok(terminal) => terminal
                    .ack
                    .map(|ack| ack.event_id)
                    .unwrap_or_else(|| tool_event_id.clone()),
                Err(error) => {
                    let _ = self
                        .sessions
                        .record_effect_terminal(
                            EffectEventRequest {
                                agent_session_id: command.agent_session_id.clone(),
                                event_id: stable_event_id(
                                    "effect-uncertain",
                                    &command.agent_session_id,
                                    command.idempotency_key.as_ref(),
                                ),
                                producer_id: EventProducerId::from("capability_host"),
                                idempotency_key: IdempotencyKey::from(format!(
                                    "{}:effect-uncertain",
                                    command.idempotency_key.as_ref()
                                )),
                                correlation_id: command.correlation_id.clone(),
                                causation_event_id: effect_started
                                    .as_ref()
                                    .and_then(|event| event.ack.as_ref())
                                    .map(|ack| ack.event_id.clone()),
                                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(
                                    json!({"error": error.to_string()}),
                                )),
                            },
                            EffectTerminalState::Uncertain,
                        )
                        .await;
                    return Err(error.into());
                }
            }
        } else {
            tool_event_id.clone()
        };
        append_tool_result(
            &self.sessions,
            &command,
            &terminal_cause,
            Some(output.clone()),
            None,
        )
        .await?;
        Ok(output)
    }

    async fn fork_session(
        &self,
        parent_session_id: &AgentSessionId,
        request: ForkRequest,
    ) -> Result<ForkResult, AgentPlatformError> {
        let parent = self.sessions.get_live_session(parent_session_id).await?;
        if parent.owner_ref != request.child_owner_ref {
            return Err(AgentPlatformError::Contract(
                "fork child owner differs from the parent Session owner".to_owned(),
            ));
        }
        let compiled = self
            .compile_saved_binding(
                &request.child_owner_ref,
                &request.child_agent_binding,
                "fork",
                "desktop",
                "owner",
            )
            .await?;
        let result = self.sessions.fork_session(parent_session_id, request).await?;
        self.executions.write().await.insert(
            result.child_session.agent_session_id.clone(),
            Arc::new(SessionExecutionState {
                capabilities: Arc::new(SessionCapabilityState::new(&compiled)),
                compiled,
                runtime_binding: RwLock::new(None),
            }),
        );
        Ok(result)
    }
}

#[async_trait]
impl AgentSessionQueryPort for AgentPlatform {
    async fn observe_session(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
        after: Option<&SessionEventCursor>,
        limit: u32,
    ) -> Result<SessionObservation, AgentPlatformError> {
        self.require_owned_session(principal, session_id).await?;
        Ok(self.sessions.observe(session_id, after, limit).await?)
    }

    async fn session_head(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
    ) -> Result<SessionHeadProjection, AgentPlatformError> {
        self.require_owned_session(principal, session_id).await?;
        Ok(self.sessions.head(session_id).await?)
    }

    async fn session_events(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
        after: Option<&SessionEventCursor>,
        limit: u32,
    ) -> Result<SessionEventPage, AgentPlatformError> {
        self.require_owned_session(principal, session_id).await?;
        Ok(self.sessions.read_events(session_id, after, limit).await?)
    }

    async fn rehydration_input(
        &self,
        principal: &PrincipalRef,
        session_id: &AgentSessionId,
    ) -> Result<SessionRehydrationInput, AgentPlatformError> {
        self.require_owned_session(principal, session_id).await?;
        Ok(self.sessions.rehydration_input(session_id).await?)
    }
}

#[async_trait]
impl AgentSessionDeletePort for AgentPlatform {
    async fn delete_session(
        &self,
        command: DeleteAgentSessionCommand,
        deleted_at: i64,
    ) -> Result<DeleteResult, AgentPlatformError> {
        self.require_owned_session(&command.owner_ref, &command.agent_session_id)
            .await?;
        let execution = self
            .executions
            .read()
            .await
            .get(&command.agent_session_id)
            .cloned();
        let runtime_binding = match execution {
            Some(execution) => execution.runtime_binding.read().await.clone(),
            None => None,
        };
        self.sessions.fence_delete(&command).await?;
        if let Some(binding) = runtime_binding {
            self.runtime
                .dispose(RuntimeSessionDisposeParams {
                    agent_session_id: command.agent_session_id.clone(),
                    runtime_binding_id: binding.runtime_binding_id,
                    operation_id: OperationId::from(format!(
                        "delete-dispose:{}",
                        command.operation_id.as_ref()
                    )),
                    reason: nomifun_agent_contracts::CanonicalErrorCode::from(
                        "SESSION_DELETE",
                    ),
                })
                .await?;
        }
        self.executions
            .write()
            .await
            .remove(&command.agent_session_id);
        Ok(self
            .sessions
            .complete_delete(&command, &ZeroOutstandingProof::verified(), deleted_at)
            .await?)
    }
}

async fn append_turn_failure(
    sessions: &AgentSessionStore,
    session_id: &AgentSessionId,
    operation_id: &OperationId,
    turn_event_id: &EventId,
    error: &RuntimeError,
) -> Result<SessionEventAppendResult, SessionStoreError> {
    sessions
        .append_event(&SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: stable_event_id(
                "turn-failed",
                session_id,
                operation_id.as_ref(),
            ),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(format!(
                "turn-failed:{}",
                operation_id.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("turn/failed".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(operation_id.as_ref().to_owned()),
                causation_event_id: Some(turn_event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "error": error.to_string()
                }))),
            },
        })
        .await
}

async fn append_tool_result(
    sessions: &AgentSessionStore,
    command: &InvokeCapabilityCommand,
    causation_event_id: &EventId,
    output: Option<StrictJsonValue>,
    error: Option<String>,
) -> Result<SessionEventAppendResult, SessionStoreError> {
    sessions
        .append_event(&SessionEventAppend {
            agent_session_id: command.agent_session_id.clone(),
            event_id: stable_event_id(
                "tool-result",
                &command.agent_session_id,
                command.idempotency_key.as_ref(),
            ),
            producer_id: EventProducerId::from("capability_host"),
            idempotency_key: IdempotencyKey::from(format!(
                "{}:tool-result",
                command.idempotency_key.as_ref()
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("tool/result-recorded".to_owned()),
                kind_version: 1,
                correlation_id: command.correlation_id.clone(),
                causation_event_id: Some(causation_event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "operation_id": &command.operation_id,
                    "capability_id": &command.invocation.capability_id,
                    "action_id": &command.invocation.action_id,
                    "output": output,
                    "error": error
                }))),
            },
        })
        .await
}

fn is_effectful(effect_class: EffectClass) -> bool {
    !matches!(
        effect_class,
        EffectClass::Pure | EffectClass::ReadLocal | EffectClass::ReadSensitive
    )
}

fn validate_compiler_convergence(
    persisted: &ResolvedSnapshotEnvelope,
    compiled: &CompiledSnapshot,
) -> Result<(), AgentPlatformError> {
    let persisted_content = &persisted.content;
    let compiled_content = compiled.content();
    if persisted_content.preset_revision_ref != compiled_content.preset_revision_ref
        || persisted_content.required_runtime_protocol_version
            != compiled_content.required_runtime_protocol_version
        || persisted_content.required_runtime_profile != compiled_content.required_runtime_profile
        || persisted_content.runtime_feature_inventory_digest
            != compiled_content.runtime_feature_inventory_digest
        || persisted_content.required_runtime_features
            != compiled_content.required_runtime_features
        || persisted_content.model_route_refs != compiled_content.model_route_refs
        || persisted_content.capability_allowlist != compiled_content.capability_allowlist
        || persisted_content.typed_resource_bindings
            != compiled_content.typed_resource_bindings
        || persisted_content.canonical_schema_manifest_digest
            != compiled_content.canonical_schema_manifest_digest
        || persisted_content.target_contribution_manifest_digest
            != compiled_content.target_contribution_manifest_digest
    {
        return Err(AgentPlatformError::Contract(
            "persisted control-plane Snapshot and Kernel compiler ceiling diverged".to_owned(),
        ));
    }
    Ok(())
}

async fn replay_capability_state(
    sessions: &AgentSessionStore,
    session_id: &AgentSessionId,
    capabilities: &SessionCapabilityState,
) -> Result<(), AgentPlatformError> {
    let mut cursor: Option<SessionEventCursor> = None;
    loop {
        let page = sessions.read_events(session_id, cursor.as_ref(), 500).await?;
        if page.events.is_empty() {
            break;
        }
        for event in &page.events {
            if event.kind.0 != "capability/active-set-committed" {
                continue;
            }
            let SessionEventPayloadRef::InlineJson(payload) = &event.payload else {
                return Err(AgentPlatformError::Contract(
                    "active-set event must use inline canonical JSON".to_owned(),
                ));
            };
            let generation = payload
                .0
                .get("generation")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    AgentPlatformError::Contract(
                        "active-set event has no generation".to_owned(),
                    )
                })?;
            if generation == 0 {
                continue;
            }
            let requested = payload
                .0
                .get("requested_capability_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AgentPlatformError::Contract(
                        "active-set replay requires requested_capability_id".to_owned(),
                    )
                })?;
            let current = capabilities.snapshot()?;
            if current.generation >= generation {
                continue;
            }
            capabilities.activate_at_boundary(
                current.generation,
                &CapabilityId::from(requested.to_owned()),
                CompletedTurnBoundary::committed(OperationId::from(format!(
                    "replay:{}",
                    event.event_id.as_ref()
                ))),
            )?;
        }
        let next = page.next_cursor;
        if cursor.as_ref().is_some_and(|cursor| cursor.seq == next.seq) {
            break;
        }
        cursor = Some(next);
    }
    Ok(())
}

fn runtime_open_context(command: &RuntimeCommand) -> Result<&RuntimeCommandContext, RuntimeError> {
    match command {
        RuntimeCommand::Create(params) => Ok(&params.context),
        RuntimeCommand::Resume(params) => Ok(&params.context),
        RuntimeCommand::Fork(params) => Ok(&params.child_context),
        _ => Err(RuntimeError::Protocol(
            "Runtime launch requires create, resume, or fork".to_owned(),
        )),
    }
}

async fn find_supervised_binding(
    runtime: &Arc<dyn CodexRuntimePort>,
    session_id: &AgentSessionId,
) -> Option<RuntimeBindingContract> {
    // The binding id is not a Session fact outside runtime/bound. Recovered
    // processes are intentionally not guessed; a new compatible binding must
    // be admitted through the normal resume path.
    let _ = (runtime, session_id);
    None
}

#[async_trait]
impl RuntimeIngressPort for AgentPlatform {
    async fn append_runtime_event(
        &self,
        event: RuntimeEventWireEnvelope,
    ) -> Result<RuntimeEventWireAck, RuntimeError> {
        let agent_session_id = match self
            .opening_bindings
            .read()
            .await
            .get(&event.runtime_binding_id)
            .cloned()
        {
            Some(agent_session_id) => agent_session_id,
            None => self
                .runtime
                .binding(&event.runtime_binding_id)
                .await
                .map(|binding| binding.agent_session_id)
                .ok_or(RuntimeError::SessionNotFound)?,
        };
        let result = self
            .sessions
            .append_runtime_event(RuntimeAppendContext {
                agent_session_id,
                envelope: event,
            })
            .await
            .map_err(runtime_session_error)?;
        result.ack.ok_or_else(|| {
            RuntimeError::Protocol(
                "persistent Runtime event committed without a RuntimeEventAck".to_owned(),
            )
        })
    }

    async fn commit_native_action_start(
        &self,
        start: NativeActionStart,
    ) -> Result<NativeActionStartAck, RuntimeError> {
        let session = self
            .sessions
            .get_live_session(&start.agent_session_id)
            .await
            .map_err(runtime_session_error)?;
        let execution = self
            .execution_for(&start.agent_session_id)
            .await
            .map_err(runtime_platform_error)?;
        let active = execution
            .capabilities
            .snapshot()
            .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
        let invocation = CapabilityInvocationRequest {
            principal: session.owner_ref.clone(),
            session_owner: session.owner_ref,
            resolved_snapshot_ref: execution.compiled.snapshot_ref().clone(),
            active_set_generation: start.active_set_generation,
            capability_id: start.capability_id.clone(),
            action_id: start.action_id.clone(),
            resource_binding_ids: start.resource_binding_ids.clone(),
            state_scope_key: ScopeKey::from(format!(
                "session:{}",
                start.agent_session_id.as_ref()
            )),
            input: StrictJsonValue(Value::Null),
        };
        if invocation.resolved_snapshot_ref.snapshot_digest != start.snapshot_digest
            || active.generation != start.active_set_generation
        {
            return Err(RuntimeError::NativeActionAck(
                "native action Snapshot or active generation differs from the Session"
                    .to_owned(),
            ));
        }
        ThinAuthority::enforce(&execution.compiled, &active, &invocation)
            .map_err(|error| RuntimeError::NativeActionAck(error.to_string()))?;

        let effect_started_event_id = stable_event_id(
            "effect-started",
            &start.agent_session_id,
            start.effect_id.as_ref(),
        );
        let append = self
            .sessions
            .record_effect_started(EffectEventRequest {
                agent_session_id: start.agent_session_id.clone(),
                event_id: effect_started_event_id.clone(),
                producer_id: EventProducerId::from("runtime_supervisor"),
                idempotency_key: IdempotencyKey::from(format!(
                    "{}:effect-started",
                    start.idempotency_key.as_ref()
                )),
                correlation_id: CorrelationId::from(start.effect_id.as_ref().to_owned()),
                causation_event_id: None,
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(
                    serde_json::to_value(&start)
                        .map_err(RuntimeError::Json)?,
                )),
            })
            .await
            .map_err(runtime_session_error)?;
        let ack = append.ack.ok_or_else(|| {
            RuntimeError::NativeActionAck(
                "effect/started committed without a SessionEventAck".to_owned(),
            )
        })?;
        Ok(NativeActionStartAck {
            agent_session_id: start.agent_session_id,
            runtime_binding_id: start.runtime_binding_id,
            turn_operation_id: start.turn_operation_id,
            action_id: start.action_id,
            effect_id: start.effect_id,
            idempotency_key: start.idempotency_key,
            capability_id: start.capability_id,
            active_set_generation: start.active_set_generation,
            snapshot_digest: start.snapshot_digest,
            effect_started_event_id,
            committed_session_seq: ack.cursor.seq,
        })
    }
}

fn stable_event_id(namespace: &str, session_id: &AgentSessionId, key: &str) -> EventId {
    EventId::from(format!("{namespace}:{}:{key}", session_id.as_ref()))
}

fn runtime_session_error(error: SessionStoreError) -> RuntimeError {
    RuntimeError::Protocol(error.to_string())
}

fn runtime_platform_error(error: AgentPlatformError) -> RuntimeError {
    match error {
        AgentPlatformError::Runtime(error) => error,
        other => RuntimeError::Protocol(other.to_string()),
    }
}

async fn persist_plugin_registrations_tx(
    tx: &mut Transaction<'_, Sqlite>,
    registrations: &[PluginRegistration],
) -> Result<(), AgentPlatformError> {
    for registration in registrations {
        let metadata = &registration.metadata;
        let manifest = &metadata.manifest.payload;
        let package_id = manifest.package_id.as_ref();
        let package_version = manifest.package_version.as_ref();
        sqlx::query(
            "INSERT INTO plugin_packages \
             (package_id, package_version, manifest_json, manifest_digest, display_json) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (package_id, package_version) DO UPDATE SET \
                 manifest_json = excluded.manifest_json, \
                 manifest_digest = excluded.manifest_digest, \
                 display_json = excluded.display_json",
        )
        .bind(package_id)
        .bind(package_version)
        .bind(platform_json(manifest)?)
        .bind(metadata.manifest.payload_digest.as_ref())
        .bind(platform_json(&manifest.display)?)
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "INSERT INTO plugin_mounts \
             (mount_id, package_id, package_version, source_json, desired_state, \
              effective_state, criticality) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (mount_id) DO UPDATE SET \
                 package_id = excluded.package_id, \
                 package_version = excluded.package_version, \
                 source_json = excluded.source_json, \
                 desired_state = excluded.desired_state, \
                 effective_state = excluded.effective_state, \
                 criticality = excluded.criticality",
        )
        .bind(metadata.mount_id.as_ref())
        .bind(package_id)
        .bind(package_version)
        .bind(platform_json(&metadata.source)?)
        .bind(platform_wire(&metadata.boot_state.desired_state)?)
        .bind(platform_wire(&metadata.boot_state.effective_state)?)
        .bind(platform_wire(&metadata.boot_state.criticality)?)
        .execute(&mut **tx)
        .await?;

        sqlx::query(
            "INSERT INTO plugin_configs \
             (package_id, mount_id, config_json, revision) VALUES (?, ?, ?, ?) \
             ON CONFLICT (package_id, mount_id) DO UPDATE SET \
                 config_json = excluded.config_json, revision = excluded.revision",
        )
        .bind(package_id)
        .bind(metadata.mount_id.as_ref())
        .bind(platform_json(&metadata.context.validated_config.value.0)?)
        .bind(i64::try_from(metadata.context.validated_config.config_revision).map_err(
            |_| AgentPlatformError::Contract("plugin config revision exceeds SQLite i64".into()),
        )?)
        .execute(&mut **tx)
        .await?;

        for capability in &manifest.contributions.capabilities {
            sqlx::query(
                "INSERT INTO capability_definitions \
                 (capability_id, capability_version, package_id, package_version, \
                  manifest_json, manifest_digest) \
                 VALUES (?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (capability_id, capability_version) DO UPDATE SET \
                     package_id = excluded.package_id, \
                     package_version = excluded.package_version, \
                     manifest_json = excluded.manifest_json, \
                     manifest_digest = excluded.manifest_digest",
            )
            .bind(capability.id.as_ref())
            .bind(capability.version.as_ref())
            .bind(package_id)
            .bind(package_version)
            .bind(platform_json(capability)?)
            .bind(digest_payload(capability)?.as_ref())
            .execute(&mut **tx)
            .await?;
        }
        for skill in &manifest.contributions.skills {
            sqlx::query(
                "INSERT INTO skill_instructions \
                 (skill_id, skill_version, package_id, package_version, \
                  definition_json, definition_digest) \
                 VALUES (?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (skill_id, skill_version) DO UPDATE SET \
                     package_id = excluded.package_id, \
                     package_version = excluded.package_version, \
                     definition_json = excluded.definition_json, \
                     definition_digest = excluded.definition_digest",
            )
            .bind(skill.id.as_ref())
            .bind(skill.version.as_ref())
            .bind(package_id)
            .bind(package_version)
            .bind(platform_json(skill)?)
            .bind(digest_payload(skill)?.as_ref())
            .execute(&mut **tx)
            .await?;
        }
        for mapping in &manifest.contributions.mcp_tools {
            let connection_ref = format!(
                "package:{}@{}:{}",
                package_id,
                package_version,
                mapping.server_id.as_ref()
            );
            sqlx::query(
                "INSERT INTO mcp_servers \
                 (server_id, owner_user_id, connection_config_ref, catalog_revision) \
                 VALUES (?, 'system', ?, 0) \
                 ON CONFLICT (server_id) DO NOTHING",
            )
            .bind(mapping.server_id.as_ref())
            .bind(connection_ref)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "INSERT INTO mcp_tool_materializations \
                 (server_id, canonical_tool_key, schema_hash, capability_id, \
                  capability_version, materialization_revision, package_id, package_version) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (server_id, canonical_tool_key) DO UPDATE SET \
                     schema_hash = excluded.schema_hash, \
                     capability_id = excluded.capability_id, \
                     capability_version = excluded.capability_version, \
                     materialization_revision = excluded.materialization_revision, \
                     package_id = excluded.package_id, \
                     package_version = excluded.package_version",
            )
            .bind(mapping.server_id.as_ref())
            .bind(mapping.canonical_tool_key.as_ref())
            .bind(mapping.schema_digest.as_ref())
            .bind(mapping.capability.id.as_ref())
            .bind(mapping.capability.version.as_ref())
            .bind(materialization_revision(&mapping.materialization_version))
            .bind(package_id)
            .bind(package_version)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

fn materialization_revision(version: &VersionString) -> i64 {
    version
        .as_ref()
        .split('.')
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

fn platform_json<T: Serialize>(value: &T) -> Result<String, AgentPlatformError> {
    Ok(String::from_utf8(canonical_json_bytes(value)?)
        .map_err(|error| AgentPlatformError::Contract(error.to_string()))?)
}

fn platform_wire<T: Serialize>(value: &T) -> Result<String, AgentPlatformError> {
    match serde_json::to_value(value)? {
        Value::String(value) => Ok(value),
        _ => Err(AgentPlatformError::Contract(
            "wire enum did not serialize to a string".to_owned(),
        )),
    }
}
