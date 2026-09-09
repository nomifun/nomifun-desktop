//! One host-owned typed Session facade for the Nomi-core product.
//!
//! The current product runtime is still the original Nomi engine.  The
//! facade keeps that fact in one composition boundary while exposing only the
//! narrow domain ports each consumer needs.  Domain crates retain their
//! Conversation-backed test factories, but production assembly does not create
//! one adapter per consumer anymore.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use dashmap::DashMap;
use futures_util::FutureExt;
use nomifun_ai_agent::types::AgentRuntimeBuildOptions;
use nomifun_ai_agent::{
    AgentRuntimeRegistry, AgentStreamEvent, KernelNomiPluginToolSession,
    NomiMiniAppToolInvoker, NomiMiniAppToolInvocation,
    NomiMiniAppToolSchemaResolver, NomiPluginToolError,
    NomiPluginToolSchemaResolver, NomiPluginToolSession,
    NomiPluginToolSessionProvider, NomiPluginToolSessionRequest,
};
use nomifun_agent_contracts::{
    AgentBindingValue, AgentSessionId, AgentSessionLiveRecord, AgentSessionMetadata,
    MiniAppBridgeCallId, OperationId, PrincipalRef, RemoteBindingProvenance,
    ResolvedMiniAppCapability, ScopeKey, StrictJsonValue, UserId,
};
use nomifun_agent_control_plane::{
    AgentControlPlane, AuthenticatedOwner, ControlPlaneError,
    control_plane_router_without_legacy_skills,
};
use nomifun_api_types::{
    AgentBindingValueDto, ApiResponse, ConversationResponse, ConversationRuntimeStateKind,
    CreateAgentSessionRequestDto, CreateConversationRequest,
    CreateAgentSessionResponseDto, CreateAgentSessionTurnRequestDto,
    CreateAgentSessionTurnResponseDto, ErrorResponse, ForkAgentSessionRequestDto,
    ForkAgentSessionResponseDto, ListMessagesQuery, MessageListResponse, MessageResponse,
    ListConversationsQuery,
    RemoteCancelRequestDto, RemoteMutationResponseDto, RemoteObserveRequestDto,
    RemoteObserveResponseDto, RemoteOpenRequestDto, RemoteOpenResponseDto,
    RemoteOpenStateViewDto, RemoteTurnRequestDto,
    AgentResolvedSnapshot, SessionCursorDto,
    SendMessageRequest, UpdateConversationRequest,
};
use nomifun_common::{AppError, MessagePosition, MessageType};
use nomifun_conversation::runtime_state::RuntimeBuildLease;
use nomifun_conversation::service::{
    BackgroundTurnPreSendHook, BackgroundTurnReconciliationDisposition,
    BackgroundTurnRuntimePreparation, IdempotentMessageDelivery,
    PublicTurnDeliveryState,
};
use nomifun_conversation::{AgentExecutionConversationPort, ConversationService, IdmmTurnScope};
use nomifun_db::{
    AgentExecutionTurnAuthority, AppendNomiRemoteEventParams, GetOrCreateRemoteSessionParams,
    IRemoteBindingRepository, RemoteOpenResult, SortOrder, TransitionNomiRemoteSessionParams,
};
use nomifun_db::models::{MessageRow, NomiRemoteEventRow, NomiRemoteSessionRow};
use nomifun_agent_session::{
    MessageProjection, SessionHeadProjection, SessionObservation,
};
use nomifun_agent_kernel::{
    AgentPresetCompiler, CompileRequest, CompiledSnapshot,
    CompilerEnvironment, KernelRegistry,
};
use nomifun_miniapp_platform::{
    MiniAppAgentCapabilityInvocation, MiniAppAgentCapabilityPort,
};
use nomifun_auth::{
    CurrentUser, InstanceTokenValidator, JwtService, extract_token_from_headers,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use uuid::Uuid;

/// The single Nomi-core Session owner exposed to production domain wiring.
///
/// `ConversationService` remains the implementation owner for the current
/// Nomi engine, while this type is the only app-level object that adapts it to
/// the domain-specific typed ports.  It owns no duplicate caches or identity
/// maps; all durable state and runtime state stay in the supplied service and
/// registry.
pub(crate) struct NomiCoreSessionOwner {
    service: ConversationService,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    execution: AgentExecutionConversationPort,
    autowork_runtime_lease_issuer: nomifun_requirement::AutoWorkRuntimeLeaseIssuer,
    autowork_config_locks: Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl NomiCoreSessionOwner {
    pub(crate) fn new(
        service: ConversationService,
        runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    ) -> Self {
        let execution = service.agent_execution_port(runtime_registry.clone());
        Self {
            service,
            runtime_registry,
            execution,
            autowork_runtime_lease_issuer:
                nomifun_requirement::AutoWorkRuntimeLeaseIssuer::new(),
            autowork_config_locks: Arc::new(DashMap::new()),
        }
    }

    pub(crate) fn service(&self) -> &ConversationService {
        &self.service
    }

    /// Idempotent counterpart of [`Self::create_session`].
    ///
    /// The creation key is interpreted and durably owned by
    /// `ConversationService`; this facade does not keep a second identity map.
    pub(crate) async fn create_session_idempotent(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        snapshot: Option<AgentResolvedSnapshot>,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        match snapshot {
            Some(snapshot) => {
                self.service
                    .create_from_agent_snapshot_idempotent(
                        owner_id,
                        request,
                        snapshot,
                        creation_key,
                    )
                    .await
            }
            None => {
                self.service
                    .create_idempotent(owner_id, request, creation_key)
                    .await
            }
        }
    }

    pub(crate) async fn get_session(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.service.get(owner_id, session_id).await
    }

    /// Deliver an owner-visible turn through the one public at-most-once Nomi
    /// boundary and the registry already owned by this facade.
    pub(crate) async fn send_session_message_idempotent(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
        request: SendMessageRequest,
    ) -> Result<IdempotentMessageDelivery, AppError> {
        self.service
            .send_message_with_idempotency_key(
                owner_id,
                session_id,
                idempotency_key,
                request,
                &self.runtime_registry,
            )
            .await
    }

    /// Read the durable outcome of the exact keyed public turn without
    /// creating send authority or synthesizing runtime events.
    pub(crate) async fn session_turn_delivery_state(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
    ) -> Result<PublicTurnDeliveryState, AppError> {
        self.service
            .public_turn_delivery_state(owner_id, session_id, idempotency_key)
            .await
    }

    pub(crate) async fn cancel_session(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<(), AppError> {
        self.service
            .cancel(owner_id, session_id, &self.runtime_registry)
            .await
    }

    /// Delete through `ConversationService`, whose lifecycle owner tears down
    /// the runtime from the same registry before committing durable deletion.
    pub(crate) async fn delete_session(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<(), AppError> {
        self.service.delete(owner_id, session_id).await
    }

    fn autowork_config_lock(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Arc<tokio::sync::Mutex<()>> {
        let key = format!("{owner_id}\n{session_id}");
        self.autowork_config_locks
            .entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    async fn raw_owned_session_extra(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<(String, Value), AppError> {
        let row = self
            .service
            .conversation_repo()
            .get(session_id)
            .await?
            .filter(|row| row.user_id == owner_id)
            .ok_or_else(|| AppError::NotFound(format!("AgentSession {session_id} not found")))?;
        let extra: Value = serde_json::from_str(&row.extra).map_err(|error| {
            AppError::Internal(format!(
                "AgentSession {session_id} has invalid extra JSON: {error}"
            ))
        })?;
        if !extra.is_object() {
            return Err(AppError::Internal(format!(
                "AgentSession {session_id} extra must be a JSON object"
            )));
        }
        Ok((row.extra, extra))
    }
}

/// App-owned bridge from a Conversation-backed Nomi session to one exact
/// Kernel-backed Plugin Tool session.
///
/// It resolves every authority fact from the persisted owner/session/binding
/// chain. The runtime registry supplies only the first-class owner and
/// conversation IDs; no `extra` field is interpreted as Mount, Artifact,
/// action, schema, or activation authority.
pub(crate) struct NomiCorePluginToolSessionProvider {
    session_owner: Arc<NomiCoreSessionOwner>,
    control_plane: Arc<AgentControlPlane>,
    kernel: Arc<KernelRegistry>,
    compiler_environment: CompilerEnvironment,
    schema_resolver: Arc<dyn NomiPluginToolSchemaResolver>,
    miniapp_application:
        Arc<nomifun_miniapp_platform::MiniAppM1ApplicationService>,
}

impl NomiCorePluginToolSessionProvider {
    pub(crate) fn new(
        session_owner: Arc<NomiCoreSessionOwner>,
        control_plane: Arc<AgentControlPlane>,
        kernel: Arc<KernelRegistry>,
        compiler_environment: CompilerEnvironment,
        schema_resolver: Arc<dyn NomiPluginToolSchemaResolver>,
        miniapp_application: Arc<
            nomifun_miniapp_platform::MiniAppM1ApplicationService,
        >,
    ) -> Self {
        Self {
            session_owner,
            control_plane,
            kernel,
            compiler_environment,
            schema_resolver,
            miniapp_application,
        }
    }
}

#[async_trait]
impl NomiPluginToolSessionProvider for NomiCorePluginToolSessionProvider {
    async fn resolve(
        &self,
        request: NomiPluginToolSessionRequest,
    ) -> Result<Option<NomiPluginToolSession>, AppError> {
        let common_owner = nomifun_common::UserId::parse(
            request.owner_id.clone(),
        )
        .map_err(|error| {
            AppError::Forbidden(format!(
                "invalid Nomi Plugin Tool session owner: {error}"
            ))
        })?;
        let session_id = parse_agent_session_id(&request.conversation_id)
            .map_err(|error| AppError::Conflict(error.message))?;
        let response = self
            .session_owner
            .get_session(common_owner.as_ref(), session_id.as_ref())
            .await?;
        if response.extra.get(NOMI_CORE_SESSION_METADATA_KEY).is_none() {
            return Ok(None);
        }
        let owner = AuthenticatedOwner(UserId::from(
            common_owner.as_ref().to_owned(),
        ));
        let metadata =
            session_metadata(&response, &owner).map_err(|error| {
                AppError::Conflict(error.message)
            })?;
        let binding_dto =
            agent_binding_dto(&metadata.binding).map_err(|error| {
                AppError::Conflict(error.message)
            })?;
        let (binding, revision, snapshot) = self
            .control_plane
            .saved_binding_artifacts(&owner.0, &binding_dto)
            .await
            .map_err(control_plane_error_to_app)?;
        if binding != metadata.binding {
            return Err(AppError::Conflict(
                "Nomi Plugin Tool Session binding differs from the persisted Conversation binding"
                    .to_owned(),
            ));
        }
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: common_owner.as_ref().to_owned(),
        };
        let compiled = compile_nomi_plugin_snapshot(
            &self.kernel,
            &self.compiler_environment,
            binding,
            revision,
            snapshot,
            &principal,
        )?;
        let compiled = Arc::new(compiled);
        let plugin_session = KernelNomiPluginToolSession::materialize(
            Arc::clone(&self.kernel),
            Arc::clone(&compiled),
            principal.clone(),
            session_id.clone(),
            ScopeKey::from(format!(
                "session:{}",
                request.conversation_id
            )),
            Arc::clone(&self.schema_resolver),
        )
        .await
        .map_err(|error| {
            AppError::Conflict(format!(
                "Nomi Plugin Tool session materialization failed: {error}"
            ))
        })?;
        let miniapp_actions =
            KernelNomiPluginToolSession::materialize_miniapp_actions(
                &compiled,
                &principal,
                &session_id,
                &ScopeKey::from(format!(
                    "session:{}",
                    request.conversation_id
                )),
                Arc::new(NomiCoreMiniAppSchemaResolver {
                    application: Arc::clone(&self.miniapp_application),
                }),
            )
            .await
            .map_err(|error| {
                AppError::Conflict(format!(
                    "Nomi MiniApp Tool session materialization failed: {error}"
                ))
            })?;
        plugin_session
            .with_miniapp_actions(
                miniapp_actions,
                Arc::new(NomiCoreMiniAppToolInvoker {
                    application: Arc::clone(&self.miniapp_application),
                    owner_user_id: common_owner.as_ref().to_owned(),
                }),
            )
            .map(Some)
        .map_err(|error| {
            AppError::Conflict(format!(
                "Nomi hosted Tool session assembly failed: {error}"
            ))
        })
    }
}

struct NomiCoreMiniAppSchemaResolver {
    application:
        Arc<nomifun_miniapp_platform::MiniAppM1ApplicationService>,
}

#[async_trait]
impl NomiMiniAppToolSchemaResolver for NomiCoreMiniAppSchemaResolver {
    async fn resolve(
        &self,
        owner: &PrincipalRef,
        capability: &ResolvedMiniAppCapability,
        reference: &nomifun_agent_contracts::CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        if owner.principal_kind != "user" {
            return Err("MiniApp Agent Tool owner must be a user principal".to_owned());
        }
        self.application
            .resolve_agent_capability_schema(
                &owner.principal_id,
                capability,
                reference,
            )
            .await
            .map_err(|error| error.to_string())
    }
}

struct NomiCoreMiniAppToolInvoker {
    application:
        Arc<nomifun_miniapp_platform::MiniAppM1ApplicationService>,
    owner_user_id: String,
}

#[async_trait]
impl NomiMiniAppToolInvoker for NomiCoreMiniAppToolInvoker {
    async fn invoke(
        &self,
        request: NomiMiniAppToolInvocation,
    ) -> Result<StrictJsonValue, NomiPluginToolError> {
        let operation_id = request.operation_id().clone();
        self.application
            .invoke_agent_capability(
                MiniAppAgentCapabilityInvocation {
                    owner_user_id: self.owner_user_id.clone(),
                    miniapp_id: request.capability().miniapp_id.clone(),
                    capability: request.capability().capability.clone(),
                    action_id: request.action().action_id.clone(),
                    action_allowlist: request
                        .capability()
                        .action_allowlist
                        .clone(),
                    active_release: request.capability().active_release.clone(),
                    active_release_epoch: request
                        .capability()
                        .active_release_epoch,
                    catalog_digest: request.capability().catalog_digest.clone(),
                    operation_id: operation_id.clone(),
                    call_id: MiniAppBridgeCallId::from(format!(
                        "nomi-miniapp:{}",
                        operation_id.as_ref()
                    )),
                    payload: request.input().clone(),
                },
            )
            .await
            .map_err(|error| NomiPluginToolError::Contract(error.to_string()))
    }
}

fn compile_nomi_plugin_snapshot(
    kernel: &KernelRegistry,
    compiler_environment: &CompilerEnvironment,
    binding: AgentBindingValue,
    revision: nomifun_agent_contracts::AgentPresetRevision,
    persisted: nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    principal: &PrincipalRef,
) -> Result<CompiledSnapshot, AppError> {
    if binding.preset_revision_ref != revision.reference
        || binding.resolved_snapshot_ref != persisted.snapshot_ref
        || persisted.content.preset_revision_ref != revision.reference
        || persisted.actor != *principal
    {
        return Err(AppError::Conflict(
            "Nomi Plugin Tool Binding/Revision/Snapshot identity chain is inconsistent"
                .to_owned(),
        ));
    }
    let registry = kernel.snapshot().map_err(kernel_error_to_app)?;
    let mut environment = compiler_environment.clone();
    environment.required_runtime_protocol_version = persisted
        .content
        .required_runtime_protocol_version
        .clone();
    environment.required_runtime_profile =
        persisted.content.required_runtime_profile;
    environment.runtime_feature_inventory_digest =
        persisted.content.runtime_feature_inventory_digest.clone();
    environment.canonical_schema_manifest_digest =
        persisted.content.canonical_schema_manifest_digest.clone();
    environment.target_contribution_manifest_digest =
        persisted.content.target_contribution_manifest_digest.clone();
    environment.host_surface = persisted.surface.clone();
    environment.availability_evidence_revision =
        persisted.availability_evidence_revision.clone();
    let compiled = AgentPresetCompiler::compile(
        &registry,
        &environment,
        CompileRequest {
            revision,
            principal: principal.clone(),
            scene: persisted.scene.clone(),
            surface: persisted.surface.clone(),
            audience: persisted.audience.clone(),
            created_at_ms: persisted.created_at_ms,
            resolver_run_id: persisted.resolver_run_id.clone(),
            miniapp_capabilities: persisted
                .content
                .initial_miniapp_capabilities
                .iter()
                .cloned()
                .chain(
                    persisted
                        .content
                        .on_demand_miniapp_capabilities
                        .iter()
                        .cloned(),
                )
                .collect(),
        },
    )
    .map_err(kernel_error_to_app)?;
    if compiled.envelope != persisted {
        return Err(AppError::Conflict(
            "current Kernel compilation differs from the persisted Nomi resolved Snapshot"
                .to_owned(),
        ));
    }
    CompiledSnapshot {
        envelope: persisted,
        ..compiled
    }
    .with_target_resource_bindings(principal, binding.typed_resource_bindings)
    .map_err(kernel_error_to_app)
}

fn kernel_error_to_app(error: nomifun_agent_kernel::KernelError) -> AppError {
    AppError::Conflict(format!("Nomi Plugin Tool Kernel admission failed: {error}"))
}

fn control_plane_error_to_app(
    error: nomifun_agent_control_plane::ControlPlaneError,
) -> AppError {
    let message = format!("{}: {error}", error.code().as_ref());
    match error.status() {
        StatusCode::BAD_REQUEST => AppError::BadRequest(message),
        StatusCode::FORBIDDEN => AppError::Forbidden(message),
        StatusCode::NOT_FOUND => AppError::NotFound(message),
        StatusCode::CONFLICT => AppError::Conflict(message),
        StatusCode::UNPROCESSABLE_ENTITY => {
            AppError::UnprocessableEntity(message)
        }
        _ => AppError::Internal(message),
    }
}

#[async_trait]
impl nomifun_cron::CronSessionPort for NomiCoreSessionOwner {
    async fn get_session(
        &self,
        query: &nomifun_cron::CronSessionLookup,
    ) -> Result<nomifun_cron::CronSessionProjection, AppError> {
        let response = self
            .service
            .get(&query.owner_id, query.agent_session_id.as_ref())
            .await?;
        let row = self
            .service
            .conversation_repo()
            .get(query.agent_session_id.as_ref())
            .await?
            .filter(|row| row.user_id == query.owner_id)
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "AgentSession {} not found",
                    query.agent_session_id.as_ref()
                ))
            })?;
        cron_session_projection_from_response(&query.owner_id, response, row.cron_job_id)
    }

    async fn lookup_scheduled_sessions(
        &self,
        query: &nomifun_cron::CronScheduledSessionLookup,
    ) -> Result<Vec<nomifun_cron::CronScheduledSession>, AppError> {
        self.service
            .list_by_cron_job(&query.owner_id, &query.cron_job_id)
            .await?
            .into_iter()
            .map(|response| {
                cron_session_projection_from_response(
                    &query.owner_id,
                    response,
                    Some(query.cron_job_id.clone()),
                )
            })
            .collect()
    }

    async fn list_conversation_responses_for_cron(
        &self,
        query: &nomifun_cron::CronScheduledSessionLookup,
    ) -> Result<Vec<ConversationResponse>, AppError> {
        self.service
            .list_by_cron_job(&query.owner_id, &query.cron_job_id)
            .await
    }

    async fn bind_cron_relation(
        &self,
        request: &nomifun_cron::CronSessionCronBindingRequest,
    ) -> Result<(), AppError> {
        self.service
            .conversation_repo()
            .bind_cron_relation(
                &request.owner_id,
                request.agent_session_id.as_ref(),
                &request.cron_job_id,
                nomifun_common::now_ms(),
            )
            .await
            .map_err(AppError::from)
    }

    async fn read_turn_receipt(
        &self,
        query: &nomifun_cron::CronTurnReceiptQuery,
    ) -> Result<nomifun_cron::CronTurnReceiptState, AppError> {
        Ok(cron_turn_state_from_conversation(
            self.service
                .public_turn_delivery_state(
                    &query.owner_id,
                    query.agent_session_id.as_ref(),
                    &query.idempotency_key,
                )
                .await?,
        ))
    }

    async fn reconcile_turn_receipt(
        &self,
        request: &nomifun_cron::CronTurnReconciliationRequest,
    ) -> Result<nomifun_cron::CronTurnReconciliation, AppError> {
        Ok(cron_reconciliation_from_conversation(
            self.service
                .reconcile_quiescent_running_turn_for_background(
                    &request.owner_id,
                    request.agent_session_id.as_ref(),
                    &request.idempotency_key,
                    &self.runtime_registry,
                )
                .await?,
        ))
    }

    async fn create_idempotent(
        &self,
        user_id: &str,
        request: CreateConversationRequest,
        snapshot: Option<AgentResolvedSnapshot>,
        creation_key: &str,
    ) -> Result<nomifun_cron::CronSessionHandle, AppError> {
        let response = match snapshot {
            Some(snapshot) => {
                self.service
                    .create_from_agent_snapshot_idempotent(
                        user_id,
                        request,
                        snapshot,
                        creation_key,
                    )
                    .await
            }
            None => {
                self.service
                    .create_idempotent(user_id, request, creation_key)
                    .await
            }
        }?;
        cron_session_handle_from_response(response)
    }

    async fn prepare_runtime_and_send(
        &self,
        request: nomifun_cron::CronRuntimePreparationRequest,
    ) -> Result<nomifun_cron::CronPreparedTurnDelivery, AppError> {
        let nomifun_cron::CronRuntimePreparationRequest {
            owner_id,
            agent_session_id,
            idempotency_key,
            turn,
        } = request;
        let nomifun_cron::CronTurnRequest {
            message,
            runtime:
                nomifun_cron::CronTurnRuntimePreparation {
                    overlay,
                    clear_context,
                },
        } = turn;
        let session_id = agent_session_id.as_ref();
        nomifun_common::CronJobId::parse(&overlay.cron_job_id).map_err(|error| {
            AppError::BadRequest(format!("invalid Cron runtime annotation: {error}"))
        })?;
        let build_lease = self
            .service
            .begin_public_runtime_preparation(session_id, &owner_id)?;
        let relation = self
            .service
            .conversation_repo()
            .get(session_id)
            .await?
            .filter(|row| row.user_id == owner_id)
            .ok_or_else(|| {
                AppError::NotFound(format!("AgentSession {session_id} not found"))
            })?;
        if relation.cron_job_id.as_deref() != Some(overlay.cron_job_id.as_str()) {
            return Err(AppError::Conflict(format!(
                "AgentSession {session_id} is not bound to Cron job {}",
                overlay.cron_job_id
            )));
        }
        let session = self.service.get(&owner_id, session_id).await?;
        build_lease.ensure_active()?;
        let (runtime_options, workspace) =
            runtime_options_from_session(&owner_id, session, Some(&overlay))?;
        let observed = self
            .service
            .send_observed_background_message_with_idempotency_key(
                &owner_id,
                session_id,
                &idempotency_key,
                cron_turn_message_to_request(message),
                &self.runtime_registry,
                build_lease,
                BackgroundTurnRuntimePreparation {
                    runtime_options,
                    clear_context,
                    pre_send_hook: None,
                },
            )
            .await?;
        Ok(nomifun_cron::CronPreparedTurnDelivery {
            delivery: cron_delivery_from_conversation(observed.delivery),
            workspace,
        })
    }

    async fn delivery_result(
        &self,
        query: &nomifun_cron::CronTurnDeliveryQuery,
    ) -> Result<Option<nomifun_cron::CronTurnDelivery>, AppError> {
        Ok(self
            .service
            .idempotent_delivery_result_with_idempotency_key(
                &query.owner_id,
                query.agent_session_id.as_ref(),
                &query.idempotency_key,
                &cron_turn_message_to_request(query.message.clone()),
            )
            .await?
            .map(cron_delivery_from_conversation))
    }
}

#[async_trait]
impl nomifun_channel::ChannelSessionPort for NomiCoreSessionOwner {
    async fn is_busy(&self, session_id: &str) -> bool {
        let summary = self.service.runtime_summary_for(session_id).await;
        matches!(
            summary.state,
            ConversationRuntimeStateKind::Starting | ConversationRuntimeStateKind::Running
        )
    }

    async fn turn_outcome(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
    ) -> Result<nomifun_channel::ChannelTurnReceiptState, AppError> {
        self.service
            .public_turn_delivery_state(owner_id, session_id, idempotency_key)
            .await
            .map(channel_turn_receipt_state_from_conversation)
    }

    async fn cancel(&self, owner_id: &str, session_id: &str) -> Result<(), AppError> {
        self.service
            .cancel(owner_id, session_id, &self.runtime_registry)
            .await
    }

    async fn list_messages(
        &self,
        owner_id: &str,
        session_id: &str,
        query: ListMessagesQuery,
    ) -> Result<MessageListResponse, AppError> {
        self.service.list_messages(owner_id, session_id, query).await
    }

    async fn send_turn(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
        request: SendMessageRequest,
    ) -> Result<nomifun_channel::ChannelTurnDelivery, AppError> {
        let delivery = self
            .service
            .send_message_with_idempotency_key(
                owner_id,
                session_id,
                idempotency_key,
                request,
                &self.runtime_registry,
            )
            .await?;
        let events = if delivery.completed {
            None
        } else {
            wait_for_runtime_subscription(&self.runtime_registry, session_id).await
        };
        Ok(nomifun_channel::ChannelTurnDelivery {
            delivery: channel_delivery_from_conversation(delivery),
            events,
        })
    }

    async fn get(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.service.get(owner_id, session_id).await
    }

    async fn create_idempotent(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.service
            .create_idempotent(owner_id, request, creation_key)
            .await
    }
}

#[async_trait]
impl nomifun_requirement::AutoWorkScheduledSessionLookup for NomiCoreSessionOwner {
    async fn list_enabled_scheduled_sessions(
        &self,
        owner_id: &str,
    ) -> Result<nomifun_requirement::ScheduledAutoWorkSessionScan, AppError> {
        const PAGE_SIZE: u32 = 200;
        let mut cursor = None;
        let mut sessions = Vec::new();
        let mut quarantined = Vec::new();
        loop {
            let page = self
                .service
                .list(
                    owner_id,
                    ListConversationsQuery {
                        cursor: cursor.clone(),
                        limit: Some(PAGE_SIZE),
                        source: None,
                        cron_job_id: None,
                        pinned: None,
                    },
                    false,
                )
                .await?;
            let next_cursor = page.items.last().map(|session| session.conversation_id.clone());
            for session in page.items {
                let Some(raw) = session.extra.get("autowork") else {
                    continue;
                };
                match conversation_autowork_config_snapshot(
                    &session.conversation_id,
                    Some(raw),
                ) {
                    Ok(snapshot) if snapshot.config.enabled => {
                        match snapshot.config.enabled_tag() {
                            Ok(tag) => {
                                sessions.push(nomifun_requirement::ScheduledAutoWorkSession {
                                    session_id: session.conversation_id,
                                    display_name: session.name,
                                    tag: tag.to_owned(),
                                    max_requirements: snapshot.config.max_requirements,
                                    config_revision: snapshot.revision,
                                });
                            }
                            Err(error) => quarantined.push(
                                nomifun_requirement::AutoWorkBindingIssue {
                                    target_id: Some(session.conversation_id),
                                    code: "missing_tag",
                                    detail: error.to_string(),
                                },
                            ),
                        }
                    }
                    Ok(_) => {}
                    Err(error) => quarantined.push(
                        nomifun_requirement::AutoWorkBindingIssue {
                            target_id: Some(session.conversation_id),
                            code: "invalid_config",
                            detail: error.to_string(),
                        },
                    ),
                }
            }
            if !page.has_more {
                break;
            }
            let next_cursor = next_cursor.ok_or_else(|| {
                AppError::Internal(
                    "AgentSession AutoWork lookup returned an empty non-terminal page".to_owned(),
                )
            })?;
            if cursor.as_deref() == Some(next_cursor.as_str()) {
                return Err(AppError::Internal(
                    "AgentSession AutoWork lookup cursor did not advance".to_owned(),
                ));
            }
            cursor = Some(next_cursor);
        }
        Ok(nomifun_requirement::ScheduledAutoWorkSessionScan {
            sessions,
            quarantined,
        })
    }
}

#[async_trait]
impl nomifun_requirement::AutoWorkSessionPort for NomiCoreSessionOwner {
    async fn prepare_autowork_turn(
        &self,
        owner_id: &str,
        session_id: &str,
        build_lease: &nomifun_requirement::AutoWorkRuntimeBuildLease,
    ) -> Result<nomifun_requirement::AutoWorkSessionPreparation, AppError> {
        build_lease.ensure_scope(owner_id, session_id)?;
        build_lease.ensure_active()?;
        let session = self.service.get(owner_id, session_id).await?;
        build_lease.ensure_active()?;
        let workspace = session_workspace(&session)?;
        let revision = session_projection_revision(&session)?;
        let snapshot = self
            .autowork_runtime_lease_issuer
            .issue_snapshot(owner_id, session_id, revision)?;
        Ok(nomifun_requirement::AutoWorkSessionPreparation {
            agent_type: session.r#type,
            workspace,
            snapshot,
        })
    }

    fn begin_runtime_preparation(
        &self,
        conversation_id: &str,
        requester_user_id: &str,
    ) -> Result<nomifun_requirement::AutoWorkRuntimeBuildLease, AppError> {
        let lease = self
            .service
            .begin_public_runtime_preparation(conversation_id, requester_user_id)?;
        self.autowork_runtime_lease_issuer.issue(
            requester_user_id,
            conversation_id,
            lease,
            RuntimeBuildLease::ensure_active,
        )
    }

    fn user_cancelled_since(&self, conversation_id: &str, since_ms: i64) -> bool {
        self.service.user_cancelled_since(conversation_id, since_ms)
    }

    async fn cancel_active_turn(&self, conversation_id: &str) -> Result<(), AppError> {
        if let Some(runtime) = self.runtime_registry.get_runtime(conversation_id) {
            runtime.cancel().await?;
        }
        Ok(())
    }

    async fn read_config(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<nomifun_requirement::AutoWorkConfigSnapshot, AppError> {
        let (_, extra) = self.raw_owned_session_extra(owner_id, session_id).await?;
        conversation_autowork_config_snapshot(session_id, extra.get("autowork"))
    }

    async fn save_config(
        &self,
        command: nomifun_requirement::AutoWorkSessionConfigCommand,
    ) -> Result<nomifun_requirement::AutoWorkConfigSnapshot, AppError> {
        let canonical = nomifun_requirement::AutoWorkConfig::normalize(
            command.config.enabled,
            command.config.tag.as_deref(),
            command.config.max_requirements,
        )?;
        if canonical != command.config {
            return Err(AppError::BadRequest(
                "AutoWork config must use its canonical normalized tag".to_owned(),
            ));
        }
        if command.expected_revision.trim().is_empty() {
            return Err(AppError::Conflict(
                "AutoWork config write requires an expected revision".to_owned(),
            ));
        }
        if command
            .operation_id
            .as_deref()
            .is_some_and(|operation_id| operation_id.trim().is_empty())
        {
            return Err(AppError::BadRequest(
                "AutoWork config operation identity must not be empty".to_owned(),
            ));
        }
        let lock = self.autowork_config_lock(&command.owner_id, &command.session_id);
        let _guard = lock.lock().await;
        let (expected_extra, stored_extra) = self
            .raw_owned_session_extra(&command.owner_id, &command.session_id)
            .await?;
        let current = conversation_autowork_config_snapshot(
            &command.session_id,
            stored_extra.get("autowork"),
        )?;
        if command.operation_id.is_some()
            && current.operation_id == command.operation_id
        {
            if current.config == command.config {
                return Ok(current);
            }
            return Err(AppError::Conflict(
                "AutoWork operation identity was replayed with a different config".to_owned(),
            ));
        }
        if current.revision != command.expected_revision {
            return Err(AppError::Conflict(format!(
                "AutoWork config for conversation {} changed concurrently",
                command.session_id
            )));
        }
        if current.config == command.config && command.operation_id.is_none() {
            return Ok(current);
        }

        let current_sequence = conversation_autowork_sequence(&current.revision);
        let next_sequence = if current.config == command.config
            && !current.revision.starts_with("conversation:legacy:")
        {
            current_sequence
        } else {
            current_sequence.checked_add(1).ok_or_else(|| {
                AppError::Conflict(format!(
                    "AutoWork config revision overflow for conversation {}",
                    command.session_id
                ))
            })?
        };
        let mut autowork = stored_extra
            .get("autowork")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let object = autowork.as_object_mut().ok_or_else(|| {
            AppError::Conflict(format!(
                "AgentSession {} has an invalid AutoWork config",
                command.session_id
            ))
        })?;
        object.insert("enabled".to_owned(), Value::Bool(command.config.enabled));
        object.insert(
            "tag".to_owned(),
            command
                .config
                .tag
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        object.insert(
            "max_requirements".to_owned(),
            command
                .config
                .max_requirements
                .map(|value| Value::Number(value.into()))
                .unwrap_or(Value::Null),
        );
        object.insert(
            "_revision".to_owned(),
            Value::Number(next_sequence.into()),
        );
        object.insert(
            "_operation_id".to_owned(),
            command
                .operation_id
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        );
        let mut replacement_extra = stored_extra;
        replacement_extra["autowork"] = Value::Object(object.clone());
        let replacement_extra = serde_json::to_string(&replacement_extra).map_err(|error| {
            AppError::Internal(format!(
                "failed to serialize AutoWork Session extra {}: {error}",
                command.session_id
            ))
        })?;
        let swapped = self
            .service
            .conversation_repo()
            .compare_and_swap_extra(
                &command.owner_id,
                &command.session_id,
                &expected_extra,
                &replacement_extra,
                nomifun_common::now_ms(),
            )
            .await?;
        if !swapped {
            return Err(AppError::Conflict(format!(
                "AutoWork config for conversation {} changed concurrently",
                command.session_id
            )));
        }
        let (_, updated_extra) = self
            .raw_owned_session_extra(&command.owner_id, &command.session_id)
            .await?;
        let snapshot =
            conversation_autowork_config_snapshot(&command.session_id, updated_extra.get("autowork"))?;
        if snapshot.config != command.config
            || snapshot.revision != format!("conversation:{next_sequence}")
            || snapshot.operation_id != command.operation_id
        {
            return Err(AppError::Conflict(
                "Session host returned a different AutoWork config after save".to_owned(),
            ));
        }
        Ok(snapshot)
    }

    async fn send_turn(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
        request: nomifun_requirement::AutoWorkTurnRequest,
        build_lease: nomifun_requirement::AutoWorkRuntimeBuildLease,
        authority: nomifun_db::RequirementConversationTurnAuthority,
    ) -> Result<nomifun_requirement::AutoWorkMessageDelivery, AppError> {
        let nomifun_requirement::AutoWorkTurnRequest {
            message,
            runtime_overlay,
            session_snapshot,
        } = request;
        build_lease.ensure_snapshot(&session_snapshot)?;
        let build_lease = self.autowork_runtime_lease_issuer.consume::<RuntimeBuildLease>(
            build_lease,
            user_id,
            conversation_id,
        )?;
        let session = self.service.get(user_id, conversation_id).await?;
        build_lease.ensure_active()?;
        let revision = session_projection_revision(&session)?;
        self.autowork_runtime_lease_issuer.validate_snapshot(
            &session_snapshot,
            user_id,
            conversation_id,
            &revision,
        )?;
        build_lease.ensure_active()?;
        let (runtime_options, _) = runtime_options_from_session(user_id, session, None)?;
        let observed = self
            .service
            .send_observed_autowork_message_with_idempotency_key(
                user_id,
                conversation_id,
                operation_id,
                autowork_message_to_request(message),
                &self.runtime_registry,
                build_lease,
                BackgroundTurnRuntimePreparation {
                    runtime_options,
                    clear_context: runtime_overlay.clear_context,
                    pre_send_hook: runtime_overlay.pre_send_hook.map(|hook| {
                        Arc::new(NomiCoreAutoWorkPreSendHook { inner: hook })
                            as Arc<dyn BackgroundTurnPreSendHook>
                    }),
                },
                authority,
            )
            .await?;
        Ok(autowork_delivery_from_conversation(observed.delivery))
    }

    async fn delivery_result(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
        request: &nomifun_requirement::AutoWorkMessage,
        authority: &nomifun_db::RequirementConversationTurnAuthority,
    ) -> Result<Option<nomifun_requirement::AutoWorkMessageDelivery>, AppError> {
        let request = autowork_message_to_request(request.clone());
        self.service
            .autowork_delivery_result_with_idempotency_key(
                user_id,
                conversation_id,
                operation_id,
                &request,
                authority,
            )
            .await
            .map(|delivery| delivery.map(autowork_delivery_from_conversation))
    }

    async fn public_turn_delivery_state(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<nomifun_requirement::AutoWorkTurnDeliveryState, AppError> {
        self.service
            .public_turn_delivery_state(user_id, conversation_id, operation_id)
            .await
            .map(autowork_turn_state_from_conversation)
    }

    async fn reconcile_quiescent_running_turn(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<nomifun_requirement::AutoWorkReconciliationDisposition, AppError> {
        self.service
            .reconcile_quiescent_running_turn_for_background(
                user_id,
                conversation_id,
                operation_id,
                &self.runtime_registry,
            )
            .await
            .map(autowork_reconciliation_from_conversation)
    }
}

#[async_trait]
impl nomifun_companion::CompanionSessionPort for NomiCoreSessionOwner {
    async fn get(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.service.get(owner_id, session_id).await
    }

    async fn replace_skill_snapshot(
        &self,
        session_id: &str,
        skills: &[String],
    ) -> Result<bool, AppError> {
        self.service.replace_skill_snapshot(session_id, skills).await
    }

    async fn update_extra(
        &self,
        session_id: &str,
        patch: serde_json::Value,
    ) -> Result<(), AppError> {
        self.service.update_extra(session_id, patch).await
    }

    async fn create(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
    ) -> Result<ConversationResponse, AppError> {
        self.service.create(owner_id, request).await
    }

    async fn delete(&self, owner_id: &str, session_id: &str) -> Result<(), AppError> {
        self.service.delete(owner_id, session_id).await
    }

    async fn update(
        &self,
        owner_id: &str,
        session_id: &str,
        request: UpdateConversationRequest,
    ) -> Result<ConversationResponse, AppError> {
        self.service
            .update(owner_id, session_id, request, &self.runtime_registry)
            .await
    }

    async fn message_local_day_index(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<Vec<nomifun_db::MessageDayBucket>, AppError> {
        self.service
            .message_local_day_index(owner_id, session_id)
            .await
    }
}

#[async_trait]
impl nomifun_companion::CompanionArchiveSessionPort for NomiCoreSessionOwner {
    async fn window_messages(
        &self,
        owner_id: &str,
        session_id: &str,
        since_ts: i64,
        limit: u32,
    ) -> Result<Vec<nomifun_companion::archiver::WindowMessage>, AppError> {
        let query = ListMessagesQuery {
            cursor: Some(String::new()),
            page_size: Some(limit.clamp(1, 400)),
            ..Default::default()
        };
        let response = self.service.list_messages(owner_id, session_id, query).await?;
        Ok(response
            .items
            .into_iter()
            .filter_map(|message| companion_archive_message(message, since_ts))
            .collect())
    }

    async fn reset_context(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<(), AppError> {
        self.service.clear_context(owner_id, session_id).await
    }
}

fn companion_archive_message(
    message: MessageResponse,
    since_ts: i64,
) -> Option<nomifun_companion::archiver::WindowMessage> {
    if message.hidden
        || message.created_at <= since_ts
        || message.r#type != MessageType::Text
    {
        return None;
    }
    let is_user = match message.position {
        Some(MessagePosition::Right) => true,
        Some(MessagePosition::Left) => false,
        _ => return None,
    };
    let content = match message.content {
        Value::String(content) => content,
        Value::Object(object) => object
            .get("text")
            .or_else(|| object.get("content"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    };
    (!content.trim().is_empty()).then_some(nomifun_companion::archiver::WindowMessage {
        is_user,
        content,
        created_at: message.created_at,
    })
}

#[async_trait]
impl nomifun_idmm::ConversationSessionPort for NomiCoreSessionOwner {
    fn subscribe(
        &self,
        conversation_id: &str,
    ) -> Option<broadcast::Receiver<AgentStreamEvent>> {
        self.runtime_registry
            .get_runtime(conversation_id)
            .map(|runtime| runtime.subscribe())
    }

    async fn runtime_summary(
        &self,
        conversation_id: &str,
    ) -> nomifun_api_types::ConversationRuntimeSummary {
        self.service.runtime_summary_for(conversation_id).await
    }

    fn user_cancelled_since(&self, conversation_id: &str, since_ms: i64) -> bool {
        self.service.user_cancelled_since(conversation_id, since_ms)
    }

    fn is_alive(&self, conversation_id: &str) -> bool {
        self.runtime_registry.get_runtime(conversation_id).is_some()
    }

    async fn active_turn_scope(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Result<nomifun_idmm::SupervisionTurnScope, AppError> {
        self.service
            .idmm_active_turn_scope(owner_id, conversation_id, &self.runtime_registry)
            .await
            .map(|scope| {
                nomifun_idmm::SupervisionTurnScope::new(scope.wire_turn_id, scope.generation)
            })
    }

    async fn continue_active_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        expected_scope: &nomifun_idmm::SupervisionTurnScope,
        request: SendMessageRequest,
    ) -> Result<String, AppError> {
        let conversation_scope = IdmmTurnScope {
            wire_turn_id: expected_scope.wire_turn_id().to_owned(),
            generation: expected_scope.generation(),
        };
        self.service
            .idmm_continue_active_turn(
                owner_id,
                conversation_id,
                &conversation_scope,
                request,
                &self.runtime_registry,
            )
            .await
    }

    async fn failover(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Result<bool, AppError> {
        self.service
            .idmm_failover_conversation(owner_id, conversation_id, &self.runtime_registry)
            .await
    }
}

#[async_trait]
impl nomifun_agent_execution::AgentExecutionSessionPort for NomiCoreSessionOwner {
    async fn create_idempotent(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.service
            .create_idempotent(owner_id, request, creation_key)
            .await
    }

    async fn create_from_agent_snapshot_idempotent(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        snapshot: AgentResolvedSnapshot,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.service
            .create_from_agent_snapshot_idempotent(owner_id, request, snapshot, creation_key)
            .await
    }

    async fn discard_unlinked_creation(
        &self,
        owner_id: &str,
        creation_key: &str,
    ) -> Result<(), AppError> {
        self.service
            .discard_unlinked_creation(owner_id, creation_key)
            .await
    }

    async fn deliver_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        authority: AgentExecutionTurnAuthority,
        request: SendMessageRequest,
    ) -> Result<nomifun_agent_execution::AgentExecutionDelivery, AppError> {
        self.execution
            .deliver_turn(
                owner_id,
                conversation_id,
                operation_id,
                authority,
                request,
            )
            .await
            .map(agent_execution_delivery_from_conversation)
    }

    async fn delivery_result(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<Option<nomifun_agent_execution::AgentExecutionDelivery>, AppError> {
        self.execution
            .delivery_result(owner_id, conversation_id, operation_id)
            .await
            .map(|delivery| delivery.map(agent_execution_delivery_from_conversation))
    }

    async fn list_messages(
        &self,
        owner_id: &str,
        conversation_id: &str,
        query: ListMessagesQuery,
    ) -> Result<MessageListResponse, AppError> {
        self.service
            .list_messages(owner_id, conversation_id, query)
            .await
    }

    async fn get(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.service.get(owner_id, conversation_id).await
    }

    fn take_turn_tokens(&self, conversation_id: &str) -> Option<i64> {
        self.service.take_turn_tokens(conversation_id)
    }

    async fn cancel_for_execution(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        self.service
            .cancel_for_execution(owner_id, conversation_id, &self.runtime_registry)
            .await
    }

    async fn steer_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        request: SendMessageRequest,
    ) -> Result<String, AppError> {
        self.execution
            .steer_turn(owner_id, conversation_id, operation_id, request)
            .await
    }

    async fn project_assistant_message_idempotent(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        content: &str,
        origin: &str,
    ) -> Result<String, AppError> {
        self.service
            .project_assistant_message_idempotent(
                owner_id,
                conversation_id,
                operation_id,
                content,
                origin,
            )
            .await
    }
}

fn agent_execution_delivery_from_conversation(
    delivery: IdempotentMessageDelivery,
) -> nomifun_agent_execution::AgentExecutionDelivery {
    nomifun_agent_execution::AgentExecutionDelivery {
        message_id: delivery.message_id,
        replayed: delivery.replayed,
        completed: delivery.completed,
        result_ok: delivery.result_ok,
        result_text: delivery.result_text,
        result_error: delivery.result_error,
        result_error_code: delivery.result_error_code,
        result_error_retryable: delivery.result_error_retryable,
    }
}

fn channel_delivery_from_conversation(
    delivery: IdempotentMessageDelivery,
) -> nomifun_channel::ChannelTurnDeliveryReceipt {
    nomifun_channel::ChannelTurnDeliveryReceipt {
        message_id: delivery.message_id,
        replayed: delivery.replayed,
        completed: delivery.completed,
        result_ok: delivery.result_ok,
        result_text: delivery.result_text,
        result_error: delivery.result_error,
        result_error_code: delivery.result_error_code,
        result_error_retryable: delivery.result_error_retryable,
    }
}

fn channel_turn_receipt_state_from_conversation(
    state: PublicTurnDeliveryState,
) -> nomifun_channel::ChannelTurnReceiptState {
    match state {
        PublicTurnDeliveryState::Missing => nomifun_channel::ChannelTurnReceiptState::Missing,
        PublicTurnDeliveryState::Accepted { message_id } => {
            nomifun_channel::ChannelTurnReceiptState::Accepted { message_id }
        }
        PublicTurnDeliveryState::Completed(delivery) => {
            nomifun_channel::ChannelTurnReceiptState::Completed(
                nomifun_channel::ChannelCompletedTurnReceipt {
                    message_id: delivery.message_id,
                    replayed: delivery.replayed,
                    result_ok: delivery.result_ok,
                    result_text: delivery.result_text,
                    result_error: delivery.result_error,
                    result_error_code: delivery.result_error_code,
                    result_error_retryable: delivery.result_error_retryable,
                },
            )
        }
    }
}

fn cron_turn_message_to_request(
    message: nomifun_cron::CronTurnMessage,
) -> SendMessageRequest {
    SendMessageRequest {
        content: message.content,
        files: message.files,
        inject_skills: message.inject_skills,
        hidden: message.hidden,
        origin: message.origin,
        channel_platform: message.channel_platform,
    }
}

fn cron_delivery_from_conversation(
    delivery: IdempotentMessageDelivery,
) -> nomifun_cron::CronTurnDelivery {
    nomifun_cron::CronTurnDelivery {
        message_id: delivery.message_id,
        replayed: delivery.replayed,
        completed: delivery.completed,
        result_ok: delivery.result_ok,
        result_text: delivery.result_text,
        result_error: delivery.result_error,
        result_error_code: delivery.result_error_code,
        result_error_retryable: delivery.result_error_retryable,
    }
}

fn cron_turn_state_from_conversation(
    state: PublicTurnDeliveryState,
) -> nomifun_cron::CronTurnReceiptState {
    match state {
        PublicTurnDeliveryState::Missing => nomifun_cron::CronTurnReceiptState::Missing,
        PublicTurnDeliveryState::Accepted { message_id } => {
            nomifun_cron::CronTurnReceiptState::Accepted { message_id }
        }
        PublicTurnDeliveryState::Completed(delivery) => {
            nomifun_cron::CronTurnReceiptState::Completed(
                cron_delivery_from_conversation(delivery),
            )
        }
    }
}

fn cron_reconciliation_from_conversation(
    disposition: BackgroundTurnReconciliationDisposition,
) -> nomifun_cron::CronTurnReconciliation {
    match disposition {
        BackgroundTurnReconciliationDisposition::LiveExactOwnerWait => {
            nomifun_cron::CronTurnReconciliation::LiveExactOwnerWait
        }
        BackgroundTurnReconciliationDisposition::ReconciledOrTerminalReRead => {
            nomifun_cron::CronTurnReconciliation::ReconciledOrTerminalReRead
        }
        BackgroundTurnReconciliationDisposition::ExternalProofRequiredFailClosed => {
            nomifun_cron::CronTurnReconciliation::ExternalProofRequiredFailClosed
        }
        BackgroundTurnReconciliationDisposition::StaleConflict => {
            nomifun_cron::CronTurnReconciliation::StaleConflict
        }
    }
}

fn cron_session_handle_from_response(
    response: ConversationResponse,
) -> Result<nomifun_cron::CronSessionHandle, AppError> {
    let workspace = session_workspace(&response)?;
    let agent_session_id = AgentSessionId::from(response.conversation_id);
    nomifun_common::validate_uuidv7(agent_session_id.as_ref()).map_err(|error| {
        AppError::Conflict(format!(
            "AgentSession identity is not canonical UUIDv7: {error}"
        ))
    })?;
    Ok(nomifun_cron::CronSessionHandle {
        agent_session_id,
        workspace,
    })
}

fn cron_session_projection_from_response(
    owner_id: &str,
    response: ConversationResponse,
    cron_job_id: Option<String>,
) -> Result<nomifun_cron::CronSessionProjection, AppError> {
    let agent_session_id = AgentSessionId::from(response.conversation_id.clone());
    nomifun_common::validate_uuidv7(agent_session_id.as_ref()).map_err(|error| {
        AppError::Conflict(format!(
            "AgentSession identity is not canonical UUIDv7: {error}"
        ))
    })?;
    let workspace = session_workspace(&response)?;
    let optional_string = |key: &str| {
        response
            .extra
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let skills = match response.extra.get("skills") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        AppError::Conflict(format!(
                            "AgentSession {} has an invalid skills projection",
                            response.conversation_id
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(AppError::Conflict(format!(
                "AgentSession {} has an invalid skills projection",
                response.conversation_id
            )));
        }
    };
    let temp_workspace_id = optional_string("temp_workspace_id");
    let agent_name = optional_string("agent_name");
    let cli_path = optional_string("cli_path").or_else(|| {
        response
            .extra
            .get("gateway")
            .and_then(|gateway| gateway.get("cli_path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    });
    let custom_agent_id = optional_string("custom_agent_id");
    Ok(nomifun_cron::CronSessionProjection {
        agent_session_id,
        owner_id: owner_id.to_owned(),
        name: response.name,
        agent_type: response.r#type,
        model: response.model,
        workspace,
        cron_job_id,
        temp_workspace_id,
        skills,
        agent_name,
        cli_path,
        custom_agent_id,
        preset_id: response.preset_id,
        preset_revision: response.preset_revision,
        agent_snapshot: response.agent_snapshot,
    })
}

struct NomiCoreAutoWorkPreSendHook {
    inner: Arc<dyn nomifun_requirement::AutoWorkPreSendHook>,
}

#[async_trait]
impl BackgroundTurnPreSendHook for NomiCoreAutoWorkPreSendHook {
    async fn prepare(&self) -> Result<(), AppError> {
        self.inner.prepare().await
    }
}

fn autowork_message_to_request(
    message: nomifun_requirement::AutoWorkMessage,
) -> SendMessageRequest {
    SendMessageRequest {
        content: message.content,
        files: message.files,
        inject_skills: message.inject_skills,
        hidden: message.hidden,
        origin: message.origin,
        channel_platform: message.channel_platform,
    }
}

fn autowork_delivery_from_conversation(
    delivery: IdempotentMessageDelivery,
) -> nomifun_requirement::AutoWorkMessageDelivery {
    nomifun_requirement::AutoWorkMessageDelivery {
        message_id: delivery.message_id,
        replayed: delivery.replayed,
        completed: delivery.completed,
        result_ok: delivery.result_ok,
        result_text: delivery.result_text,
        result_error: delivery.result_error,
        result_error_code: delivery.result_error_code,
        result_error_retryable: delivery.result_error_retryable,
    }
}

fn autowork_turn_state_from_conversation(
    state: PublicTurnDeliveryState,
) -> nomifun_requirement::AutoWorkTurnDeliveryState {
    match state {
        PublicTurnDeliveryState::Missing => {
            nomifun_requirement::AutoWorkTurnDeliveryState::Missing
        }
        PublicTurnDeliveryState::Accepted { message_id } => {
            nomifun_requirement::AutoWorkTurnDeliveryState::Accepted { message_id }
        }
        PublicTurnDeliveryState::Completed(delivery) => {
            nomifun_requirement::AutoWorkTurnDeliveryState::Completed(
                autowork_delivery_from_conversation(delivery),
            )
        }
    }
}

fn autowork_reconciliation_from_conversation(
    disposition: BackgroundTurnReconciliationDisposition,
) -> nomifun_requirement::AutoWorkReconciliationDisposition {
    match disposition {
        BackgroundTurnReconciliationDisposition::LiveExactOwnerWait => {
            nomifun_requirement::AutoWorkReconciliationDisposition::LiveExactOwnerWait
        }
        BackgroundTurnReconciliationDisposition::ReconciledOrTerminalReRead => {
            nomifun_requirement::AutoWorkReconciliationDisposition::ReconciledOrTerminalReRead
        }
        BackgroundTurnReconciliationDisposition::ExternalProofRequiredFailClosed => {
            nomifun_requirement::AutoWorkReconciliationDisposition::ExternalProofRequiredFailClosed
        }
        BackgroundTurnReconciliationDisposition::StaleConflict => {
            nomifun_requirement::AutoWorkReconciliationDisposition::StaleConflict
        }
    }
}

fn session_workspace(session: &ConversationResponse) -> Result<String, AppError> {
    session
        .extra
        .get("workspace")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|workspace| !workspace.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError::Conflict(format!(
                "AgentSession {} has no canonical workspace",
                session.conversation_id
            ))
        })
}

fn runtime_options_from_session(
    user_id: &str,
    session: ConversationResponse,
    cron_overlay: Option<&nomifun_cron::CronTurnRuntimeOverlay>,
) -> Result<(AgentRuntimeBuildOptions, String), AppError> {
    let ConversationResponse {
        conversation_id,
        r#type: agent_type,
        model,
        delegation_policy,
        created_at,
        extra: session_extra,
        ..
    } = session;

    let mut session_extra = session_extra.as_object().cloned().ok_or_else(|| {
        AppError::Internal(format!(
            "conversation {conversation_id} extra must be a JSON object"
        ))
    })?;
    if let Some(overlay) = cron_overlay {
        nomifun_common::CronJobId::parse(&overlay.cron_job_id).map_err(|error| {
            AppError::BadRequest(format!("invalid Cron runtime annotation: {error}"))
        })?;
        session_extra.insert(
            "cron_job_id".to_owned(),
            Value::String(overlay.cron_job_id.clone()),
        );
    }

    let workspace = session_extra
        .get("workspace")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::Internal(format!(
                "conversation {conversation_id} has no canonical workspace"
            ))
        })?
        .to_owned();

    Ok((
        AgentRuntimeBuildOptions {
            user_id: user_id.to_owned(),
            agent_type,
            workspace: workspace.clone(),
            model,
            conversation_id,
            delegation_policy,
            extra: Value::Object(session_extra).into(),
            conversation_created_at: Some(created_at),
            workspace_binding_lease: None,
        },
        workspace,
    ))
}

fn conversation_autowork_config_snapshot(
    session_id: &str,
    raw: Option<&Value>,
) -> Result<nomifun_requirement::AutoWorkConfigSnapshot, AppError> {
    let Some(raw) = raw else {
        return Ok(nomifun_requirement::AutoWorkConfigSnapshot {
            config: nomifun_requirement::AutoWorkConfig::default(),
            revision: "conversation:0".to_owned(),
            operation_id: None,
        });
    };
    let object = raw.as_object().ok_or_else(|| {
        AppError::Conflict(format!(
            "AgentSession {session_id} has an invalid AutoWork config"
        ))
    })?;
    let enabled = match object.get("enabled") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(AppError::Conflict(format!(
                "AgentSession {session_id} AutoWork enabled must be a boolean"
            )));
        }
    };
    let tag = match object.get("tag") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.as_str()),
        Some(_) => {
            return Err(AppError::Conflict(format!(
                "AgentSession {session_id} AutoWork tag must be a string"
            )));
        }
    };
    let max_requirements = match object.get("max_requirements") {
        None | Some(Value::Null) => None,
        Some(Value::Number(value)) => {
            let value = value.as_u64().ok_or_else(|| {
                AppError::Conflict(format!(
                    "AgentSession {session_id} AutoWork max_requirements must be an unsigned integer"
                ))
            })?;
            Some(u32::try_from(value).map_err(|_| {
                AppError::Conflict(format!(
                    "AgentSession {session_id} AutoWork max_requirements exceeds u32"
                ))
            })?)
        }
        Some(_) => {
            return Err(AppError::Conflict(format!(
                "AgentSession {session_id} AutoWork max_requirements must be an unsigned integer"
            )));
        }
    };
    let config = nomifun_requirement::AutoWorkConfig::normalize(
        enabled,
        tag,
        max_requirements,
    )
    .map_err(|error| {
        AppError::Conflict(format!(
            "AgentSession {session_id} has an invalid persisted AutoWork config: {error}"
        ))
    })?;
    let sequence = match object.get("_revision") {
        None | Some(Value::Null) => None,
        Some(Value::Number(value)) => Some(value.as_u64().ok_or_else(|| {
            AppError::Conflict(format!(
                "AgentSession {session_id} AutoWork revision must be an unsigned integer"
            ))
        })?),
        Some(_) => {
            return Err(AppError::Conflict(format!(
                "AgentSession {session_id} AutoWork revision must be an unsigned integer"
            )));
        }
    };
    let operation_id = match object.get("_operation_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
        Some(_) => {
            return Err(AppError::Conflict(format!(
                "AgentSession {session_id} AutoWork operation identity must be a non-empty string"
            )));
        }
    };
    let revision = match sequence {
        Some(sequence) => format!("conversation:{sequence}"),
        None => {
            let fingerprint = serde_json::to_string(&config).map_err(|error| {
                AppError::Internal(format!(
                    "failed to fingerprint AutoWork config for {session_id}: {error}"
                ))
            })?;
            format!(
                "conversation:legacy:{:x}",
                Sha256::digest(fingerprint.as_bytes())
            )
        }
    };
    Ok(nomifun_requirement::AutoWorkConfigSnapshot {
        config,
        revision,
        operation_id,
    })
}

fn conversation_autowork_sequence(revision: &str) -> u64 {
    revision
        .strip_prefix("conversation:")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

fn session_projection_revision(session: &ConversationResponse) -> Result<String, AppError> {
    let projection = json!({
        "conversation_id": session.conversation_id,
        "type": session.r#type,
        "model": session.model,
        "delegation_policy": session.delegation_policy,
        "execution_model_pool": session.execution_model_pool,
        "decision_policy": session.decision_policy,
        "execution_template_id": session.execution_template_id,
        "preset_id": session.preset_id,
        "preset_revision": session.preset_revision,
        "agent_snapshot": session.agent_snapshot,
        "source": session.source,
        "channel_chat_id": session.channel_chat_id,
        "created_at": session.created_at,
        "extra": session.extra,
    });
    let bytes = serde_json::to_vec(&projection).map_err(|error| {
        AppError::Internal(format!(
            "failed to fingerprint AgentSession {}: {error}",
            session.conversation_id
        ))
    })?;
    Ok(format!(
        "session:{:x}",
        Sha256::digest(bytes)
    ))
}

#[cfg(test)]
mod session_boundary_tests {
    use super::{
        companion_archive_message, conversation_autowork_config_snapshot,
        conversation_autowork_sequence, session_projection_revision,
    };
    use nomifun_api_types::MessageResponse;
    use nomifun_common::{
        AgentType, ConversationSource, ConversationStatus, DecisionPolicy, DelegationPolicy,
        MessagePosition, MessageType,
    };
    use serde_json::json;

    const SESSION_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678901";

    #[test]
    fn conversation_autowork_config_has_stable_legacy_and_explicit_revisions() {
        let legacy = json!({
            "enabled": true,
            "tag": "  release  ",
            "max_requirements": 3,
        });
        let first =
            conversation_autowork_config_snapshot(SESSION_ID, Some(&legacy)).unwrap();
        let second =
            conversation_autowork_config_snapshot(SESSION_ID, Some(&legacy)).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.config.tag.as_deref(), Some("release"));
        assert!(first.revision.starts_with("conversation:legacy:"));

        let versioned = json!({
            "enabled": true,
            "tag": "release",
            "max_requirements": 3,
            "_revision": 7,
            "_operation_id": "gateway:request-7",
        });
        let snapshot =
            conversation_autowork_config_snapshot(SESSION_ID, Some(&versioned)).unwrap();
        assert_eq!(snapshot.revision, "conversation:7");
        assert_eq!(
            snapshot.operation_id.as_deref(),
            Some("gateway:request-7")
        );
        assert_eq!(conversation_autowork_sequence(&snapshot.revision), 7);
    }

    #[test]
    fn conversation_autowork_config_rejects_malformed_persisted_values() {
        for raw in [
            json!({"enabled": "yes", "tag": "release"}),
            json!({"enabled": true, "tag": " \t "}),
            json!({"enabled": true, "tag": "release", "_revision": -1}),
            json!({"enabled": true, "tag": "release", "_operation_id": ""}),
        ] {
            assert!(
                conversation_autowork_config_snapshot(SESSION_ID, Some(&raw)).is_err()
            );
        }
    }

    #[test]
    fn companion_archive_projection_drops_non_dialogue_rows() {
        let message = |r#type, position, hidden, content| MessageResponse {
            message_id: "0190f5fe-7c00-7a00-8abc-000000000001".to_owned(),
            conversation_id: SESSION_ID.to_owned(),
            msg_id: None,
            r#type,
            content,
            position,
            status: None,
            hidden,
            created_at: 11,
        };
        assert_eq!(
            companion_archive_message(
                message(
                    MessageType::Text,
                    Some(MessagePosition::Right),
                    false,
                    json!({"content": "owner"}),
                ),
                10,
            )
            .map(|item| (item.is_user, item.content, item.created_at)),
            Some((true, "owner".to_owned(), 11))
        );
        assert!(
            companion_archive_message(
                message(
                    MessageType::ToolCall,
                    Some(MessagePosition::Left),
                    false,
                    json!({"content": "tool"}),
                ),
                10,
            )
            .is_none()
        );
        assert!(
            companion_archive_message(
                message(
                    MessageType::Text,
                    Some(MessagePosition::Center),
                    false,
                    json!("system"),
                ),
                10,
            )
            .is_none()
        );
    }

    #[test]
    fn session_revision_ignores_presentation_only_changes() {
        let mut base = super::ConversationResponse {
            conversation_id: SESSION_ID.to_owned(),
            name: "original".to_owned(),
            r#type: AgentType::Nomi,
            model: None,
            status: ConversationStatus::Finished,
            runtime: None,
            source: Some(ConversationSource::Nomifun),
            pinned: false,
            pinned_at: None,
            channel_chat_id: None,
            preset_id: None,
            preset_revision: None,
            agent_snapshot: None,
            delegation_policy: DelegationPolicy::Automatic,
            execution_model_pool: None,
            decision_policy: DecisionPolicy::Automatic,
            execution_template_id: None,
            linked_execution_id: None,
            execution_step_id: None,
            execution_attempt_id: None,
            created_at: 1,
            modified_at: 2,
            extra: json!({"workspace": "C:/workspace"}),
        };
        let original = session_projection_revision(&base).unwrap();
        base.name = "renamed".to_owned();
        base.pinned = true;
        base.pinned_at = Some(3);
        base.modified_at = 4;
        assert_eq!(session_projection_revision(&base).unwrap(), original);

        base.extra["workspace"] = json!("C:/other");
        assert_ne!(session_projection_revision(&base).unwrap(), original);
    }
}

async fn wait_for_runtime_subscription(
    runtime_registry: &Arc<dyn AgentRuntimeRegistry>,
    session_id: &str,
) -> Option<broadcast::Receiver<AgentStreamEvent>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(handle) = runtime_registry.get_runtime(session_id) {
            return Some(handle.subscribe());
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(
                session_id,
                "Nomi-core runtime did not register before channel relay subscription timeout"
            );
            return None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ---------------------------------------------------------------------------
// App-local Nomi-core HTTP adapter
// ---------------------------------------------------------------------------

/// Namespace for metadata persisted in the existing Conversation `extra`
/// object.  This is deliberately not a second table or a second runtime
/// identity: the Conversation id remains the Nomi-core Session id and the
/// ConversationService remains the lifecycle owner.
const NOMI_CORE_SESSION_METADATA_KEY: &str = "nomi_core_session";
const NOMI_CORE_SESSION_METADATA_VERSION: u64 = 1;
const NOMI_CORE_SESSION_KIND: &str = "agent_session";
const NOMI_CORE_REMOTE_KIND: &str = "remote_session";
const NOMI_CORE_MESSAGE_PAGE_SIZE: u32 = 100;
const NOMI_CORE_MAX_CURSOR_SCAN_PAGES: u32 = 512;
const NOMI_CORE_EVENT_LOG_UNAVAILABLE_CODE: &str = "NOMI_CORE_SESSION_EVENT_LOG_UNAVAILABLE";
const NOMI_CORE_FORK_UNAVAILABLE_CODE: &str = "NOMI_CORE_SESSION_FORK_UNAVAILABLE";
const NOMI_CORE_REMOTE_TURN_FINALIZER_TIMEOUT: Duration = Duration::from_secs(90);
const NOMI_CORE_REMOTE_TURN_FINALIZER_POLL: Duration = Duration::from_millis(100);
const NOMI_CORE_REMOTE_INITIAL_COMMAND_TIMEOUT: Duration = Duration::from_secs(150);
const NOMI_CORE_REMOTE_TURN_COMMAND_TIMEOUT: Duration = Duration::from_secs(150);
const NOMI_CORE_REMOTE_CANCEL_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// State shared by the app-local Agent Settings, AgentSession, and Remote
/// route builders.
///
/// `session_owner` is the same object that the normal Conversation, Channel,
/// Cron, AutoWork, Companion, IDMM, and AgentExecution wiring receives.  The
/// adapter never constructs an AgentRuntimeRegistry or a ConversationService.
#[derive(Clone)]
pub(crate) struct NomiCoreAgentApiState {
    pub(crate) session_owner: Arc<NomiCoreSessionOwner>,
    pub(crate) control_plane: Arc<AgentControlPlane>,
    pub(crate) remote_repository: Arc<dyn IRemoteBindingRepository>,
    pub(crate) remote_runtime: super::remote_runtime::NomiCoreRemoteRuntimeCoordinator,
}

impl NomiCoreAgentApiState {
    pub(crate) fn new(
        session_owner: Arc<NomiCoreSessionOwner>,
        control_plane: Arc<AgentControlPlane>,
        remote_repository: Arc<dyn IRemoteBindingRepository>,
        remote_runtime: super::remote_runtime::NomiCoreRemoteRuntimeCoordinator,
    ) -> Self {
        Self {
            session_owner,
            control_plane,
            remote_repository,
            remote_runtime,
        }
    }

}

#[derive(Debug)]
enum NomiCoreRemoteDetachedFailure<E> {
    Failed(E),
    TimedOut,
    Panicked,
    Admission(super::remote_runtime::RemoteDetachedMutationAdmissionError),
}

/// Run one Nomi-core Remote mutation under a bounded waiter while retaining
/// the actual future after a timeout. Dropping the JoinHandle deliberately
/// detaches the operation; the permit remains owned by that task so a retry
/// with the same idempotency key cannot start a second command.
async fn run_nomi_core_remote_detached<T, E, F>(
    state: &NomiCoreAgentApiState,
    key: String,
    timeout: Duration,
    future: F,
) -> Result<T, NomiCoreRemoteDetachedFailure<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Future<Output = Result<T, E>> + Send + 'static,
{
    let (result_tx, result_rx) =
        tokio::sync::oneshot::channel::<Result<Result<T, E>, ()>>();
    state
        .remote_runtime
        .start_once(key, move || async move {
            let result = std::panic::AssertUnwindSafe(future).catch_unwind().await;
            let signal = match result {
                Ok(result) => Ok(result),
                Err(_) => Err(()),
            };
            let _ = result_tx.send(signal);
        })
        .map_err(NomiCoreRemoteDetachedFailure::Admission)?;
    match tokio::time::timeout(timeout, result_rx).await {
        Ok(Ok(Ok(Ok(value)))) => Ok(value),
        Ok(Ok(Ok(Err(error)))) => Err(NomiCoreRemoteDetachedFailure::Failed(error)),
        Ok(Ok(Err(()))) | Ok(Err(_)) => Err(NomiCoreRemoteDetachedFailure::Panicked),
        Err(_) => Err(NomiCoreRemoteDetachedFailure::TimedOut),
    }
}

/// Build all app-local Nomi-core Agent Settings and AgentSession endpoints.
///
/// The returned router is intentionally independent of the top-level auth
/// middleware.  It only projects `CurrentUser` into the control-plane's
/// `AuthenticatedOwner` extension; `routes.rs` remains responsible for
/// choosing the authentication/installation-owner policy around this router.
pub(crate) fn build_nomi_core_agent_router(state: NomiCoreAgentApiState) -> Router {
    Router::new()
        .merge(nomi_core_session_routes(state.clone()))
        .merge(control_plane_router_without_legacy_skills(
            state.control_plane.clone(),
        ))
        .route_layer(from_fn(project_authenticated_owner))
}

/// Build the Nomi-core Remote REST surface with both local/JWT owner
/// authentication and the installation-scoped Bearer token.
///
/// Remote is a separately authenticated transport boundary. The installation
/// token is translated directly to the canonical owner identity; it never
/// creates a second Session authority or bypasses the owner checks in the
/// handlers.
pub(crate) fn build_nomi_core_remote_router(
    state: NomiCoreAgentApiState,
    validator: Arc<InstanceTokenValidator>,
    authoritative_user_id: Arc<str>,
    jwt_service: Arc<JwtService>,
    user_repo: Arc<dyn nomifun_db::IUserRepository>,
) -> Router {
    Router::new()
        .merge(nomi_core_remote_routes(state))
        .route_layer(from_fn(reject_nomi_core_remote_query_parameters))
        .route_layer(from_fn_with_state(
            NomiCoreRemoteAuthState {
                validator,
                authoritative_user_id,
                jwt_service,
                user_repo,
            },
            authenticate_nomi_core_remote,
        ))
}

fn nomi_core_session_routes(state: NomiCoreAgentApiState) -> Router {
    Router::new()
        .route("/api/agent-sessions", post(create_nomi_core_agent_session))
        .route(
            "/api/agent-sessions/{agent_session_id}",
            get(get_nomi_core_agent_session).delete(delete_nomi_core_agent_session),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/capabilities",
            get(get_nomi_core_agent_session_capabilities),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/turns",
            post(start_nomi_core_agent_session_turn),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/events",
            get(get_nomi_core_agent_session_events),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/messages",
            get(get_nomi_core_agent_session_messages),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/forks",
            post(fork_nomi_core_agent_session),
        )
        .with_state(state)
}

fn nomi_core_remote_routes(state: NomiCoreAgentApiState) -> Router {
    Router::new()
        .route("/api/remote/open", post(open_nomi_core_remote))
        .route("/api/remote/turn", post(turn_nomi_core_remote))
        .route("/api/remote/observe", get(observe_nomi_core_remote))
        .route("/api/remote/cancel", post(cancel_nomi_core_remote))
        .with_state(state)
}

pub(crate) async fn run_nomi_core_remote_open(
    state: NomiCoreAgentApiState,
    owner: &nomifun_common::UserId,
    request: RemoteOpenRequestDto,
) -> Result<Value, nomifun_public::CanonicalRemoteOperationError> {
    let response = open_nomi_core_remote(
        State(state),
        Extension(AuthenticatedOwner(nomifun_agent_contracts::UserId::from(
            owner.as_ref().to_owned(),
        ))),
        Json(request),
    )
    .await
    .map_err(NomiCoreApiError::into_remote_operation_error)?;
    serde_json::to_value(response.0).map_err(|error| {
        nomifun_public::CanonicalRemoteOperationError::new(
            "REMOTE_RESPONSE_SERIALIZATION_FAILED",
            format!("Nomi-core Remote open response could not be serialized: {error}"),
        )
    })
}

pub(crate) async fn run_nomi_core_remote_turn(
    state: NomiCoreAgentApiState,
    owner: &nomifun_common::UserId,
    request: RemoteTurnRequestDto,
) -> Result<Value, nomifun_public::CanonicalRemoteOperationError> {
    let response = turn_nomi_core_remote(
        State(state),
        Extension(AuthenticatedOwner(nomifun_agent_contracts::UserId::from(
            owner.as_ref().to_owned(),
        ))),
        Json(request),
    )
    .await
    .map_err(NomiCoreApiError::into_remote_operation_error)?;
    serde_json::to_value(response.0).map_err(|error| {
        nomifun_public::CanonicalRemoteOperationError::new(
            "REMOTE_RESPONSE_SERIALIZATION_FAILED",
            format!("Nomi-core Remote turn response could not be serialized: {error}"),
        )
    })
}

pub(crate) async fn run_nomi_core_remote_observe(
    state: NomiCoreAgentApiState,
    owner: &nomifun_common::UserId,
    request: RemoteObserveRequestDto,
) -> Result<Value, nomifun_public::CanonicalRemoteOperationError> {
    if request.after_cursor.agent_session_id != request.agent_session_id {
        return Err(nomifun_public::CanonicalRemoteOperationError::new(
            "REMOTE_SESSION_NOT_FOUND",
            "after_cursor must reference the same AgentSession",
        ));
    }
    let response = observe_nomi_core_remote(
        State(state),
        Extension(AuthenticatedOwner(nomifun_agent_contracts::UserId::from(
            owner.as_ref().to_owned(),
        ))),
        Query(NomiCoreRemoteObserveQuery {
            agent_session_id: request.agent_session_id,
            after_seq: request.after_cursor.seq,
            limit: request.limit,
        }),
    )
    .await
    .map_err(NomiCoreApiError::into_remote_operation_error)?;
    serde_json::to_value(response.0).map_err(|error| {
        nomifun_public::CanonicalRemoteOperationError::new(
            "REMOTE_RESPONSE_SERIALIZATION_FAILED",
            format!("Nomi-core Remote observe response could not be serialized: {error}"),
        )
    })
}

pub(crate) async fn run_nomi_core_remote_cancel(
    state: NomiCoreAgentApiState,
    owner: &nomifun_common::UserId,
    request: RemoteCancelRequestDto,
) -> Result<Value, nomifun_public::CanonicalRemoteOperationError> {
    let response = cancel_nomi_core_remote(
        State(state),
        Extension(AuthenticatedOwner(nomifun_agent_contracts::UserId::from(
            owner.as_ref().to_owned(),
        ))),
        Json(request),
    )
    .await
    .map_err(NomiCoreApiError::into_remote_operation_error)?;
    serde_json::to_value(response.0).map_err(|error| {
        nomifun_public::CanonicalRemoteOperationError::new(
            "REMOTE_RESPONSE_SERIALIZATION_FAILED",
            format!("Nomi-core Remote cancel response could not be serialized: {error}"),
        )
    })
}

#[derive(Clone)]
struct NomiCoreRemoteAuthState {
    validator: Arc<InstanceTokenValidator>,
    authoritative_user_id: Arc<str>,
    jwt_service: Arc<JwtService>,
    user_repo: Arc<dyn nomifun_db::IUserRepository>,
}

async fn authenticate_nomi_core_remote(
    State(state): State<NomiCoreRemoteAuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    let owner = if let Some(current) = request.extensions().get::<CurrentUser>() {
        if current.id.as_str() != state.authoritative_user_id.as_ref() {
            return (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse::new(
                    "Remote access is restricted to the installation owner",
                    "REMOTE_OWNER_REQUIRED",
                )),
            )
                .into_response();
        }
        current.id.as_str().to_owned().into()
    } else {
        let presented = extract_token_from_headers(request.headers()).unwrap_or_default();
        if state.validator.validate(&presented) {
            state.authoritative_user_id.as_ref().to_owned().into()
        } else {
            let Ok(payload) = state.jwt_service.verify(&presented) else {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorResponse::new(
                        "Remote installation authentication is required",
                        "REMOTE_AUTH_REQUIRED",
                    )),
                )
                    .into_response();
            };
            let user = match state.user_repo.find_by_id(payload.user_id.as_str()).await {
                Ok(Some(user)) => user,
                Ok(None) => {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(ErrorResponse::new(
                            "Remote installation authentication is required",
                            "REMOTE_AUTH_REQUIRED",
                        )),
                    )
                        .into_response();
                }
                Err(_) => {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(ErrorResponse::new(
                            "Remote authentication is temporarily unavailable",
                            "REMOTE_AUTH_UNAVAILABLE",
                        )),
                    )
                        .into_response();
                }
            };
            if user.user_id.as_str() != state.authoritative_user_id.as_ref() {
                return (
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse::new(
                        "Remote access is restricted to the installation owner",
                        "REMOTE_OWNER_REQUIRED",
                    )),
                )
                    .into_response();
            }
            payload.user_id.as_str().to_owned().into()
        }
    };
    request.extensions_mut().insert(AuthenticatedOwner(owner));
    next.run(request).await
}

async fn reject_nomi_core_remote_query_parameters(request: Request, next: Next) -> Response {
    let Some(query) = request.uri().query() else {
        return next.run(request).await;
    };
    let allow_observe = request.uri().path() == "/api/remote/observe";
    let invalid = url::form_urlencoded::parse(query.as_bytes()).any(|(key, _)| {
        !allow_observe || !matches!(key.as_ref(), "agent_session_id" | "after_seq" | "limit")
    });
    if invalid {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "Nomi-core Remote endpoints do not accept undeclared query parameters",
                "REMOTE_INVALID_REQUEST",
            )),
        )
            .into_response();
    }
    next.run(request).await
}

async fn project_authenticated_owner(
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let owner_id = request
        .extensions()
        .get::<nomifun_auth::CurrentUser>()
        .map(|current| current.id.as_str().to_owned())
        .ok_or_else(|| AppError::Forbidden("Authentication required".into()))?;
    request
        .extensions_mut()
        .insert(AuthenticatedOwner(owner_id.into()));
    Ok(next.run(request).await)
}

#[derive(Debug)]
pub(crate) struct NomiCoreApiError {
    status: StatusCode,
    code: String,
    message: String,
    details: Option<Value>,
}

impl NomiCoreApiError {
    fn new(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    fn with_details(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            details: Some(details),
        }
    }

    fn unsupported_session_events(session_id: &AgentSessionId) -> Self {
        Self::with_details(
            StatusCode::NOT_IMPLEMENTED,
            NOMI_CORE_EVENT_LOG_UNAVAILABLE_CODE,
            "Nomi-core ConversationService has no durable SessionEvent replay port",
            json!({
                "agent_session_id": session_id,
                "outcome": "not_available",
                "recovery": "use the messages projection or integrate a canonical SessionEvent store",
                "cursor": "not_issued",
            }),
        )
    }

    pub(crate) fn into_remote_operation_error(
        self,
    ) -> nomifun_public::CanonicalRemoteOperationError {
        match self.details {
            Some(details) => nomifun_public::CanonicalRemoteOperationError::with_details(
                self.code,
                self.message,
                details,
            ),
            None => nomifun_public::CanonicalRemoteOperationError::new(self.code, self.message),
        }
    }
}

impl IntoResponse for NomiCoreApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse::new_with_details(
                self.message,
                self.code,
                self.details,
            )),
        )
            .into_response()
    }
}

impl From<AppError> for NomiCoreApiError {
    fn from(error: AppError) -> Self {
        Self::with_details(
            error.status_code(),
            error.error_code(),
            error.to_string(),
            error.error_details().unwrap_or(Value::Null),
        )
    }
}

impl From<ControlPlaneError> for NomiCoreApiError {
    fn from(error: ControlPlaneError) -> Self {
        Self::with_details(
            error.status(),
            error.code().as_ref(),
            error.to_string(),
            error.details().unwrap_or(Value::Null),
        )
    }
}

impl From<serde_json::Error> for NomiCoreApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "NOMI_CORE_INVALID_REQUEST",
            format!("Nomi-core request conversion failed: {error}"),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NomiCoreSessionMetadata {
    version: u64,
    kind: String,
    binding: AgentBindingValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote: Option<RemoteBindingProvenance>,
}

#[derive(Debug, Serialize)]
struct NomiCoreAgentSessionCapabilityResponse {
    resolved_snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef,
    generation: u64,
    initial_capabilities: Vec<String>,
    on_demand_capabilities: Vec<String>,
    active_capabilities: Vec<String>,
    compact_on_demand_index:
        Vec<nomifun_agent_contracts::CompactOnDemandCapabilityEntry>,
    /// Explicitly distinguishes saved-preset projection from a live Kernel
    /// active-set query.  It prevents a consumer from mistaking generation 0
    /// for an unimplemented empty response.
    state_source: &'static str,
}

#[derive(Debug, Serialize)]
struct NomiCoreAgentSessionMessagePageResponse {
    agent_session_id: String,
    messages: Vec<MessageProjection>,
    next_cursor: SessionCursorDto,
}

#[derive(Debug, Serialize)]
struct NomiCoreAgentSessionDeleteResponse {
    agent_session_id: String,
    state: &'static str,
    deleted_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NomiCoreSessionPageQuery {
    #[serde(default)]
    after_seq: u64,
    #[serde(default = "default_nomi_core_page_limit")]
    limit: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NomiCoreRemoteObserveQuery {
    agent_session_id: String,
    #[serde(default)]
    after_seq: u64,
    #[serde(default = "default_nomi_core_page_limit")]
    limit: u32,
}

fn default_nomi_core_page_limit() -> u32 {
    100
}

fn remote_store_unavailable(operation: &'static str) -> NomiCoreApiError {
    NomiCoreApiError::with_details(
        StatusCode::SERVICE_UNAVAILABLE,
        "NOMI_CORE_REMOTE_STORE_UNAVAILABLE",
        format!("Nomi-core Remote {operation} could not be durably recorded"),
        json!({
            "outcome": "unknown",
            "recovery": "retry the same idempotency key and inspect the existing Session",
        }),
    )
}

fn remote_db_error(
    error: nomifun_db::DbError,
    operation: &'static str,
) -> NomiCoreApiError {
    match error {
        nomifun_db::DbError::NotFound(_) => NomiCoreApiError::new(
            StatusCode::NOT_FOUND,
            "REMOTE_SESSION_NOT_FOUND",
            "the Remote Session does not exist for the authenticated owner",
        ),
        nomifun_db::DbError::Conflict(_) => NomiCoreApiError::with_details(
            StatusCode::CONFLICT,
            "REMOTE_IDEMPOTENCY_CONFLICT",
            format!("Nomi-core Remote {operation} conflicts with an existing durable state"),
            json!({
                "outcome": "rejected",
                "recovery": "reuse the original request identity or choose a new idempotency key",
            }),
        ),
        _ => remote_store_unavailable(operation),
    }
}

fn remote_state_view(state: &str) -> Result<RemoteOpenStateViewDto, NomiCoreApiError> {
    match state {
        "opening" => Ok(RemoteOpenStateViewDto::Opening),
        "ready" => Ok(RemoteOpenStateViewDto::Ready),
        "failed" => Ok(RemoteOpenStateViewDto::Failed {
            code: "REMOTE_OPEN_FAILED".to_owned(),
            recoverable: true,
        }),
        "cancelled" => Ok(RemoteOpenStateViewDto::Failed {
            code: "REMOTE_SESSION_CANCELLED".to_owned(),
            recoverable: false,
        }),
        _ => Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_REMOTE_STATE_INVALID",
            "the persisted Remote Session state is invalid",
        )),
    }
}

fn remote_status_label(state: &str) -> &'static str {
    match state {
        "opening" => "opening",
        "ready" => "ready",
        "failed" => "failed",
        "cancelled" => "cancelled",
        _ => "unknown",
    }
}

fn remote_binding_digest(binding: &AgentBindingValue) -> Result<String, NomiCoreApiError> {
    nomifun_agent_contracts::digest_payload(binding)
        .map(|digest| digest.as_ref().to_owned())
        .map_err(|error| {
            NomiCoreApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "NOMI_CORE_REMOTE_BINDING_DIGEST_FAILED",
                format!("Nomi-core Remote binding digest could not be computed: {error}"),
            )
        })
}

fn remote_operation_key_digest(key: &str) -> Result<String, NomiCoreApiError> {
    nomifun_agent_contracts::digest_payload(&json!({ "idempotency_key": key }))
        .map(|digest| digest.as_ref().to_owned())
        .map_err(|error| {
            NomiCoreApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "NOMI_CORE_REMOTE_IDEMPOTENCY_DIGEST_FAILED",
                format!("Nomi-core Remote operation identity could not be computed: {error}"),
            )
        })
}

fn remote_event_value(row: &NomiRemoteEventRow) -> Result<Value, NomiCoreApiError> {
    let payload: Value = serde_json::from_str(&row.payload_json).map_err(|error| {
        NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_REMOTE_EVENT_INVALID",
            format!("persisted Remote event payload is invalid: {error}"),
        )
    })?;
    Ok(json!({
        "event_id": row.event_id,
        "agent_session_id": row.agent_session_id,
        "seq": row.seq,
        "kind": row.event_type,
        "event_type": row.event_type,
        "payload": payload,
        "created_at": row.created_at,
    }))
}

fn is_remote_terminal_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "turn/completed"
            | "turn/failed"
            | "turn/unknown"
            | "session/cancelled"
            | "session/cancel-rejected"
            | "session/cancel-unknown"
    )
}

async fn remote_terminal_event_type(
    repository: &Arc<dyn IRemoteBindingRepository>,
    owner_id: &str,
    session_id: &str,
    operation_key: &str,
) -> Result<Option<String>, NomiCoreApiError> {
    let operation_digest = remote_operation_key_digest(operation_key)?;
    let page = repository
        .read_events(owner_id, session_id, 0, 1000)
        .await
        .map_err(|error| remote_db_error(error, "terminal event lookup"))?;
    for event in page.events {
        if !is_remote_terminal_event(&event.event_type) {
            continue;
        }
        let same_operation = serde_json::from_str::<Value>(&event.payload_json)
            .ok()
            .and_then(|payload| {
                payload
                    .get("operation_key_digest")
                    .and_then(Value::as_str)
                    .map(|digest| digest == operation_digest)
            })
            .unwrap_or(false);
        if same_operation {
            return Ok(Some(event.event_type));
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteCancelEventState {
    Requested,
    Unknown,
    Rejected,
    Cancelled,
}

fn remote_cancel_request_digest(
    session_id: &AgentSessionId,
    operation_key: &str,
) -> Result<String, NomiCoreApiError> {
    remote_input_digest(&json!({
        "operation": "remote.cancel",
        "agent_session_id": session_id,
        "idempotency_key": operation_key,
    }))
}

/// Read the latest durable cancellation fact for the requested key and report
/// whether another unresolved cancellation currently fences the Session.
///
/// Cancellation is a mutation, so the idempotency key itself is not enough:
/// the first request also persists a digest of the exact `(Session, key)`
/// scope.  Reusing a key for a different scope is rejected before any runtime
/// command is issued.
async fn remote_cancel_event_state(
    repository: &Arc<dyn IRemoteBindingRepository>,
    owner_id: &str,
    session_id: &str,
    operation_key: &str,
    request_digest: &str,
) -> Result<(Option<RemoteCancelEventState>, bool), NomiCoreApiError> {
    let operation_digest = remote_operation_key_digest(operation_key)?;
    let page = repository
        .read_events(owner_id, session_id, 0, 1000)
        .await
        .map_err(|error| remote_db_error(error, "cancel event lookup"))?;
    let mut latest = BTreeMap::<String, RemoteCancelEventState>::new();
    for event in page.events {
        let state = match event.event_type.as_str() {
            "session/cancel-requested" => RemoteCancelEventState::Requested,
            "session/cancel-unknown" => RemoteCancelEventState::Unknown,
            "session/cancel-rejected" => RemoteCancelEventState::Rejected,
            "session/cancelled" => RemoteCancelEventState::Cancelled,
            _ => continue,
        };
        let payload: Value = serde_json::from_str(&event.payload_json).map_err(|error| {
            NomiCoreApiError::new(
                StatusCode::CONFLICT,
                "NOMI_CORE_REMOTE_EVENT_INVALID",
                format!("persisted Remote cancellation event is invalid: {error}"),
            )
        })?;
        let Some(event_operation_digest) = payload
            .get("operation_key_digest")
            .and_then(Value::as_str)
        else {
            continue;
        };
        if event_operation_digest == operation_digest
            && let Some(stored_request_digest) =
                payload.get("request_digest").and_then(Value::as_str)
            && stored_request_digest != request_digest
        {
            return Err(NomiCoreApiError::with_details(
                StatusCode::CONFLICT,
                "REMOTE_IDEMPOTENCY_CONFLICT",
                "the Remote cancel key was reused for a different Session request",
                json!({
                    "outcome": "rejected",
                    "recovery": "reuse the original cancel request or choose a new idempotency key",
                }),
            ));
        }
        latest.insert(event_operation_digest.to_owned(), state);
    }
    let requested = latest.get(&operation_digest).copied();
    let other_active = latest.iter().any(|(digest, state)| {
        digest != &operation_digest
            && matches!(
                state,
                RemoteCancelEventState::Requested | RemoteCancelEventState::Unknown
            )
    });
    Ok((requested, other_active))
}

async fn remote_has_active_cancel_fence(
    repository: &Arc<dyn IRemoteBindingRepository>,
    owner_id: &str,
    session_id: &str,
) -> Result<bool, NomiCoreApiError> {
    let page = repository
        .read_events(owner_id, session_id, 0, 1000)
        .await
        .map_err(|error| remote_db_error(error, "cancel fence lookup"))?;
    let mut latest = BTreeMap::<String, RemoteCancelEventState>::new();
    for event in page.events {
        let state = match event.event_type.as_str() {
            "session/cancel-requested" => RemoteCancelEventState::Requested,
            "session/cancel-unknown" => RemoteCancelEventState::Unknown,
            "session/cancel-rejected" => RemoteCancelEventState::Rejected,
            "session/cancelled" => RemoteCancelEventState::Cancelled,
            _ => continue,
        };
        let payload: Value = serde_json::from_str(&event.payload_json).map_err(|error| {
            NomiCoreApiError::new(
                StatusCode::CONFLICT,
                "NOMI_CORE_REMOTE_EVENT_INVALID",
                format!("persisted Remote cancellation event is invalid: {error}"),
            )
        })?;
        if let Some(operation_digest) = payload
            .get("operation_key_digest")
            .and_then(Value::as_str)
        {
            latest.insert(operation_digest.to_owned(), state);
        }
    }
    Ok(latest.values().any(|state| {
        matches!(
            state,
            RemoteCancelEventState::Requested | RemoteCancelEventState::Unknown
        )
    }))
}

async fn append_remote_event_once(
    repository: &Arc<dyn IRemoteBindingRepository>,
    owner_id: &str,
    session_id: &str,
    event_type: &str,
    operation_key: Option<&str>,
    payload: Value,
) -> Result<Option<Value>, NomiCoreApiError> {
    Ok(
        append_remote_event_with_inserted(
            repository,
            owner_id,
            session_id,
            event_type,
            operation_key,
            payload,
        )
        .await?
        .map(|(event, _inserted)| event),
    )
}

fn remote_delivery_terminal_event_type(
    completed: bool,
    result_ok: Option<bool>,
) -> (&'static str, &'static str, bool) {
    if !completed {
        return ("turn/accepted", "accepted", false);
    }
    match result_ok {
        Some(true) => ("turn/completed", "completed", true),
        Some(false) => ("turn/failed", "failed", false),
        // `completed=true` without an explicit result is not success. Keep
        // the Remote projection fail-closed and preserve the unknown outcome
        // against any late completion callback.
        None => ("turn/unknown", "unknown", false),
    }
}

/// Append one Remote event and retain whether this call won the repository
/// idempotency race.  Callers that cross an external/runtime boundary must use
/// the `inserted` bit: a repository replay row is not permission to execute
/// the mutation a second time.
async fn append_remote_event_with_inserted(
    repository: &Arc<dyn IRemoteBindingRepository>,
    owner_id: &str,
    session_id: &str,
    event_type: &str,
    operation_key: Option<&str>,
    mut payload: Value,
) -> Result<Option<(Value, bool)>, NomiCoreApiError> {
    let operation_digest = operation_key
        .map(remote_operation_key_digest)
        .transpose()?;
    let object = payload.as_object_mut().ok_or_else(|| {
        NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "NOMI_CORE_REMOTE_EVENT_INVALID",
            "Remote event payload must be a JSON object",
        )
    })?;
    if let Some(operation_digest) = operation_digest.as_ref() {
        object.insert(
            "operation_key_digest".to_owned(),
            Value::String(operation_digest.clone()),
        );
    }
    let canonical_payload =
        nomifun_agent_contracts::canonical_json_bytes(&payload).map_err(|error| {
            NomiCoreApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "NOMI_CORE_REMOTE_EVENT_INVALID",
                format!("Remote event payload cannot be canonicalized: {error}"),
            )
        })?;
    if let Some(operation_digest) = operation_digest.as_deref() {
        let existing = repository
            .read_events(owner_id, session_id, 0, 1000)
            .await
            .map_err(|error| remote_db_error(error, "event lookup"))?;
        for event in &existing.events {
            let same_operation = serde_json::from_str::<Value>(&event.payload_json)
                .ok()
                .and_then(|value| {
                    value
                        .get("operation_key_digest")
                        .and_then(Value::as_str)
                        .map(|value| value == operation_digest)
                })
                .unwrap_or(false);
            if !same_operation {
                continue;
            }
            if event.event_type == event_type {
                let existing_payload: Value =
                    serde_json::from_str(&event.payload_json).map_err(|error| {
                        NomiCoreApiError::new(
                            StatusCode::CONFLICT,
                            "NOMI_CORE_REMOTE_EVENT_INVALID",
                            format!("persisted Remote event payload is invalid: {error}"),
                        )
                    })?;
                let existing_payload =
                    nomifun_agent_contracts::canonical_json_bytes(&existing_payload).map_err(
                        |error| {
                            NomiCoreApiError::new(
                                StatusCode::CONFLICT,
                                "NOMI_CORE_REMOTE_EVENT_INVALID",
                                format!(
                                    "persisted Remote event payload cannot be canonicalized: \
                                     {error}"
                                ),
                            )
                        },
                    )?;
                if existing_payload != canonical_payload {
                    return Err(NomiCoreApiError::with_details(
                        StatusCode::CONFLICT,
                        "REMOTE_IDEMPOTENCY_CONFLICT",
                        "the Remote event key was reused with a different payload",
                        json!({
                            "outcome": "rejected",
                            "recovery": "reuse the original event request or choose a new key",
                        }),
                    ));
                }
                // Exact same event identity: the durable repository row is
                // the replay result, and no boundary operation may run again.
                return Ok(None);
            }
            if is_remote_terminal_event(&event.event_type)
                && is_remote_terminal_event(event_type)
            {
                // A terminal outcome is absorbing. In particular, a late
                // completion must never rewrite a previously recorded
                // `unknown` outcome into success.
                return Ok(None);
            }
        }
    }

    let payload_json = serde_json::to_string(&payload)?;
    let row = repository
        .append_event_once(AppendNomiRemoteEventParams {
            owner_user_id: owner_id.to_owned(),
            agent_session_id: session_id.to_owned(),
            event_type: event_type.to_owned(),
            payload_json,
        })
        .await
        .map_err(|error| remote_db_error(error, "event append"))?;
    if !row.inserted {
        return Ok(None);
    }
    Ok(Some((remote_event_value(&row.event)?, true)))
}

fn validate_remote_session_binding(
    row: &NomiRemoteSessionRow,
    owner_id: &str,
    binding: &AgentBindingValue,
    remote_binding_id: &str,
) -> Result<(), NomiCoreApiError> {
    let expected_digest = remote_binding_digest(binding)?;
    if row.owner_user_id != owner_id
        || row.remote_binding_id != remote_binding_id
        || row.binding_version
            != i64::try_from(binding.binding_version).unwrap_or_default()
        || row.agent_binding_digest != expected_digest
    {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_REMOTE_PROVENANCE_CONFLICT",
            "the durable Remote Session provenance does not match the requested binding",
        ));
    }
    let persisted: AgentBindingValue =
        serde_json::from_str(&row.agent_binding_json).map_err(|error| {
            NomiCoreApiError::new(
                StatusCode::CONFLICT,
                "NOMI_CORE_REMOTE_PROVENANCE_INVALID",
                format!("persisted Remote binding is invalid: {error}"),
            )
        })?;
    if persisted != *binding {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_REMOTE_PROVENANCE_CONFLICT",
            "the durable Remote Session binding differs from the requested binding",
        ));
    }
    Ok(())
}

async fn remote_session_projection(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
) -> Result<NomiRemoteSessionRow, NomiCoreApiError> {
    state
        .remote_repository
        .get_session(owner.as_ref(), session_id.as_ref())
        .await
        .map_err(|error| remote_db_error(error, "session lookup"))?
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::NOT_FOUND,
                "REMOTE_SESSION_NOT_FOUND",
                "the Remote Session does not exist for the authenticated owner",
            )
        })
}

fn remote_input_digest(input: &Value) -> Result<String, NomiCoreApiError> {
    nomifun_agent_contracts::digest_payload(input)
        .map(|digest| digest.as_ref().to_owned())
        .map_err(|error| {
            NomiCoreApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "NOMI_CORE_REMOTE_INPUT_DIGEST_FAILED",
                format!("Nomi-core Remote input identity could not be computed: {error}"),
            )
        })
}

async fn remote_open_input_digest(
    repository: &Arc<dyn IRemoteBindingRepository>,
    owner_id: &str,
    session_id: &str,
) -> Result<Option<String>, NomiCoreApiError> {
    let page = repository
        .read_events(owner_id, session_id, 0, 1000)
        .await
        .map_err(|error| remote_db_error(error, "open event lookup"))?;
    let opening = page
        .events
        .iter()
        .find(|event| event.event_type == "session/opening");
    let Some(event) = opening else {
        return Ok(None);
    };
    let payload: Value = serde_json::from_str(&event.payload_json).map_err(|error| {
        NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_REMOTE_EVENT_INVALID",
            format!("persisted Remote opening event is invalid: {error}"),
        )
    })?;
    Ok(payload
        .get("initial_input_digest")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned))
}

async fn remote_open_response(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    row: &NomiRemoteSessionRow,
    binding: AgentBindingValueDto,
) -> Result<Json<RemoteOpenResponseDto>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&row.agent_session_id)?;
    let response = load_owned_nomi_core_session(state, owner, &session_id).await?;
    let metadata = session_metadata(&response, owner)?;
    let remote = metadata.remote.as_ref().ok_or_else(remote_session_not_found)?;
    if remote.remote_binding_id.as_ref() != row.remote_binding_id
        || remote.binding_version != u64::try_from(row.binding_version).unwrap_or_default()
    {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_REMOTE_PROVENANCE_CONFLICT",
            "the Conversation metadata does not match the durable Remote projection",
        ));
    }
    let event_cursor = remote_event_cursor(state, owner, &session_id).await?;
    Ok(Json(RemoteOpenResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        agent_binding: binding,
        open_state: remote_state_view(&row.state)?,
        cursor: event_cursor,
    }))
}

async fn remote_event_cursor(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
) -> Result<SessionCursorDto, NomiCoreApiError> {
    let seq = state
        .remote_repository
        .current_event_cursor(owner.as_ref(), session_id.as_ref())
        .await
        .map_err(|error| remote_db_error(error, "event cursor lookup"))?;
    let seq = u64::try_from(seq).map_err(|_| {
        NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_REMOTE_CURSOR_INVALID",
            "the persisted Remote event cursor is invalid",
        )
    })?;
    Ok(session_cursor(session_id, seq))
}

async fn transition_remote_state(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
    current: NomiRemoteSessionRow,
    next_state: &str,
    operation_key: &str,
    payload: Value,
) -> Result<NomiRemoteSessionRow, NomiCoreApiError> {
    transition_remote_state_with_repository(
        &state.remote_repository,
        owner.as_ref(),
        session_id,
        current,
        next_state,
        operation_key,
        payload,
    )
    .await
}

async fn transition_remote_state_with_repository(
    repository: &Arc<dyn IRemoteBindingRepository>,
    owner_id: &str,
    session_id: &AgentSessionId,
    current: NomiRemoteSessionRow,
    next_state: &str,
    operation_key: &str,
    mut payload: Value,
) -> Result<NomiRemoteSessionRow, NomiCoreApiError> {
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "state".to_owned(),
            Value::String(next_state.to_owned()),
        );
    }
    let event_type = match next_state {
        "ready" => "session/ready",
        "failed" => "session/open-failed",
        "cancelled" => "session/cancelled",
        _ => "session/state-changed",
    };
    let object = payload.as_object_mut().ok_or_else(|| {
        NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "NOMI_CORE_REMOTE_EVENT_INVALID",
            "Remote state transition payload must be a JSON object",
        )
    })?;
    let operation_digest = remote_operation_key_digest(operation_key)?;
    object.insert(
        "operation_key_digest".to_owned(),
        Value::String(operation_digest.clone()),
    );
    let payload_json = serde_json::to_string(&payload)?;
    let canonical_payload = nomifun_agent_contracts::canonical_json_bytes(&payload).map_err(
        |error| {
            NomiCoreApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "NOMI_CORE_REMOTE_EVENT_INVALID",
                format!("Remote state transition payload cannot be canonicalized: {error}"),
            )
        },
    )?;
    let event_type = event_type.to_owned();

    let result = repository
        .transition_session_state_and_append_event(TransitionNomiRemoteSessionParams {
            owner_user_id: owner_id.to_owned(),
            agent_session_id: session_id.as_ref().to_owned(),
            expected_state: current.state.clone(),
            next_state: next_state.to_owned(),
            event_type: event_type.clone(),
            payload_json,
        })
        .await;
    match result {
        Ok(result) => Ok(result.session),
        Err(error) => {
            // Another finalizer may have won the same transition while this
            // request was waiting on SQLite. A matching Session state alone
            // is not proof that this exact operation committed: an older
            // crash or a different operation could have produced the same
            // state. Confirm the operation-keyed event and payload first.
            if matches!(&error, nomifun_db::DbError::Conflict(_)) {
                let existing = repository
                    .get_session(owner_id, session_id.as_ref())
                    .await
                    .map_err(|lookup| remote_db_error(lookup, "state transition replay lookup"))?;
                let committed_event = repository
                    .find_event_by_operation_key(
                        owner_id,
                        session_id.as_ref(),
                        &event_type,
                        &operation_digest,
                    )
                    .await
                    .map_err(|lookup| {
                        remote_db_error(lookup, "state transition event replay lookup")
                    })?;
                if let (Some(existing), Some(event)) = (existing, committed_event) {
                    let event_payload: Value =
                        serde_json::from_str(&event.payload_json).map_err(|parse_error| {
                            NomiCoreApiError::new(
                                StatusCode::CONFLICT,
                                "NOMI_CORE_REMOTE_EVENT_INVALID",
                                format!(
                                    "persisted Remote transition payload is invalid: {parse_error}"
                                ),
                            )
                        })?;
                    let event_payload =
                        nomifun_agent_contracts::canonical_json_bytes(&event_payload).map_err(
                            |canonical_error| {
                                NomiCoreApiError::new(
                                    StatusCode::CONFLICT,
                                    "NOMI_CORE_REMOTE_EVENT_INVALID",
                                    format!(
                                        "persisted Remote transition payload cannot be \
                                         canonicalized: {canonical_error}"
                                    ),
                                )
                            },
                        )?;
                    if event_payload != canonical_payload {
                        return Err(NomiCoreApiError::with_details(
                            StatusCode::CONFLICT,
                            "REMOTE_IDEMPOTENCY_CONFLICT",
                            "the Remote transition key was reused with a different payload",
                            json!({
                                "outcome": "rejected",
                                "recovery": "reuse the original transition request or choose a new key",
                            }),
                        ));
                    }
                    if existing.state == next_state {
                        return Ok(existing);
                    }
                }
            }
            Err(remote_db_error(error, "state transition"))
        }
    }
}

async fn reconcile_existing_remote_open(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    row: NomiRemoteSessionRow,
) -> Result<NomiRemoteSessionRow, NomiCoreApiError> {
    if row.state != "opening" {
        return Ok(row);
    }
    let session_id = parse_agent_session_id(&row.agent_session_id)?;
    let open_key = row.open_idempotency_key.clone();
    let events = state
        .remote_repository
        .read_events(owner.as_ref(), session_id.as_ref(), 0, 1000)
        .await
        .map_err(|error| remote_db_error(error, "open reconciliation"))?
        .events;
    let initial_key = format!("remote-initial:{}", row.open_idempotency_key);
    let initial_digest = remote_operation_key_digest(&initial_key)?;
    let initial_events = events.iter().filter(|event| {
        event
            .payload_json
            .parse::<Value>()
            .ok()
            .and_then(|payload| {
                payload
                    .get("operation_key_digest")
                    .and_then(Value::as_str)
                    .map(|digest| digest == initial_digest)
            })
            .unwrap_or(false)
    });
    let initial_events = initial_events.collect::<Vec<_>>();

    if row.initial_input_digest.is_none() && initial_events.is_empty() {
        return transition_remote_state(
            state,
            owner,
            &session_id,
            row,
            "ready",
            &format!("open-ready:{open_key}"),
            json!({ "outcome": "ready", "recovered": true }),
        )
        .await;
    }

    let Some(initial_event) = initial_events.first().copied() else {
        // The original input is intentionally not replayed after a crash: the
        // durable Session may have accepted it, and a blind resend would
        // duplicate an external effect. Mark the open outcome recoverably
        // failed and require an explicit Remote turn.
        return transition_remote_state(
            state,
            owner,
            &session_id,
            row,
            "failed",
            &format!("open-failed:{open_key}"),
            json!({
                "code": "REMOTE_OPEN_FAILED",
                "recoverable": true,
                "reason": "the initial turn outcome was not durably observable after restart",
                "outcome": "unknown",
            }),
        )
        .await;
    };
    // `turn/accepted` is not a successful open.  It only proves that the
    // initial request crossed the durable receiver boundary; the Nomi runtime
    // may still be executing it.  Re-arm the bounded observer and leave the
    // Remote projection in `opening` until a terminal turn fact is visible.
    if initial_event.event_type == "turn/accepted" {
        let terminal = initial_events
            .iter()
            .find(|event| {
                matches!(
                    event.event_type.as_str(),
                    "turn/completed" | "turn/failed" | "turn/unknown"
                )
            })
            .copied();
        if let Some(terminal) = terminal {
            if terminal.event_type == "turn/completed" {
                return transition_remote_state(
                    state,
                    owner,
                    &session_id,
                    row,
                    "ready",
                    &format!("open-ready:{open_key}"),
                    json!({ "outcome": "ready", "recovered": true }),
                )
                .await;
            }
            return transition_remote_state(
                state,
                owner,
                &session_id,
                row,
                "failed",
                &format!("open-failed:{open_key}"),
                json!({
                    "code": "REMOTE_OPEN_FAILED",
                    "recoverable": true,
                    "reason": "the initial Remote turn did not complete successfully",
                }),
            )
            .await;
        }
        schedule_remote_turn_finalizer(
            state,
            owner.as_ref(),
            &session_id,
            &initial_key,
        )?;
        return Ok(row);
    }

    let initial_type = initial_event.event_type.as_str();
    if initial_type == "turn/completed" {
        return transition_remote_state(
            state,
            owner,
            &session_id,
            row,
            "ready",
            &format!("open-ready:{open_key}"),
            json!({ "outcome": "ready", "recovered": true }),
        )
        .await;
    }
    if matches!(initial_type, "turn/failed" | "turn/unknown") {
        return transition_remote_state(
            state,
            owner,
            &session_id,
            row,
            "failed",
            &format!("open-failed:{open_key}"),
            json!({
                "code": "REMOTE_OPEN_FAILED",
                "recoverable": true,
                "reason": "the initial turn did not complete successfully",
            }),
        )
        .await;
    }

    // Any other event carrying the initial operation identity is not a
    // terminal success proof. Keep the Session opening rather than promoting
    // it from an unrecognised metadata event.
    schedule_remote_turn_finalizer(
        state,
        owner.as_ref(),
        &session_id,
        &initial_key,
    )?;
    Ok(row)
}

async fn record_remote_turn_delivery(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
    operation_key: &str,
    delivery: &IdempotentMessageDelivery,
) -> Result<(bool, bool), NomiCoreApiError> {
    // A previously persisted `turn/unknown` or `turn/failed` is absorbing.
    // Never let a late receipt lookup rewrite that durable uncertainty into a
    // successful completion.
    if let Some(existing) = remote_terminal_event_type(
        &state.remote_repository,
        owner.as_ref(),
        session_id.as_ref(),
        operation_key,
    )
    .await?
    {
        return Ok((true, existing == "turn/completed"));
    }
    let terminal = delivery.completed;
    let (event_type, outcome, succeeded) =
        remote_delivery_terminal_event_type(delivery.completed, delivery.result_ok);
    append_remote_event_once(
        &state.remote_repository,
        owner.as_ref(),
        session_id.as_ref(),
        event_type,
        Some(operation_key),
        json!({
            "message_id": delivery.message_id,
            "outcome": outcome,
            "result_ok": delivery.result_ok,
            "result_error_code": delivery.result_error_code,
        }),
    )
    .await?;

    if !terminal {
        schedule_remote_turn_finalizer(
            state,
            owner.as_ref(),
            session_id,
            operation_key,
        )?;
    }
    Ok((terminal, succeeded))
}

/// Execute and durably settle the optional initial Remote turn. The complete
/// workflow lives inside the coordinator task so an HTTP waiter timeout cannot
/// strand a successful delivery without its Remote event/state projection.
async fn execute_initial_remote_turn(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
    current: NomiRemoteSessionRow,
    open_key: &str,
    input: SendMessageRequest,
) -> Result<NomiRemoteSessionRow, NomiCoreApiError> {
    let initial_key = format!("remote-initial:{open_key}");
    let delivery = state
        .session_owner
        .send_session_message_idempotent(
            owner.as_ref(),
            session_id.as_ref(),
            &initial_key,
            input,
        )
        .await;

    let delivery = match delivery {
        Ok(delivery) => delivery,
        Err(send_error) => {
            // A send error does not prove that the durable receiver boundary
            // was never crossed. Re-read the exact receipt before deciding
            // whether to fail the Remote open; never resend the initial input.
            match state
                .session_owner
                .session_turn_delivery_state(
                    owner.as_ref(),
                    session_id.as_ref(),
                    &initial_key,
                )
                .await
            {
                Ok(PublicTurnDeliveryState::Completed(delivery)) => delivery,
                Ok(PublicTurnDeliveryState::Accepted { message_id }) => {
                    record_remote_turn_delivery(
                        state,
                        owner,
                        session_id,
                        &initial_key,
                        &IdempotentMessageDelivery {
                            message_id,
                            replayed: true,
                            completed: false,
                            result_ok: None,
                            result_text: None,
                            result_error: None,
                            result_error_code: None,
                            result_error_retryable: None,
                        },
                    )
                    .await?;
                    return Ok(current);
                }
                Ok(PublicTurnDeliveryState::Missing) => {
                    return transition_remote_state(
                        state,
                        owner,
                        session_id,
                        current,
                        "failed",
                        &format!("open-failed:{open_key}"),
                        json!({
                            "code": "REMOTE_OPEN_FAILED",
                            "recoverable": true,
                            "reason": "initial turn was not durably admitted",
                            "send_error": send_error.error_code(),
                        }),
                    )
                    .await;
                }
                Err(observation_error) => {
                    return Err(NomiCoreApiError::with_details(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "NOMI_CORE_REMOTE_INITIAL_OUTCOME_UNKNOWN",
                        "the initial Remote turn outcome could not be read; the Session remains opening",
                        json!({
                            "agent_session_id": session_id,
                            "outcome": "unknown",
                            "recovery": "retry the same open key and inspect the existing Session",
                            "cause_code": observation_error.error_code(),
                        }),
                    ));
                }
            }
        }
    };

    let (terminal, succeeded) =
        record_remote_turn_delivery(state, owner, session_id, &initial_key, &delivery).await?;
    if !terminal {
        return Ok(current);
    }
    transition_remote_state(
        state,
        owner,
        session_id,
        current,
        if succeeded { "ready" } else { "failed" },
        &if succeeded {
            format!("open-ready:{open_key}")
        } else {
            format!("open-failed:{open_key}")
        },
        if succeeded {
            json!({ "outcome": "ready" })
        } else {
            json!({
                "code": "REMOTE_OPEN_FAILED",
                "recoverable": true,
                "reason": "the initial Remote turn failed",
            })
        },
    )
    .await
}

fn schedule_remote_turn_finalizer(
    state: &NomiCoreAgentApiState,
    owner_id: &str,
    session_id: &AgentSessionId,
    operation_key: &str,
) -> Result<(), NomiCoreApiError> {
    let operation_digest = remote_operation_key_digest(operation_key)?;
    let task_key = format!(
        "nomi-core-remote-turn-finalizer:{}:{}",
        session_id.as_ref(),
        operation_digest
    );
    let coordinator = state.remote_runtime.clone();
    let repository = state.remote_repository.clone();
    let session_owner = state.session_owner.clone();
    let owner_id = owner_id.to_owned();
    let session_id = session_id.clone();
    let operation_key = operation_key.to_owned();
    match coordinator.start_once(task_key, move || async move {
            let deadline =
                tokio::time::Instant::now() + NOMI_CORE_REMOTE_TURN_FINALIZER_TIMEOUT;
            loop {
                match session_owner
                    .session_turn_delivery_state(
                        &owner_id,
                        session_id.as_ref(),
                        &operation_key,
                    )
                    .await
                {
                    Ok(PublicTurnDeliveryState::Completed(delivery)) => {
                        if remote_terminal_event_type(
                            &repository,
                            &owner_id,
                            session_id.as_ref(),
                            &operation_key,
                        )
                        .await
                        .ok()
                        .flatten()
                        .is_some()
                        {
                            return;
                        }
                        let (event_type, outcome, succeeded) =
                            remote_delivery_terminal_event_type(true, delivery.result_ok);
                        let failed = !succeeded;
                        if let Err(error) = append_remote_event_once(
                            &repository,
                            &owner_id,
                            session_id.as_ref(),
                            event_type,
                            Some(&operation_key),
                            json!({
                                "message_id": delivery.message_id,
                                "outcome": outcome,
                                "result_ok": delivery.result_ok,
                                "result_error_code": delivery.result_error_code,
                            }),
                        )
                        .await
                        {
                            tracing::error!(
                                session_id = %session_id.as_ref(),
                                error = ?error,
                                "Nomi-core Remote turn terminal event could not be persisted"
                            );
                            return;
                        }
                        if operation_key.starts_with("remote-initial:") {
                            let open_key = operation_key
                                .strip_prefix("remote-initial:")
                                .unwrap_or_default();
                            let event_key = if failed {
                                format!("open-failed:{open_key}")
                            } else {
                                format!("open-ready:{open_key}")
                            };
                            let next_state = if failed { "failed" } else { "ready" };
                            let payload = if failed {
                                json!({
                                    "code": "REMOTE_OPEN_FAILED",
                                    "recoverable": true,
                                    "reason": "the initial Remote turn failed",
                                })
                            } else {
                                json!({ "outcome": "ready" })
                            };
                            let current = match repository
                                .get_session(&owner_id, session_id.as_ref())
                                .await
                            {
                                Ok(Some(current)) => current,
                                Ok(None) => return,
                                Err(error) => {
                                    tracing::error!(
                                        session_id = %session_id.as_ref(),
                                        error = %error,
                                        "Nomi-core Remote initial state lookup failed"
                                    );
                                    return;
                                }
                            };
                            if let Err(error) = transition_remote_state_with_repository(
                                &repository,
                                &owner_id,
                                &session_id,
                                current,
                                next_state,
                                &event_key,
                                payload,
                            )
                            .await
                            {
                                tracing::error!(
                                    session_id = %session_id.as_ref(),
                                    error = ?error,
                                    "Nomi-core Remote initial state transition failed"
                                );
                            }
                        }
                        return;
                    }
                    Ok(PublicTurnDeliveryState::Missing) => return,
                    Ok(PublicTurnDeliveryState::Accepted { .. }) => {}
                    Err(error) => {
                        tracing::warn!(
                            session_id = %session_id.as_ref(),
                            error = ?error,
                            "Nomi-core Remote turn receipt observation failed"
                        );
                    }
                }

                if tokio::time::Instant::now() >= deadline {
                    if let Err(error) = append_remote_event_once(
                        &repository,
                        &owner_id,
                        session_id.as_ref(),
                        "turn/unknown",
                        Some(&operation_key),
                        json!({
                            "outcome": "unknown",
                            "recovery": "retry observe with the same Session and operation identity",
                        }),
                    )
                    .await
                    {
                        tracing::error!(
                            session_id = %session_id.as_ref(),
                            error = ?error,
                            "Nomi-core Remote unknown turn outcome could not be persisted"
                        );
                    }
                    if operation_key.starts_with("remote-initial:") {
                        let open_key = operation_key
                            .strip_prefix("remote-initial:")
                            .unwrap_or_default();
                        let current = match repository
                            .get_session(&owner_id, session_id.as_ref())
                            .await
                        {
                            Ok(Some(current)) => current,
                            _ => return,
                        };
                        if let Err(error) = transition_remote_state_with_repository(
                            &repository,
                            &owner_id,
                            &session_id,
                            current,
                            "failed",
                            &format!("open-failed:{open_key}"),
                            json!({
                                "code": "REMOTE_OPEN_FAILED",
                                "recoverable": true,
                                "reason": "the initial Remote turn outcome remained unknown",
                            }),
                        )
                        .await
                        {
                            tracing::error!(
                                session_id = %session_id.as_ref(),
                                error = ?error,
                                "Nomi-core Remote unknown open outcome could not be recorded"
                            );
                        }
                    }
                    return;
                }
                tokio::time::sleep(NOMI_CORE_REMOTE_TURN_FINALIZER_POLL).await;
            }
        }) {
        Ok(()) | Err(super::remote_runtime::RemoteDetachedMutationAdmissionError::AlreadyRunning) => {
            Ok(())
        }
        Err(super::remote_runtime::RemoteDetachedMutationAdmissionError::Closed) => {
            Err(remote_store_unavailable("turn finalization"))
        }
        Err(super::remote_runtime::RemoteDetachedMutationAdmissionError::CapacityExceeded) => {
            Err(NomiCoreApiError::with_details(
                StatusCode::SERVICE_UNAVAILABLE,
                "NOMI_CORE_REMOTE_CAPACITY_EXCEEDED",
                "Nomi-core Remote background capacity is exhausted",
                json!({
                    "outcome": "unknown",
                    "recovery": "retry the same idempotency key after existing operations settle",
                }),
            ))
        }
    }
}

fn schedule_remote_cancel_finalizer(
    state: &NomiCoreAgentApiState,
    owner_id: &str,
    session_id: &AgentSessionId,
    operation_key: &str,
) -> Result<(), NomiCoreApiError> {
    let operation_digest = remote_operation_key_digest(operation_key)?;
    let task_key = format!(
        "nomi-core-remote-cancel-finalizer:{}:{}",
        session_id.as_ref(),
        operation_digest
    );
    let coordinator = state.remote_runtime.clone();
    let repository = state.remote_repository.clone();
    let session_owner = state.session_owner.clone();
    let owner_id = owner_id.to_owned();
    let session_id = session_id.clone();
    let operation_key = operation_key.to_owned();
    let request_digest = remote_cancel_request_digest(&session_id, &operation_key)?;
    match coordinator.start_once(task_key, move || async move {
        let deadline =
            tokio::time::Instant::now() + NOMI_CORE_REMOTE_TURN_FINALIZER_TIMEOUT;
        loop {
            match remote_cancel_event_state(
                &repository,
                &owner_id,
                session_id.as_ref(),
                &operation_key,
                &request_digest,
            )
            .await
            {
                Ok((Some(RemoteCancelEventState::Cancelled | RemoteCancelEventState::Rejected), _)) => {
                    return;
                }
                Ok((Some(RemoteCancelEventState::Requested | RemoteCancelEventState::Unknown), _))
                | Ok((None, _)) => {}
                Err(error) => {
                    tracing::error!(
                        session_id = %session_id.as_ref(),
                        error = ?error,
                        "Nomi-core Remote cancellation fence lookup failed"
                    );
                    return;
                }
            }

            let summary = session_owner
                .service()
                .runtime_summary_for(session_id.as_ref())
                .await;
            if matches!(summary.state, ConversationRuntimeStateKind::Idle)
                && !summary.is_processing
            {
                let current = match repository
                    .get_session(&owner_id, session_id.as_ref())
                    .await
                {
                    Ok(Some(current)) => current,
                    Ok(None) => return,
                    Err(error) => {
                        tracing::error!(
                            session_id = %session_id.as_ref(),
                            error = %error,
                            "Nomi-core Remote cancellation state lookup failed"
                        );
                        return;
                    }
                };
                if let Err(error) = transition_remote_state_with_repository(
                    &repository,
                    &owner_id,
                    &session_id,
                    current,
                    "cancelled",
                    &operation_key,
                    json!({
                        "outcome": "cancelled",
                        "request_digest": request_digest,
                        "recovered": true,
                    }),
                )
                .await
                {
                    tracing::error!(
                        session_id = %session_id.as_ref(),
                        error = ?error,
                        "Nomi-core Remote cancellation state/event could not be persisted"
                    );
                }
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                if let Err(error) = append_remote_event_once(
                    &repository,
                    &owner_id,
                    session_id.as_ref(),
                    "session/cancel-unknown",
                    Some(&operation_key),
                    json!({
                        "outcome": "unknown",
                        "request_digest": request_digest,
                        "recovery": "retry the same cancel key and observe the Session",
                    }),
                )
                .await
                {
                    tracing::error!(
                        session_id = %session_id.as_ref(),
                        error = ?error,
                        "Nomi-core Remote cancellation unknown outcome could not be persisted"
                    );
                }
                return;
            }
            tokio::time::sleep(NOMI_CORE_REMOTE_TURN_FINALIZER_POLL).await;
        }
    }) {
        Ok(()) | Err(
            super::remote_runtime::RemoteDetachedMutationAdmissionError::AlreadyRunning,
        ) => Ok(()),
        Err(super::remote_runtime::RemoteDetachedMutationAdmissionError::Closed) => {
            Err(remote_store_unavailable("cancel finalization"))
        }
        Err(super::remote_runtime::RemoteDetachedMutationAdmissionError::CapacityExceeded) => {
            Err(NomiCoreApiError::with_details(
                StatusCode::SERVICE_UNAVAILABLE,
                "NOMI_CORE_REMOTE_CAPACITY_EXCEEDED",
                "Nomi-core Remote background capacity is exhausted",
                json!({
                    "outcome": "unknown",
                    "recovery": "retry the same cancel key after existing operations settle",
                }),
            ))
        }
    }
}

async fn create_nomi_core_agent_session(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    headers: HeaderMap,
    Json(request): Json<CreateAgentSessionRequestDto>,
) -> Result<Json<ApiResponse<CreateAgentSessionResponseDto>>, NomiCoreApiError> {
    let binding = state
        .control_plane
        .resolve_agent_session_binding_with_model(&owner.0, &request.preset_id, request.model.as_ref())
        .await?;
    let projection =
        resolve_saved_binding_projection(&state, &owner, &binding, request.title.as_deref())
            .await?;
    let mut create_request = projection.projection.request;
    attach_session_metadata(&mut create_request.extra, &projection.binding, None)?;
    let creation_key = request_idempotency_key(
        &headers,
        "nomi-core-agent-session-create",
    )?;
    let created = state
        .session_owner
        .create_session_idempotent(
            owner.as_ref(),
            create_request,
            Some(projection.projection.snapshot),
            &creation_key,
        )
        .await?;
    let session_id = parse_agent_session_id(&created.conversation_id)?;
    let response = state
        .session_owner
        .get_session(owner.as_ref(), session_id.as_ref())
        .await?;
    let cursor = durable_message_cursor(&state.session_owner, &session_id).await?;
    Ok(Json(ApiResponse::ok(CreateAgentSessionResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        agent_binding: binding,
        state: projected_session_status(&response),
        cursor,
    })))
}

async fn get_nomi_core_agent_session(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<SessionObservation>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let response = load_owned_nomi_core_session(&state, &owner, &session_id).await?;
    let metadata = session_metadata(&response, &owner)?;
    let observation = build_session_observation(
        &state.session_owner,
        &owner,
        &response,
        metadata,
        0,
        default_nomi_core_page_limit(),
    )
    .await?;
    Ok(Json(ApiResponse::ok(observation)))
}

async fn get_nomi_core_agent_session_capabilities(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<NomiCoreAgentSessionCapabilityResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let response = load_owned_nomi_core_session(&state, &owner, &session_id).await?;
    let metadata = session_metadata(&response, &owner)?;
    let binding_dto = agent_binding_dto(&metadata.binding)?;
    let projection =
        resolve_saved_binding_projection(&state, &owner, &binding_dto, response.name.as_str().into())
            .await?;
    if projection.snapshot.snapshot_ref != metadata.binding.resolved_snapshot_ref {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_SNAPSHOT_IDENTITY_CONFLICT",
            "the capability projection Snapshot differs from the Session binding",
        ));
    }
    let initial_capabilities = projection
        .revision
        .payload
        .initial_capabilities
        .iter()
        .map(|selection| selection.capability.id.as_ref().to_owned())
        .collect::<Vec<_>>();
    let on_demand_capabilities = projection
        .revision
        .payload
        .on_demand_capabilities
        .iter()
        .map(|selection| selection.capability.id.as_ref().to_owned())
        .collect::<Vec<_>>();
    let compact_on_demand_index = projection.snapshot.content.compact_on_demand_index;
    Ok(Json(ApiResponse::ok(
        NomiCoreAgentSessionCapabilityResponse {
            resolved_snapshot_ref: metadata.binding.resolved_snapshot_ref,
            generation: 0,
            initial_capabilities: initial_capabilities.clone(),
            on_demand_capabilities,
            active_capabilities: initial_capabilities,
            compact_on_demand_index,
            state_source: "nomi_core_saved_binding",
        },
    )))
}

async fn start_nomi_core_agent_session_turn(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(request): Json<CreateAgentSessionTurnRequestDto>,
) -> Result<Json<ApiResponse<CreateAgentSessionTurnResponseDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let response = load_owned_nomi_core_session(&state, &owner, &session_id).await?;
    let _metadata = session_metadata(&response, &owner)?;
    let input = bounded_turn_input(request.input)?;
    let idempotency_key = canonical_nonempty(&request.idempotency_key, "idempotency_key")?;
    let operation_id = OperationId::from(format!(
        "nomi-core-turn:{}:{}",
        session_id.as_ref(),
        idempotency_key
    ));
    let delivery = state
        .session_owner
        .send_session_message_idempotent(
            owner.as_ref(),
            session_id.as_ref(),
            &idempotency_key,
            input,
        )
        .await?;
    let updated = state
        .session_owner
        .get_session(owner.as_ref(), session_id.as_ref())
        .await?;
    let cursor = durable_message_cursor(&state.session_owner, &session_id).await?;
    let status = if delivery.completed {
        projected_session_status(&updated)
    } else {
        "running".to_owned()
    };
    Ok(Json(ApiResponse::ok(CreateAgentSessionTurnResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        operation_id: operation_id.as_ref().to_owned(),
        cursor,
        status,
    })))
}

async fn get_nomi_core_agent_session_messages(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Query(query): Query<NomiCoreSessionPageQuery>,
) -> Result<Json<ApiResponse<NomiCoreAgentSessionMessagePageResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let response = load_owned_nomi_core_session(&state, &owner, &session_id).await?;
    let _metadata = session_metadata(&response, &owner)?;
    validate_page_limit(query.limit)?;
    let (messages, next_seq) = read_message_projection_page(
        &state.session_owner,
        &session_id,
        query.after_seq,
        query.limit,
    )
    .await?;
    Ok(Json(ApiResponse::ok(
        NomiCoreAgentSessionMessagePageResponse {
            agent_session_id: session_id.as_ref().to_owned(),
            messages,
            next_cursor: session_cursor(&session_id, next_seq),
        },
    )))
}

async fn get_nomi_core_agent_session_events(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Query(query): Query<NomiCoreSessionPageQuery>,
) -> Result<Json<ApiResponse<Value>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let response = load_owned_nomi_core_session(&state, &owner, &session_id).await?;
    let _metadata = session_metadata(&response, &owner)?;
    validate_page_limit(query.limit)?;
    Err(NomiCoreApiError::unsupported_session_events(&session_id))
}

async fn fork_nomi_core_agent_session(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(_request): Json<ForkAgentSessionRequestDto>,
) -> Result<Json<ApiResponse<ForkAgentSessionResponseDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let response = load_owned_nomi_core_session(&state, &owner, &session_id).await?;
    let _metadata = session_metadata(&response, &owner)?;
    Err(NomiCoreApiError::with_details(
        StatusCode::NOT_IMPLEMENTED,
        NOMI_CORE_FORK_UNAVAILABLE_CODE,
        "Nomi-core has no transcript-preserving Conversation fork primitive",
        json!({
            "agent_session_id": session_id,
            "outcome": "not_available",
            "recovery": "create a new Session explicitly or integrate a canonical fork owner",
        }),
    ))
}

async fn delete_nomi_core_agent_session(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<NomiCoreAgentSessionDeleteResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let response = load_owned_nomi_core_session(&state, &owner, &session_id).await?;
    let _metadata = session_metadata(&response, &owner)?;
    let deleted_at = now_ms();
    state
        .session_owner
        .delete_session(owner.as_ref(), session_id.as_ref())
        .await?;
    Ok(Json(ApiResponse::ok(NomiCoreAgentSessionDeleteResponse {
        agent_session_id: session_id.as_ref().to_owned(),
        state: "deleted",
        deleted_at,
    })))
}

async fn open_nomi_core_remote(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Json(request): Json<RemoteOpenRequestDto>,
) -> Result<Json<RemoteOpenResponseDto>, NomiCoreApiError> {
    let binding_id = canonical_nonempty(&request.binding_id, "binding_id")?;
    let idempotency_key = canonical_nonempty(&request.idempotency_key, "idempotency_key")?;
    // Validate and normalize the optional initial turn before creating either
    // a Conversation or a Remote projection.  A malformed initial payload
    // must not leave an apparently usable orphan Session behind.
    let (initial_input, requested_input_digest) = match request.initial_input {
        Some(value) => {
            let digest = remote_input_digest(&value)?;
            let input = bounded_turn_input(value)?;
            (Some(input), Some(digest))
        }
        None => (None, None),
    };

    // Replay the durable Remote projection before reading the mutable
    // RemoteBinding catalog.  Binding updates/deletes intentionally do not
    // rewrite or invalidate an already-open Session; the frozen binding in
    // `nomi_remote_sessions` is therefore the authority for an idempotent
    // replay.
    if let Some(existing) = state
        .remote_repository
        .get_session_by_open_key(owner.as_ref(), &idempotency_key)
        .await
        .map_err(|error| remote_db_error(error, "open lookup"))?
    {
        if existing.remote_binding_id != binding_id {
            return Err(NomiCoreApiError::with_details(
                StatusCode::CONFLICT,
                "REMOTE_IDEMPOTENCY_CONFLICT",
                "the Remote open key was reused for a different binding",
                json!({
                    "outcome": "rejected",
                    "recovery": "reuse the original binding or choose a new idempotency key",
                }),
            ));
        }
        let session_id = parse_agent_session_id(&existing.agent_session_id)?;
        let frozen_binding: AgentBindingValue =
            serde_json::from_str(&existing.agent_binding_json).map_err(|error| {
                NomiCoreApiError::new(
                    StatusCode::CONFLICT,
                    "NOMI_CORE_REMOTE_PROVENANCE_INVALID",
                    format!("persisted Remote binding is invalid: {error}"),
                )
            })?;
        let stored_input_digest = existing.initial_input_digest.clone().or(
            remote_open_input_digest(
                &state.remote_repository,
                owner.as_ref(),
                session_id.as_ref(),
            )
            .await?,
        );
        if stored_input_digest != requested_input_digest {
            return Err(NomiCoreApiError::with_details(
                StatusCode::CONFLICT,
                "REMOTE_IDEMPOTENCY_CONFLICT",
                "the Remote open key was reused with a different initial input",
                json!({
                    "outcome": "rejected",
                    "recovery": "reuse the original open request or choose a new idempotency key",
                }),
            ));
        }
        validate_remote_session_binding(
            &existing,
            owner.as_ref(),
            &frozen_binding,
            &existing.remote_binding_id,
        )?;
        let existing = reconcile_existing_remote_open(&state, &owner, existing).await?;
        return remote_open_response(
            &state,
            &owner,
            &existing,
            agent_binding_dto(&frozen_binding)?,
        )
        .await;
    }

    let remote_binding = state
        .control_plane
        .get_remote_binding(&owner.0, &binding_id)
        .await?
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::NOT_FOUND,
                "REMOTE_BINDING_NOT_FOUND",
                "RemoteBinding does not exist for the authenticated owner",
            )
        })?;
    let projection = resolve_saved_binding_projection(
        &state,
        &owner,
        &remote_binding.agent_binding,
        Some(remote_binding.name.as_str()),
    )
    .await?;
    let binding_digest = remote_binding_digest(&projection.binding)?;

    let remote_id = nomifun_agent_contracts::RemoteBindingId::from(
        remote_binding.remote_binding_id.clone(),
    );
    let mut create_request = projection.projection.request;
    attach_session_metadata(
        &mut create_request.extra,
        &projection.binding,
        Some(RemoteBindingProvenance {
            remote_binding_id: remote_id,
            binding_version: projection.binding.binding_version,
        }),
    )?;
    let created = state
        .session_owner
        .create_session_idempotent(
            owner.as_ref(),
            create_request,
            Some(projection.projection.snapshot),
            &format!("nomi-core-remote-open:{idempotency_key}"),
        )
        .await?;
    let session_id = parse_agent_session_id(&created.conversation_id)?;
    let open_projection = state
        .remote_repository
        .get_or_create_session(GetOrCreateRemoteSessionParams {
            owner_user_id: owner.as_ref().to_owned(),
            remote_binding_id: remote_binding.remote_binding_id.clone(),
            expected_binding_version: projection.binding.binding_version as i64,
            expected_agent_binding_digest: binding_digest.clone(),
            open_idempotency_key: idempotency_key.clone(),
            agent_session_id: session_id.as_ref().to_owned(),
            initial_input_digest: requested_input_digest.clone(),
        })
        .await
        .map_err(|error| remote_db_error(error, "open admission"))?;

    let (remote_session, created_projection) = match open_projection {
        RemoteOpenResult::Created(row) => (row, true),
        RemoteOpenResult::Existing(row) => {
            if row.agent_session_id != session_id.as_ref() {
                // The Conversation creation key and Remote projection key
                // must identify one logical Session.  Never leave a second
                // Conversation behind when this invariant is violated.
                let _ = state
                    .session_owner
                    .service()
                    .discard_unlinked_creation(
                        owner.as_ref(),
                        &format!("nomi-core-remote-open:{idempotency_key}"),
                    )
                    .await;
                return Err(NomiCoreApiError::new(
                    StatusCode::CONFLICT,
                    "NOMI_CORE_REMOTE_PROVENANCE_CONFLICT",
                    "Remote open resolved two different Session identities",
                ));
            }
            (row, false)
        }
    };

    if created_projection {
        let mut opening_payload = json!({
            "remote_binding_id": remote_binding.remote_binding_id,
            "binding_version": projection.binding.binding_version,
        });
        if let Some(input_digest) = requested_input_digest.as_deref() {
            opening_payload["initial_input_digest"] = Value::String(input_digest.to_owned());
        }
        append_remote_event_once(
            &state.remote_repository,
            owner.as_ref(),
            session_id.as_ref(),
            "session/opening",
            Some(&idempotency_key),
            opening_payload,
        )
        .await?;
    }

    let mut current = remote_session;
    let has_initial_input = initial_input.is_some();
    if created_projection {
        if let Some(input) = initial_input {
            let task_state = state.clone();
            let task_owner = owner.clone();
            let task_session_id = session_id.clone();
            let task_open_key = idempotency_key.clone();
            let task_current = current.clone();
            let result = run_nomi_core_remote_detached(
                &state,
                nomi_core_remote_mutation_key(
                    "initial",
                    &owner,
                    &session_id,
                    &idempotency_key,
                ),
                NOMI_CORE_REMOTE_INITIAL_COMMAND_TIMEOUT,
                async move {
                    execute_initial_remote_turn(
                        &task_state,
                        &task_owner,
                        &task_session_id,
                        task_current,
                        &task_open_key,
                        input,
                    )
                    .await
                },
            )
            .await;
            match result {
                Ok(updated) => {
                    current = updated;
                }
                Err(NomiCoreRemoteDetachedFailure::Failed(error)) => return Err(error),
                Err(NomiCoreRemoteDetachedFailure::TimedOut) => {
                    return Err(NomiCoreApiError::with_details(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "NOMI_CORE_REMOTE_INITIAL_OUTCOME_UNKNOWN",
                        "the initial Remote turn exceeded its bounded wait; the Session remains opening",
                        json!({
                            "agent_session_id": session_id,
                            "outcome": "unknown",
                            "recovery": "retry the same open key and inspect the existing Session",
                        }),
                    ));
                }
                Err(NomiCoreRemoteDetachedFailure::Panicked) => {
                    return Err(NomiCoreApiError::with_details(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "NOMI_CORE_REMOTE_INITIAL_OUTCOME_UNKNOWN",
                        "the initial Remote turn panicked before its durable outcome was known",
                        json!({
                            "agent_session_id": session_id,
                            "outcome": "unknown",
                            "recovery": "retry the same open key and inspect the existing Session",
                        }),
                    ));
                }
                Err(NomiCoreRemoteDetachedFailure::Admission(error)) => {
                    return Err(nomi_core_remote_admission_error(
                        "remote.initial",
                        &session_id,
                        error,
                    ));
                }
            }
        }
    }

    // A Remote Session is ready to accept subsequent turns once its
    // Conversation aggregate and immutable binding projection are committed.
    // The actual Nomi runtime remains lazy and is owned by ConversationService.
    if created_projection && !has_initial_input && current.state == "opening" {
        current = transition_remote_state(
            &state,
            &owner,
            &session_id,
            current,
            "ready",
            &format!("open-ready:{idempotency_key}"),
            json!({ "outcome": "ready" }),
        )
        .await?;
    }

    remote_open_response(
        &state,
        &owner,
        &current,
        remote_binding.agent_binding,
    )
    .await
}

async fn turn_nomi_core_remote(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Json(request): Json<RemoteTurnRequestDto>,
) -> Result<Json<RemoteMutationResponseDto>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&request.agent_session_id)?;
    let key = canonical_nonempty(&request.idempotency_key, "idempotency_key")?;
    let remote_session = remote_session_projection(&state, &owner, &session_id).await?;
    let response = load_owned_nomi_core_session(&state, &owner, &session_id).await?;
    let metadata = session_metadata(&response, &owner)?;
    if metadata.remote.is_none() {
        return Err(remote_session_not_found());
    }
    if remote_session.state == "opening" {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "REMOTE_SESSION_OPENING",
            "the Remote Session is still opening",
        ));
    }
    if remote_session.state == "failed" {
        return Err(NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "REMOTE_OPEN_FAILED",
            "the Remote Session failed to open",
        ));
    }
    if remote_session.state == "cancelled" {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "REMOTE_SESSION_NOT_FOUND",
            "the Remote Session has been cancelled",
        ));
    }
    if remote_has_active_cancel_fence(
        &state.remote_repository,
        owner.as_ref(),
        session_id.as_ref(),
    )
    .await?
    {
        return Err(NomiCoreApiError::with_details(
            StatusCode::CONFLICT,
            "REMOTE_SESSION_CANCEL_PENDING",
            "the Remote Session is fenced while a cancellation outcome is unresolved",
            json!({
                "agent_session_id": session_id,
                "recovery": "retry the same cancel key and observe the Session",
            }),
        ));
    }
    let input = bounded_turn_input(request.input)?;
    let task_state = state.clone();
    let task_owner = owner.clone();
    let task_session_id = session_id.clone();
    let task_key = key.clone();
    let result = run_nomi_core_remote_detached(
        &state,
        nomi_core_remote_mutation_key("turn", &owner, &session_id, &key),
        NOMI_CORE_REMOTE_TURN_COMMAND_TIMEOUT,
        async move {
            let delivery = task_state
                .session_owner
                .send_session_message_idempotent(
                    task_owner.as_ref(),
                    task_session_id.as_ref(),
                    &task_key,
                    input,
                )
                .await?;
            record_remote_turn_delivery(
                &task_state,
                &task_owner,
                &task_session_id,
                &task_key,
                &delivery,
            )
            .await?;
            let current = task_state
                .remote_repository
                .get_session(task_owner.as_ref(), task_session_id.as_ref())
                .await
                .map_err(|error| remote_db_error(error, "turn state lookup"))?
                .ok_or_else(remote_session_not_found)?;
            Ok::<_, NomiCoreApiError>(RemoteMutationResponseDto {
                agent_session_id: task_session_id.as_ref().to_owned(),
                cursor: remote_event_cursor(&task_state, &task_owner, &task_session_id).await?,
                session_status: remote_status_label(&current.state).to_owned(),
            })
        },
    )
    .await;
    match result {
        Ok(response) => Ok(Json(response)),
        Err(NomiCoreRemoteDetachedFailure::Failed(error)) => Err(error),
        Err(NomiCoreRemoteDetachedFailure::TimedOut) => Err(
            nomi_core_remote_timeout(
                "remote.turn",
                NOMI_CORE_REMOTE_TURN_COMMAND_TIMEOUT,
                Some(&session_id),
            ),
        ),
        Err(NomiCoreRemoteDetachedFailure::Panicked) => Err(NomiCoreApiError::with_details(
            StatusCode::SERVICE_UNAVAILABLE,
            "NOMI_CORE_REMOTE_OPERATION_UNKNOWN",
            "the Remote turn panicked before its durable outcome was known",
            json!({
                "agent_session_id": session_id,
                "outcome": "unknown",
                "recovery": "retry the same turn key and inspect the Session",
            }),
        )),
        Err(NomiCoreRemoteDetachedFailure::Admission(error)) => {
            Err(nomi_core_remote_admission_error("remote.turn", &session_id, error))
        }
    }
}

async fn observe_nomi_core_remote(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Query(query): Query<NomiCoreRemoteObserveQuery>,
) -> Result<Json<RemoteObserveResponseDto>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&query.agent_session_id)?;
    let remote_session = remote_session_projection(&state, &owner, &session_id).await?;
    let response = load_owned_nomi_core_session(&state, &owner, &session_id).await?;
    let metadata = session_metadata(&response, &owner)?;
    if metadata.remote.is_none() {
        return Err(remote_session_not_found());
    }
    validate_page_limit(query.limit)?;
    let after_seq = i64::try_from(query.after_seq).map_err(|_| {
        NomiCoreApiError::new(
            StatusCode::BAD_REQUEST,
            "NOMI_CORE_REMOTE_CURSOR_INVALID",
            "Remote cursor exceeds the supported range",
        )
    })?;
    let limit = i64::from(query.limit.min(NOMI_CORE_MESSAGE_PAGE_SIZE).max(1));
    let page = state
        .remote_repository
        .read_events(owner.as_ref(), session_id.as_ref(), after_seq, limit)
        .await
        .map_err(|error| remote_db_error(error, "event page"))?;
    let mut event_values = Vec::with_capacity(page.events.len());
    let mut message_ids = Vec::new();
    for event in &page.events {
        let value = remote_event_value(event)?;
        if let Some(message_id) = value
            .get("payload")
            .and_then(Value::as_object)
            .and_then(|payload| payload.get("message_id"))
            .and_then(Value::as_str)
        {
            message_ids.push(message_id.to_owned());
        }
        event_values.push(value);
    }
    let messages = read_message_projections_by_ids(
        &state.session_owner,
        &session_id,
        &message_ids,
    )
    .await?;
    let next_seq = u64::try_from(page.next_cursor).map_err(|_| {
        NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_REMOTE_CURSOR_INVALID",
            "persisted Remote cursor is invalid",
        )
    })?;
    let _ = remote_session;
    Ok(Json(RemoteObserveResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        events: event_values,
        messages,
        next_cursor: session_cursor(&session_id, next_seq),
    }))
}

async fn cancel_nomi_core_remote(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Json(request): Json<RemoteCancelRequestDto>,
) -> Result<Json<RemoteMutationResponseDto>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&request.agent_session_id)?;
    let key = canonical_nonempty(&request.idempotency_key, "idempotency_key")?;
    let request_digest = remote_cancel_request_digest(&session_id, &key)?;
    let remote_session = remote_session_projection(&state, &owner, &session_id).await?;
    let response = load_owned_nomi_core_session(&state, &owner, &session_id).await?;
    let metadata = session_metadata(&response, &owner)?;
    if metadata.remote.is_none() {
        return Err(remote_session_not_found());
    }

    let (existing_cancel, another_cancel_pending) = remote_cancel_event_state(
        &state.remote_repository,
        owner.as_ref(),
        session_id.as_ref(),
        &key,
        &request_digest,
    )
    .await?;
    match existing_cancel {
        Some(RemoteCancelEventState::Cancelled) => {
            return Ok(Json(RemoteMutationResponseDto {
                agent_session_id: session_id.as_ref().to_owned(),
                cursor: remote_event_cursor(&state, &owner, &session_id).await?,
                session_status: "cancelled".to_owned(),
            }));
        }
        Some(RemoteCancelEventState::Rejected) => {
            return Err(NomiCoreApiError::with_details(
                StatusCode::CONFLICT,
                "REMOTE_CANCEL_REJECTED",
                "the original Remote cancellation request was rejected and will not be replayed",
                json!({
                    "agent_session_id": session_id,
                    "idempotency_key": key,
                    "outcome": "rejected",
                    "recovery": "use a new cancel key only after inspecting the Session state",
                }),
            ));
        }
        Some(RemoteCancelEventState::Requested | RemoteCancelEventState::Unknown) => {
            schedule_remote_cancel_finalizer(&state, owner.as_ref(), &session_id, &key)?;
            return Err(NomiCoreApiError::with_details(
                StatusCode::GATEWAY_TIMEOUT,
                "NOMI_CORE_REMOTE_CANCEL_UNKNOWN",
                "Remote cancellation remains fenced while Nomi-core cleanup continues",
                json!({
                    "agent_session_id": session_id,
                    "idempotency_key": key,
                    "outcome": "unknown",
                    "recovery": "retry the same cancel key and observe the same AgentSession",
                }),
            ));
        }
        None if another_cancel_pending => {
            return Err(NomiCoreApiError::with_details(
                StatusCode::CONFLICT,
                "REMOTE_SESSION_CANCEL_PENDING",
                "another Remote cancellation is already unresolved for this Session",
                json!({
                    "agent_session_id": session_id,
                    "recovery": "observe the Session and retry after the existing cancellation settles",
                }),
            ));
        }
        None => {}
    }

    if remote_session.state == "cancelled" {
        return Ok(Json(RemoteMutationResponseDto {
            agent_session_id: session_id.as_ref().to_owned(),
            cursor: remote_event_cursor(&state, &owner, &session_id).await?,
            session_status: "cancelled".to_owned(),
        }));
    }
    if remote_session.state == "failed" {
        return Err(NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "REMOTE_OPEN_FAILED",
            "the Remote Session failed to open",
        ));
    }

    // Persist the cancellation request before crossing the runtime boundary.
    // A retry that arrives after an HTTP timeout can then observe the same
    // request identity and must not issue a second cancel command.
    let requested = append_remote_event_with_inserted(
        &state.remote_repository,
        owner.as_ref(),
        session_id.as_ref(),
        "session/cancel-requested",
        Some(&key),
        json!({
            "outcome": "requested",
            "request_digest": request_digest,
        }),
    )
    .await?;
    if !requested.as_ref().is_some_and(|(_, inserted)| *inserted) {
        schedule_remote_cancel_finalizer(&state, owner.as_ref(), &session_id, &key)?;
        return Err(NomiCoreApiError::with_details(
            StatusCode::GATEWAY_TIMEOUT,
            "NOMI_CORE_REMOTE_CANCEL_UNKNOWN",
            "the Remote cancellation request is already in flight",
            json!({
                "agent_session_id": session_id,
                "idempotency_key": key,
                "outcome": "unknown",
                "recovery": "retry the same cancel key and observe the same AgentSession",
            }),
        ));
    }

    let task_state = state.clone();
    let task_owner = owner.clone();
    let task_session_id = session_id.clone();
    let task_key = key.clone();
    let task_result = run_nomi_core_remote_detached(
        &state,
        nomi_core_remote_mutation_key("cancel", &owner, &session_id, &key),
        NOMI_CORE_REMOTE_CANCEL_COMMAND_TIMEOUT,
        async move {
            task_state
                .session_owner
                .cancel_session(task_owner.as_ref(), task_session_id.as_ref())
                .await
        },
    )
    .await;
    match task_result {
        Ok(()) => {}
        Err(NomiCoreRemoteDetachedFailure::Failed(error)) => {
            // Explicit client/ownership/precondition failures prove that the
            // command did not cross the runtime boundary. Transient/internal
            // failures do not prove that, so they remain an unknown fenced
            // outcome and are handled by the bounded finalizer.
            if cancel_error_is_known_rejection(&error) {
                append_remote_event_once(
                    &state.remote_repository,
                    owner.as_ref(),
                    session_id.as_ref(),
                    "session/cancel-rejected",
                    Some(&task_key),
                    json!({
                        "outcome": "rejected",
                        "request_digest": request_digest,
                        "error_code": error.error_code(),
                    }),
                )
                .await?;
                return Err(NomiCoreApiError::from(error));
            }

            append_remote_event_once(
                &state.remote_repository,
                owner.as_ref(),
                session_id.as_ref(),
                "session/cancel-unknown",
                Some(&task_key),
                json!({
                    "outcome": "unknown",
                    "request_digest": request_digest,
                    "error_code": error.error_code(),
                    "recovery": "retry the same cancel key and observe the Session",
                }),
            )
            .await?;
            schedule_remote_cancel_finalizer(&state, owner.as_ref(), &session_id, &task_key)?;
            return Err(nomi_core_remote_unknown_error(
                "Remote cancellation returned an unresolved runtime error and remains fenced",
                &session_id,
                &task_key,
            ));
        }
        Err(NomiCoreRemoteDetachedFailure::TimedOut) => {
            schedule_remote_cancel_finalizer(&state, owner.as_ref(), &session_id, &task_key)?;
            return Err(nomi_core_remote_unknown_error(
                "Remote cancellation remains fenced while Nomi-core cleanup continues",
                &session_id,
                &task_key,
            ));
        }
        Err(NomiCoreRemoteDetachedFailure::Panicked) => {
            append_remote_event_once(
                &state.remote_repository,
                owner.as_ref(),
                session_id.as_ref(),
                "session/cancel-unknown",
                Some(&task_key),
                json!({
                    "outcome": "unknown",
                    "request_digest": request_digest,
                    "reason": "cancel command panicked before its durable outcome was known",
                    "recovery": "retry the same cancel key and observe the Session",
                }),
            )
            .await?;
            schedule_remote_cancel_finalizer(&state, owner.as_ref(), &session_id, &task_key)?;
            return Err(nomi_core_remote_unknown_error(
                "Remote cancellation panicked before its durable outcome was known",
                &session_id,
                &task_key,
            ));
        }
        Err(NomiCoreRemoteDetachedFailure::Admission(error)) => {
            schedule_remote_cancel_finalizer(&state, owner.as_ref(), &session_id, &task_key)?;
            return Err(nomi_core_remote_admission_error(
                "remote.cancel",
                &session_id,
                error,
            ));
        }
    }

    let current = match transition_remote_state(
        &state,
        &owner,
        &session_id,
        remote_session,
        "cancelled",
        &key,
        json!({
            "outcome": "cancelled",
            "request_digest": request_digest,
        }),
    )
    .await
    {
        Ok(current) => current,
        Err(error) => {
            // The runtime command succeeded, but the durable state/event
            // commit did not. Keep the request fenced and let the same-key
            // finalizer retry the atomic convergence.
            schedule_remote_cancel_finalizer(&state, owner.as_ref(), &session_id, &key)?;
            return Err(error);
        }
    };
    Ok(Json(RemoteMutationResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        cursor: remote_event_cursor(&state, &owner, &session_id).await?,
        session_status: remote_status_label(&current.state).to_owned(),
    }))
}

/// Only classify errors that prove the cancel command was rejected before it
/// could reach the live Nomi runtime as a terminal `rejected` fact. Provider,
/// transport, timeout, and internal errors are deliberately uncertain: the
/// command may have crossed the runtime boundary before the error surfaced.
fn cancel_error_is_known_rejection(error: &AppError) -> bool {
    matches!(
        error,
        AppError::NotFound(_)
            | AppError::BadRequest(_)
            | AppError::Unauthorized(_)
            | AppError::Forbidden(_)
            | AppError::Conflict(_)
            | AppError::RevisionConflict(_)
            | AppError::UnprocessableEntity(_)
            | AppError::WorkspacePathEdgeWhitespace(_)
            | AppError::WorkspacePathEdgeWhitespaceRuntimeUnsupported(_)
    )
}

fn nomi_core_remote_mutation_key(
    operation: &str,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
    idempotency_key: &str,
) -> String {
    let scope = format!(
        "nomi-core-remote-mutation-v1\0{operation}\0{}\0{}\0{idempotency_key}",
        owner.as_ref(),
        session_id.as_ref(),
    );
    format!(
        "nomi-core-remote:{operation}:{}",
        nomifun_auth::token_sha256_hex(&scope)
    )
}

fn nomi_core_remote_timeout(
    operation: &'static str,
    timeout: Duration,
    session_id: Option<&AgentSessionId>,
) -> NomiCoreApiError {
    let details = match session_id {
        Some(session_id) => json!({
            "operation": operation,
            "agent_session_id": session_id,
            "timeout_ms": timeout.as_millis() as u64,
            "outcome": "unknown",
            "recovery": "retry the same idempotency key and inspect the Session",
        }),
        None => json!({
            "operation": operation,
            "timeout_ms": timeout.as_millis() as u64,
            "outcome": "unknown",
            "recovery": "retry the same idempotency key and inspect the Session",
        }),
    };
    NomiCoreApiError::with_details(
        StatusCode::GATEWAY_TIMEOUT,
        "NOMI_CORE_REMOTE_OPERATION_TIMEOUT",
        format!(
            "Nomi-core Remote {operation} exceeded its {} ms deadline",
            timeout.as_millis()
        ),
        details,
    )
}

fn nomi_core_remote_unknown_error(
    message: &'static str,
    session_id: &AgentSessionId,
    idempotency_key: &str,
) -> NomiCoreApiError {
    NomiCoreApiError::with_details(
        StatusCode::GATEWAY_TIMEOUT,
        "NOMI_CORE_REMOTE_CANCEL_UNKNOWN",
        message,
        json!({
            "agent_session_id": session_id,
            "idempotency_key": idempotency_key,
            "outcome": "unknown",
            "recovery": "retry the same idempotency key and observe the same AgentSession",
        }),
    )
}

fn nomi_core_remote_admission_error(
    operation: &'static str,
    session_id: &AgentSessionId,
    error: super::remote_runtime::RemoteDetachedMutationAdmissionError,
) -> NomiCoreApiError {
    let (status, reason, recovery) = match error {
        super::remote_runtime::RemoteDetachedMutationAdmissionError::AlreadyRunning => (
            StatusCode::CONFLICT,
            "already_in_flight",
            "retry the same idempotency key after observing the Session",
        ),
        super::remote_runtime::RemoteDetachedMutationAdmissionError::CapacityExceeded => (
            StatusCode::SERVICE_UNAVAILABLE,
            "capacity_exhausted",
            "retry the same idempotency key after capacity recovers",
        ),
        super::remote_runtime::RemoteDetachedMutationAdmissionError::Closed => (
            StatusCode::SERVICE_UNAVAILABLE,
            "coordinator_closed",
            "restart the host and retry the same idempotency key",
        ),
    };
    NomiCoreApiError::with_details(
        status,
        "NOMI_CORE_REMOTE_OPERATION_BLOCKED",
        format!("Nomi-core Remote {operation} was not admitted"),
        json!({
            "operation": operation,
            "agent_session_id": session_id,
            "outcome": if reason == "already_in_flight" { "unknown" } else { "not_started" },
            "reason": reason,
            "recovery": recovery,
        }),
    )
}

#[cfg(test)]
mod cancel_error_tests {
    use super::{cancel_error_is_known_rejection, remote_delivery_terminal_event_type};
    use nomifun_common::AppError;

    #[test]
    fn only_precondition_errors_become_cancel_rejected() {
        assert!(cancel_error_is_known_rejection(&AppError::BadRequest(
            "invalid session".to_owned()
        )));
        assert!(cancel_error_is_known_rejection(&AppError::Conflict(
            "already cancelled".to_owned()
        )));
        assert!(!cancel_error_is_known_rejection(
            &AppError::ProviderUnavailable("upstream unavailable".to_owned())
        ));
        assert!(!cancel_error_is_known_rejection(&AppError::RateLimited));
        assert!(!cancel_error_is_known_rejection(&AppError::Timeout(
            "runtime did not answer".to_owned()
        )));
        assert!(!cancel_error_is_known_rejection(&AppError::Internal(
            "database actor failed".to_owned()
        )));
    }

    #[test]
    fn completed_delivery_without_explicit_result_is_unknown_not_success() {
        assert_eq!(
            remote_delivery_terminal_event_type(false, None),
            ("turn/accepted", "accepted", false)
        );
        assert_eq!(
            remote_delivery_terminal_event_type(true, Some(true)),
            ("turn/completed", "completed", true)
        );
        assert_eq!(
            remote_delivery_terminal_event_type(true, Some(false)),
            ("turn/failed", "failed", false)
        );
        assert_eq!(
            remote_delivery_terminal_event_type(true, None),
            ("turn/unknown", "unknown", false)
        );
    }
}

async fn resolve_saved_binding_projection(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    binding: &AgentBindingValueDto,
    title: Option<&str>,
) -> Result<
    super::nomi_core_agent_projection::NomiCoreSavedBindingProjection,
    NomiCoreApiError,
> {
    let (binding, revision, snapshot) = state
        .control_plane
        .saved_binding_artifacts(&owner.0, binding)
        .await?;
    super::nomi_core_agent_projection::project_saved_artifacts(
        &common_owner_id(owner)?,
        binding,
        revision,
        snapshot,
        title,
    )
    .map_err(Into::into)
}

async fn load_owned_nomi_core_session(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
) -> Result<ConversationResponse, NomiCoreApiError> {
    let response = state
        .session_owner
        .get_session(owner.as_ref(), session_id.as_ref())
        .await
        .map_err(NomiCoreApiError::from)?;
    if parse_agent_session_id(&response.conversation_id)? != *session_id {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_SESSION_IDENTITY_CONFLICT",
            "ConversationService returned a different Session identity",
        ));
    }
    Ok(response)
}

fn session_metadata(
    response: &ConversationResponse,
    owner: &AuthenticatedOwner,
) -> Result<NomiCoreSessionMetadata, NomiCoreApiError> {
    let metadata = response
        .extra
        .get(NOMI_CORE_SESSION_METADATA_KEY)
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::NOT_FOUND,
                "NOMI_CORE_AGENT_SESSION_NOT_FOUND",
                "the Conversation is not an app-local Nomi-core AgentSession",
            )
        })?;
    let metadata: NomiCoreSessionMetadata = serde_json::from_value(metadata.clone())?;
    if metadata.version != NOMI_CORE_SESSION_METADATA_VERSION
        || !matches!(
            metadata.kind.as_str(),
            NOMI_CORE_SESSION_KIND | NOMI_CORE_REMOTE_KIND
        )
        || metadata
            .binding
            .typed_resource_bindings
            .iter()
            .any(|resource| resource.owner_id.as_str() != owner.as_ref())
    {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_SESSION_METADATA_INVALID",
            "Nomi-core Session metadata is not an exact owner-scoped binding",
        ));
    }
    Ok(metadata)
}

fn attach_session_metadata(
    extra: &mut Value,
    binding: &AgentBindingValue,
    remote: Option<RemoteBindingProvenance>,
) -> Result<(), NomiCoreApiError> {
    let object = extra.as_object_mut().ok_or_else(|| {
        NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "NOMI_CORE_SESSION_EXTRA_INVALID",
            "Nomi-core projection extra must be a JSON object",
        )
    })?;
    object.insert(
        NOMI_CORE_SESSION_METADATA_KEY.to_owned(),
        serde_json::to_value(NomiCoreSessionMetadata {
            version: NOMI_CORE_SESSION_METADATA_VERSION,
            kind: if remote.is_some() {
                NOMI_CORE_REMOTE_KIND.to_owned()
            } else {
                NOMI_CORE_SESSION_KIND.to_owned()
            },
            binding: binding.clone(),
            remote,
        })?,
    );
    Ok(())
}

async fn build_session_observation(
    owner: &Arc<NomiCoreSessionOwner>,
    authenticated_owner: &AuthenticatedOwner,
    response: &ConversationResponse,
    metadata: NomiCoreSessionMetadata,
    after_seq: u64,
    limit: u32,
) -> Result<SessionObservation, NomiCoreApiError> {
    validate_page_limit(limit)?;
    let session_id = parse_agent_session_id(&response.conversation_id)?;
    let (messages, next_seq) =
        read_message_projection_page(owner, &session_id, after_seq, limit).await?;
    let head = SessionHeadProjection {
        session_id: session_id.clone(),
        status: projected_session_status(response),
        active_turn_id: response
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.active_turn_id.clone()),
        active_set_generation: 0,
        runtime_checkpoint_locator: None,
        runtime_checkpoint_digest: None,
        runtime_bound_event_id: None,
        runtime_protocol_version: None,
        snapshot_digest: Some(
            metadata
                .binding
                .resolved_snapshot_ref
                .snapshot_digest
                .as_ref()
                .to_owned(),
        ),
        checkpoint_through_seq: None,
        last_seq: next_seq,
        unread_count: 0,
    };
    let live = AgentSessionLiveRecord {
        agent_session_id: session_id.clone(),
        owner_ref: PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: authenticated_owner.as_ref().to_owned(),
        },
        metadata: AgentSessionMetadata {
            title: Some(response.name.clone()),
            archived: false,
            pinned: response.pinned,
        },
        agent_binding: metadata.binding,
        remote_binding_provenance: metadata.remote,
        parent_session_id: None,
        fork_base_payload_id: None,
        next_seq: next_seq.saturating_add(1),
    };
    Ok(SessionObservation {
        session: live,
        head,
        events: Vec::new(),
        messages,
        next_cursor: nomifun_agent_contracts::SessionEventCursor {
            agent_session_id: session_id,
            seq: next_seq,
        },
    })
}

async fn read_message_projection_page(
    owner: &Arc<NomiCoreSessionOwner>,
    session_id: &AgentSessionId,
    after_seq: u64,
    limit: u32,
) -> Result<(Vec<MessageProjection>, u64), NomiCoreApiError> {
    let limit = limit.min(NOMI_CORE_MESSAGE_PAGE_SIZE).max(1) as usize;
    let repository = owner.service().conversation_repo().clone();
    let mut page_number = 1_u32;
    let mut max_seen = after_seq;
    let mut projections = Vec::with_capacity(limit);
    loop {
        let page = repository
            .get_messages(
                session_id.as_ref(),
                page_number,
                NOMI_CORE_MESSAGE_PAGE_SIZE,
                SortOrder::Asc,
            )
            .await
            .map_err(|error| {
                NomiCoreApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "NOMI_CORE_MESSAGE_PROJECTION_FAILED",
                    error.to_string(),
                )
            })?;
        for row in &page.items {
            let seq = u64::try_from(row.id).map_err(|_| {
                NomiCoreApiError::new(
                    StatusCode::CONFLICT,
                    "NOMI_CORE_MESSAGE_CURSOR_INVALID",
                    "a persisted message row has a negative cursor identity",
                )
            })?;
            max_seen = max_seen.max(seq);
            if seq <= after_seq || row.hidden {
                continue;
            }
            projections.push(message_projection(session_id, row, seq)?);
            if projections.len() >= limit {
                let next_seq = projections
                    .last()
                    .map_or(max_seen, |message| message.last_seq);
                return Ok((
                    projections,
                    next_seq,
                ));
            }
        }
        if !page.has_more || page.items.is_empty() {
            break;
        }
        page_number = page_number.saturating_add(1);
        if page_number > NOMI_CORE_MAX_CURSOR_SCAN_PAGES {
            return Err(NomiCoreApiError::with_details(
                StatusCode::SERVICE_UNAVAILABLE,
                "NOMI_CORE_CURSOR_SCAN_LIMIT",
                "the Nomi-core message cursor window exceeded the bounded adapter scan",
                json!({
                    "agent_session_id": session_id,
                    "after_seq": after_seq,
                    "max_pages": NOMI_CORE_MAX_CURSOR_SCAN_PAGES,
                    "recovery": "retry with a newer cursor or integrate a native keyset query",
                }),
            ));
        }
    }
    Ok((projections, max_seen))
}

async fn read_message_projections_by_ids(
    owner: &Arc<NomiCoreSessionOwner>,
    session_id: &AgentSessionId,
    message_ids: &[String],
) -> Result<Vec<Value>, NomiCoreApiError> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }
    let wanted = message_ids.iter().cloned().collect::<HashSet<_>>();
    let repository = owner.service().conversation_repo().clone();
    let mut found = HashMap::<String, MessageProjection>::new();
    let mut page_number = 1_u32;
    loop {
        let page = repository
            .get_messages(
                session_id.as_ref(),
                page_number,
                NOMI_CORE_MESSAGE_PAGE_SIZE,
                SortOrder::Asc,
            )
            .await
            .map_err(|error| {
                NomiCoreApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "NOMI_CORE_MESSAGE_PROJECTION_FAILED",
                    error.to_string(),
                )
            })?;
        for row in &page.items {
            if !wanted.contains(&row.message_id) || row.hidden {
                continue;
            }
            let seq = u64::try_from(row.id).map_err(|_| {
                NomiCoreApiError::new(
                    StatusCode::CONFLICT,
                    "NOMI_CORE_MESSAGE_CURSOR_INVALID",
                    "a persisted message row has a negative cursor identity",
                )
            })?;
            found
                .entry(row.message_id.clone())
                .or_insert(message_projection(session_id, row, seq)?);
        }
        if found.len() == wanted.len() || !page.has_more || page.items.is_empty() {
            break;
        }
        page_number = page_number.saturating_add(1);
        if page_number > NOMI_CORE_MAX_CURSOR_SCAN_PAGES {
            return Err(NomiCoreApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "NOMI_CORE_CURSOR_SCAN_LIMIT",
                "the Nomi-core message projection exceeded the bounded scan",
            ));
        }
    }
    message_ids
        .iter()
        .filter_map(|id| found.get(id))
        .map(|projection| serde_json::to_value(projection).map_err(Into::into))
        .collect()
}

fn message_projection(
    session_id: &AgentSessionId,
    row: &MessageRow,
    seq: u64,
) -> Result<MessageProjection, NomiCoreApiError> {
    let projection: Value = serde_json::from_str(&row.content).map_err(|error| {
        NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_MESSAGE_CONTENT_INVALID",
            format!("persisted message {} is not valid JSON: {error}", row.message_id),
        )
    })?;
    let semantic_digest = nomifun_agent_contracts::digest_payload(&projection)
        .map_err(|error| {
            NomiCoreApiError::new(
                StatusCode::CONFLICT,
                "NOMI_CORE_MESSAGE_DIGEST_FAILED",
                error.to_string(),
            )
        })?
        .as_ref()
        .to_owned();
    Ok(MessageProjection {
        session_id: session_id.clone(),
        projection_id: row.message_id.clone(),
        first_seq: seq,
        last_seq: seq,
        presentation_intent: row
            .position
            .clone()
            .unwrap_or_else(|| "message".to_owned()),
        projection,
        semantic_digest,
    })
}

async fn durable_message_cursor(
    owner: &Arc<NomiCoreSessionOwner>,
    session_id: &AgentSessionId,
) -> Result<SessionCursorDto, NomiCoreApiError> {
    let (_, seq) = read_message_projection_page(
        owner,
        session_id,
        0,
        NOMI_CORE_MESSAGE_PAGE_SIZE,
    )
    .await?;
    Ok(session_cursor(session_id, seq))
}

fn agent_binding_dto(
    binding: &AgentBindingValue,
) -> Result<AgentBindingValueDto, NomiCoreApiError> {
    serde_json::from_value(serde_json::to_value(binding)?).map_err(Into::into)
}

fn common_owner_id(
    owner: &AuthenticatedOwner,
) -> Result<nomifun_common::UserId, NomiCoreApiError> {
    nomifun_common::UserId::parse(owner.as_ref().to_owned()).map_err(|error| {
        NomiCoreApiError::new(
            StatusCode::FORBIDDEN,
            "NOMI_CORE_OWNER_ID_INVALID",
            format!("authenticated owner is not a canonical UserId: {error}"),
        )
    })
}

fn parse_agent_session_id(value: &str) -> Result<AgentSessionId, NomiCoreApiError> {
    let uuid = Uuid::parse_str(value).map_err(|_| {
        NomiCoreApiError::new(
            StatusCode::NOT_FOUND,
            "NOMI_CORE_AGENT_SESSION_NOT_FOUND",
            "agent_session_id must be a canonical UUIDv7",
        )
    })?;
    if uuid.get_version_num() != 7 || uuid.hyphenated().to_string() != value {
        return Err(NomiCoreApiError::new(
            StatusCode::NOT_FOUND,
            "NOMI_CORE_AGENT_SESSION_NOT_FOUND",
            "agent_session_id must be a canonical UUIDv7",
        ));
    }
    Ok(AgentSessionId::from(value.to_owned()))
}

fn projected_session_status(response: &ConversationResponse) -> String {
    if response
        .runtime
        .as_ref()
        .is_some_and(|runtime| runtime.state == ConversationRuntimeStateKind::Starting)
    {
        return "opening".to_owned();
    }
    if response
        .runtime
        .as_ref()
        .is_some_and(|runtime| runtime.state == ConversationRuntimeStateKind::Running)
        || response.status == nomifun_common::ConversationStatus::Running
    {
        return "running".to_owned();
    }
    "ready".to_owned()
}

fn bounded_turn_input(value: Value) -> Result<SendMessageRequest, NomiCoreApiError> {
    let value = {
        let bytes = nomifun_agent_contracts::canonical_json_bytes(&value).map_err(|error| {
            NomiCoreApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "NOMI_CORE_INVALID_REQUEST",
                error.to_string(),
            )
        })?;
        if bytes.len() > nomifun_agent_session::MAX_INLINE_JSON_BYTES {
            return Err(NomiCoreApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "NOMI_CORE_INPUT_TOO_LARGE",
                "Nomi-core turn input exceeds the bounded inline JSON limit",
            ));
        }
        value
    };
    if let Some(content) = value.as_str() {
        return nonempty_turn_content(content);
    }
    let object = value.as_object().ok_or_else(|| {
        NomiCoreApiError::new(
            StatusCode::BAD_REQUEST,
            "NOMI_CORE_INVALID_REQUEST",
            "turn input must be a string or an object containing content",
        )
    })?;
    let content = object
        .get("content")
        .or_else(|| object.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::BAD_REQUEST,
                "NOMI_CORE_INVALID_REQUEST",
                "turn input requires a non-empty content string",
            )
        })?;
    let mut request = nonempty_turn_content(content)?;
    if let Some(files) = object.get("files") {
        request.files = serde_json::from_value(files.clone())?;
    }
    if let Some(skills) = object.get("inject_skills") {
        request.inject_skills = serde_json::from_value(skills.clone())?;
    }
    if let Some(hidden) = object.get("hidden") {
        request.hidden = hidden.as_bool().ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::BAD_REQUEST,
                "NOMI_CORE_INVALID_REQUEST",
                "turn input hidden must be boolean",
            )
        })?;
    }
    if let Some(origin) = object.get("origin") {
        request.origin = Some(origin.as_str().ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::BAD_REQUEST,
                "NOMI_CORE_INVALID_REQUEST",
                "turn input origin must be string",
            )
        })?
        .to_owned());
    }
    if let Some(channel_platform) = object.get("channel_platform") {
        request.channel_platform = Some(channel_platform.as_str().ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::BAD_REQUEST,
                "NOMI_CORE_INVALID_REQUEST",
                "turn input channel_platform must be string",
            )
        })?
        .to_owned());
    }
    Ok(request)
}

fn nonempty_turn_content(content: &str) -> Result<SendMessageRequest, NomiCoreApiError> {
    if content.trim().is_empty() {
        return Err(NomiCoreApiError::new(
            StatusCode::BAD_REQUEST,
            "NOMI_CORE_INVALID_REQUEST",
            "turn content must not be empty",
        ));
    }
    Ok(SendMessageRequest {
        content: content.to_owned(),
        files: Vec::new(),
        inject_skills: Vec::new(),
        hidden: false,
        origin: None,
        channel_platform: None,
    })
}

fn request_idempotency_key(
    headers: &HeaderMap,
    prefix: &str,
) -> Result<String, NomiCoreApiError> {
    if let Some(value) = headers.get("Idempotency-Key") {
        let value = value.to_str().map_err(|_| {
            NomiCoreApiError::new(
                StatusCode::BAD_REQUEST,
                "NOMI_CORE_INVALID_REQUEST",
                "Idempotency-Key must be visible ASCII",
            )
        })?;
        return canonical_nonempty(value, "Idempotency-Key");
    }
    Ok(format!("{prefix}:{}", Uuid::now_v7()))
}

fn canonical_nonempty(value: &str, field: &str) -> Result<String, NomiCoreApiError> {
    if value.trim().is_empty()
        || value.trim() != value
        || !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        || value.len() > nomifun_common::MAX_IDEMPOTENCY_KEY_LEN
    {
        return Err(NomiCoreApiError::new(
            StatusCode::BAD_REQUEST,
            "NOMI_CORE_INVALID_REQUEST",
            format!("{field} must be non-empty visible ASCII within the bounded key size"),
        ));
    }
    Ok(value.to_owned())
}

fn validate_page_limit(limit: u32) -> Result<(), NomiCoreApiError> {
    if limit == 0 {
        return Err(NomiCoreApiError::new(
            StatusCode::BAD_REQUEST,
            "NOMI_CORE_INVALID_REQUEST",
            "limit must be greater than zero",
        ));
    }
    Ok(())
}

fn remote_session_not_found() -> NomiCoreApiError {
    NomiCoreApiError::new(
        StatusCode::NOT_FOUND,
        "REMOTE_SESSION_NOT_FOUND",
        "AgentSession is not a Remote Session owned by the authenticated owner",
    )
}

fn session_cursor(session_id: &AgentSessionId, seq: u64) -> SessionCursorDto {
    SessionCursorDto {
        agent_session_id: session_id.as_ref().to_owned(),
        seq,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
