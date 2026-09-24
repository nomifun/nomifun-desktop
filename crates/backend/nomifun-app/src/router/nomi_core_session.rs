//! One host-owned typed Session facade for the Nomi-core product.
//!
//! AgentSession HTTP and model-control entrypoints use the canonical generation
//! 5 Store. The legacy Conversation service remains behind this composition
//! boundary only for runtime/domain consumers that have explicit later cutover
//! owners; it is not an AgentSession identity or receipt authority.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use dashmap::DashMap;
use futures_util::FutureExt;
use nomifun_ai_agent::types::{AgentRuntimeBuildOptions, SendMessageData};
use nomifun_ai_agent::{
    AgentRuntimeSessions, AgentSendError, AgentStreamEvent, KernelNomiPluginToolSession,
    NomiPluginToolSchemaResolver, NomiPluginToolSession,
    NomiPluginToolSessionProvider, NomiPluginToolSessionRequest,
    NomiPlatformBuiltinContextAdmission,
    NomiPlatformBuiltinLifecycleAdmission,
    NomiPlatformBuiltinToolAdmission,
    SessionControlSink,
};
use nomifun_agent_contracts::{
    AgentBindingValue, AgentHandoffBindingRefV1, AgentHandoffCompletionAccountV1,
    AgentHandoffCompletionCriterionV1, AgentHandoffEnvelopeV1, AgentHandoffInputCitationV1,
    AgentHandoffMode, AgentHandoffPlanStepV1, AgentHandoffPlanV1,
    AgentHandoffRequirementOriginV1, AgentHandoffRequirementV1,
    AgentHandoffVerifiedArtifactV1, AgentSessionId, ArtifactId, ChatRouteFeature,
    ChatRouteProtocol, ContributionSourceKind,
    DeleteAgentSessionCommand, EffectClass, OperationId, PrincipalRef, RemoteBindingProvenance,
    ReasoningEffort, ScopeKey, SessionPayloadBody, StrictJsonValue, UserId, digest_bytes,
    digest_payload,
};
use nomifun_agent_control_plane::{
    AgentControlPlane, AuthenticatedOwner, ControlPlaneError,
};
use nomifun_agent_runtime::{
    AgentCompletionReport, AgentCriterionDisposition, AgentEngineEvent, AgentInputCitation,
    AgentPlan, AgentPlanStatus, AgentTaskRequirement,
};
use super::nomi_core_control_plane::control_plane_router_without_legacy_skills;
use nomifun_api_types::{
    AgentBindingValueDto, AgentHandoffAvailabilityDto, AgentHandoffModeDto,
    AgentResourceSelectionDto, AgentSwitchBlockerDto, AgentSwitchCapabilityDiffDto,
    AgentSwitchIdentityDto, AgentSwitchModelPreviewDto, AgentSwitchResourceDiffDto,
    AgentSwitchSelectionDto, ApplyAgentSessionSwitchRequestDto,
    ApplyAgentSessionSwitchResponseDto,
    AgentSessionKnowledgeBindingDto, AgentSessionKnowledgePolicyDto,
    AgentSessionKnowledgeWritebackEagernessDto,
    ApiResponse, ConversationListResponse, ConversationResponse,
    ConversationRuntimeStateKind, ConversationRuntimeSummary, CreateAgentSessionRequestDto, CreateConversationRequest,
    CreateAgentSessionResponseDto, CreateAgentSessionTurnRequestDto,
    CreateAgentSessionTurnResponseDto, AgentSessionTurnMutationResponseDto,
    CancelAgentSessionTurnRequestDto, SteerAgentSessionTurnRequestDto,
    ErrorResponse, ForkAgentSessionRequestDto,
    ForkAgentSessionResponseDto, ListMessagesQuery, MessageListResponse, MessageResponse,
    MessageSearchItem, MessageSearchResponse, SearchMessagesQuery,
    PreviewAgentSessionSwitchRequestDto, PreviewAgentSessionSwitchResponseDto,
    RemoteCancelRequestDto, RemoteMutationResponseDto, RemoteObserveRequestDto,
    RemoteObserveResponseDto, RemoteOpenRequestDto, RemoteOpenResponseDto,
    RemoteOpenStateViewDto, RemoteTurnRequestDto,
    AgentChatModelSelectionDto, AgentResolvedSnapshot, SessionCursorDto,
    SessionReasoningEffortDto, UpdateAgentSessionReasoningRequestDto,
    UpdateAgentSessionReasoningResponseDto,
    McpServerId,
    SendMessageRequest, SideQuestionRequest, SideQuestionResponse,
    TypedResourceBindingDto, UpdateConversationRequest, WebSocketMessage, WorkspaceBrowseQuery, WorkspaceEntry,
    CreateAgentPresetFromTemplateRequest, PutAgentBindingRequest,
};
use nomifun_common::{
    AgentKillReason, AppError, ConversationStatus, MessagePosition, MessageStatus, MessageType,
    PaginatedResult,
    normalize_keys_to_snake_case,
};
use nomifun_common::paths::{
    WorkspaceDirectoryCheck, canonical_existing_workspace_directory,
};
use nomifun_conversation::{
    AgentMutationReceipt, BackgroundTaskRegistrar, CanonicalAgentSessionOwner,
    IdempotentMessageDelivery, PreparedAgentSessionDelete, ProductAgentResolution,
    ProductAgentSnapshotResolver, ProductAgentTarget, PublicTurnDeliveryState,
};
use nomifun_conversation::{
    CreativeStudioAgentHistoryMessage, CreativeStudioAgentHistoryRole,
    CreativeStudioAgentHistoryStatus, CreativeStudioAgentModelRef,
    CreativeStudioCanvasAgentSessionBindingResponse,
    ResolveCreativeStudioCanvasAgentSessionRequest,
    ResolveCreativeStudioCanvasAgentSessionResponse,
};
use nomifun_db::{
    AgentExecutionTurnAuthority, AppendNomiRemoteEventParams, GetOrCreateRemoteSessionParams,
    IRemoteBindingRepository, RemoteOpenResult, TransitionNomiRemoteSessionParams,
};
use nomifun_db::models::{NomiRemoteEventRow, NomiRemoteSessionRow};
use nomifun_agent_session::{MessageProjection, SessionObservation};
use super::history_process_display::{HistoricalToolObservation, load_historical_tool_observations};
use nomifun_realtime::UserEventSink;
use nomifun_agent_kernel::{
    AgentPresetCompiler, CompileRequest, CompiledSnapshot,
    CompilerEnvironment, KernelRegistry,
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
/// The canonical owner is authoritative for AgentSession identity, Turn/Event
/// receipts, resources, forks, runtime dispatch and deletion.
pub(crate) struct NomiCoreSessionOwner {
    official_runtime: std::sync::OnceLock<Arc<super::official_runtime::OfficialRuntimeHost>>,
    runtime_control_plane: std::sync::OnceLock<std::sync::Weak<AgentControlPlane>>,
    product_agent_resolver:
        std::sync::OnceLock<std::sync::Weak<NomiCoreProductAgentResolver>>,
    idmm: std::sync::OnceLock<std::sync::Weak<nomifun_idmm::IdmmService>>,
    canonical: CanonicalAgentSessionOwner,
    runtime_sessions: Arc<dyn AgentRuntimeSessions>,
    user_events: Arc<dyn UserEventSink>,
    background_tasks: Arc<dyn BackgroundTaskRegistrar>,
    /// Current installation-owned root for per-Session managed workspaces.
    /// Every default workspace is materialized as `<root>/<AgentSessionId>`;
    /// user-selected workspaces never use this root.
    managed_workspace_root: std::path::PathBuf,
    pool: nomifun_db::SqlitePool,
    creation_service: Arc<nomifun_creation::CreationService>,
    session_operation_locks:
        Arc<DashMap<String, Arc<tokio::sync::RwLock<()>>>>,
}

pub(crate) struct CanonicalAgentTranscriptSource {
    sessions: Arc<NomiCoreSessionOwner>,
    owner_id: Arc<str>,
}

impl CanonicalAgentTranscriptSource {
    pub(crate) fn new(sessions: Arc<NomiCoreSessionOwner>, owner_id: Arc<str>) -> Self {
        Self { sessions, owner_id }
    }
}

fn redact_transcript_value(value: &str) -> String {
    let redacted = nomi_redact::redact_secrets_owned(value.to_owned());
    if redacted.chars().count() <= 600 {
        redacted
    } else {
        format!("{}…", redacted.chars().take(600).collect::<String>())
    }
}

#[async_trait]
impl nomifun_companion::evolution::TranscriptSource for CanonicalAgentTranscriptSource {
    async fn window(
        &self,
        anchor: &nomifun_companion::evolution::TranscriptAnchor,
    ) -> Result<Option<Vec<nomifun_companion::evolution::TranscriptTurn>>, AppError> {
        if nomifun_common::validate_uuidv7(&anchor.conversation_id).is_err() {
            return Ok(None);
        }
        let session_id = AgentSessionId::from(anchor.conversation_id.clone());
        match self
            .sessions
            .canonical
            .get(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: self.owner_id.to_string(),
                },
                &session_id,
            )
            .await
        {
            Ok(_) => {}
            Err(AppError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(error),
        }
        let (mut projections, _, _) = self
            .sessions
            .canonical
            .store()
            .messages_before(&session_id, None, 500)
            .await
            .map_err(agent_session_store_error)?;
        projections.reverse();
        if projections.is_empty() {
            return Ok(None);
        }
        let wanted = anchor.call_ids.iter().map(String::as_str).collect::<HashSet<_>>();
        let hits = projections
            .iter()
            .enumerate()
            .filter_map(|(index, projection)| {
                projection
                    .projection
                    .get("tool_summary")
                    .and_then(|summary| summary.get("call_id"))
                    .and_then(Value::as_str)
                    .is_some_and(|call_id| wanted.contains(call_id))
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        let (start, end) = if hits.is_empty() {
            (0, projections.len() - 1)
        } else {
            (
                hits.iter().min().copied().unwrap_or_default().saturating_sub(anchor.pad_turns),
                (hits.iter().max().copied().unwrap_or_default() + anchor.pad_turns)
                    .min(projections.len() - 1),
            )
        };
        let mut turns = Vec::new();
        for projection in &projections[start..=end] {
            match projection.presentation_intent.as_str() {
                "message" => {
                    let text = projection
                        .projection
                        .get("content")
                        .and_then(Value::as_str)
                        .filter(|text| !text.trim().is_empty());
                    let Some(text) = text else { continue };
                    let text = redact_transcript_value(text);
                    if projection
                        .projection
                        .get("state")
                        .and_then(Value::as_str)
                        == Some("accepted")
                    {
                        turns.push(nomifun_companion::evolution::TranscriptTurn::user(text));
                    } else {
                        turns.push(nomifun_companion::evolution::TranscriptTurn::assistant(text));
                    }
                }
                "tool" => {
                    let Some(summary) = projection
                        .projection
                        .get("tool_summary")
                        .and_then(Value::as_object)
                    else {
                        continue;
                    };
                    let name = summary
                        .get("name")
                        .and_then(Value::as_str)
                        .or_else(|| summary.get("action_id").and_then(Value::as_str))
                        .unwrap_or("tool");
                    turns.push(nomifun_companion::evolution::TranscriptTurn::tool(
                        redact_transcript_value(name),
                        None,
                        None,
                    ));
                }
                _ => {}
            }
        }
        Ok((!turns.is_empty()).then_some(turns))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ProductAgentSelection {
    Template { template_key: String },
    Preset { preset_id: String },
}

impl ProductAgentSelection {
    fn template_id(&self) -> Option<&str> {
        match self { Self::Template { template_key } => Some(template_key), _ => None }
    }
    fn preset_id(&self) -> Option<&str> {
        match self { Self::Preset { preset_id } => Some(preset_id), _ => None }
    }
}

pub(crate) struct NomiCoreProductAgentResolver {
    control_plane: Arc<AgentControlPlane>,
    official_runtime: Arc<super::official_runtime::OfficialRuntimeHost>,
    resource_bindings: super::nomi_core_resource_bindings::NomiCoreResourceBindingResolverRegistry,
    owner_id: Arc<str>,
    pool: nomifun_db::SqlitePool,
    default_binding_lock: tokio::sync::Mutex<()>,
}

impl NomiCoreProductAgentResolver {
    pub(crate) fn new(control_plane: Arc<AgentControlPlane>, owner_id: Arc<str>, pool: nomifun_db::SqlitePool, official_runtime: Arc<super::official_runtime::OfficialRuntimeHost>, resource_bindings: super::nomi_core_resource_bindings::NomiCoreResourceBindingResolverRegistry) -> Self {
        Self {
            control_plane,
            official_runtime,
            resource_bindings,
            owner_id,
            pool,
            default_binding_lock: tokio::sync::Mutex::new(()),
        }
    }

    async fn selection(&self, owner: &UserId, kind: &str, id: &str) -> Result<Option<ProductAgentSelection>, AppError> {
        let raw: Option<String> = sqlx::query_scalar("SELECT selection_json FROM product_agent_selections WHERE owner_user_id = ? AND target_kind = ? AND target_id = ?")
            .bind(owner.as_ref()).bind(kind).bind(id).fetch_optional(&self.pool).await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(|error| AppError::Internal(error.to_string()))).transpose()
    }

    async fn save_selection(&self, owner: &UserId, kind: &str, id: &str, selection: &ProductAgentSelection) -> Result<(), AppError> {
        let raw = serde_json::to_string(selection).map_err(|error| AppError::Internal(error.to_string()))?;
        sqlx::query("INSERT INTO product_agent_selections (owner_user_id, target_kind, target_id, selection_json) VALUES (?, ?, ?, ?) ON CONFLICT(owner_user_id, target_kind, target_id) DO UPDATE SET selection_json = excluded.selection_json")
            .bind(owner.as_ref()).bind(kind).bind(id).bind(raw).execute(&self.pool).await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        Ok(())
    }

    async fn materialize(&self, owner: &UserId, selection: &ProductAgentSelection, model: Option<&AgentChatModelSelectionDto>) -> Result<AgentBindingValueDto, AppError> {
        self.control_plane.validate_product_selection(owner, selection.template_id(), selection.preset_id(), model).await.map_err(control_plane_error_to_app)?;
        let preset_id = match selection {
            ProductAgentSelection::Preset { preset_id } => preset_id.clone(),
            ProductAgentSelection::Template { template_key } => self.control_plane.create_from_template(owner, template_key,
                CreateAgentPresetFromTemplateRequest {
                    model: model.cloned(), reuse_existing: true, display_name: template_key.clone(), description: None,
                    model_route_refs: BTreeMap::new(), chat_route_records: BTreeMap::new(),
                }).await.map_err(control_plane_error_to_app)?.preset.preset_id,
        };
        self.control_plane.resolve_agent_session_binding_with_model(owner, &preset_id, model).await.map_err(control_plane_error_to_app)
    }
}

#[async_trait]
impl ProductAgentSnapshotResolver for NomiCoreProductAgentResolver {
    async fn resolve(
        &self,
        owner_id: &str,
        target: &ProductAgentTarget,
        requested_model: Option<&nomifun_common::ProviderWithModel>,
    ) -> Result<ProductAgentResolution, AppError> {
        let owner = UserId::from(owner_id.to_owned());
        let _guard = self.default_binding_lock.lock().await;
        let existing = self
            .control_plane
            .get_agent_binding(
                &owner,
                target.target_kind.clone(),
                target.target_id.clone(),
            )
            .await
            .map_err(control_plane_error_to_app)?;
        // A Companion is a product identity, not a generic Agent launcher. Keep
        // its official recipe authoritative even if an older client persisted a
        // custom product selection before this invariant was introduced.
        let mut selection = if target.target_kind == "companion" {
            Some(ProductAgentSelection::Template {
                template_key: target.default_template_key.clone(),
            })
        } else {
            self.selection(&owner, &target.target_kind, &target.target_id).await?
        };
        if selection.is_none() && let Some(record) = existing.as_ref() {
            // An implicit official choice follows its current seed just like
            // an explicit template choice. User-authored presets stay pinned.
            if let Some(key) = self.control_plane.internal_official_template(&owner,
                &record.agent_binding.preset_revision_ref.preset_id).await.map_err(control_plane_error_to_app)? {
                selection = Some(ProductAgentSelection::Template { template_key: key.as_str().to_owned() });
            }
        }
        let binding = if let Some(selection) = selection {
            let model = requested_model.map(|model| AgentChatModelSelectionDto { provider_id: model.provider_id.clone(), model: model.model.clone() });
            let mut binding = self.materialize(&owner, &selection, model.as_ref()).await?;
            if existing.as_ref().is_some_and(|record| record.agent_binding.preset_revision_ref == binding.preset_revision_ref
                && record.agent_binding.resolved_snapshot_ref == binding.resolved_snapshot_ref) {
                existing.unwrap().agent_binding
            } else {
                if let Some(previous) = existing.as_ref()
                    && !previous.agent_binding.typed_resource_bindings.is_empty()
                {
                    let (_, _, next_snapshot) = self.control_plane
                        .saved_binding_artifacts(&owner, &binding)
                        .await
                        .map_err(control_plane_error_to_app)?;
                    let selections = previous.agent_binding.typed_resource_bindings.iter()
                        .filter(|resource| next_snapshot.content.required_resource_kinds.iter()
                            .any(|kind| kind.as_ref() == resource.resource_kind))
                        .map(|resource| AgentResourceSelectionDto {
                            resource_kind: resource.resource_kind.clone(),
                            resource_id: resource.resource_id.clone(),
                        })
                        .collect::<Vec<_>>();
                    binding = self.resource_bindings
                        .resolve_for_saved_binding(
                            &self.control_plane,
                            &owner,
                            binding,
                            &selections,
                        )
                        .await
                        .map_err(|error| AppError::UnprocessableEntity(format!(
                            "{}: {}",
                            error.code(),
                            error.message(),
                        )))?;
                }
                let previous = existing.as_ref().map(|record| record.agent_binding.binding_version);
                binding.binding_version = previous.unwrap_or(0) + 1;
                self.control_plane.put_agent_binding(&owner, target.target_kind.clone(), target.target_id.clone(),
                    PutAgentBindingRequest { expected_binding_version: previous, agent_binding: binding }).await.map_err(control_plane_error_to_app)?.agent_binding
            }
        } else if let Some(existing) = existing {
            if let Some(model) = requested_model {
                self
                    .control_plane
                    .resolve_agent_session_binding_with_model(
                        &owner,
                        &existing.agent_binding.preset_revision_ref.preset_id,
                        Some(&AgentChatModelSelectionDto {
                            provider_id: model.provider_id.clone(),
                            model: model.model.clone(),
                        }),
                    )
                    .await
                    .map_err(control_plane_error_to_app)?
            } else {
                existing.agent_binding
            }
        } else {
            let editor = self
                .control_plane
                .create_from_template(
                    &owner,
                    &target.default_template_key,
                    CreateAgentPresetFromTemplateRequest {
                        model: requested_model.map(|model| AgentChatModelSelectionDto {
                            provider_id: model.provider_id.clone(),
                            model: model.model.clone(),
                        }),
                        reuse_existing: true,
                        display_name: target.default_template_key.clone(),
                        description: None,
                        model_route_refs: BTreeMap::new(),
                        chat_route_records: BTreeMap::new(),
                    },
                )
                .await
                .map_err(control_plane_error_to_app)?;
            let binding = self
                .control_plane
                .resolve_agent_session_binding(&owner, &editor.preset.preset_id)
                .await
                .map_err(control_plane_error_to_app)?;
            let (_, _, unbound_snapshot) = self
                .control_plane
                .saved_binding_artifacts(&owner, &binding)
                .await
                .map_err(control_plane_error_to_app)?;
            // Product-owned entry points already carry the authoritative
            // target identity. Materialize only resource kinds whose identity
            // is deterministic from that target; every other required kind
            // still fails closed in the resource resolver.
            let selections = unbound_snapshot
                .content
                .required_resource_kinds
                .iter()
                .filter_map(|kind| {
                    let resource_id = match (target.target_kind.as_str(), kind.as_ref()) {
                        (_, "workspace") => {
                            super::nomi_core_resource_bindings::DEFAULT_WORKSPACE_RESOURCE_ID
                                .to_owned()
                        }
                        (_, "project_memory") => {
                            super::nomi_core_resource_bindings::DEFAULT_PROJECT_MEMORY_RESOURCE_ID
                                .to_owned()
                        }
                        (_, "process_session") => {
                            super::nomi_core_resource_bindings::MANAGED_PROCESS_SESSION_RESOURCE_ID
                                .to_owned()
                        }
                        (_, "terminal") => {
                            super::nomi_core_resource_bindings::MANAGED_TERMINAL_RESOURCE_ID
                                .to_owned()
                        }
                        (_, "scheduler") => {
                            super::nomi_core_resource_bindings::INSTALLATION_SCHEDULER_RESOURCE_ID
                                .to_owned()
                        }
                        ("creative_studio_canvas", "canvas") => target.target_id.clone(),
                        ("creative_studio_canvas", "asset_library") => {
                            super::nomi_core_resource_bindings::CREATIVE_ASSET_LIBRARY_RESOURCE_ID
                                .to_owned()
                        }
                        ("companion", "companion" | "companion_memory") => {
                            target.target_id.clone()
                        }
                        ("customer", "customer") => target.target_id.clone(),
                        _ => return None,
                    };
                    Some(AgentResourceSelectionDto {
                        resource_kind: kind.as_ref().to_owned(),
                        resource_id,
                    })
                })
                .collect::<Vec<_>>();
            let mut binding = self
                .resource_bindings
                .resolve_for_saved_binding(
                    &self.control_plane,
                    &owner,
                    binding,
                    &selections,
                )
                .await
                .map_err(|error| AppError::UnprocessableEntity(format!(
                    "{}: {}",
                    error.code(),
                    error.message(),
                )))?;
            binding.binding_version = 1;
            self.control_plane
                .put_agent_binding(
                    &owner,
                    target.target_kind.clone(),
                    target.target_id.clone(),
                    PutAgentBindingRequest {
                        expected_binding_version: None,
                        agent_binding: binding,
                    },
                )
                .await
                .map_err(control_plane_error_to_app)?
                .agent_binding
        };
        let (binding, revision, snapshot) = self
            .control_plane
            .saved_binding_artifacts(&owner, &binding)
            .await
            .map_err(control_plane_error_to_app)?;
        let target_engine = self.official_runtime.validate_agent(&snapshot)?;
        let editor = self
            .control_plane
            .editor(
                &owner,
                binding.preset_revision_ref.preset_id.as_ref(),
                Some(binding.preset_revision_ref.revision),
            )
            .await
            .map_err(control_plane_error_to_app)?;
        let common_owner = nomifun_common::UserId::parse(owner_id.to_owned())
            .map_err(|error| AppError::Forbidden(format!("invalid product Agent owner: {error}")))?;
        let mut projected = super::agent_binding_projection::project_saved_artifacts(
            &common_owner,
            binding,
            revision,
            snapshot,
            Some(&editor.preset.display_name),
        )?;
        attach_session_metadata(
            &mut projected.projection.request.extra,
            &projected.binding,
            None,
        )
        .map_err(|error| AppError::Conflict(error.message))?;
        projected.projection.request.extra[nomifun_api_types::RUNTIME_BUILD_BINDING_KEY] =
            serde_json::to_value(&target_engine)
                .map_err(|error| AppError::Internal(error.to_string()))?;
        self.official_runtime
            .provider()?
            .validate_session_extra(&projected.projection.request.extra)?;
        Ok(ProductAgentResolution {
            snapshot: projected.projection.snapshot,
            runtime_extra: projected.projection.request.extra,
        })
    }
}

#[async_trait]
impl nomifun_customer_service::CustomerServiceAgentPolicyResolver
    for NomiCoreProductAgentResolver
{
    async fn resolve(
        &self,
        cs_agent_id: &str,
        provider_id: &str,
        model: &str,
    ) -> Result<nomifun_customer_service::CustomerServiceAgentPolicy, AppError> {
        let target = ProductAgentTarget {
            target_kind: "customer".to_owned(),
            target_id: cs_agent_id.to_owned(),
            default_template_key: "customer-service.default".to_owned(),
        };
        let requested_model = nomifun_common::ProviderWithModel {
            provider_id: provider_id.to_owned(),
            model: model.to_owned(),
            use_model: Some(model.to_owned()),
        };
        let resolved = ProductAgentSnapshotResolver::resolve(
            self,
            // Customer-service product resources are installation-owner scoped.
            // The target binding itself carries the authenticated owner and the
            // control plane rejects cross-owner reads.
            self.owner_id.as_ref(),
            &target,
            Some(&requested_model),
        )
        .await?;
        Ok(nomifun_customer_service::CustomerServiceAgentPolicy {
            capabilities: resolved
                .snapshot
                .enabled_capability_actions
                .into_values()
                .flatten()
                .collect(),
            instructions: resolved.snapshot.instructions,
        })
    }
}

impl NomiCoreSessionOwner {
    pub(crate) fn new(
        canonical: CanonicalAgentSessionOwner,
        runtime_sessions: Arc<dyn AgentRuntimeSessions>,
        user_events: Arc<dyn UserEventSink>,
        background_tasks: Arc<dyn BackgroundTaskRegistrar>,
        managed_workspace_root: std::path::PathBuf,
        pool: nomifun_db::SqlitePool,
        creation_service: Arc<nomifun_creation::CreationService>,
    ) -> Self {
        Self {
            canonical,
            official_runtime: std::sync::OnceLock::new(),
            runtime_control_plane: std::sync::OnceLock::new(),
            product_agent_resolver: std::sync::OnceLock::new(),
            idmm: std::sync::OnceLock::new(),
            runtime_sessions,
            user_events,
            background_tasks,
            managed_workspace_root,
            pool,
            creation_service,
            session_operation_locks: Arc::new(DashMap::new()),
        }
    }

    pub(crate) fn canonical(&self) -> &CanonicalAgentSessionOwner {
        &self.canonical
    }

    fn session_operation_lock(
        &self,
        session_id: &str,
    ) -> Arc<tokio::sync::RwLock<()>> {
        self.session_operation_locks
            .entry(session_id.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::RwLock::new(())))
            .clone()
    }

    async fn materialize_workspace_for_binding(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        binding: &AgentBindingValue,
    ) -> Result<Option<String>, AppError> {
        let workspace = frozen_workspace_root(
            &self.managed_workspace_root,
            owner_id,
            session_id,
            binding,
        )?;
        if uses_managed_session_workspace(binding) {
            return materialize_managed_session_workspace(
                &self.managed_workspace_root,
                session_id,
            )
            .await
            .map(Some);
        }
        workspace
            .map(|workspace| {
                canonical_existing_workspace_directory(
                    std::path::Path::new(&workspace),
                    WorkspaceDirectoryCheck::Runtime,
                )
                .map(|path| path.to_string_lossy().into_owned())
            })
            .transpose()
    }

    async fn materialize_session_workspace(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
    ) -> Result<Option<String>, AppError> {
        let _operation_fence = self
            .session_operation_lock(session_id.as_ref())
            .try_read_owned()
            .map_err(|_| {
                AppError::Conflict(
                    "AgentSession workspace is unavailable during a concurrent lifecycle mutation"
                        .to_owned(),
                )
            })?;
        let observed = self
            .canonical
            .get(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner_id.to_owned(),
                },
                session_id,
            )
            .await?;
        self.materialize_workspace_for_binding(
            owner_id,
            session_id,
            &observed.session.agent_binding,
        )
        .await
    }

    async fn remove_workspace_for_binding(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        binding: &AgentBindingValue,
    ) -> Result<(), AppError> {
        // Validate the complete frozen authority even though only a managed
        // Session directory may be reclaimed. A selected/custom path is never
        // removed by Session lifecycle cleanup.
        let frozen_workspace = frozen_workspace_root(
            &self.managed_workspace_root,
            owner_id,
            session_id,
            binding,
        )?;
        if uses_managed_session_workspace(binding) || frozen_workspace.is_none() {
            remove_managed_session_workspace(&self.managed_workspace_root, session_id).await?;
        }
        Ok(())
    }

    pub(crate) fn install_official_runtime(&self, host: Arc<super::official_runtime::OfficialRuntimeHost>, control_plane: std::sync::Weak<AgentControlPlane>) -> Result<(), AppError> {
        self.runtime_control_plane.set(control_plane).map_err(|_| AppError::Conflict("Session control plane already installed".into()))?;
        self.official_runtime.set(host).map_err(|_| AppError::Conflict("Session runtime host already installed".into()))
    }

    pub(crate) fn install_product_agent_resolver(
        &self,
        resolver: std::sync::Weak<NomiCoreProductAgentResolver>,
    ) -> Result<(), AppError> {
        self.product_agent_resolver
            .set(resolver)
            .map_err(|_| AppError::Conflict("Session product Agent resolver already installed".into()))
    }

    pub(crate) fn install_idmm(
        &self,
        service: std::sync::Weak<nomifun_idmm::IdmmService>,
    ) -> Result<(), AppError> {
        self.idmm
            .set(service)
            .map_err(|_| AppError::Conflict("IDMM supervisor already installed".into()))
    }

    async fn remove_idmm_state(&self, session_id: &str) -> Result<(), AppError> {
        if let Some(service) = self.idmm.get().and_then(std::sync::Weak::upgrade) {
            service.remove(session_id).await?;
        }
        Ok(())
    }

    async fn initialize_idmm_state(
        &self,
        session_id: &str,
        config: nomifun_api_types::IdmmConfig,
    ) -> Result<(), AppError> {
        if config.mode == nomifun_api_types::IdmmMode::Off {
            return Ok(());
        }
        let service = self
            .idmm
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                AppError::Conflict(
                    "Agent IDMM policy is enabled but the IDMM supervisor is unavailable".into(),
                )
            })?;
        service.initialize_config(session_id, config).await?;
        Ok(())
    }

    async fn validate_idmm_state(
        &self,
        config: &nomifun_api_types::IdmmConfig,
    ) -> Result<(), AppError> {
        if config.mode == nomifun_api_types::IdmmMode::Off {
            return Ok(());
        }
        let service = self
            .idmm
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                AppError::Conflict(
                    "Agent IDMM policy is enabled but the IDMM supervisor is unavailable".into(),
                )
            })?;
        service.validate_configuration(config).await
    }

    async fn inherit_idmm_state(
        &self,
        parent_session_id: &str,
        child_session_id: &str,
    ) -> Result<(), AppError> {
        let Some(service) = self.idmm.get().and_then(std::sync::Weak::upgrade) else {
            return Ok(());
        };
        let parent = service.state(parent_session_id).await?;
        service
            .initialize_config(child_session_id, parent.config)
            .await?;
        Ok(())
    }

    /// Create one canonical Session for every product/automation consumer.
    /// The supplied request remains a presentation/runtime projection only;
    /// identity, immutable binding, resources and replay live in AgentStore.
    pub(crate) async fn create_session_idempotent(
        &self,
        owner_id: &str,
        mut request: CreateConversationRequest,
        snapshot: Option<AgentResolvedSnapshot>,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        nomifun_common::UserId::parse(owner_id.to_owned()).map_err(|error| {
            AppError::Forbidden(format!("Invalid canonical Agent owner: {error}"))
        })?;
        let mut snapshot = snapshot;
        if let Some(target) = product_agent_target_from_request(&request.extra) {
            let owner = UserId::from(owner_id.to_owned());
            let control_plane = self
                .runtime_control_plane
                .get()
                .and_then(std::sync::Weak::upgrade)
                .ok_or_else(|| AppError::Conflict(
                    "Session control plane is unavailable".to_owned(),
                ))?;
            let existing = control_plane
                .get_agent_binding(
                    &owner,
                    target.target_kind.clone(),
                    target.target_id.clone(),
                )
                .await
                .map_err(control_plane_error_to_app)?;
            if let Some(existing) = existing {
                let (binding, revision, resolved) = control_plane
                    .saved_binding_artifacts(&owner, &existing.agent_binding)
                    .await
                    .map_err(control_plane_error_to_app)?;
                let editor = control_plane
                    .editor(
                        &owner,
                        binding.preset_revision_ref.preset_id.as_ref(),
                        Some(binding.preset_revision_ref.revision),
                    )
                    .await
                    .map_err(control_plane_error_to_app)?;
                let common_owner = nomifun_common::UserId::parse(owner_id.to_owned())
                    .map_err(|error| AppError::Forbidden(error.to_string()))?;
                let projected = super::agent_binding_projection::project_saved_artifacts(
                    &common_owner,
                    binding,
                    revision,
                    resolved,
                    Some(&editor.preset.display_name),
                )?;
                attach_session_metadata(
                    &mut request.extra,
                    &projected.binding,
                    None,
                )
                .map_err(|error| AppError::Conflict(error.message))?;
                merge_product_agent_resolution(
                    &mut request.extra,
                    &target,
                    &ProductAgentResolution {
                        snapshot: projected.projection.snapshot.clone(),
                        runtime_extra: projected.projection.request.extra,
                    },
                )?;
                snapshot = Some(projected.projection.snapshot);
            }
        }
        if request.extra.get(NOMI_CORE_SESSION_METADATA_KEY).is_none() {
            if let Some(binding) = snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.canonical_binding.as_ref())
            {
                let binding: AgentBindingValue = serde_json::to_value(binding)
                    .and_then(serde_json::from_value)
                    .map_err(|error| AppError::Conflict(format!(
                        "Invalid consumer Agent binding: {error}"
                    )))?;
                attach_session_metadata(&mut request.extra, &binding, None)
                    .map_err(|error| AppError::Conflict(error.message))?;
            } else {
                let target = product_agent_target_from_request(&request.extra).ok_or_else(|| {
                    AppError::Conflict(
                        "AgentSession creation requires an immutable Agent binding"
                        .to_owned(),
                    )
                })?;
                let owner = UserId::from(owner_id.to_owned());
                let control_plane = self
                    .runtime_control_plane
                    .get()
                    .and_then(std::sync::Weak::upgrade)
                    .ok_or_else(|| AppError::Conflict(
                        "Session control plane is unavailable".to_owned(),
                    ))?;
                let resolution = if let Some(existing) = control_plane
                    .get_agent_binding(
                        &owner,
                        target.target_kind.clone(),
                        target.target_id.clone(),
                    )
                    .await
                    .map_err(control_plane_error_to_app)?
                {
                    let (binding, revision, resolved) = control_plane
                        .saved_binding_artifacts(&owner, &existing.agent_binding)
                        .await
                        .map_err(control_plane_error_to_app)?;
                    let editor = control_plane
                        .editor(
                            &owner,
                            binding.preset_revision_ref.preset_id.as_ref(),
                            Some(binding.preset_revision_ref.revision),
                        )
                        .await
                        .map_err(control_plane_error_to_app)?;
                    let common_owner = nomifun_common::UserId::parse(owner_id.to_owned())
                        .map_err(|error| AppError::Forbidden(error.to_string()))?;
                    let mut projected = super::agent_binding_projection::project_saved_artifacts(
                        &common_owner,
                        binding,
                        revision,
                        resolved,
                        Some(&editor.preset.display_name),
                    )?;
                    attach_session_metadata(
                        &mut projected.projection.request.extra,
                        &projected.binding,
                        None,
                    )
                    .map_err(|error| AppError::Conflict(error.message))?;
                    ProductAgentResolution {
                        snapshot: projected.projection.snapshot,
                        runtime_extra: projected.projection.request.extra,
                    }
                } else {
                    let resolver = self
                        .product_agent_resolver
                        .get()
                        .and_then(std::sync::Weak::upgrade)
                        .ok_or_else(|| AppError::Conflict(
                            "Session product Agent resolver is unavailable".to_owned(),
                        ))?;
                    resolver
                        .resolve(owner_id, &target, request.model.as_ref())
                        .await?
                };
                merge_product_agent_resolution(&mut request.extra, &target, &resolution)?;
                snapshot = Some(resolution.snapshot);
            }
        }
        nomifun_api_types::ExecutionConstraints::from_extra(&request.extra)?;
        let mut metadata: NomiCoreSessionMetadata = serde_json::from_value(
            request
                .extra
                .get(NOMI_CORE_SESSION_METADATA_KEY)
                .cloned()
                .ok_or_else(|| AppError::Conflict(
                    "canonical AgentSession metadata is missing".to_owned(),
                ))?,
        )
        .map_err(|error| AppError::Conflict(format!(
            "canonical AgentSession metadata is invalid: {error}"
        )))?;
        if let Some(workspace) = request
            .extra
            .get("workspace")
            .and_then(Value::as_str)
            .filter(|workspace| !workspace.is_empty())
            .map(str::to_owned)
        {
            let mut binding = agent_binding_dto(&metadata.binding)
                .map_err(|error| AppError::Conflict(error.message))?;
            let canonical_workspace = freeze_selected_workspace(
                &mut binding,
                owner_id,
                &workspace,
                WorkspaceDirectoryCheck::Create,
            )?;
            metadata.binding = serde_json::to_value(&binding)
                .and_then(serde_json::from_value)
                .map_err(|error| {
                    AppError::Conflict(format!(
                        "selected workspace binding could not be frozen: {error}"
                    ))
                })?;
            attach_session_metadata_with_fork(
                &mut request.extra,
                &metadata.binding,
                metadata.remote.clone(),
                metadata.parent_session_id.clone(),
                metadata.fork_base_payload_id.clone(),
            )
            .map_err(|error| AppError::Conflict(error.message))?;
            request.extra["workspace"] = Value::String(canonical_workspace);
            if let Some(snapshot) = snapshot.as_mut() {
                snapshot.canonical_binding = Some(binding);
            }
        }
        if metadata
            .binding
            .typed_resource_bindings
            .iter()
            .any(|resource| resource.owner_id.as_str() != owner_id)
        {
            return Err(AppError::Forbidden(
                "AgentSession resource binding belongs to another owner".to_owned(),
            ));
        }
        let binding_dto = agent_binding_dto(&metadata.binding)
            .map_err(|error| AppError::Conflict(error.message))?;
        let control_plane = self
            .runtime_control_plane
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Session control plane is unavailable".into()))?;
        let (saved_binding, revision, resolved) = control_plane
            .saved_binding_artifacts(
                &UserId::from(owner_id.to_owned()),
                &binding_dto,
            )
            .await
            .map_err(control_plane_error_to_app)?;
        if saved_binding != metadata.binding {
            return Err(AppError::Conflict(
                "AgentSession binding differs from its saved immutable artifacts".to_owned(),
            ));
        }
        let common_owner = nomifun_common::UserId::parse(owner_id.to_owned())
            .map_err(|error| AppError::Forbidden(error.to_string()))?;
        let idmm_config = idmm_config_from_runtime_policy(&revision.payload.runtime_policy)?;
        let projected = super::agent_binding_projection::project_saved_artifacts(
            &common_owner,
            saved_binding.clone(),
            revision,
            resolved.clone(),
            request.name.as_deref(),
        )?;
        if let Some(snapshot) = snapshot.as_ref() {
            let mut normalized = snapshot.clone();
            // Product owners may supply their user-facing thread title as the
            // projected preset name. It is presentation only; all immutable
            // authority fields must still match the saved artifacts exactly.
            normalized.preset_name = projected.projection.snapshot.preset_name.clone();
            if normalized != projected.projection.snapshot {
                return Err(AppError::Conflict(
                    "consumer Agent snapshot differs from its saved immutable artifacts"
                        .to_owned(),
                ));
            }
        }
        let host = self.official_runtime.get().ok_or_else(|| {
            AppError::Conflict("Canonical Agent consumer requires the Runtime host".into())
        })?;
        host.validate_agent(&resolved)?;
        host.provider()?.validate_session_extra(&request.extra)?;
        super::nomi_core_mcp_catalog::validate_product_session_selection(
            &resolved,
            &saved_binding.typed_resource_bindings,
            &request.extra,
        )?;
        let active_capabilities = resolved
            .content
            .enabled_capabilities
            .iter()
            .filter(|capability| capability.consumption.is_contribution())
            .map(|capability| capability.capability.id.as_ref().to_owned())
            .collect();
        self.validate_idmm_state(&idmm_config).await?;
        let opened = self
            .canonical
            .open_with_provenance(
                PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner_id.to_owned(),
                },
                saved_binding,
                request.name.or_else(|| Some(projected.projection.snapshot.preset_name)),
                active_capabilities,
                metadata.remote,
                creation_key,
                now_ms(),
            )
            .await?;
        self.materialize_workspace_for_binding(
            owner_id,
            &opened.session.agent_session_id,
            &opened.session.agent_binding,
        )
        .await?;
        self.initialize_idmm_state(
            opened.session.agent_session_id.as_ref(),
            idmm_config,
        )
        .await?;
        self.canonical_conversation_projection(owner_id, &opened.session.agent_session_id)
            .await?
            .ok_or_else(|| AppError::Internal(
                "created AgentSession has no canonical projection".to_owned(),
            ))
    }

    pub(crate) async fn get_session(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<ConversationResponse, AppError> {
        let session_id = AgentSessionId::from(session_id.to_owned());
        self.canonical_conversation_projection(owner_id, &session_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!(
                "AgentSession {} not found",
                session_id.as_ref(),
            )))
    }

    /// Turn Cron's explicit provider/model-only selection into the same saved
    /// immutable binding used by an ordinary minimal AgentSession. This is a
    /// host application-service operation: Cron never reads Preset/compiler
    /// storage and the Session owner never accepts provider/model as identity.
    async fn materialize_cron_model_only_snapshot(
        &self,
        owner_id: &str,
        model: Option<&nomifun_common::ProviderWithModel>,
    ) -> Result<AgentResolvedSnapshot, AppError> {
        let model = model.ok_or_else(|| {
            AppError::Conflict(
                "model-only Cron AgentSession requires an exact provider/model".to_owned(),
            )
        })?;
        let owner = UserId::from(owner_id.to_owned());
        let resolver = self
            .product_agent_resolver
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                AppError::Conflict("Session product Agent resolver is unavailable".to_owned())
            })?;
        let selected_model = AgentChatModelSelectionDto {
            provider_id: model.provider_id.clone(),
            model: model.model.clone(),
        };
        let binding = resolver
            .materialize(
                &owner,
                &ProductAgentSelection::Template {
                    template_key: "chat.minimal".to_owned(),
                },
                Some(&selected_model),
            )
            .await?;
        let control_plane = self
            .runtime_control_plane
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Session control plane is unavailable".to_owned()))?;
        let (binding, revision, snapshot) = control_plane
            .saved_binding_artifacts(&owner, &binding)
            .await
            .map_err(control_plane_error_to_app)?;
        let editor = control_plane
            .editor(
                &owner,
                binding.preset_revision_ref.preset_id.as_ref(),
                Some(binding.preset_revision_ref.revision),
            )
            .await
            .map_err(control_plane_error_to_app)?;
        let common_owner = nomifun_common::UserId::parse(owner_id.to_owned())
            .map_err(|error| AppError::Forbidden(error.to_string()))?;
        super::agent_binding_projection::project_saved_artifacts(
            &common_owner,
            binding,
            revision,
            snapshot,
            Some(&editor.preset.display_name),
        )
        .map(|projected| projected.projection.snapshot)
    }

    /// Cron historically supplies no project for a model-only task. Keep that
    /// valid without manufacturing workspace Actions: the path below is only
    /// a runtime/presentation cwd fallback and is never added to the immutable
    /// Agent resource binding. Sessions with an explicitly frozen workspace
    /// continue to use that authoritative resource instead.
    async fn ensure_cron_workspace_projection(
        &self,
        mut response: ConversationResponse,
    ) -> Result<ConversationResponse, AppError> {
        let has_workspace = response
            .extra
            .get("workspace")
            .and_then(Value::as_str)
            .is_some_and(|workspace| !workspace.trim().is_empty());
        if has_workspace {
            return Ok(response);
        }
        let session_id = AgentSessionId::from(response.conversation_id.clone());
        let fallback = materialize_managed_session_workspace(
            &self.managed_workspace_root,
            &session_id,
        )
        .await?;
        let extra = response.extra.as_object_mut().ok_or_else(|| {
            AppError::Conflict("canonical AgentSession extra must be an object".to_owned())
        })?;
        extra.insert(
            "workspace".to_owned(),
            Value::String(fallback),
        );
        extra.insert("custom_workspace".to_owned(), Value::Bool(false));
        extra.insert("is_temporary_workspace".to_owned(), Value::Bool(true));
        extra.insert(
            "temp_workspace_id".to_owned(),
            Value::String(session_id.as_ref().to_owned()),
        );
        Ok(response)
    }

    fn turn_operation_id(
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
    ) -> OperationId {
        OperationId::from(format!(
            "turn:user:{owner_id}:{session_id}:{idempotency_key}"
        ))
    }

    async fn canonical_turn_input_with_admission(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        request: &SendMessageRequest,
    ) -> Result<Value, AppError> {
        let session = self
            .canonical
            .get(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner_id.to_owned(),
                },
                session_id,
            )
            .await?;
        let binding = agent_binding_dto(&session.session.agent_binding)
            .map_err(|error| AppError::Conflict(error.message))?;
        let control_plane = self
            .runtime_control_plane
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Session control plane is unavailable".into()))?;
        let (saved_binding, _, snapshot) = control_plane
            .saved_binding_artifacts(&UserId::from(owner_id.to_owned()), &binding)
            .await
            .map_err(control_plane_error_to_app)?;
        if saved_binding != session.session.agent_binding {
            return Err(AppError::Conflict(
                "AgentSession binding differs from its saved immutable artifacts".to_owned(),
            ));
        }
        self.materialize_workspace_for_binding(owner_id, session_id, &saved_binding)
            .await?;
        let route_identity = snapshot.content.chat_route_identity.clone().ok_or_else(|| {
            AppError::UnprocessableEntity(
                "AgentSession Snapshot has no exact Chat route".to_owned(),
            )
        })?;
        let mut input = canonical_turn_input(request);
        input["admission"] = json!({
            "route_identity": route_identity,
            "resolved_snapshot_ref": snapshot.snapshot_ref,
        });
        Ok(input)
    }

    async fn canonical_delivery_state_for_operation(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        operation_id: &OperationId,
        replayed: bool,
    ) -> Result<PublicTurnDeliveryState, AppError> {
        self.canonical
            .get(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner_id.to_owned(),
                },
                session_id,
            )
            .await?;
        let receipt = self
            .canonical
            .store()
            .read_turn_receipt(session_id, operation_id)
            .await
            .map_err(agent_session_store_error)?;
        if receipt.status == nomifun_agent_session::TurnReceiptStatus::NotFound {
            return Ok(PublicTurnDeliveryState::Missing);
        }
        let started = receipt.started_event.as_ref().ok_or_else(|| {
            AppError::Conflict("canonical Turn receipt has no start fact".to_owned())
        })?;
        let source_message_id = match &started.payload {
            nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) => payload
                .0
                .get("source_message_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        }
        .ok_or_else(|| {
            AppError::Conflict("canonical Turn receipt has no source message".to_owned())
        })?;
        if receipt.status == nomifun_agent_session::TurnReceiptStatus::Running {
            return Ok(PublicTurnDeliveryState::Accepted {
                message_id: source_message_id,
            });
        }
        let terminal_payload = receipt
            .terminal_event
            .as_ref()
            .and_then(|event| match &event.payload {
                nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) => {
                    Some(&payload.0)
                }
                _ => None,
            });
        let projections = self
            .canonical
            .store()
            .messages_after(session_id, 0)
            .await
            .map_err(agent_session_store_error)?;
        let result_text = projections
            .iter()
            .filter(|message| {
                message.presentation_intent == "message"
                    && message.first_seq > started.seq
                    && message
                        .projection
                        .get("correlation_id")
                        .and_then(Value::as_str)
                        != Some(source_message_id.as_str())
            })
            .max_by_key(|message| message.last_seq)
            .and_then(|message| message.projection.get("content"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (result_ok, result_error, result_error_code, retryable) = match receipt.status {
            nomifun_agent_session::TurnReceiptStatus::Completed => {
                (Some(true), None, None, Some(false))
            }
            nomifun_agent_session::TurnReceiptStatus::Failed => (
                Some(false),
                terminal_payload
                    .and_then(|payload| payload.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                Some("turn_failed".to_owned()),
                Some(true),
            ),
            nomifun_agent_session::TurnReceiptStatus::Cancelled => (
                Some(false),
                Some("Turn cancelled".to_owned()),
                Some("cancelled".to_owned()),
                Some(false),
            ),
            nomifun_agent_session::TurnReceiptStatus::NotFound
            | nomifun_agent_session::TurnReceiptStatus::Running => unreachable!(),
        };
        Ok(PublicTurnDeliveryState::Completed(IdempotentMessageDelivery {
            message_id: source_message_id,
            replayed,
            completed: true,
            result_ok,
            result_text,
            result_error,
            result_error_code,
            result_error_retryable: retryable,
        }))
    }

    async fn settle_dispatch_failure(
        &self,
        session_id: &AgentSessionId,
        operation_id: &OperationId,
        message: &str,
    ) -> Result<(), AppError> {
        let receipt = self
            .canonical
            .store()
            .read_turn_receipt(session_id, operation_id)
            .await
            .map_err(agent_session_store_error)?;
        if receipt.status != nomifun_agent_session::TurnReceiptStatus::Running {
            return Ok(());
        }
        let started = receipt.started_event.ok_or_else(|| {
            AppError::Conflict("failed Turn has no start fact".to_owned())
        })?;
        let identity = format!(
            "turn-dispatch-failed:{}:{}",
            session_id.as_ref(),
            operation_id.as_ref(),
        );
        let error = AgentSendError::from_app_error(AppError::Conflict(message.to_owned()))
            .into_stream_error();
        self.canonical
            .store()
            .append_turn_terminal(
                &nomifun_agent_contracts::SessionEventAppend {
                    agent_session_id: session_id.clone(),
                    event_id: nomifun_agent_contracts::EventId::from(identity.clone()),
                    producer_id: nomifun_agent_contracts::EventProducerId::from(
                        "runtime_supervisor",
                    ),
                    idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(identity),
                    runtime_binding_id: None,
                    runtime_producer_seq: None,
                    semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                        kind: nomifun_agent_contracts::SessionEventKind(
                            "turn/failed".to_owned(),
                        ),
                        kind_version: 1,
                        correlation_id: nomifun_agent_contracts::CorrelationId::from(
                            operation_id.as_ref().to_owned(),
                        ),
                        causation_event_id: Some(started.event_id),
                        payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                            StrictJsonValue(json!({
                                "message": message,
                                "code": "runtime_dispatch_failed",
                                "error": error,
                                "finished_at_ms": now_ms(),
                            })),
                        ),
                    },
                },
                operation_id,
            )
            .await
            .map_err(agent_session_store_error)?;
        Ok(())
    }

    /// A process restart cannot retain an in-memory Runtime owner. Reconcile
    /// every durable running Turn before publishing routes so the Session is
    /// immediately usable again instead of remaining permanently busy.
    pub(crate) async fn reconcile_orphaned_active_turns(&self) -> Result<usize, AppError> {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT head.session_id, head.active_turn_id \
             FROM agent_session_heads head \
             JOIN agent_sessions session ON session.agent_session_id = head.session_id \
             WHERE session.state = 'live' AND head.status = 'running' \
               AND head.active_turn_id IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        let mut reconciled = 0;
        for (session_id, operation_id) in rows {
            if self.runtime_sessions.get_runtime(&session_id).is_some() {
                continue;
            }
            self.settle_dispatch_failure(
                &AgentSessionId::from(session_id),
                &OperationId::from(operation_id),
                "Runtime owner was not recoverable after restart",
            )
            .await?;
            reconciled += 1;
        }
        Ok(reconciled)
    }

    /// Allocate one renderer-stream segment for the assistant side of an exact
    /// canonical Turn. The accepted user message remains the Turn root; sharing
    /// its ID with assistant output causes the renderer to merge both text rows.
    fn canonical_assistant_stream_message_id(
        root_message_id: &str,
    ) -> Result<String, AppError> {
        super::engine_journal::canonical_assistant_message_id(root_message_id)
    }

    fn canonical_stream_wire_event(
        session_id: &AgentSessionId,
        root_message_id: &str,
        assistant_message_id: &str,
        event: &AgentStreamEvent,
    ) -> Option<WebSocketMessage<Value>> {
        let mut event_data = serde_json::to_value(event).ok()?;
        normalize_keys_to_snake_case(&mut event_data);
        Some(WebSocketMessage::new(
            "message.stream",
            json!({
                "conversation_id": session_id,
                "msg_id": assistant_message_id,
                "turn_id": root_message_id,
                "type": event_data.get("type").cloned().unwrap_or(json!("unknown")),
                "data": event_data.get("data").cloned().unwrap_or_else(|| json!({})),
                "hidden": false,
            }),
        ))
    }

    /// Realtime delivery for an already-finalized assistant projection (for
    /// example the terminal AgentExecution synthesis). It is intentionally not
    /// attached to a model turn and has no later finish frame; `stream_complete`
    /// tells renderers to append it without raising busy/processing state.
    fn canonical_projected_assistant_wire_event(
        session_id: &AgentSessionId,
        message_id: &str,
        content: &str,
    ) -> WebSocketMessage<Value> {
        WebSocketMessage::new(
            "message.stream",
            json!({
                "conversation_id": session_id,
                "msg_id": message_id,
                "type": "content",
                "data": { "content": content },
                "hidden": false,
                "stream_complete": true,
                "created_at": now_ms(),
            }),
        )
    }

    fn canonical_turn_completed_wire_event(
        session_id: &AgentSessionId,
        root_message_id: &str,
        terminal: &AgentStreamEvent,
    ) -> WebSocketMessage<Value> {
        let (state, detail) = match terminal {
            AgentStreamEvent::Error(error) => ("error", error.message.as_str()),
            _ => ("ai_waiting_input", ""),
        };
        WebSocketMessage::new(
            "turn.completed",
            json!({
                "conversation_id": session_id,
                "turn_id": root_message_id,
                "status": "finished",
                "state": state,
                "detail": detail,
                "can_send_message": true,
                "runtime": {
                    "state": "idle",
                    "can_send_message": true,
                    "has_runtime": false,
                    "runtime_status": "finished",
                    "is_processing": false,
                    "active_turn_id": null,
                },
            }),
        )
    }

    fn canonical_turn_dispatch_failed_wire_event(
        session_id: &AgentSessionId,
        root_message_id: &str,
        detail: &str,
    ) -> WebSocketMessage<Value> {
        WebSocketMessage::new(
            "turn.completed",
            json!({
                "conversation_id": session_id,
                "turn_id": root_message_id,
                "status": "finished",
                "state": "error",
                "detail": detail,
                "can_send_message": true,
                "runtime": {
                    "state": "idle",
                    "can_send_message": true,
                    "has_runtime": false,
                    "runtime_status": "finished",
                    "is_processing": false,
                    "active_turn_id": null,
                },
            }),
        )
    }

    fn canonical_turn_started_wire_event(
        session_id: &AgentSessionId,
        root_message_id: &str,
    ) -> WebSocketMessage<Value> {
        WebSocketMessage::new(
            "turn.started",
            json!({
                "conversation_id": session_id,
                "turn_id": root_message_id,
                "status": "running",
                "phase": "starting",
                "state": "ai_generating",
                "detail": "",
                "can_send_message": false,
                "runtime": {
                    "state": "running",
                    "can_send_message": false,
                    "has_runtime": true,
                    "runtime_status": "running",
                    "is_processing": true,
                    "active_turn_id": root_message_id,
                    "processing_started_at": now_ms(),
                },
            }),
        )
    }

    fn spawn_canonical_stream_relay(
        &self,
        owner_id: String,
        session_id: AgentSessionId,
        root_message_id: String,
        assistant_message_id: String,
        turn_generation: u64,
        cancellation: tokio_util::sync::CancellationToken,
        mut events: broadcast::Receiver<AgentStreamEvent>,
    ) {
        let sink = self.user_events.clone();
        let runtimes = self.runtime_sessions.clone();
        let relay_session_id = session_id.clone();
        let task = async move {
            loop {
                let event = tokio::select! {
                    _ = cancellation.cancelled() => break,
                    received = events.recv() => match received {
                        Ok(event) => event,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(
                                agent_session_id = session_id.as_ref(),
                                skipped,
                                "canonical stream relay lagged; continuing toward the terminal frame"
                            );
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                };
                let terminal = matches!(
                    event,
                    AgentStreamEvent::Finish(_) | AgentStreamEvent::Error(_)
                );
                if let Some(message) = Self::canonical_stream_wire_event(
                    &session_id,
                    &root_message_id,
                    &assistant_message_id,
                    &event,
                ) {
                    sink.send_to_user(&owner_id, message);
                }
                if terminal {
                    sink.send_to_user(
                        &owner_id,
                        Self::canonical_turn_completed_wire_event(
                            &session_id,
                            &root_message_id,
                            &event,
                        ),
                    );
                    break;
                }
            }
            if let Err(error) = runtimes
                .release_runtime_turn(session_id.as_ref(), turn_generation)
                .await
            {
                tracing::warn!(
                    agent_session_id = session_id.as_ref(),
                    %error,
                    "canonical Runtime turn release failed"
                );
            }
        };
        if !self.background_tasks.spawn(Box::pin(task)) {
            tracing::warn!(
                agent_session_id = relay_session_id.as_ref(),
                "canonical stream relay rejected during shutdown"
            );
        }
    }

    async fn dispatch_canonical_turn(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        request: SendMessageRequest,
        initial_only: bool,
    ) -> Result<IdempotentMessageDelivery, AppError> {
        let _operation_fence = self
            .session_operation_lock(session_id.as_ref())
            .read_owned()
            .await;
        let input = self
            .canonical_turn_input_with_admission(owner_id, session_id, &request)
            .await?;
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: owner_id.to_owned(),
        };
        let receipt = if initial_only {
            self.canonical
                .start_initial_turn(&principal, session_id, idempotency_key, input)
                .await?
        } else {
            self.canonical
                .start_turn(&principal, session_id, idempotency_key, input)
                .await?
        };
        let operation_id = receipt.operation_id.clone();
        let state = self
            .canonical_delivery_state_for_operation(
                owner_id,
                session_id,
                &operation_id,
                receipt.duplicate,
            )
            .await?;
        if receipt.duplicate {
            return match state {
                PublicTurnDeliveryState::Accepted { message_id } => Ok(IdempotentMessageDelivery {
                    message_id,
                    replayed: true,
                    completed: false,
                    result_ok: None,
                    result_text: None,
                    result_error: None,
                    result_error_code: None,
                    result_error_retryable: None,
                }),
                PublicTurnDeliveryState::Completed(delivery) => Ok(delivery),
                PublicTurnDeliveryState::Missing => Err(AppError::Conflict(
                    "canonical turn replay lost its durable receipt".to_owned(),
                )),
            };
        }
        let root_message_id = match state {
            PublicTurnDeliveryState::Accepted { message_id } => message_id,
            _ => {
                return Err(AppError::Conflict(
                    "new canonical turn did not remain accepted".to_owned(),
                ));
            }
        };
        let mut projection = self
            .canonical_conversation_projection(owner_id, session_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!(
                "AgentSession {} not found",
                session_id.as_ref(),
            )))?;
        if projection
            .extra
            .get("workspace")
            .and_then(Value::as_str)
            .is_none_or(|workspace| workspace.trim().is_empty())
        {
            let fallback = materialize_managed_session_workspace(
                &self.managed_workspace_root,
                session_id,
            )
            .await?;
            projection.extra["workspace"] = Value::String(fallback);
            projection.extra["custom_workspace"] = Value::Bool(false);
            projection.extra["is_temporary_workspace"] = Value::Bool(true);
            projection.extra["temp_workspace_id"] =
                Value::String(session_id.as_ref().to_owned());
        }
        let (options, _) = runtime_options_from_session(owner_id, projection, None)?;
        let cancellation = tokio_util::sync::CancellationToken::new();
        let relay_cancellation = cancellation.clone();
        let generation = receipt.cursor.seq;
        let runtime = match self
            .runtime_sessions
            .get_or_create_runtime_for_turn(
                session_id.as_ref(),
                generation,
                cancellation,
                options,
            )
            .await
        {
            Ok(runtime) => runtime,
            Err(error) => {
                self.settle_dispatch_failure(session_id, &operation_id, &error.to_string())
                    .await?;
                return Err(error);
            }
        };
        let events = runtime.subscribe();
        let assistant_message_id =
            Self::canonical_assistant_stream_message_id(&root_message_id)?;
        self.user_events.send_to_user(
            owner_id,
            Self::canonical_turn_started_wire_event(session_id, &root_message_id),
        );
        self.spawn_canonical_stream_relay(
            owner_id.to_owned(),
            session_id.clone(),
            root_message_id.clone(),
            assistant_message_id,
            generation,
            relay_cancellation.clone(),
            events,
        );
        let delivery = SendMessageData {
            content: request.content,
            msg_id: root_message_id.clone(),
            source_message_id: Some(root_message_id.clone()),
            files: request.files,
            inject_skills: request.inject_skills,
            origin: request.origin,
        };
        if let Err(error) = runtime.send_message(delivery).await {
            let detail = error.to_string();
            // Stop the already-subscribed relay before the reusable Runtime can
            // publish a successor Turn; otherwise a rejected dispatch may
            // misattribute that successor's frames to this failed root.
            relay_cancellation.cancel();
            self.settle_dispatch_failure(session_id, &operation_id, &detail)
                    .await?;
            self.user_events.send_to_user(
                owner_id,
                Self::canonical_turn_dispatch_failed_wire_event(
                    session_id,
                    &root_message_id,
                    &detail,
                ),
            );
            if let Err(release_error) = self
                .runtime_sessions
                .release_runtime_turn(session_id.as_ref(), generation)
                .await
            {
                tracing::warn!(
                    agent_session_id = session_id.as_ref(),
                    %release_error,
                    "failed to release rejected canonical Runtime turn"
                );
            }
            return Err(AppError::BadGateway(detail));
        }
        Ok(IdempotentMessageDelivery {
            message_id: root_message_id,
            replayed: false,
            completed: false,
            result_ok: None,
            result_text: None,
            result_error: None,
            result_error_code: None,
            result_error_retryable: None,
        })
    }

    async fn validate_agent_execution_turn_authority(
        &self,
        owner_id: &str,
        session_id: &str,
        authority: &AgentExecutionTurnAuthority,
    ) -> Result<(), AppError> {
        if authority.lease_owner.trim().is_empty()
            || authority.expected_step_version < 0
            || authority.expected_attempt_version < 0
        {
            return Err(AppError::Conflict(
                "invalid AgentExecution turn authority".to_owned(),
            ));
        }
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let now = now_ms();
        let execution = sqlx::query(
            "UPDATE agent_executions SET lease_owner = lease_owner \
             WHERE execution_id = ? AND user_id = ? AND lease_owner = ? \
               AND lease_expires_at > ? AND deleted_at IS NULL \
               AND status IN ('running', 'waiting_input')",
        )
        .bind(&authority.execution_id)
        .bind(owner_id)
        .bind(&authority.lease_owner)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        if execution.rows_affected() != 1 {
            return Err(AppError::Conflict(
                "AgentExecution lease generation is no longer authoritative".to_owned(),
            ));
        }
        let exact: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) \
             FROM agent_execution_steps step \
             JOIN agent_execution_attempts attempt \
               ON attempt.execution_id = step.execution_id AND attempt.step_id = step.step_id \
             JOIN conversation_execution_links link \
               ON link.execution_id = attempt.execution_id \
              AND link.step_id = attempt.step_id AND link.attempt_id = attempt.attempt_id \
             JOIN agent_sessions session ON session.agent_session_id = link.conversation_id \
             WHERE step.execution_id = ? AND step.step_id = ? \
               AND step.version = ? AND step.status = 'running' \
               AND step.superseded_in_revision IS NULL \
               AND attempt.attempt_id = ? AND attempt.version = ? \
               AND attempt.status = 'running' \
               AND link.conversation_id = ? \
               AND link.relation IN ('attempt', 'automation') AND link.active = 1 \
               AND session.state = 'live' \
               AND json_extract(session.owner_ref_json, '$.principal_kind') = 'user' \
               AND json_extract(session.owner_ref_json, '$.principal_id') = ?",
        )
        .bind(&authority.execution_id)
        .bind(&authority.step_id)
        .bind(authority.expected_step_version)
        .bind(&authority.attempt_id)
        .bind(authority.expected_attempt_version)
        .bind(session_id)
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        if exact != 1 {
            return Err(AppError::Conflict(
                "AgentExecution no longer owns the exact running Attempt Session"
                    .to_owned(),
            ));
        }
        tx.commit()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        Ok(())
    }

    /// Materialize the retiring Conversation-shaped consumer projection from
    /// a Store-only canonical AgentSession. `None` means there is no canonical
    /// row and permits an explicit legacy fallback; every other canonical
    /// state (foreign owner, deleting/tombstoned row, invalid saved artifacts)
    /// fails closed.
    pub(crate) async fn canonical_conversation_projection(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
    ) -> Result<Option<ConversationResponse>, AppError> {
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: owner_id.to_owned(),
        };
        let mut observed = match self.canonical.get(&principal, session_id).await {
            Ok(observed) => observed,
            Err(AppError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        if let Some(operation) = observed.head.active_turn_id.clone()
            && !observed.events.iter().any(|event| {
                event.kind.0 == "turn/started"
                    && event.correlation_id.as_ref() == operation
            })
            && let Some(started) = self
                .canonical
                .store()
                .read_turn_receipt(session_id, &OperationId::from(operation))
                .await
                .map_err(agent_session_store_error)?
                .started_event
        {
            observed.events.push(started);
        }
        let control_plane = self
            .runtime_control_plane
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                AppError::Conflict(
                    "canonical AgentSession projection requires the assembled control plane"
                        .to_owned(),
                )
            })?;
        let binding_dto: AgentBindingValueDto =
            serde_json::to_value(&observed.session.agent_binding)
                .and_then(serde_json::from_value)
                .map_err(|error| {
                    AppError::Conflict(format!(
                        "canonical AgentSession has an invalid frozen binding: {error}"
                    ))
                })?;
        let owner = UserId::from(owner_id.to_owned());
        let (binding, revision, snapshot) = control_plane
            .saved_binding_artifacts(&owner, &binding_dto)
            .await
            .map_err(control_plane_error_to_app)?;
        if binding != observed.session.agent_binding {
            return Err(AppError::Conflict(
                "canonical AgentSession binding differs from its exact saved artifacts"
                    .to_owned(),
            ));
        }
        let official_template = control_plane
            .internal_official_template(
                &owner,
                binding.preset_revision_ref.preset_id.as_ref(),
            )
            .await
            .map_err(control_plane_error_to_app)?;
        let companion_id = if official_template
            .as_ref()
            .is_some_and(|template| template.as_str() == "companion.default")
        {
            companion_id_from_binding(&binding)?
        } else {
            None
        };
        let common_owner = nomifun_common::UserId::parse(owner_id.to_owned()).map_err(|error| {
            AppError::Forbidden(format!("invalid canonical AgentSession owner: {error}"))
        })?;
        let agent_name = control_plane
            .editor(
                &owner,
                binding.preset_revision_ref.preset_id.as_ref(),
                Some(binding.preset_revision_ref.revision),
            )
            .await
            .map_err(control_plane_error_to_app)?
            .preset
            .display_name;
        let mut projected = super::agent_binding_projection::project_saved_artifacts(
            &common_owner,
            binding,
            revision,
            snapshot,
            Some(&agent_name),
        )?;
        if let Some(template_key) = official_template.as_ref() {
            projected.projection.request.extra["official_template_key"] =
                Value::String(template_key.as_str().to_owned());
        }
        let runtime_host = self.official_runtime.get().ok_or_else(|| {
            AppError::Conflict(
                "canonical AgentSession projection requires the assembled Runtime host"
                    .to_owned(),
            )
        })?;
        let engine = runtime_host.validate_agent(&projected.snapshot)?;
        projected.projection.request.extra[nomifun_api_types::RUNTIME_BUILD_BINDING_KEY] =
            serde_json::to_value(engine).map_err(|error| AppError::Internal(error.to_string()))?;
        let workspace = frozen_workspace_root(
            &self.managed_workspace_root,
            owner_id,
            session_id,
            &projected.binding,
        )?;
        let created_at = self
            .canonical
            .store()
            .session_created_at(session_id)
            .await
            .map_err(agent_session_store_error)?;
        let execution_link: Option<ConversationExecutionLinkProjection> =
            sqlx::query_as(
                "SELECT link.execution_id, link.relation, link.step_id, link.attempt_id, \
                        json_extract(execution.initial_plan_input, '$.mode') \
                 FROM conversation_execution_links link \
                 JOIN agent_executions execution ON execution.execution_id = link.execution_id \
                 WHERE link.conversation_id = ? AND execution.user_id = ? \
                   AND execution.deleted_at IS NULL \
                 ORDER BY link.active DESC, link.updated_at DESC, link.id DESC LIMIT 1",
            )
            .bind(session_id.as_ref())
            .bind(owner_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| {
                AppError::Internal(format!(
                    "read canonical AgentExecution Conversation link: {error}"
                ))
            })?;
        Ok(Some(canonical_conversation_response(
            observed,
            projected,
            workspace,
            created_at,
            execution_link,
            companion_id,
        )?))
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
        self.dispatch_canonical_turn(
            owner_id,
            &AgentSessionId::from(session_id.to_owned()),
            idempotency_key,
            request,
            false,
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
        let session_id = AgentSessionId::from(session_id.to_owned());
        self.canonical_delivery_state_for_operation(
            owner_id,
            &session_id,
            &Self::turn_operation_id(owner_id, session_id.as_ref(), idempotency_key),
            true,
        )
        .await
    }

    pub(crate) async fn cancel_session(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<(), AppError> {
        let session_id = AgentSessionId::from(session_id.to_owned());
        let head = self.canonical.store().head(&session_id).await
            .map_err(agent_session_store_error)?;
        if head.active_turn_id.is_none() {
            return Ok(());
        }
        self.cancel_turn(
            owner_id,
            &session_id,
            &format!("cancel:{}", Uuid::now_v7()),
            nomifun_common::AgentKillReason::UserCancelled,
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn cancel_session_for_idmm(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<(), AppError> {
        let session_id = AgentSessionId::from(session_id.to_owned());
        if self
            .canonical
            .store()
            .head(&session_id)
            .await
            .map_err(agent_session_store_error)?
            .active_turn_id
            .is_none()
        {
            return Ok(());
        }
        self.cancel_turn(
            owner_id,
            &session_id,
            &format!("idmm-timeout:{}", Uuid::now_v7()),
            nomifun_common::AgentKillReason::IdleTimeout,
        )
        .await?;
        Ok(())
    }

    async fn cancel_turn(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        reason: nomifun_common::AgentKillReason,
    ) -> Result<AgentMutationReceipt, AppError> {
        let head = self
            .canonical
            .store()
            .head(session_id)
            .await
            .map_err(agent_session_store_error)?;
        let active_turn_id = head
            .active_turn_id
            .ok_or_else(|| AppError::Conflict("AgentSession has no active turn".into()))?;
        let generation = self
            .canonical
            .store()
            .read_turn_receipt(
                session_id,
                &OperationId::from(active_turn_id),
            )
            .await
            .map_err(agent_session_store_error)?
            .started_event
            .map(|event| event.seq)
            .unwrap_or(head.last_seq);
        let receipt = self.canonical
            .cancel(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner_id.to_owned(),
                },
                session_id,
                idempotency_key,
            )
            .await?;
        if let Some(runtime) = self.runtime_sessions.get_runtime(session_id.as_ref()) {
            runtime.cancel().await?;
        }
        self.runtime_sessions.cancel_runtime_turn(
            session_id.as_ref(),
            generation,
            Some(reason),
        )?;
        Ok(receipt)
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
    hosted_effects: super::hosted_effect_receipts::HostedEffectReceipts,
    wave2_owner: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
    session_owner: Arc<NomiCoreSessionOwner>,
    control_plane: Arc<AgentControlPlane>,
    kernel: Arc<KernelRegistry>,
    compiler_environment: CompilerEnvironment,
    schema_resolver: Arc<dyn NomiPluginToolSchemaResolver>,
    platform_builtin_tool_admission:
        Arc<NomiPlatformBuiltinToolAdmission>,
    platform_builtin_context_admission:
        Arc<NomiPlatformBuiltinContextAdmission>,
    platform_builtin_lifecycle_admission:
        Arc<NomiPlatformBuiltinLifecycleAdmission>,
    robot_owner: Option<Arc<super::nomi_core_robot::RobotModuleOwner>>,
}

impl NomiCorePluginToolSessionProvider {
    pub(crate) fn new(
        session_owner: Arc<NomiCoreSessionOwner>,
        control_plane: Arc<AgentControlPlane>,
        kernel: Arc<KernelRegistry>,
        compiler_environment: CompilerEnvironment,
        schema_resolver: Arc<dyn NomiPluginToolSchemaResolver>,
        platform_builtin_tool_admission: Arc<
            NomiPlatformBuiltinToolAdmission,
        >,
        platform_builtin_context_admission: Arc<
            NomiPlatformBuiltinContextAdmission,
        >,
        platform_builtin_lifecycle_admission: Arc<
            NomiPlatformBuiltinLifecycleAdmission,
        >,
        robot_owner: Option<Arc<super::nomi_core_robot::RobotModuleOwner>>,
        wave2_owner: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
        pool: nomifun_db::SqlitePool,
    ) -> Self {
        Self {
            hosted_effects: super::hosted_effect_receipts::HostedEffectReceipts::new(pool),
            wave2_owner,
            session_owner,
            control_plane,
            kernel,
            compiler_environment,
            schema_resolver,
            platform_builtin_tool_admission,
            platform_builtin_context_admission,
            platform_builtin_lifecycle_admission,
            robot_owner,
        }
    }

    // The same persisted binding and Compiler admission serve description and
    // execution. Everything below this method's result is runtime materialization.
    async fn compile_request(
        &self,
        request: NomiPluginToolSessionRequest,
    ) -> Result<Option<PreparedNomiPluginSession>, AppError> {
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
        self.hosted_effects.ensure_settled(common_owner.as_ref(), session_id.as_ref()).await?;
        self.session_owner
            .materialize_session_workspace(common_owner.as_ref(), &session_id)
            .await?;
        let response = self
            .session_owner
            .get_session(common_owner.as_ref(), session_id.as_ref())
            .await?;
        if response.extra.get(NOMI_CORE_SESSION_METADATA_KEY).is_none() {
            return Ok(None);
        }
        let constraints = nomifun_api_types::ExecutionConstraints::from_extra(&response.extra)?;
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
        // Revalidate the same contribution-driven consumer contract on every
        // Session load. Runtime family identity never decides capability support.
        let materialized = self
            .kernel
            .snapshot()
            .map_err(|error| AppError::Conflict(error.to_string()))?;
        super::nomi_core_tool_discovery::validate_snapshot(&materialized, &snapshot)
            .map_err(control_plane_error_to_app)?;
        super::nomi_core_mcp_catalog::validate_product_session_selection(
            &snapshot,
            &binding.typed_resource_bindings,
            &response.extra,
        )?;
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: common_owner.as_ref().to_owned(),
        };
        let wave2_workspace_selected = revision
            .payload
            .enabled_capabilities
            .iter()
            .any(|selection| {
                matches!(
                    selection.capability.id.as_ref(),
                    "workspace.files" | "workspace.vcs" | "workspace.artifacts"
                )
            });
        let wave2_process_selected = revision
            .payload
            .enabled_capabilities
            .iter()
            .any(|selection| {
                selection.capability.id.as_ref()
                    == nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID
            });
        let server_workspace = (wave2_workspace_selected || wave2_process_selected)
            .then(|| {
                response
                    .extra
                    .get("workspace")
                    .and_then(Value::as_str)
                    .filter(|workspace| !workspace.trim().is_empty())
                    .ok_or_else(|| {
                        AppError::Conflict(
                            "Nomi Wave 2 AgentSession has no server-resolved workspace"
                                .to_owned(),
                        )
                    })
            })
            .transpose()?;
        let mut runtime_binding = binding;
        if wave2_workspace_selected {
            let server_workspace = server_workspace.ok_or_else(|| {
                AppError::Internal(
                    "workspace capability lost its resolved Session root".to_owned(),
                )
            })?;
            let workspace_resources = runtime_binding
                .typed_resource_bindings
                .iter()
                .filter(|binding| {
                    binding.resource_kind.as_ref()
                        == nomifun_file::WORKSPACE_RESOURCE_KIND
                })
                .collect::<Vec<_>>();
            let [workspace_authority] = workspace_resources.as_slice() else {
                return Err(AppError::Conflict(
                    "Nomi Wave 2 requires one server-resolved Session workspace resource"
                        .to_owned(),
                ));
            };
            let workspace = super::nomi_core_wave2::session_workspace_binding(
                server_workspace,
                &principal,
                &session_id,
                workspace_authority,
            )?;
            runtime_binding.typed_resource_bindings =
                super::nomi_core_wave2::with_session_workspace_binding(
                    runtime_binding.typed_resource_bindings,
                    workspace.clone(),
                );
        }
        if wave2_process_selected {
            let server_workspace = server_workspace.ok_or_else(|| {
                AppError::Internal(
                    "process capability lost its resolved Session root".to_owned(),
                )
            })?;
            let process_resources = runtime_binding
                .typed_resource_bindings
                .iter()
                .enumerate()
                .filter(|(_, binding)| binding.resource_kind.as_ref() == "process_session")
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let [index] = process_resources.as_slice() else {
                return Err(AppError::Conflict(
                    "Nomi Wave 2 requires one server-resolved Session process resource"
                        .to_owned(),
                ));
            };
            runtime_binding.typed_resource_bindings[*index] =
                super::nomi_core_wave2::session_process_binding(
                    server_workspace,
                    &principal,
                    &session_id,
                    &runtime_binding.typed_resource_bindings[*index],
                )?;
        }
        let resource_image_model = revision.payload.chat_route_records
            .get(nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT)
            .is_some_and(|route| route.primary.features.contains(&nomifun_agent_contracts::ChatRouteFeature::ImageInput));
        let compiled = compile_nomi_plugin_snapshot(
            &self.kernel,
            &self.compiler_environment,
            runtime_binding,
            revision,
            snapshot,
            &principal,
        )?;
        let compiled = Arc::new(compiled);
        Ok(Some(PreparedNomiPluginSession { owner, session_id, principal, compiled, response, constraints, resource_image_model }))
    }

    async fn compile_command_request(
        &self,
        request: NomiPluginToolSessionRequest,
    ) -> Result<Arc<CompiledSnapshot>, AppError> {
        let common_owner = nomifun_common::UserId::parse(request.owner_id)
            .map_err(|error| AppError::Forbidden(format!(
                "invalid Nomi Plugin Tool session owner: {error}"
            )))?;
        let session_id = parse_agent_session_id(&request.conversation_id)
            .map_err(|error| AppError::Conflict(error.message))?;
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: common_owner.as_ref().to_owned(),
        };
        let observation = self
            .session_owner
            .canonical()
            .get(&principal, &session_id)
            .await?;
        let binding_dto = agent_binding_dto(&observation.session.agent_binding)
            .map_err(|error| AppError::Conflict(error.message))?;
        let owner = AuthenticatedOwner(UserId::from(common_owner.as_ref().to_owned()));
        let (binding, revision, snapshot) = self
            .control_plane
            .saved_binding_artifacts(&owner.0, &binding_dto)
            .await
            .map_err(control_plane_error_to_app)?;
        if binding != observation.session.agent_binding {
            return Err(AppError::Conflict(
                "Skill discovery binding differs from the canonical AgentSession binding"
                    .to_owned(),
            ));
        }
        let materialized = self
            .kernel
            .snapshot()
            .map_err(|error| AppError::Conflict(error.to_string()))?;
        super::nomi_core_tool_discovery::validate_snapshot(&materialized, &snapshot)
            .map_err(control_plane_error_to_app)?;
        let compiled = compile_nomi_plugin_snapshot(
            &self.kernel,
            &self.compiler_environment,
            binding,
            revision,
            snapshot,
            &principal,
        )?;
        // Cold discovery is description-only, but it must authenticate the
        // same owner-scoped typed resources as execution. It may skip Context
        // activation and resource acquisition; it may not skip Binding
        // validation merely because no runtime has been started.
        super::nomi_core_mcp_catalog::validate_resources(
            &compiled,
            &materialized,
            &principal,
        )?;
        Ok(Arc::new(compiled))
    }

    async fn command_items(
        &self,
        compiled: Arc<CompiledSnapshot>,
        extra: Option<&Value>,
    ) -> Result<Vec<nomifun_api_types::SlashCommandItem>, AppError> {
        let registry = self
            .kernel
            .snapshot()
            .map_err(|error| AppError::Conflict(error.to_string()))?;
        let skills = super::engine_skills::compile_commands(
            &compiled,
            &registry,
        )
        .await?;
        if let Some(extra) = extra {
            skills.validate_extra(extra)?;
        }
        let commands = nomifun_ai_agent::plugin_skills::verified_skill_commands(
            Arc::clone(&self.kernel),
            compiled,
            None,
            skills.commands,
        )
        .map_err(|error| {
            AppError::Conflict(format!("Nomi Skill discovery failed: {error}"))
        })?;
        let mut items = commands
            .iter()
            .filter(|skill| skill.metadata().user_invocable)
            .map(|skill| nomifun_api_types::SlashCommandItem {
                command: skill.command_name(),
                description: skill.metadata().description.clone(),
            })
            .collect::<Vec<_>>();
        items.sort_by(|a, b| a.command.cmp(&b.command));
        Ok(items)
    }

    /// Canonical AgentSession discovery never accepts legacy Conversation
    /// `extra`. Its saved Binding and typed resources are the complete input.
    pub(crate) async fn discover_canonical_skill_commands(
        &self,
        owner: &AuthenticatedOwner,
        session_id: &AgentSessionId,
    ) -> Result<Vec<nomifun_api_types::SlashCommandItem>, AppError> {
        let compiled = self
            .compile_command_request(NomiPluginToolSessionRequest {
                owner_id: owner.0.as_ref().to_owned(),
                conversation_id: session_id.as_ref().to_owned(),
            })
            .await?;
        self.command_items(compiled, None).await
    }
}

struct PreparedNomiPluginSession {
    owner: AuthenticatedOwner,
    session_id: AgentSessionId,
    principal: PrincipalRef,
    compiled: Arc<CompiledSnapshot>,
    response: ConversationResponse,
    constraints: nomifun_api_types::ExecutionConstraints,
    resource_image_model: bool,
}

#[async_trait]
impl NomiPluginToolSessionProvider for NomiCorePluginToolSessionProvider {
    async fn discover_skill_commands(
        &self,
        request: NomiPluginToolSessionRequest,
    ) -> Result<Vec<nomifun_api_types::SlashCommandItem>, AppError> {
        let Some(prepared) = self.compile_request(request).await? else {
            return Ok(Vec::new());
        };
        if prepared.constraints.restricted() {
            return Ok(Vec::new());
        }
        self.command_items(prepared.compiled, Some(&prepared.response.extra))
            .await
    }

    async fn resolve(
        &self,
        request: NomiPluginToolSessionRequest,
    ) -> Result<Option<NomiPluginToolSession>, AppError> {
        let Some(PreparedNomiPluginSession { owner, session_id, principal, compiled, response, constraints, resource_image_model }) =
            self.compile_request(request).await? else { return Ok(None); };
        let registry = self.kernel.snapshot().map_err(|error| AppError::Conflict(error.to_string()))?;
        super::nomi_core_mcp_catalog::validate_resources(&compiled, &registry, &principal)?;
        let skills = super::engine_skills::compile(&compiled, &registry).await?;
        skills.validate_extra(&response.extra)?;
        let mut mcp_schemas = BTreeMap::new();
        for selected in compiled.content().enabled_capabilities.iter() {
            if selected.contribution_lock.source_kind == nomifun_agent_contracts::ContributionSourceKind::McpBinding {
                let tool = super::nomi_core_mcp_catalog::frozen_tool(&registry, selected)?;
                mcp_schemas.insert(selected.capability.id.clone(), StrictJsonValue(tool.input_schema));
            }
        }
        let tool_admission = Arc::new(self.platform_builtin_tool_admission.as_ref().clone()
            .with_mcp_tools(&registry, mcp_schemas)
            .map_err(|error| AppError::Conflict(error.to_string()))?);
        let plugin_session = KernelNomiPluginToolSession::materialize_for_execution(
            Arc::clone(&self.kernel),
            Arc::clone(&compiled),
            principal.clone(),
            session_id.clone(),
            ScopeKey::from(format!(
                "session:{}",
                session_id.as_ref()
            )),
            Arc::clone(&self.schema_resolver),
            tool_admission,
            Arc::clone(&self.platform_builtin_context_admission),
            Arc::clone(&self.platform_builtin_lifecycle_admission),
            constraints,
        )
        .await
        .map_err(|error| {
            AppError::Conflict(format!(
                "Nomi Plugin Tool session materialization failed: {error}"
            ))
        })?;
        let plugin_session = if skills.ids.is_empty() { plugin_session } else {
            plugin_session.with_selected_skills(nomifun_ai_agent::nomi_skills::NomiSelectedSkills::new(
                skills.instructions, skills.resources, resource_image_model,
            ).map_err(|error| AppError::Conflict(error.to_string()))?)
                .map_err(|error| AppError::Conflict(error.to_string()))?
        };
        let plugin_session = if constraints.restricted() { plugin_session } else {
            plugin_session.with_verified_skill_commands(
                Arc::clone(&self.kernel), Arc::clone(&compiled), skills.commands,
            ).map_err(|error| AppError::Conflict(format!("Nomi Skill command materialization failed: {error}")))?
        };
        let robot_module_id = super::nomi_core_robot::module_capability_id();
        let robot_selected = compiled.resolved_capability(&robot_module_id);
        let robot_resources_bound = robot_selected
            .is_some()
            && compiled
                .capability_resources_bound(&robot_module_id)
                .map_err(kernel_error_to_app)?;
        let dynamic = if !robot_resources_bound || constraints.restricted() {
            None
        } else {
            let selected = robot_selected.expect("checked above");
            let policy = compiled.policy(&robot_module_id).ok_or_else(|| {
                AppError::Conflict(
                    "Nomi Robot Module has no compiled Action/resource policy".to_owned(),
                )
            })?;
            let declared_actions = super::nomi_core_robot::action_ids();
            if selected.contribution_lock.source_kind != ContributionSourceKind::PlatformBuiltin
                || policy.allowed_actions.is_empty()
                || policy
                    .allowed_actions
                    .iter()
                    .any(|action| !declared_actions.contains(action))
            {
                return Err(AppError::Conflict(
                    "Nomi Robot Module requires its bundled owner and exact Action grants"
                        .to_owned(),
                ));
            }
            let owner = self.robot_owner.as_ref().ok_or_else(|| {
                AppError::Conflict(
                    "Nomi Robot Module owner is unavailable for this Session".to_owned(),
                )
            })?;
            let robot_bindings = compiled
                .target_resource_bindings
                .iter()
                .filter(|binding| {
                    binding.resource_kind.as_ref() == "robot"
                        && policy.resource_binding_ids.contains(&binding.binding_id)
                })
                .collect::<Vec<_>>();
            let [robot_binding] = robot_bindings.as_slice() else {
                return Err(AppError::Conflict(
                    "Nomi Robot Module requires one exact server-resolved Robot binding"
                        .to_owned(),
                ));
            };
            match owner
                .resolve_session_tools_if_available(
                    &principal,
                    &session_id,
                    robot_binding,
                    &policy.allowed_actions,
                )
                .await
                .map_err(|error| AppError::Conflict(error.to_string()))?
            {
                None => None,
                Some(resolved) => {
                    let mut provider_actions = BTreeMap::new();
                    let descriptors = resolved
                        .descriptors
                        .iter()
                        .map(|descriptor| {
                            if provider_actions
                                .insert(
                                    descriptor.provider_name.clone(),
                                    descriptor.action_id.clone(),
                                )
                                .is_some()
                            {
                                return Err(AppError::Conflict(format!(
                                    "Nomi Robot Module published duplicate provider tool {}",
                                    descriptor.provider_name
                                )));
                            }
                            Ok(nomifun_ai_agent::NomiHostDynamicToolDescriptor {
                                capability_id: robot_module_id.clone(),
                                provider_name: descriptor.provider_name.clone(),
                                description: descriptor.description.clone(),
                                input_schema: descriptor.input_schema.clone(),
                                effect_class: if descriptor.action_id.as_ref()
                                    == nomifun_robot::capability::ROBOT_VISION_ACTION_ID
                                {
                                    EffectClass::ReadSensitive
                                } else {
                                    EffectClass::Physical
                                },
                                deferred: false,
                            })
                        })
                        .collect::<Result<Vec<_>, AppError>>()?;
                    let provider_actions = Arc::new(provider_actions);
                    Some((descriptors, Arc::new(super::hosted_effect_receipts::RobotReceiptInvoker {
                            receipts: self.hosted_effects.clone(), user: principal.principal_id.clone(),
                            session: session_id.as_ref().to_owned(),
                            provider_actions,
                            delegate: Arc::clone(&resolved.invoker),
                        }) as Arc<dyn nomifun_ai_agent::NomiHostDynamicToolInvoker>))
                }
            }
        };
        let hosted_witness = self.hosted_effects.witness(principal.principal_id.clone(), session_id.as_ref().to_owned());
        let git_witness = if !constraints.restricted()
            && compiled.content().enabled_capabilities.iter().any(|capability| {
                capability.capability.id.as_ref() == "workspace.vcs"
                    && capability.action_allowlist.contains(
                        &nomifun_agent_contracts::ActionId::from("workspace.vcs/push"),
                    )
            })
        {
            Some(super::engine_git_lifecycle::WorkspaceGitWitness::new(
                self.wave2_owner.clone(),
                response.extra.get("workspace").and_then(Value::as_str)
                    .ok_or_else(|| AppError::Conflict("Git Session workspace is missing".into()))?,
                self.hosted_effects.clone(), principal.principal_id.clone(), session_id.as_ref().to_owned(),
            )?)
        } else { None };
        let mut witnesses: Vec<Arc<dyn nomifun_ai_agent::engine_effect_scope::EngineEffectSettlement>> = vec![
            hosted_witness.clone(),
            super::nomi_core_mcp_catalog::settlement_witness(
                Arc::clone(&self.wave2_owner), principal.principal_id.clone(), session_id.as_ref().to_owned(),
            ),
        ];
        if let Some(witness) = &git_witness { witnesses.push(witness.clone()); }
        let effect_scope = Arc::new(nomifun_ai_agent::engine_effect_scope::EngineEffectScope::new(witnesses)?);
        let mcp_resources = if !constraints.restricted()
            && compiled
                .resource_bindings()
                .iter()
                .any(|binding| binding.resource_kind.as_ref() == "mcp_server")
        {
            let active = plugin_session.capability_state().ok_or_else(|| AppError::Conflict("MCP resource binding state is unavailable".into()))?;
            let resources = super::nomi_core_mcp_resources::adapter(self.kernel.clone(), compiled.clone(),
                active, self.wave2_owner.clone(), principal.clone(), session_id.clone(), resource_image_model, constraints)?;
            Some(resources)
        } else { None };
        self.wave2_owner.ensure_mcp_settled(&principal.principal_id, session_id.as_ref()).await?;
        let mut context: Vec<Arc<dyn nomifun_ai_agent::ContextContributor>> = vec![
            super::nomi_core_mcp_catalog::recovery_context(Arc::clone(&self.wave2_owner),
                principal.principal_id.clone(), session_id.as_ref().to_owned()),
            hosted_witness,
        ];
        if let Some(witness) = git_witness { context.push(witness); }
        plugin_session.bind_hosted_execution(nomifun_ai_agent::NomiHostedSessionBindings {
            effect_scope,
            dynamic,
            context,
            mcp_resources,
            session_control: Some(Arc::new(NomiCoreSessionControlSink {
                session_owner: Arc::clone(&self.session_owner),
                owner,
                session_id: session_id.clone(),
            })),
        })
            .map(Some)
        .map_err(|error| {
            AppError::Conflict(format!(
                "Nomi hosted Tool session assembly failed: {error}"
            ))
        })
    }
}

#[derive(Clone, Debug)]
struct ExactSessionMcpSelection {
    ids: Vec<McpServerId>,
}

async fn exact_session_mcp_selection(
    repository: &Arc<dyn nomifun_db::IMcpServerRepository>,
    owner: &AuthenticatedOwner,
    binding: &AgentBindingValue,
) -> Result<ExactSessionMcpSelection, NomiCoreApiError> {
    let bindings = binding
        .typed_resource_bindings
        .iter()
        .filter(|binding| binding.resource_kind.as_ref() == "mcp_server")
        .collect::<Vec<_>>();
    if bindings.len() > super::nomi_core_mcp_catalog::MAX_SESSION_SERVERS {
        return Err(NomiCoreApiError::new(StatusCode::UNPROCESSABLE_ENTITY,
            "MCP_RESOURCE_CARDINALITY_INVALID", "the AgentSession MCP server bound was exceeded"));
    }
    let mut selection = ExactSessionMcpSelection { ids: Vec::new() };
    let mut seen = BTreeSet::new();
    for binding in bindings {
        if !seen.insert(binding.resource_id.clone()) {
            return Err(NomiCoreApiError::new(StatusCode::UNPROCESSABLE_ENTITY,
                "MCP_RESOURCE_CARDINALITY_INVALID", "duplicate MCP server resource"));
        }
        if binding.owner_id != owner.as_ref() {
            return Err(NomiCoreApiError::new(
                StatusCode::FORBIDDEN,
                "RESOURCE_OWNER_MISMATCH",
                "the selected MCP server belongs to a different owner",
            ));
        }
        let row = repository
            .find_by_id(binding.resource_id.as_ref())
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
            .ok_or_else(|| {
                NomiCoreApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "MCP_SERVER_NOT_FOUND",
                    "the selected MCP server no longer exists",
                )
            })?;
        if !row.enabled || row.deleted_at.is_some() {
            return Err(NomiCoreApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "MCP_SERVER_DISABLED",
                "the selected MCP server is disabled",
            ));
        }
        let expected_ref = format!(
            "mcp-server:{}@{}",
            row.mcp_server_id, row.updated_at
        );
        if binding.connection_config_ref.as_ref().map(AsRef::as_ref)
            != Some(expected_ref.as_str())
        {
            return Err(NomiCoreApiError::new(
                StatusCode::CONFLICT,
                "MCP_CONNECTION_CONFIG_STALE",
                "the selected MCP server changed after the Agent binding was resolved",
            ));
        }
        let server = nomifun_mcp::McpServer::from_row(row).map_err(|error| {
            NomiCoreApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "MCP_SERVER_CONFIG_INVALID",
                error.to_string(),
            )
        })?;
        selection.ids.push(server.mcp_server_id);
    }
    Ok(selection)
}

fn install_creation_mcp_selection(
    extra: &mut Value,
    selection: &ExactSessionMcpSelection,
) -> Result<(), NomiCoreApiError> {
    let object = extra.as_object_mut().ok_or_else(|| {
        NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "NOMI_CORE_SESSION_EXTRA_INVALID",
            "Nomi-core Session extra must be a JSON object",
        )
    })?;
    object.insert(
        "selected_mcp_server_ids".to_owned(),
        serde_json::to_value(&selection.ids)?,
    );
    object.remove("selected_session_mcp_servers");
    object.remove("session_mcp_servers");
    Ok(())
}

pub(super) fn compile_nomi_plugin_snapshot(
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
    // A saved Snapshot freezes inherited choices too. Revalidate these exact
    // targets; never consult current installation defaults during execution.
    environment.installation_role_bindings = persisted.content.resolved_role_providers.iter()
        .map(|(role_id, lock)| (role_id.clone(), nomifun_agent_contracts::InstallationRoleBinding {
            selection: nomifun_agent_contracts::RoleProviderSelection {
                role: lock.provider.role.clone(),
                provider_mount_id: lock.provider.mount_id.clone(),
            }, binding_version: 1, updated_at_ms: 0,
        })).collect();
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
        },
    )
    .map_err(kernel_error_to_app)?;
    // Existing Sessions must enforce the same source-aware consumer admission
    // as preview/save; Mount snapshots intentionally do not embed action lists.
    super::nomi_core_tool_discovery::validate_snapshot(&registry, &compiled.envelope)
        .map_err(control_plane_error_to_app)?;
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

fn idmm_config_from_runtime_policy(
    policy: &nomifun_agent_contracts::AgentRuntimePolicy,
) -> Result<nomifun_api_types::IdmmConfig, AppError> {
    serde_json::to_value(&policy.idmm)
        .and_then(serde_json::from_value)
        .map_err(|error| {
            AppError::Conflict(format!("Agent IDMM policy is invalid: {error}"))
        })
}

fn product_agent_target_from_request(extra: &Value) -> Option<ProductAgentTarget> {
    let object = extra.as_object()?;
    if let (Some(target_kind), Some(target_id)) = (
        object
            .get("product_agent_target_kind")
            .and_then(Value::as_str),
        object
            .get("product_agent_target_id")
            .and_then(Value::as_str),
    ) {
        let default_template_key = match target_kind {
            "companion" => "companion.default",
            "customer" => "customer-service.default",
            "creative_studio_canvas" => "creative-studio.default",
            _ => return None,
        };
        return Some(ProductAgentTarget {
            target_kind: target_kind.to_owned(),
            target_id: target_id.to_owned(),
            default_template_key: default_template_key.to_owned(),
        });
    }
    let companion_id = object.get("companion_id").and_then(Value::as_str)?;
    let is_companion = object
        .get("companion_session")
        .and_then(Value::as_bool)
        == Some(true)
        || object.get("channel_platform").and_then(Value::as_str).is_some();
    is_companion.then(|| ProductAgentTarget {
        target_kind: "companion".to_owned(),
        target_id: companion_id.to_owned(),
        default_template_key: "companion.default".to_owned(),
    })
}

fn merge_product_agent_resolution(
    extra: &mut Value,
    target: &ProductAgentTarget,
    resolution: &ProductAgentResolution,
) -> Result<(), AppError> {
    let object = extra.as_object_mut().ok_or_else(|| {
        AppError::Conflict("product Agent Session extra must be an object".to_owned())
    })?;
    let projected = resolution.runtime_extra.as_object().ok_or_else(|| {
        AppError::Internal("product Agent resolution is not an object".to_owned())
    })?;
    for key in [
        NOMI_CORE_SESSION_METADATA_KEY,
        nomifun_api_types::EXECUTION_CONSTRAINTS_KEY,
        "chat_config_revision_digest",
        "system_prompt",
        "selected_mcp_server_ids",
    ] {
        if let Some(value) = projected.get(key) {
            object.insert(key.to_owned(), value.clone());
        }
    }
    object.insert(
        "product_agent_target_kind".to_owned(),
        Value::String(target.target_kind.clone()),
    );
    object.insert(
        "product_agent_target_id".to_owned(),
        Value::String(target.target_id.clone()),
    );
    Ok(())
}

#[async_trait]
impl nomifun_cron::CronSessionPort for NomiCoreSessionOwner {
    async fn get_session(
        &self,
        query: &nomifun_cron::CronSessionLookup,
    ) -> Result<nomifun_cron::CronSessionProjection, AppError> {
        let response = self
            .canonical_conversation_projection(&query.owner_id, &query.agent_session_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!(
                "AgentSession {} not found",
                query.agent_session_id.as_ref(),
            )))?;
        let response = self.ensure_cron_workspace_projection(response).await?;
        let cron_job_id = sqlx::query_scalar::<_, String>(
            "SELECT cron_job_id FROM cron_jobs \
             WHERE user_id = ? AND conversation_id = ? \
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(&query.owner_id)
        .bind(query.agent_session_id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        cron_session_projection_from_response(&query.owner_id, response, cron_job_id)
    }

    async fn list_conversation_responses_for_cron(
        &self,
        query: &nomifun_cron::CronScheduledSessionLookup,
    ) -> Result<Vec<ConversationResponse>, AppError> {
        let session_ids = sqlx::query_scalar::<_, String>(
            "SELECT conversation_id FROM cron_jobs \
             WHERE user_id = ? AND cron_job_id = ? AND conversation_id IS NOT NULL",
        )
        .bind(&query.owner_id)
        .bind(&query.cron_job_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        let mut sessions = Vec::with_capacity(session_ids.len());
        for session_id in session_ids {
            let session_id = AgentSessionId::from(session_id);
            if let Some(session) = self
                .canonical_conversation_projection(&query.owner_id, &session_id)
                .await?
            {
                sessions.push(self.ensure_cron_workspace_projection(session).await?);
            }
        }
        Ok(sessions)
    }

    async fn bind_cron_relation(
        &self,
        request: &nomifun_cron::CronSessionCronBindingRequest,
    ) -> Result<(), AppError> {
        self
            .canonical_conversation_projection(&request.owner_id, &request.agent_session_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!(
                "AgentSession {} not found",
                request.agent_session_id.as_ref(),
            )))?;
        let relation_exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS( \
                SELECT 1 FROM cron_jobs job \
                 WHERE job.user_id = ? AND job.cron_job_id = ? \
                   AND job.conversation_id = ? \
                UNION ALL \
                SELECT 1 FROM cron_run_reservations reservation \
                  JOIN cron_jobs job ON job.cron_job_id = reservation.cron_job_id \
                 WHERE job.user_id = ? AND reservation.cron_job_id = ? \
                   AND reservation.conversation_id = ? \
                   AND reservation.status = 'reserved' \
            )",
        )
        .bind(&request.owner_id)
        .bind(&request.cron_job_id)
        .bind(request.agent_session_id.as_ref())
        .bind(&request.owner_id)
        .bind(&request.cron_job_id)
        .bind(request.agent_session_id.as_ref())
        .fetch_one(&self.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        if relation_exists == 0 {
            return Err(AppError::Conflict(
                "Cron relation was not committed to its job or exact run reservation"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    async fn read_turn_receipt(
        &self,
        query: &nomifun_cron::CronTurnReceiptQuery,
    ) -> Result<nomifun_cron::CronTurnReceiptState, AppError> {
        Ok(cron_turn_state_from_conversation(
            self.session_turn_delivery_state(
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
        match self
            .session_turn_delivery_state(
                &request.owner_id,
                request.agent_session_id.as_ref(),
                &request.idempotency_key,
            )
            .await?
        {
            PublicTurnDeliveryState::Missing => Ok(
                nomifun_cron::CronTurnReconciliation::StaleConflict,
            ),
            PublicTurnDeliveryState::Completed(_) => Ok(
                nomifun_cron::CronTurnReconciliation::ReconciledOrTerminalReRead,
            ),
            PublicTurnDeliveryState::Accepted { .. }
                if self.runtime_sessions.get_runtime(request.agent_session_id.as_ref()).is_some() =>
            {
                Ok(nomifun_cron::CronTurnReconciliation::LiveExactOwnerWait)
            }
            PublicTurnDeliveryState::Accepted { .. } => {
                let operation = Self::turn_operation_id(
                    &request.owner_id,
                    request.agent_session_id.as_ref(),
                    &request.idempotency_key,
                );
                self.settle_dispatch_failure(
                    &request.agent_session_id,
                    &operation,
                    "Runtime owner was not recoverable after restart",
                )
                .await?;
                Ok(nomifun_cron::CronTurnReconciliation::ReconciledOrTerminalReRead)
            }
        }
    }

    async fn create_idempotent(
        &self,
        user_id: &str,
        request: CreateConversationRequest,
        agent_binding: nomifun_cron::CronSessionAgentBinding,
        creation_key: &str,
    ) -> Result<nomifun_cron::CronSessionHandle, AppError> {
        let snapshot = match agent_binding {
            nomifun_cron::CronSessionAgentBinding::Frozen(snapshot) => snapshot,
            nomifun_cron::CronSessionAgentBinding::ModelOnly => {
                self.materialize_cron_model_only_snapshot(user_id, request.model.as_ref())
                    .await?
            }
        };
        let response = self
            .create_session_idempotent(user_id, request, Some(snapshot), creation_key)
            .await?;
        let response = self.ensure_cron_workspace_projection(response).await?;
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
        let nomifun_cron::CronTurnRequest { message, runtime } = turn;
        let session_id = agent_session_id.as_ref();
        nomifun_common::CronJobId::parse(&runtime.overlay.cron_job_id).map_err(|error| {
            AppError::BadRequest(format!("invalid Cron runtime annotation: {error}"))
        })?;
        nomifun_common::CronJobRunId::parse(&runtime.overlay.cron_job_run_id).map_err(|error| {
            AppError::BadRequest(format!("invalid Cron run annotation: {error}"))
        })?;
        // The exact durable run reservation owns the relation for both modes.
        // `new_conversation` never writes a per-run Session onto cron_jobs, and
        // lazy `existing` cannot do so before its first turn succeeds. Checking
        // cron_jobs.conversation_id here therefore rejects every legitimate
        // first run; the reservation was atomically attached before dispatch.
        let relation: Option<String> = sqlx::query_scalar(
            "SELECT reservation.conversation_id \
             FROM cron_run_reservations reservation \
             JOIN cron_jobs job ON job.cron_job_id = reservation.cron_job_id \
             WHERE job.user_id = ? AND reservation.cron_job_id = ? \
               AND reservation.cron_job_run_id = ? AND reservation.status = 'reserved'",
        )
        .bind(&owner_id)
        .bind(&runtime.overlay.cron_job_id)
        .bind(&runtime.overlay.cron_job_run_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        if relation.as_deref() != Some(session_id) {
            return Err(AppError::Conflict(format!(
                "AgentSession {session_id} is not bound to Cron run {}",
                runtime.overlay.cron_job_run_id
            )));
        }
        let session = self.get_session(&owner_id, session_id).await?;
        let session = self.ensure_cron_workspace_projection(session).await?;
        let workspace = session
            .extra
            .get("workspace")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AppError::Conflict(format!(
                "AgentSession {session_id} has no Cron workspace projection"
            )))?;
        let delivery = self
            .send_session_message_idempotent(
                &owner_id,
                session_id,
                &idempotency_key,
                cron_turn_message_to_request(message),
            )
            .await?;
        Ok(nomifun_cron::CronPreparedTurnDelivery {
            delivery: cron_delivery_from_conversation(delivery),
            workspace,
        })
    }

    async fn delivery_result(
        &self,
        query: &nomifun_cron::CronTurnDeliveryQuery,
    ) -> Result<Option<nomifun_cron::CronTurnDelivery>, AppError> {
        let _ = &query.message;
        Ok(match self
            .session_turn_delivery_state(
                &query.owner_id,
                query.agent_session_id.as_ref(),
                &query.idempotency_key,
            )
            .await?
        {
            PublicTurnDeliveryState::Missing => None,
            PublicTurnDeliveryState::Accepted { message_id } => {
                Some(cron_delivery_from_conversation(IdempotentMessageDelivery {
                    message_id,
                    replayed: true,
                    completed: false,
                    result_ok: None,
                    result_text: None,
                    result_error: None,
                    result_error_code: None,
                    result_error_retryable: None,
                }))
            }
            PublicTurnDeliveryState::Completed(delivery) => {
                Some(cron_delivery_from_conversation(delivery))
            }
        })
    }

    async fn append_notice(
        &self,
        owner_id: &str,
        agent_session_id: &AgentSessionId,
        content: &str,
        notice_kind: &str,
    ) -> Result<(), AppError> {
        self.canonical
            .get(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner_id.to_owned(),
                },
                agent_session_id,
            )
            .await?;
        let cause: String = sqlx::query_scalar(
            "SELECT event_id FROM agent_events \
             WHERE session_id = ? AND kind IN ( \
                 'session/ready','turn/completed','turn/failed','turn/cancelled', \
                 'message/assistant-projected' \
             ) ORDER BY seq DESC LIMIT 1",
        )
        .bind(agent_session_id.as_ref())
        .fetch_one(&self.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        let digest = Sha256::digest(format!("{notice_kind}\0{content}").as_bytes());
        let identity = format!(
            "cron-notice:{}:{digest:x}",
            agent_session_id.as_ref()
        );
        let message_id = Uuid::now_v7().to_string();
        self.canonical
            .store()
            .append_event(&nomifun_agent_contracts::SessionEventAppend {
                agent_session_id: agent_session_id.clone(),
                event_id: nomifun_agent_contracts::EventId::from(message_id.clone()),
                producer_id: nomifun_agent_contracts::EventProducerId::from("session_api"),
                idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(identity),
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                    kind: nomifun_agent_contracts::SessionEventKind(
                        "message/assistant-projected".to_owned(),
                    ),
                    kind_version: 1,
                    correlation_id: nomifun_agent_contracts::CorrelationId::from(message_id),
                    causation_event_id: Some(nomifun_agent_contracts::EventId::from(cause)),
                    payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                        StrictJsonValue(json!({
                            "content": content,
                            "notice_kind": notice_kind,
                        })),
                    ),
                },
            })
            .await
            .map_err(agent_session_store_error)?;
        Ok(())
    }
}

#[async_trait]
impl nomifun_channel::ChannelSessionPort for NomiCoreSessionOwner {
    async fn is_busy(&self, session_id: &str) -> bool {
        self.canonical
            .store()
            .head(&AgentSessionId::from(session_id.to_owned()))
            .await
            .is_ok_and(|head| head.status == "running")
    }

    async fn turn_outcome(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
    ) -> Result<nomifun_channel::ChannelTurnReceiptState, AppError> {
        self.session_turn_delivery_state(owner_id, session_id, idempotency_key)
            .await
            .map(channel_turn_receipt_state_from_conversation)
    }

    async fn cancel(&self, owner_id: &str, session_id: &str) -> Result<(), AppError> {
        self.cancel_session(owner_id, session_id).await
    }

    async fn list_messages(
        &self,
        owner_id: &str,
        session_id: &str,
        query: ListMessagesQuery,
    ) -> Result<MessageListResponse, AppError> {
        let session_id = AgentSessionId::from(session_id.to_owned());
        self.canonical
            .get(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner_id.to_owned(),
                },
                &session_id,
            )
            .await?;
        let created_at = self
            .canonical
            .store()
            .session_created_at(&session_id)
            .await
            .map_err(agent_session_store_error)?;
        let limit = query.page_size.unwrap_or(100).clamp(1, 500);
        let (projections, has_more, total) = self
            .canonical
            .store()
            .messages_before(&session_id, None, limit)
            .await
            .map_err(agent_session_store_error)?;
        let mut items = Vec::new();
        for projection in projections {
            if let Some(message) = canonical_message_response(&session_id, created_at, projection)
                .map_err(|error| AppError::Conflict(error.message))?
            {
                items.push(message);
            }
        }
        Ok(PaginatedResult {
            items,
            total,
            has_more,
        })
    }

    async fn send_turn(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
        request: SendMessageRequest,
    ) -> Result<nomifun_channel::ChannelTurnDelivery, AppError> {
        let delivery = self
            .send_session_message_idempotent(
                owner_id,
                session_id,
                idempotency_key,
                request,
            )
            .await?;
        let events = if delivery.completed {
            None
        } else {
            wait_for_runtime_subscription(&self.runtime_sessions, session_id).await
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
        self.get_session(owner_id, session_id).await
    }

    async fn create_idempotent(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.create_session_idempotent(owner_id, request, None, creation_key).await
    }
}

#[async_trait]
impl nomifun_requirement::AutoWorkScheduledSessionLookup for NomiCoreSessionOwner {
    async fn list_enabled_scheduled_sessions(
        &self,
        owner_id: &str,
    ) -> Result<nomifun_requirement::ScheduledAutoWorkSessionScan, AppError> {
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: owner_id.to_owned(),
        };
        let configs = self
            .canonical
            .store()
            .list_enabled_automation_configs(&principal)
            .await
            .map_err(|error| AppError::Internal(format!(
                "list canonical AgentSession AutoWork configs: {error}"
            )))?;
        let sessions = configs
            .into_iter()
            .map(|entry| {
                let tag = entry.config.tag.ok_or_else(|| {
                    AppError::Conflict(format!(
                        "enabled AgentSession {} AutoWork config has no tag",
                        entry.session.agent_session_id.as_ref()
                    ))
                })?;
                Ok(nomifun_requirement::ScheduledAutoWorkSession {
                    session_id: entry.session.agent_session_id.as_ref().to_owned(),
                    display_name: entry
                        .session
                        .metadata
                        .title
                        .unwrap_or_else(|| "Agent Session".to_owned()),
                    tag,
                    max_requirements: entry.config.max_requirements,
                    config_revision: format!("agent-session:{}", entry.config.revision),
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(nomifun_requirement::ScheduledAutoWorkSessionScan {
            sessions,
            quarantined: Vec::new(),
        })
    }
}

#[async_trait]
impl nomifun_requirement::AutoWorkWorkspacePort for NomiCoreSessionOwner {
    async fn resolve_frozen_workspace(
        &self,
        owner_id: &str,
        agent_session_id: &str,
    ) -> Result<nomifun_requirement::AutoWorkWorkspaceResolution, AppError> {
        nomifun_common::UserId::parse(owner_id.to_owned()).map_err(|error| {
            AppError::Forbidden(format!(
                "AutoWork owner is not a canonical installation UserId: {error}"
            ))
        })?;
        nomifun_common::validate_uuidv7(agent_session_id).map_err(|error| {
            AppError::NotFound(format!(
                "AutoWork AgentSession identity is not canonical UUIDv7: {error}"
            ))
        })?;
        let session_id = AgentSessionId::from(agent_session_id.to_owned());
        let operation_guard = self
            .session_operation_lock(agent_session_id)
            .read_owned()
            .await;
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: owner_id.to_owned(),
        };
        let observed = self.canonical.get(&principal, &session_id).await?;
        let binding_dto: AgentBindingValueDto = serde_json::to_value(
            &observed.session.agent_binding,
        )
        .and_then(serde_json::from_value)
        .map_err(|error| {
            AppError::Conflict(format!(
                "AutoWork AgentSession has an invalid frozen binding: {error}"
            ))
        })?;
        let control_plane = self
            .runtime_control_plane
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                AppError::Conflict(
                    "AutoWork workspace resolution requires the assembled control plane"
                        .to_owned(),
                )
            })?;
        let (saved_binding, _, _) = control_plane
            .saved_binding_artifacts(&UserId::from(owner_id.to_owned()), &binding_dto)
            .await
            .map_err(control_plane_error_to_app)?;
        if saved_binding != observed.session.agent_binding {
            return Err(AppError::Conflict(
                "AutoWork AgentSession binding differs from its exact saved artifacts"
                    .to_owned(),
            ));
        }
        let workspace = self
            .materialize_workspace_for_binding(
                owner_id,
                &session_id,
                &observed.session.agent_binding,
            )
            .await?
            .map(nomifun_requirement::FrozenAutoWorkWorkspace::new)
            .transpose()?;
        let lease: Arc<dyn Send + Sync> =
            Arc::new(std::sync::Mutex::new(Some(operation_guard)));
        Ok(nomifun_requirement::AutoWorkWorkspaceResolution::with_operation_lease(
            workspace,
            lease,
        ))
    }
}

#[async_trait]
impl nomifun_requirement::AutoWorkSessionConfigPort for NomiCoreSessionOwner {
    async fn read_config(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<nomifun_requirement::AutoWorkConfigSnapshot, AppError> {
        let owner = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: owner_id.to_owned(),
        };
        let session_id = AgentSessionId::from(session_id.to_owned());
        self.canonical.get(&owner, &session_id).await?;
        let config = self
            .canonical
            .store()
            .automation_config(&session_id)
            .await
            .map_err(agent_session_store_error)?;
        canonical_autowork_config_snapshot(config)
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
        let expected_revision = parse_canonical_autowork_revision(&command.expected_revision)?;
        let session_id = AgentSessionId::from(command.session_id);
        let config = self
            .canonical
            .store()
            .commit_automation_config(
                nomifun_agent_session::CommitAgentSessionAutomationConfig {
                    agent_session_id: session_id,
                    owner_ref: PrincipalRef {
                        principal_kind: "user".to_owned(),
                        principal_id: command.owner_id,
                    },
                    expected_revision,
                    enabled: command.config.enabled,
                    tag: command.config.tag,
                    max_requirements: command.config.max_requirements,
                    operation_id: command.operation_id,
                    recorded_at: now_ms(),
                },
            )
            .await
            .map_err(agent_session_store_error)?;
        canonical_autowork_config_snapshot(config)
    }
}

#[async_trait]
impl nomifun_companion::CompanionSessionPort for NomiCoreSessionOwner {
    async fn get(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.get_session(owner_id, session_id).await
    }

    async fn create(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
    ) -> Result<ConversationResponse, AppError> {
        let companion_id = request
            .extra
            .get("companion_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::BadRequest(
                "Companion Session requires companion_id".to_owned(),
            ))?
            .to_owned();
        self.create_session_idempotent(
            owner_id,
            request,
            None,
            &format!("companion-session:{companion_id}"),
        )
        .await
    }

    async fn delete(&self, owner_id: &str, session_id: &str) -> Result<(), AppError> {
        let session_id = AgentSessionId::from(session_id.to_owned());
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: owner_id.to_owned(),
        };
        let command = match self
            .canonical
            .fence_delete(
                &principal,
                &session_id,
                &format!("companion-delete:{}", Uuid::now_v7()),
                now_ms(),
            )
            .await?
        {
            PreparedAgentSessionDelete::AlreadyDeleted(_) => {
                self.remove_idmm_state(session_id.as_ref()).await?;
                return Ok(());
            }
            PreparedAgentSessionDelete::Fenced(command) => command,
        };
        self.runtime_sessions
            .terminate_and_wait_result(session_id.as_ref(), Some(nomifun_common::AgentKillReason::ConfigurationChanged))
            .await?;
        let blockers = self.canonical.store().delete_blockers(&session_id).await
            .map_err(agent_session_store_error)?;
        if !blockers.is_empty() {
            return Err(AppError::Conflict(
                "Companion AgentSession deletion remains blocked by unsettled effects"
                    .to_owned(),
            ));
        }
        let deleting_session = self
            .canonical
            .store()
            .get_deleting_session(&session_id)
            .await
            .map_err(agent_session_store_error)?;
        self.remove_workspace_for_binding(
            owner_id,
            &session_id,
            &deleting_session.agent_binding,
        )
        .await?;
        self.canonical.complete_fenced_delete(&command, now_ms()).await?;
        self.remove_idmm_state(session_id.as_ref()).await?;
        Ok(())
    }

    async fn message_local_day_index(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<Vec<nomifun_db::MessageDayBucket>, AppError> {
        let _ = (owner_id, session_id);
        Ok(Vec::new())
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
        let response = <Self as nomifun_channel::ChannelSessionPort>::list_messages(
            self,
            owner_id,
            session_id,
            query,
        )
        .await?;
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
        self.get_session(owner_id, session_id).await?;
        if let Some(runtime) = self.runtime_sessions.get_runtime(session_id) {
            runtime.clear_context().await?;
        }
        Ok(())
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
impl nomifun_agent_execution::AgentExecutionSessionPort for NomiCoreSessionOwner {
    async fn create_idempotent(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.create_session_idempotent(owner_id, request, None, creation_key).await
    }

    async fn create_from_agent_snapshot_idempotent(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        snapshot: AgentResolvedSnapshot,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.create_session_idempotent(owner_id, request, Some(snapshot), creation_key).await
    }

    async fn discard_unlinked_creation(
        &self,
        owner_id: &str,
        creation_key: &str,
    ) -> Result<(), AppError> {
        let scoped = format!("user:{owner_id}:open:{creation_key}");
        let session_id: Option<String> = sqlx::query_scalar(
            "SELECT session_id FROM agent_events \
             WHERE producer_id = 'session_api' AND idempotency_key = ? \
               AND kind = 'session/opening' LIMIT 1",
        )
        .bind(scoped)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        let Some(session_id) = session_id else {
            return Ok(());
        };
        let linked: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM conversation_execution_links \
             WHERE conversation_id = ? AND relation = 'attempt' AND active = 1)",
        )
        .bind(&session_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        if linked != 0 {
            return Err(AppError::Conflict(
                "linked AgentExecution Session cannot be discarded".to_owned(),
            ));
        }
        let session_id = AgentSessionId::from(session_id);
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: owner_id.to_owned(),
        };
        let command = match self
            .canonical
            .fence_delete(
                &principal,
                &session_id,
                &format!("discard-unlinked:{creation_key}"),
                now_ms(),
            )
            .await?
        {
            PreparedAgentSessionDelete::AlreadyDeleted(_) => {
                self.remove_idmm_state(session_id.as_ref()).await?;
                return Ok(());
            }
            PreparedAgentSessionDelete::Fenced(command) => command,
        };
        let deleting_session = self
            .canonical
            .store()
            .get_deleting_session(&session_id)
            .await
            .map_err(agent_session_store_error)?;
        self.remove_workspace_for_binding(
            owner_id,
            &session_id,
            &deleting_session.agent_binding,
        )
        .await?;
        self.canonical.complete_fenced_delete(&command, now_ms()).await?;
        self.remove_idmm_state(session_id.as_ref()).await?;
        Ok(())
    }

    async fn deliver_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        authority: AgentExecutionTurnAuthority,
        request: SendMessageRequest,
    ) -> Result<nomifun_agent_execution::AgentExecutionDelivery, AppError> {
        self.validate_agent_execution_turn_authority(
            owner_id,
            conversation_id,
            &authority,
        )
        .await?;
        self.send_session_message_idempotent(
            owner_id,
            conversation_id,
            operation_id,
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
        Ok(match self
            .session_turn_delivery_state(owner_id, conversation_id, operation_id)
            .await?
        {
            PublicTurnDeliveryState::Missing => None,
            PublicTurnDeliveryState::Accepted { message_id } => Some(
                agent_execution_delivery_from_conversation(IdempotentMessageDelivery {
                    message_id,
                    replayed: true,
                    completed: false,
                    result_ok: None,
                    result_text: None,
                    result_error: None,
                    result_error_code: None,
                    result_error_retryable: None,
                }),
            ),
            PublicTurnDeliveryState::Completed(delivery) => {
                Some(agent_execution_delivery_from_conversation(delivery))
            }
        })
    }

    async fn list_messages(
        &self,
        owner_id: &str,
        conversation_id: &str,
        query: ListMessagesQuery,
    ) -> Result<MessageListResponse, AppError> {
        <Self as nomifun_channel::ChannelSessionPort>::list_messages(
            self,
            owner_id,
            conversation_id,
            query,
        )
        .await
    }

    async fn get(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Result<ConversationResponse, AppError> {
        let agent_session_id = AgentSessionId::from(conversation_id.to_owned());
        nomifun_common::validate_uuidv7(agent_session_id.as_ref()).map_err(|error| {
            AppError::NotFound(format!(
                "AgentSession identity is not canonical UUIDv7: {error}"
            ))
        })?;
        self.materialize_session_workspace(owner_id, &agent_session_id)
            .await?;
        self.canonical_conversation_projection(owner_id, &agent_session_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!(
                "AgentSession {} not found",
                agent_session_id.as_ref(),
            )))
    }

    fn take_turn_tokens(&self, conversation_id: &str) -> Option<i64> {
        let _ = conversation_id;
        None
    }

    async fn cancel_for_execution(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        self.cancel_session(owner_id, conversation_id).await
    }

    async fn steer_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        request: SendMessageRequest,
    ) -> Result<String, AppError> {
        let session_id = AgentSessionId::from(conversation_id.to_owned());
        let receipt = self
            .canonical
            .steer(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner_id.to_owned(),
                },
                &session_id,
                operation_id,
                canonical_turn_input(&request),
            )
            .await?;
        if !receipt.duplicate {
            let turn = self
                .canonical
                .store()
                .read_turn_receipt(&session_id, &receipt.target_operation_id)
                .await
                .map_err(agent_session_store_error)?;
            let started = turn.started_event.ok_or_else(|| AppError::Conflict(
                "AgentExecution steer has no active Turn start".to_owned(),
            ))?;
            let root = match &started.payload {
                nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) => payload
                    .0
                    .get("source_message_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                _ => None,
            }
            .ok_or_else(|| AppError::Conflict(
                "AgentExecution steer has no source message".to_owned(),
            ))?;
            let runtime = self.runtime_sessions.get_runtime(conversation_id).ok_or_else(|| {
                AppError::Conflict("AgentExecution Runtime is not active".to_owned())
            })?;
            let queued = runtime
                .steer_with_receipt(nomifun_ai_agent::RuntimeSteerDelivery {
                    receipt_operation_id: receipt.event_id.as_ref().to_owned(),
                    wire_turn_id: root,
                    turn_generation: started.seq,
                    text: request.content,
                    files: request.files,
                    inject_skills: request.inject_skills,
                })
                .await?;
            if !queued {
                return Err(AppError::Conflict(
                    "AgentExecution Runtime closed steering before delivery".to_owned(),
                ));
            }
        }
        Ok(receipt.event_id.as_ref().to_owned())
    }

    async fn project_assistant_message_idempotent(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        content: &str,
        origin: &str,
    ) -> Result<String, AppError> {
        if content.trim().is_empty() || content.len() > 1024 * 1024 {
            return Err(AppError::BadRequest(
                "projected assistant content must be non-empty and bounded".to_owned(),
            ));
        }
        let session_id = AgentSessionId::from(conversation_id.to_owned());
        self.canonical
            .get(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner_id.to_owned(),
                },
                &session_id,
            )
            .await?;
        let key = format!("agent-execution-projection:{operation_id}");
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT event_id FROM agent_events \
             WHERE session_id = ? AND producer_id = 'session_api' \
               AND idempotency_key = ? AND kind = 'message/assistant-projected'",
        )
        .bind(conversation_id)
        .bind(&key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        if let Some(existing) = existing {
            return Ok(existing);
        }
        let cause: String = sqlx::query_scalar(
            "SELECT event_id FROM agent_events WHERE session_id = ? ORDER BY seq DESC LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        let message_id = Uuid::now_v7().to_string();
        self.canonical
            .store()
            .append_event(&nomifun_agent_contracts::SessionEventAppend {
                agent_session_id: session_id.clone(),
                event_id: nomifun_agent_contracts::EventId::from(message_id.clone()),
                producer_id: nomifun_agent_contracts::EventProducerId::from("session_api"),
                idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(key),
                runtime_binding_id: None,
                runtime_producer_seq: None,
                semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                    kind: nomifun_agent_contracts::SessionEventKind(
                        "message/assistant-projected".to_owned(),
                    ),
                    kind_version: 1,
                    correlation_id: nomifun_agent_contracts::CorrelationId::from(
                        message_id.clone(),
                    ),
                    causation_event_id: Some(nomifun_agent_contracts::EventId::from(cause)),
                    payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                        StrictJsonValue(json!({
                            "content": content,
                            "origin": origin,
                            "operation_id": operation_id,
                        })),
                    ),
                },
            })
            .await
            .map_err(agent_session_store_error)?;
        self.user_events.send_to_user(
            owner_id,
            Self::canonical_projected_assistant_wire_event(
                &session_id,
                &message_id,
                content,
            ),
        );
        Ok(message_id)
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

const SELECTED_WORKSPACE_RESOURCE_PREFIX: &str = "selected-workspace-";

/// Freeze a user-picked host directory into the Session binding without ever
/// accepting client-supplied operations or typed authority. The host validates
/// the directory, derives an opaque resource identity, and keeps every
/// workspace-bearing resource on the same exact canonical root.
fn freeze_selected_workspace(
    binding: &mut AgentBindingValueDto,
    owner_id: &str,
    raw_workspace: &str,
    check: WorkspaceDirectoryCheck,
) -> Result<String, AppError> {
    let canonical = canonical_existing_workspace_directory(
        std::path::Path::new(raw_workspace),
        check,
    )?;
    let canonical = canonical.to_str().ok_or_else(|| match check {
        WorkspaceDirectoryCheck::Create => {
            AppError::WorkspaceDirectoryUnavailable(raw_workspace.to_owned())
        }
        WorkspaceDirectoryCheck::Runtime => {
            AppError::WorkspaceDirectoryRuntimeUnavailable(raw_workspace.to_owned())
        }
    })?;
    let resource_id = format!(
        "{SELECTED_WORKSPACE_RESOURCE_PREFIX}{:x}",
        Sha256::digest(canonical.as_bytes())
    );
    let mut has_workspace_resource = false;

    for resource in &mut binding.typed_resource_bindings {
        if resource.owner_id != owner_id {
            return Err(AppError::Forbidden(
                "AgentSession workspace resource belongs to another owner".to_owned(),
            ));
        }
        match resource.resource_kind.as_str() {
            "workspace" => {
                has_workspace_resource = true;
                resource.resource_id = resource_id.clone();
                resource.binding_id = format!("workspace:{resource_id}");
                resource
                    .typed_parameters
                    .insert("workspace_root".to_owned(), canonical.to_owned());
            }
            "process_session" => {
                resource
                    .typed_parameters
                    .insert("workspace_root".to_owned(), canonical.to_owned());
            }
            _ => {}
        }
    }

    // A project directory is also the process cwd for Agents that do not expose
    // workspace file Actions. Keep a zero-operation workspace identity so the
    // Session can freeze that cwd without manufacturing any file authority.
    if !has_workspace_resource {
        binding.typed_resource_bindings.push(TypedResourceBindingDto {
            binding_id: format!("workspace:{resource_id}"),
            resource_kind: "workspace".to_owned(),
            resource_id,
            owner_id: owner_id.to_owned(),
            operations: BTreeSet::new(),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([(
                "workspace_root".to_owned(),
                canonical.to_owned(),
            )]),
        });
        binding
            .typed_resource_bindings
            .sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
    }

    Ok(canonical.to_owned())
}

fn has_selected_workspace(binding: &AgentBindingValue) -> bool {
    binding.typed_resource_bindings.iter().any(|resource| {
        resource.resource_kind.as_ref() == "workspace"
            && resource
                .resource_id
                .as_ref()
                .starts_with(SELECTED_WORKSPACE_RESOURCE_PREFIX)
    })
}

/// A default Workspace/Process resource is installation-owned, but its
/// physical directory is Session-owned. The immutable binding deliberately
/// freezes only the installation resource identity; the Session ID selects the
/// isolated child directory at execution/projection time.
fn uses_managed_session_workspace(binding: &AgentBindingValue) -> bool {
    if has_selected_workspace(binding) {
        return false;
    }

    binding.typed_resource_bindings.iter().any(|resource| {
        (resource.resource_kind.as_ref() == "workspace"
            && resource.resource_id.as_ref()
                == super::nomi_core_resource_bindings::DEFAULT_WORKSPACE_RESOURCE_ID)
            || (resource.resource_kind.as_ref() == "process_session"
                && resource.resource_id.as_ref()
                    == super::nomi_core_resource_bindings::MANAGED_PROCESS_SESSION_RESOURCE_ID)
    })
}

fn managed_session_workspace_path(
    managed_workspace_root: &std::path::Path,
    session_id: &AgentSessionId,
) -> Result<std::path::PathBuf, AppError> {
    nomifun_common::validate_uuidv7(session_id.as_ref()).map_err(|error| {
        AppError::Conflict(format!(
            "managed Workspace requires a canonical AgentSession UUIDv7: {error}"
        ))
    })?;
    if !managed_workspace_root.is_absolute()
        || nomifun_common::workspace_path_has_edge_whitespace_segment(managed_workspace_root)
    {
        return Err(AppError::Conflict(
            "managed Workspace root is not a canonical absolute path".to_owned(),
        ));
    }
    Ok(managed_workspace_root.join(session_id.as_ref()))
}

fn frozen_workspace_root(
    managed_workspace_root: &std::path::Path,
    owner_id: &str,
    session_id: &AgentSessionId,
    binding: &AgentBindingValue,
) -> Result<Option<String>, AppError> {
    let binding: AgentBindingValueDto = serde_json::to_value(binding)
        .and_then(serde_json::from_value)
        .map_err(|error| {
            AppError::Conflict(format!(
                "canonical AgentSession has an invalid frozen binding: {error}"
            ))
        })?;
    nomifun_agent_execution::resolve_frozen_session_workspace(
        owner_id,
        &binding,
        session_id.as_ref(),
        managed_workspace_root,
    )
}

fn workspace_metadata_is_redirect(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn ensure_plain_managed_directory(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Err(create_error) = std::fs::create_dir(path)
                && create_error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(create_error);
            }
        }
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if workspace_metadata_is_redirect(&metadata) || !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "managed Workspace path is not a real directory",
        ));
    }
    Ok(())
}

async fn materialize_managed_session_workspace(
    managed_workspace_root: &std::path::Path,
    session_id: &AgentSessionId,
) -> Result<String, AppError> {
    let workspace = managed_session_workspace_path(managed_workspace_root, session_id)?;
    let managed_workspace_root = managed_workspace_root.to_path_buf();
    let display = workspace.display().to_string();
    let materialized = tokio::task::spawn_blocking(move || -> std::io::Result<std::path::PathBuf> {
        let work_root = managed_workspace_root.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "managed Workspace root has no installation work-root parent",
            )
        })?;
        ensure_plain_managed_directory(work_root)?;
        ensure_plain_managed_directory(&managed_workspace_root)?;
        ensure_plain_managed_directory(&workspace)?;
        let canonical_root = nomifun_common::paths::canonicalize_simplified(&managed_workspace_root)?;
        let canonical_workspace = nomifun_common::paths::canonicalize_simplified(&workspace)?;
        if canonical_workspace
            .parent()
            .is_none_or(|parent| !nomifun_common::paths::paths_equivalent(parent, &canonical_root))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "managed Workspace escaped its installation-owned root",
            ));
        }
        Ok(canonical_workspace)
    })
    .await
    .map_err(|error| {
        AppError::Internal(format!(
            "managed Workspace materialization task failed for {display}: {error}"
        ))
    })?
    .map_err(|error| {
        AppError::Internal(format!(
            "failed to materialize managed Workspace {display}: {error}"
        ))
    })?;
    Ok(materialized.to_string_lossy().into_owned())
}

async fn remove_managed_session_workspace(
    managed_workspace_root: &std::path::Path,
    session_id: &AgentSessionId,
) -> Result<(), AppError> {
    let workspace = managed_session_workspace_path(managed_workspace_root, session_id)?;
    let managed_workspace_root = managed_workspace_root.to_path_buf();
    let display = workspace.display().to_string();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let root_metadata = match std::fs::symlink_metadata(&managed_workspace_root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if workspace_metadata_is_redirect(&root_metadata) || !root_metadata.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "managed Workspace root is not a real directory",
            ));
        }
        let metadata = match std::fs::symlink_metadata(&workspace) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if workspace_metadata_is_redirect(&metadata) || !metadata.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "managed Workspace path is not a real directory",
            ));
        }
        let canonical_root = nomifun_common::paths::canonicalize_simplified(&managed_workspace_root)?;
        let canonical_workspace = nomifun_common::paths::canonicalize_simplified(&workspace)?;
        if canonical_workspace
            .parent()
            .is_none_or(|parent| !nomifun_common::paths::paths_equivalent(parent, &canonical_root))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "managed Workspace escaped its installation-owned root",
            ));
        }
        std::fs::remove_dir_all(canonical_workspace)
    })
    .await
    .map_err(|error| {
        AppError::Internal(format!(
            "managed Workspace cleanup task failed for {display}: {error}"
        ))
    })?
    .map_err(|error| {
        AppError::Internal(format!(
            "failed to remove managed Workspace {display}: {error}"
        ))
    })
}

/// Derive legacy Conversation workspace presentation from the immutable
/// AgentSession resource identity.
///
/// A non-empty path is not evidence that the user selected a custom workpath:
/// the server-owned `default-workspace` and managed process resource carry the
/// installation authority from which a per-Session directory is derived.
/// Losing that distinction makes a refreshed default Session jump into a path
/// drawer in the desktop sidebar.
/// Keep the old response fields while deriving them from the frozen binding,
/// never from path-shape heuristics.
fn workspace_projection_flags(
    binding: &AgentBindingValue,
    has_workspace: bool,
) -> (bool, bool) {
    if !has_workspace {
        return (false, false);
    }

    let custom_workspace = has_selected_workspace(binding);
    let uses_managed_default = uses_managed_session_workspace(binding);
    let companion_workspace = binding.typed_resource_bindings.iter().any(|resource| {
        matches!(
            resource.resource_kind.as_ref(),
            "companion" | "companion_memory"
        )
    });

    (
        custom_workspace,
        uses_managed_default && !companion_workspace,
    )
}

/// Recover the product-owned Companion identity from the immutable binding.
/// This keeps historical Sessions routable even though presentation-only
/// request extras are not stored in the canonical AgentSession record.
fn companion_id_from_binding(
    binding: &AgentBindingValue,
) -> Result<Option<String>, AppError> {
    let companion_ids = binding
        .typed_resource_bindings
        .iter()
        .filter(|resource| resource.resource_kind.as_ref() == "companion")
        .map(|resource| resource.resource_id.as_ref().to_owned())
        .collect::<BTreeSet<_>>();
    let Some(companion_id) = companion_ids.iter().next().cloned() else {
        return Ok(None);
    };
    if companion_ids.len() != 1
        || binding.typed_resource_bindings.iter().any(|resource| {
            resource.resource_kind.as_ref() == "companion_memory"
                && resource.resource_id.as_ref() != companion_id
        })
    {
        return Err(AppError::Conflict(
            "Companion AgentSession has inconsistent Companion resources".to_owned(),
        ));
    }
    Ok(Some(companion_id))
}

#[cfg(feature = "browser-use")]
fn managed_browser_profile_bindings(
    owner_id: &str,
    binding: &AgentBindingValue,
) -> Result<Vec<nomifun_browser_platform::runtime::BrowserProfileBinding>, AppError> {
    let mut profiles = Vec::new();
    for resource in binding
        .typed_resource_bindings
        .iter()
        .filter(|resource| {
            resource.resource_kind.as_ref()
                == nomifun_browser_platform::product::BROWSER_RESOURCE_KIND
        })
    {
        if resource.owner_id != owner_id {
            return Err(AppError::Forbidden(
                "Browser Resource belongs to another owner".to_owned(),
            ));
        }
        match resource
            .typed_parameters
            .get("provider_kind")
            .map(String::as_str)
        {
            Some("managed") => {
                let profile = match resource
                    .typed_parameters
                    .get("persistence")
                    .map(String::as_str)
                {
                    None | Some("persistent") => {
                        nomifun_browser_platform::runtime::BrowserProfileBinding::persistent(
                            resource.binding_id.as_ref(),
                        )
                    }
                    Some("ephemeral") => {
                        nomifun_browser_platform::runtime::BrowserProfileBinding::ephemeral(
                            resource.binding_id.as_ref(),
                        )
                    }
                    Some(_) => {
                        return Err(AppError::Conflict(
                            "managed Browser Resource has an invalid persistence policy"
                                .to_owned(),
                        ));
                    }
                }
                .map_err(|error| AppError::Conflict(error.to_string()))?;
                profiles.push(profile);
            }
            Some("attached_chrome") => {}
            _ => {
                return Err(AppError::Conflict(
                    "Browser Resource has no canonical provider kind".to_owned(),
                ));
            }
        }
    }
    Ok(profiles)
}

#[cfg(feature = "browser-use")]
fn has_attached_browser_binding(
    owner_id: &str,
    binding: &AgentBindingValue,
) -> Result<bool, AppError> {
    let mut attached = false;
    for resource in binding
        .typed_resource_bindings
        .iter()
        .filter(|resource| {
            resource.resource_kind.as_ref()
                == nomifun_browser_platform::product::BROWSER_RESOURCE_KIND
        })
    {
        if resource.owner_id != owner_id {
            return Err(AppError::Forbidden(
                "Browser Resource belongs to another owner".to_owned(),
            ));
        }
        match resource
            .typed_parameters
            .get("provider_kind")
            .map(String::as_str)
        {
            Some("managed") => {}
            Some("attached_chrome") => attached = true,
            _ => {
                return Err(AppError::Conflict(
                    "Browser Resource has no canonical provider kind".to_owned(),
                ));
            }
        }
    }
    Ok(attached)
}

type ConversationExecutionLinkProjection = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn visible_conversation_execution(
    execution_link: Option<ConversationExecutionLinkProjection>,
) -> (Option<String>, Option<String>, Option<String>) {
    execution_link.map_or(
        (None, None, None),
        |(execution_id, relation, step_id, attempt_id, initial_mode)| {
            // AutoWork uses AgentExecution for durability, but the bound main
            // AgentSession is the work surface. Never project that internal
            // aggregate as a user-created collaboration canvas/transcript.
            if initial_mode.as_deref() == Some("automation") {
                return (None, None, None);
            }
            if relation == "attempt" {
                (Some(execution_id), step_id, attempt_id)
            } else {
                (Some(execution_id), None, None)
            }
        },
    )
}

fn canonical_conversation_response(
    observed: SessionObservation,
    projected: super::agent_binding_projection::SavedAgentBindingProjection,
    workspace: Option<String>,
    created_at: i64,
    execution_link: Option<ConversationExecutionLinkProjection>,
    companion_id: Option<String>,
) -> Result<ConversationResponse, AppError> {
    let SessionObservation { session, head, events, .. } = observed;
    let super::agent_binding_projection::SavedAgentBindingProjection {
        binding,
        projection,
        ..
    } = projected;
    let snapshot = projection.snapshot;
    let mut request = projection.request;
    let extra = request.extra.as_object_mut().ok_or_else(|| {
        AppError::Conflict(
            "canonical Agent projection extra must be a JSON object".to_owned(),
        )
    })?;
    let (custom_workspace, is_temporary_workspace) =
        workspace_projection_flags(&binding, workspace.is_some());
    extra.insert(
        "custom_workspace".to_owned(),
        Value::Bool(custom_workspace),
    );
    extra.insert(
        "is_temporary_workspace".to_owned(),
        Value::Bool(is_temporary_workspace),
    );
    extra.remove("temp_workspace_id");
    if is_temporary_workspace {
        extra.insert(
            "temp_workspace_id".to_owned(),
            Value::String(session.agent_session_id.as_ref().to_owned()),
        );
    }
    if let Some(workspace) = workspace {
        extra.insert("workspace".to_owned(), Value::String(workspace));
    }
    if let Some(companion_id) = companion_id {
        extra.insert("companion_session".to_owned(), Value::Bool(true));
        extra.insert(
            "companion_id".to_owned(),
            Value::String(companion_id.clone()),
        );
        extra.insert(
            "product_agent_target_kind".to_owned(),
            Value::String("companion".to_owned()),
        );
        extra.insert(
            "product_agent_target_id".to_owned(),
            Value::String(companion_id),
        );
    }
    attach_session_metadata_with_fork(
        &mut request.extra,
        &binding,
        session.remote_binding_provenance,
        session.parent_session_id,
        session.fork_base_payload_id,
    )
    .map_err(|error| AppError::Conflict(error.message))?;
    let status = match head.status.as_str() {
        "running" => ConversationStatus::Running,
        "failed" | "open_failed" => ConversationStatus::Finished,
        "opening" => ConversationStatus::Pending,
        _ => ConversationStatus::Finished,
    };
    let active_turn = head.active_turn_id.as_deref().and_then(|operation| {
        events
            .iter()
            .rev()
            .find(|event| {
                event.kind.0 == "turn/started" && event.correlation_id.as_ref() == operation
            })
            .and_then(|event| match &event.payload {
                nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) => payload
                    .0
                    .get("source_message_id")
                    .and_then(Value::as_str)
                    .map(|source| (source.to_owned(), event.seq)),
                _ => None,
            })
    });
    let runtime = Some(ConversationRuntimeSummary {
        state: if head.status == "running" {
            ConversationRuntimeStateKind::Running
        } else {
            ConversationRuntimeStateKind::Idle
        },
        can_send_message: head.status != "running",
        has_runtime: head.status == "running",
        runtime_status: Some(status),
        is_processing: head.status == "running",
        active_turn_id: active_turn.as_ref().map(|(message_id, _)| message_id.clone()),
        processing_started_at: active_turn
            .as_ref()
            .and_then(|(_, seq)| i64::try_from(*seq).ok())
            .map(|seq| created_at.saturating_add(seq)),
    });
    let name = session
        .metadata
        .title
        .clone()
        .or(request.name)
        .unwrap_or_else(|| snapshot.preset_name.clone());
    let (linked_execution_id, execution_step_id, execution_attempt_id) =
        visible_conversation_execution(execution_link);
    Ok(ConversationResponse {
        conversation_id: session.agent_session_id.as_ref().to_owned(),
        name,
        r#type: request.r#type,
        model: request.model,
        reasoning_effort: session
            .metadata
            .reasoning_effort
            .map(session_reasoning_effort_dto),
        status,
        runtime,
        source: request.source,
        pinned: session.metadata.pinned,
        pinned_at: None,
        channel_chat_id: request.channel_chat_id,
        preset_id: Some(snapshot.preset_id.clone()),
        preset_revision: Some(snapshot.preset_revision),
        agent_snapshot: Some(snapshot),
        delegation_policy: request.delegation_policy,
        execution_model_pool: request.execution_model_pool,
        decision_policy: request.decision_policy,
        execution_template_id: request.execution_template_id,
        linked_execution_id,
        execution_step_id,
        execution_attempt_id,
        created_at,
        modified_at: created_at,
        extra: request.extra,
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
    let legacy_skills = || match response.extra.get("skills") {
        None | Some(Value::Null) => Ok(Vec::new()),
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
            .collect::<Result<Vec<_>, _>>(),
        Some(_) => Err(AppError::Conflict(format!(
            "AgentSession {} has an invalid skills projection",
            response.conversation_id
        ))),
    };
    let temp_workspace_id = optional_string("temp_workspace_id");
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
    let (skills, agent_name, custom_agent_id, preset_id, preset_revision) =
        match response.agent_snapshot.as_ref() {
            Some(snapshot) => {
                if response
                    .preset_id
                    .as_deref()
                    .is_some_and(|value| value != snapshot.preset_id)
                    || response
                        .preset_revision
                        .is_some_and(|value| value != snapshot.preset_revision)
                {
                    return Err(AppError::Conflict(format!(
                        "AgentSession {} preset lineage differs from its frozen Snapshot",
                        response.conversation_id
                    )));
                }
                if snapshot
                    .resolved_agent_type
                    .as_deref()
                    .is_some_and(|value| value != response.r#type.serde_name())
                    || snapshot.resolved_model.as_ref().is_some_and(|model| {
                        response.model.as_ref().is_none_or(|selected| {
                            selected.provider_id != model.provider_id
                                || selected.model != model.model
                        })
                    })
                {
                    return Err(AppError::Conflict(format!(
                        "AgentSession {} runtime metadata differs from its frozen Snapshot",
                        response.conversation_id
                    )));
                }
                (
                    snapshot.included_skills.clone(),
                    Some(snapshot.preset_name.clone()),
                    snapshot.resolved_agent_id.clone(),
                    Some(snapshot.preset_id.clone()),
                    Some(snapshot.preset_revision),
                )
            }
            None => (
                legacy_skills()?,
                optional_string("agent_name"),
                optional_string("custom_agent_id"),
                response.preset_id.clone(),
                response.preset_revision,
            ),
        };
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
        preset_id,
        preset_revision,
        agent_snapshot: response.agent_snapshot,
    })
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
    let workspace_binding_lease = nomifun_knowledge::WorkspaceBindingLease::acquire_unbound(
        std::path::Path::new(&workspace),
        conversation_id.clone(),
    )?;

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
            device_mcp_servers: Vec::new(),
            workspace_binding_lease: Some(workspace_binding_lease),
        },
        workspace,
    ))
}

fn canonical_autowork_config_snapshot(
    stored: nomifun_agent_session::AgentSessionAutomationConfig,
) -> Result<nomifun_requirement::AutoWorkConfigSnapshot, AppError> {
    Ok(nomifun_requirement::AutoWorkConfigSnapshot {
        config: nomifun_requirement::AutoWorkConfig {
            enabled: stored.enabled,
            tag: stored.tag,
            max_requirements: stored.max_requirements,
        },
        revision: format!("agent-session:{}", stored.revision),
        operation_id: stored.operation_id,
    })
}

fn parse_canonical_autowork_revision(revision: &str) -> Result<u64, AppError> {
    revision
        .strip_prefix("agent-session:")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            AppError::Conflict(
                "AutoWork config expected_revision is not a canonical AgentSession revision"
                    .to_owned(),
            )
        })
}

fn agent_session_store_error(error: nomifun_agent_session::SessionStoreError) -> AppError {
    match error {
        nomifun_agent_session::SessionStoreError::NotFound(message) => AppError::NotFound(message),
        nomifun_agent_session::SessionStoreError::Deleted(message)
        | nomifun_agent_session::SessionStoreError::Conflict(message)
        | nomifun_agent_session::SessionStoreError::IdempotencyConflict(message) => {
            AppError::Conflict(message)
        }
        nomifun_agent_session::SessionStoreError::InvalidEvent(message)
        | nomifun_agent_session::SessionStoreError::InvalidPayload(message)
        | nomifun_agent_session::SessionStoreError::InvalidSession(message)
        | nomifun_agent_session::SessionStoreError::Registry(message) => {
            AppError::BadRequest(message)
        }
        other => AppError::Internal(other.to_string()),
    }
}

#[cfg(test)]
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
        canonical_autowork_config_snapshot, canonical_message_response,
        canonical_message_response_with_observation, companion_archive_message,
        cron_session_projection_from_response, delete_cleanup_requires_reconciliation,
        freeze_selected_workspace, frozen_workspace_root, has_selected_workspace,
        initial_delivery_requested, materialize_managed_session_workspace, NomiCoreSessionOwner,
        parse_canonical_autowork_revision, session_projection_revision, ssh_teardown_loss,
        remove_managed_session_workspace, visible_conversation_execution, workspace_projection_flags,
        SELECTED_WORKSPACE_RESOURCE_PREFIX,
    };
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_ai_agent::AgentStreamEvent;
    use nomifun_agent_contracts::{
        AgentBindingValue, AgentPresetId, AgentSessionId, DigestHex,
        PresetRevisionRef, ResolvedSnapshotId, ResolvedSnapshotRef,
        ResourceBindingId, ResourceId, ResourceKind, TypedResourceBinding,
    };
    use nomifun_api_types::{AgentBindingValueDto, AgentKnowledgePolicy, AgentResolvedSnapshot, ExecutionModelRef, MessageResponse};
    use nomifun_agent_session::MessageProjection;
    use super::super::history_process_display::HistoricalToolObservation;
    use nomifun_common::{
        AgentType, ConversationSource, ConversationStatus, DecisionPolicy, DelegationPolicy,
        MessagePosition, MessageStatus, MessageType, ProviderWithModel,
    };
    use nomifun_common::paths::WorkspaceDirectoryCheck;
    use serde_json::json;

    const SESSION_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
    const OWNER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";

    #[test]
    fn autowork_execution_is_not_projected_as_collaboration_or_attempt_ui() {
        let execution_id = "0190f5fe-7c00-7a00-8000-000000000099".to_owned();
        assert_eq!(
            visible_conversation_execution(Some((
                execution_id.clone(),
                "automation".to_owned(),
                Some("0190f5fe-7c00-7a00-8000-000000000098".to_owned()),
                Some("0190f5fe-7c00-7a00-8000-000000000097".to_owned()),
                Some("automation".to_owned()),
            ))),
            (None, None, None),
        );
        assert_eq!(
            visible_conversation_execution(Some((
                execution_id.clone(),
                "lead".to_owned(),
                None,
                None,
                Some("explicit".to_owned()),
            ))),
            (Some(execution_id), None, None),
        );
    }

    #[test]
    fn canonical_stream_wire_separates_assistant_segment_from_user_turn_root() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let root_message_id = "0190f5fe-7c00-7a00-8abc-012345678911";
        let assistant_message_id =
            NomiCoreSessionOwner::canonical_assistant_stream_message_id(root_message_id)
                .unwrap();
        let parsed = uuid::Uuid::parse_str(&assistant_message_id).unwrap();
        assert_eq!(parsed.get_version_num(), 7);
        assert_ne!(assistant_message_id, root_message_id);
        assert_eq!(
            assistant_message_id,
            NomiCoreSessionOwner::canonical_assistant_stream_message_id(root_message_id)
                .unwrap()
        );

        let text: AgentStreamEvent = serde_json::from_value(json!({
            "type": "content",
            "data": { "content": "reply" },
        }))
        .unwrap();
        let start = AgentStreamEvent::Start(Default::default());
        let first = NomiCoreSessionOwner::canonical_stream_wire_event(
            &session_id,
            root_message_id,
            &assistant_message_id,
            &start,
        )
        .unwrap();
        let second = NomiCoreSessionOwner::canonical_stream_wire_event(
            &session_id,
            root_message_id,
            &assistant_message_id,
            &text,
        )
        .unwrap();

        assert_eq!(first.name, "message.stream");
        assert_eq!(first.data["msg_id"], assistant_message_id);
        assert_eq!(second.data["msg_id"], assistant_message_id);
        assert_eq!(first.data["turn_id"], root_message_id);
        assert_eq!(second.data["turn_id"], root_message_id);
        assert_eq!(second.data["type"], "content");
        assert_eq!(second.data["data"]["content"], "reply");
    }

    #[test]
    fn failed_turn_summary_rehydrates_as_the_same_compact_error_message() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let root_message_id = "0190f5fe-7c00-7a00-8abc-012345678911";
        let summary_message_id = "0190f5fe-7c00-7a00-8abc-012345678912";
        let message = canonical_message_response(
            &session_id,
            1_000,
            MessageProjection {
                session_id: session_id.clone(),
                projection_id: "turn-summary-test".to_owned(),
                first_seq: 2,
                last_seq: 3,
                presentation_intent: "turn_summary".to_owned(),
                message_type: Some("agent_status".to_owned()),
                message_status: Some("finish".to_owned()),
                projection: json!({
                    "correlation_id": summary_message_id,
                    "presentation_intent": "turn_summary",
                    "state": "failed",
                    "source_message_id": root_message_id,
                    "started_at_ms": 4_000_000,
                    "finished_at_ms": 4_002_000,
                    "error": {
                        "message": "The provider is temporarily unavailable",
                        "code": "USER_LLM_PROVIDER_GATEWAY_ERROR",
                        "ownership": "user_llm_provider",
                        "retryable": true
                    }
                }),
                semantic_digest: "digest".to_owned(),
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(message.r#type, MessageType::Tips);
        assert_eq!(message.position, Some(MessagePosition::Center));
        assert_eq!(message.content["type"], "error");
        assert_eq!(message.content["turn_id"], root_message_id);
        assert_eq!(message.content["started_at_ms"], 4_000_000);
        assert_eq!(message.content["finished_at_ms"], 4_002_000);
        assert_eq!(
            message.content["error"]["code"],
            "USER_LLM_PROVIDER_GATEWAY_ERROR"
        );
        let assistant_message_id =
            NomiCoreSessionOwner::canonical_assistant_stream_message_id(root_message_id)
                .unwrap();
        assert_eq!(message.msg_id.as_deref(), Some(assistant_message_id.as_str()));
    }

    #[test]
    fn history_projects_thinking_and_expandable_tool_content() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let root = "0190f5fe-7c00-7a00-8abc-012345678911";
        let thinking_id = "0190f5fe-7c00-7a00-8abc-012345678914";
        let thinking = canonical_message_response(
            &session_id,
            1_000,
            MessageProjection {
                session_id: session_id.clone(),
                projection_id: format!("thinking:{thinking_id}"),
                first_seq: 3,
                last_seq: 3,
                presentation_intent: "thinking".to_owned(),
                message_type: None,
                message_status: None,
                projection: json!({
                    "correlation_id": thinking_id, "content": "Inspect the workspace",
                    "turn_id": root, "state": "recorded"
                }),
                semantic_digest: "digest".to_owned(),
            },
        ).unwrap().unwrap();
        assert_eq!(thinking.r#type, MessageType::Thinking);
        assert_eq!(thinking.content["content"], "Inspect the workspace");
        assert_eq!(thinking.content["status"], "done");
        assert_eq!(thinking.content["turn_id"], root);

        let tool_id = "0190f5fe-7c00-7a00-8abc-012345678915";
        let tool = canonical_message_response_with_observation(
            &session_id,
            1_000,
            MessageProjection {
                session_id: session_id.clone(),
                projection_id: format!("tool:{tool_id}"),
                first_seq: 4,
                last_seq: 6,
                presentation_intent: "tool".to_owned(),
                message_type: None,
                message_status: None,
                projection: json!({
                    "correlation_id": tool_id, "state": "recorded",
                    "tool_summary": { "call_id": "call-1", "name": "read_file" }
                }),
                semantic_digest: "digest".to_owned(),
            },
            Some(&HistoricalToolObservation {
                turn_id: Some(root.to_owned()),
                args: Some(json!({"path": "src/app.ts"})),
                output: Some("file contents".to_owned()),
                is_error: Some(false),
            }),
        ).unwrap().unwrap();
        assert_eq!(tool.r#type, MessageType::ToolCall);
        assert_eq!(tool.status, Some(MessageStatus::Finish));
        assert_eq!(tool.content["args"]["path"], "src/app.ts");
        assert_eq!(tool.content["output"], "file contents");
        assert_eq!(tool.content["status"], "completed");
        assert_eq!(tool.content["turn_id"], root);
    }

    #[test]
    fn finalized_assistant_projection_is_realtime_without_reopening_a_turn() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let message_id = "0190f5fe-7c00-7a00-8abc-012345678913";
        let wire = NomiCoreSessionOwner::canonical_projected_assistant_wire_event(
            &session_id,
            message_id,
            "terminal synthesis",
        );

        assert_eq!(wire.name, "message.stream");
        assert_eq!(wire.data["conversation_id"], SESSION_ID);
        assert_eq!(wire.data["msg_id"], message_id);
        assert_eq!(wire.data["type"], "content");
        assert_eq!(wire.data["data"]["content"], "terminal synthesis");
        assert_eq!(wire.data["stream_complete"], true);
        assert!(wire.data.get("turn_id").is_none());
    }

    #[test]
    fn canonical_terminal_wire_carries_explicit_idle_runtime_authority() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let root_message_id = "0190f5fe-7c00-7a00-8abc-012345678912";
        let terminal = AgentStreamEvent::Finish(Default::default());
        let wire = NomiCoreSessionOwner::canonical_turn_completed_wire_event(
            &session_id,
            root_message_id,
            &terminal,
        );

        assert_eq!(wire.name, "turn.completed");
        assert_eq!(wire.data["turn_id"], root_message_id);
        assert_eq!(wire.data["status"], "finished");
        assert_eq!(wire.data["state"], "ai_waiting_input");
        assert_eq!(wire.data["can_send_message"], true);
        assert_eq!(wire.data["runtime"]["state"], "idle");
        assert_eq!(wire.data["runtime"]["can_send_message"], true);
        assert_eq!(wire.data["runtime"]["has_runtime"], false);
        assert_eq!(wire.data["runtime"]["runtime_status"], "finished");
        assert_eq!(wire.data["runtime"]["is_processing"], false);
        assert!(wire.data["runtime"]["active_turn_id"].is_null());
    }

    #[test]
    fn canonical_started_wire_carries_processing_authority_and_exact_turn() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let root_message_id = "0190f5fe-7c00-7a00-8abc-012345678913";
        let wire = NomiCoreSessionOwner::canonical_turn_started_wire_event(
            &session_id,
            root_message_id,
        );

        assert_eq!(wire.name, "turn.started");
        assert_eq!(wire.data["turn_id"], root_message_id);
        assert_eq!(wire.data["status"], "running");
        assert_eq!(wire.data["runtime"]["state"], "running");
        assert_eq!(wire.data["runtime"]["is_processing"], true);
        assert_eq!(wire.data["runtime"]["active_turn_id"], root_message_id);
    }

    fn frozen_binding(workspace_root: &str, owner_id: &str) -> AgentBindingValue {
        AgentBindingValue {
            preset_revision_ref: PresetRevisionRef {
                preset_id: AgentPresetId::from("0190f5fe-7c00-7a00-8abc-012345678902"),
                revision: 1,
                revision_digest: DigestHex::from("b".repeat(64)),
            },
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from("snapshot-test"),
                snapshot_digest: DigestHex::from("a".repeat(64)),
            },
            typed_resource_bindings: vec![TypedResourceBinding {
                binding_id: ResourceBindingId::from("workspace-binding"),
                resource_kind: ResourceKind::from("workspace"),
                resource_id: ResourceId::from("default-workspace"),
                owner_id: owner_id.to_owned(),
                operations: BTreeSet::from(["read".to_owned(), "write".to_owned()]),
                connection_config_ref: None,
                typed_parameters: BTreeMap::from([(
                    "workspace_root".to_owned(),
                    workspace_root.to_owned(),
                )]),
            }],
            binding_version: 1,
        }
    }

    #[test]
    fn selected_workspace_is_host_validated_and_frozen_as_an_opaque_resource() {
        let directory = tempfile::tempdir().unwrap();
        let mut binding: AgentBindingValueDto = serde_json::from_value(
            serde_json::to_value(frozen_binding(
                &std::env::temp_dir().to_string_lossy(),
                OWNER_ID,
            ))
            .unwrap(),
        )
        .unwrap();

        let canonical = freeze_selected_workspace(
            &mut binding,
            OWNER_ID,
            &directory.path().to_string_lossy(),
            WorkspaceDirectoryCheck::Create,
        )
        .unwrap();
        let binding: AgentBindingValue = serde_json::from_value(
            serde_json::to_value(binding).unwrap(),
        )
        .unwrap();

        assert!(has_selected_workspace(&binding));
        let managed_root = std::env::temp_dir().join("conversations");
        assert_eq!(
            frozen_workspace_root(
                &managed_root,
                OWNER_ID,
                &AgentSessionId::from(SESSION_ID),
                &binding,
            )
            .unwrap(),
            Some(canonical)
        );
        assert!(binding.typed_resource_bindings[0]
            .resource_id
            .as_ref()
            .starts_with("selected-workspace-"));
    }

    #[test]
    fn canonical_delete_accepts_proven_ssh_teardown_and_defers_unknown_outcome() {
        use nomifun_ssh::SshTeardown;

        assert_eq!(
            ssh_teardown_loss(&[
                SshTeardown::Reaped {
                    detail: "exit status 0".into(),
                },
                SshTeardown::AlreadyDown {
                    detail: "already closed".into(),
                },
            ]),
            None
        );
        assert_eq!(
            ssh_teardown_loss(&[
                SshTeardown::Reaped {
                    detail: "exit status 0".into(),
                },
                SshTeardown::Lost {
                    detail: "no exit evidence".into(),
                },
            ]),
            Some("no exit evidence".into())
        );
    }

    #[test]
    fn canonical_agent_session_routes_cover_the_full_lifecycle() {
        let source = include_str!("nomi_core_session.rs");
        for route in [
            "/api/agent-sessions",
            "/api/agent-sessions/{agent_session_id}/turns",
            "/api/agent-sessions/{agent_session_id}/turns/steer",
            "/api/agent-sessions/{agent_session_id}/turns/cancel",
            "/api/agent-sessions/{agent_session_id}/events",
            "/api/agent-sessions/{agent_session_id}/messages",
            "/api/agent-sessions/{agent_session_id}/slash-commands",
            "/api/agent-sessions/{agent_session_id}/forks",
        ] {
            assert!(source.contains(route), "missing {route}");
        }
        assert!(source.contains(".canonical()"));
        let retired_marker = ["unsupported", "_session_events"].concat();
        assert!(!source.contains(&retired_marker));
    }

    #[test]
    fn initial_delivery_header_is_explicit_and_strict() {
        let headers = HeaderMap::new();
        assert!(!initial_delivery_requested(&headers).unwrap());

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-nomifun-initial-delivery",
            HeaderValue::from_static("1"),
        );
        assert!(initial_delivery_requested(&headers).unwrap());

        headers.insert(
            "x-nomifun-initial-delivery",
            HeaderValue::from_static("true"),
        );
        let invalid = initial_delivery_requested(&headers).unwrap_err();
        assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
        assert_eq!(invalid.code, "INVALID_REQUEST");

        headers.insert(
            "x-nomifun-initial-delivery",
            HeaderValue::from_static("1"),
        );
        headers.append(
            "x-nomifun-initial-delivery",
            HeaderValue::from_static("1"),
        );
        assert_eq!(
            initial_delivery_requested(&headers).unwrap_err().status,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn canonical_agent_session_handlers_do_not_reenter_legacy_conversation_authority() {
        let source = include_str!("nomi_core_session.rs");
        let handlers = source
            .rsplit_once("async fn create_nomi_core_agent_session(")
            .unwrap()
            .1
            .split_once("async fn open_nomi_core_remote(")
            .unwrap()
            .0;
        for retired in [
            "create_session_idempotent",
            "load_owned_nomi_core_session",
            "load_session_from_owner",
            ".service()",
            "request.extra",
        ] {
            assert!(
                !handlers.contains(retired),
                "canonical AgentSession handlers still reach {retired}"
            );
        }
        for handler in [
            "start_nomi_core_agent_session_turn",
            "steer_nomi_core_agent_session_turn",
            "cancel_nomi_core_agent_session_turn",
            "fork_nomi_core_agent_session",
            "delete_nomi_core_agent_session",
        ] {
            assert!(handlers.contains(handler), "missing canonical handler {handler}");
        }
    }

    #[test]
    fn domain_session_ports_use_only_the_canonical_store() {
        let source = include_str!("nomi_core_session.rs");
        let cron = source
            .split_once("impl nomifun_cron::CronSessionPort for NomiCoreSessionOwner")
            .unwrap()
            .1
            .split_once("impl nomifun_channel::ChannelSessionPort")
            .unwrap()
            .0;
        let cron_get = cron
            .split_once("async fn get_session(")
            .unwrap()
            .1
            .split_once("async fn list_conversation_responses_for_cron(")
            .unwrap()
            .0;
        assert!(cron_get.contains("canonical_conversation_projection"));
        assert!(!cron_get.contains(".service"));

        let execution = source
            .split_once(
                "impl nomifun_agent_execution::AgentExecutionSessionPort for NomiCoreSessionOwner",
            )
            .unwrap()
            .1
            .split_once("fn agent_execution_delivery_from_conversation")
            .unwrap()
            .0;
        let get = execution
            .split_once("async fn get(")
            .unwrap()
            .1
            .split_once("fn take_turn_tokens")
            .unwrap()
            .0;
        assert!(get.contains("canonical_conversation_projection"));
        assert!(!get.contains("self.service"));
    }

    #[test]
    fn frozen_workspace_projection_is_session_isolated_and_owner_scoped() {
        let work_root = std::env::temp_dir().join("uarc-canonical-work-root");
        let managed_root = work_root.join("conversations");
        let workspace = work_root.to_string_lossy().into_owned();
        let session_id = AgentSessionId::from(SESSION_ID);
        assert_eq!(
            frozen_workspace_root(
                &managed_root,
                OWNER_ID,
                &session_id,
                &frozen_binding(&workspace, OWNER_ID),
            )
            .unwrap(),
            Some(
                managed_root
                    .join(SESSION_ID)
                    .to_string_lossy()
                    .into_owned()
            )
        );
        let error = frozen_workspace_root(
            &managed_root,
            OWNER_ID,
            &session_id,
            &frozen_binding(
                &std::env::temp_dir().to_string_lossy(),
                "0190f5fe-7c00-7a00-8000-000000000099",
            ),
        )
        .unwrap_err();
        assert!(matches!(error, nomifun_common::AppError::Forbidden(_)));

        let mut no_workspace = frozen_binding(
            &std::env::temp_dir().to_string_lossy(),
            OWNER_ID,
        );
        no_workspace.typed_resource_bindings.clear();
        assert_eq!(
            frozen_workspace_root(&managed_root, OWNER_ID, &session_id, &no_workspace)
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn managed_workspace_stays_below_data_root_and_cleanup_preserves_data() {
        let data_root = tempfile::tempdir().unwrap();
        std::fs::write(data_root.path().join("nomifun-backend.db"), b"database").unwrap();
        std::fs::write(data_root.path().join("encryption_key"), b"secret").unwrap();
        let managed_root = data_root.path().join("conversations");
        let session_id = AgentSessionId::from(SESSION_ID);

        let workspace = std::path::PathBuf::from(
            materialize_managed_session_workspace(&managed_root, &session_id)
                .await
                .unwrap(),
        );
        assert_eq!(workspace.parent(), Some(managed_root.as_path()));
        assert!(workspace.is_dir());
        assert!(
            nomifun_file::list_workspace_level(&workspace, ".", None)
                .unwrap()
                .is_empty(),
            "a new managed Session must not expose data-root siblings",
        );

        std::fs::write(workspace.join("result.txt"), b"owned by Session").unwrap();
        remove_managed_session_workspace(&managed_root, &session_id)
            .await
            .unwrap();
        assert!(!workspace.exists());
        assert!(data_root.path().join("nomifun-backend.db").is_file());
        assert!(data_root.path().join("encryption_key").is_file());
    }

    #[test]
    fn workspace_projection_preserves_managed_default_identity() {
        let root = std::env::temp_dir().to_string_lossy().into_owned();
        let default = frozen_binding(&root, OWNER_ID);
        assert_eq!(
            workspace_projection_flags(&default, true),
            (false, true),
            "a server-owned default workspace must remain in the default workpath"
        );

        let mut custom = default.clone();
        custom.typed_resource_bindings[0].resource_id = ResourceId::from(format!(
            "{SELECTED_WORKSPACE_RESOURCE_PREFIX}project"
        ));
        assert_eq!(workspace_projection_flags(&custom, true), (true, false));

        let mut companion = default;
        companion.typed_resource_bindings.push(TypedResourceBinding {
            binding_id: ResourceBindingId::from("companion-binding"),
            resource_kind: ResourceKind::from("companion"),
            resource_id: ResourceId::from("companion-1"),
            owner_id: OWNER_ID.to_owned(),
            operations: BTreeSet::from(["read".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        });
        assert_eq!(
            workspace_projection_flags(&companion, true),
            (false, false),
            "a permanent Companion workspace is managed but not temporary"
        );
        assert_eq!(workspace_projection_flags(&custom, false), (false, false));
    }

    #[test]
    fn cron_projection_uses_frozen_snapshot_metadata_not_legacy_extra() {
        let snapshot = AgentResolvedSnapshot {
            canonical_binding: None,
            preset_id: "0190f5fe-7c00-7a00-8abc-012345678902".to_owned(),
            preset_revision: 7,
            preset_name: "Frozen Agent".to_owned(),
            routing_description: None,
            instructions: "frozen".to_owned(),
            resolved_agent_id: Some("0190f5fe-7c00-7a00-8abc-012345678903".to_owned()),
            resolved_agent_type: Some("nomi".to_owned()),
            resolved_agent_backend: Some("nomi".to_owned()),
            resolved_model: Some(ExecutionModelRef {
                provider_id: "0190f5fe-7c00-7a00-8000-000000000002".to_owned(),
                model: "step-3.7-flash".to_owned(),
            }),
            included_skills: vec!["frozen-skill".to_owned()],
            excluded_auto_skills: Vec::new(),
            enabled_capabilities: Vec::new(),
            enabled_capability_actions: BTreeMap::new(),
            required_resource_kinds: BTreeSet::new(),
            knowledge_policy: AgentKnowledgePolicy::default(),
            warnings: Vec::new(),
        };
        let response = super::ConversationResponse {
            conversation_id: SESSION_ID.to_owned(),
            name: "Session".to_owned(),
            r#type: AgentType::Nomi,
            model: Some(ProviderWithModel {
                provider_id: "0190f5fe-7c00-7a00-8000-000000000002".to_owned(),
                model: "step-3.7-flash".to_owned(),
                use_model: None,
            }),
            reasoning_effort: None,
            status: ConversationStatus::Pending,
            runtime: None,
            source: None,
            pinned: false,
            pinned_at: None,
            channel_chat_id: None,
            preset_id: None,
            preset_revision: None,
            agent_snapshot: Some(snapshot),
            delegation_policy: DelegationPolicy::Automatic,
            execution_model_pool: None,
            decision_policy: DecisionPolicy::Automatic,
            execution_template_id: None,
            linked_execution_id: None,
            execution_step_id: None,
            execution_attempt_id: None,
            created_at: 0,
            modified_at: 0,
            extra: json!({
                "workspace": std::env::temp_dir().to_string_lossy(),
                "skills": ["forged-skill"],
                "agent_name": "Forged Agent",
                "custom_agent_id": "0190f5fe-7c00-7a00-8abc-012345678999",
            }),
        };
        let projection =
            cron_session_projection_from_response(OWNER_ID, response, None).unwrap();
        assert_eq!(projection.skills, vec!["frozen-skill"]);
        assert_eq!(projection.agent_name.as_deref(), Some("Frozen Agent"));
        assert_eq!(
            projection.custom_agent_id.as_deref(),
            Some("0190f5fe-7c00-7a00-8abc-012345678903")
        );
        assert_eq!(
            projection.model.as_ref().map(|model| model.model.as_str()),
            Some("step-3.7-flash")
        );
        assert_eq!(projection.preset_revision, Some(7));
    }

    #[test]
    fn canonical_delete_fences_admission_then_closes_exact_resources_before_tombstone() {
        let source = include_str!("nomi_core_session.rs");
        let handler = source
            .rsplit_once("async fn delete_nomi_core_agent_session(")
            .unwrap()
            .1
            .split_once("async fn open_nomi_core_remote(")
            .unwrap()
            .0;
        let fence = handler
            .find(".fence_delete(")
            .expect("canonical delete must fence Store admission");
        let runtime = handler
            .find(".terminate_agent_session_runtime_before_delete_fence(")
            .expect("canonical delete must prove Runtime teardown before fencing the Store");
        let cleanup = handler
            .find("cleanup_agent_session_resources_before_delete")
            .expect("canonical delete must run exact resource cleanup");
        let blockers = handler
            .match_indices(".delete_blockers(")
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let tombstone = handler
            .find(".complete_fenced_delete(")
            .expect("canonical delete must write the Store tombstone");
        assert!(handler.contains("tokio::spawn(async move"));
        assert!(handler.contains("quiesce_agent_session_execution_before_delete"));
        assert!(runtime < fence, "Runtime teardown must retain live Store effect authority");
        assert!(blockers.len() >= 2, "delete must check blockers before and after cleanup");
        assert!(blockers.iter().any(|index| fence < *index && *index < cleanup));
        assert!(
            blockers
                .iter()
                .any(|index| cleanup < *index && *index < tombstone),
            "resource cleanup must be followed by a final blocker check"
        );
        assert!(cleanup < tombstone, "cleanup must precede the tombstone");

        let cleanup_owner = source
            .rsplit_once("async fn cleanup_agent_session_resources_before_delete(")
            .unwrap()
            .1
            .split_once("fn ssh_teardown_loss(")
            .unwrap()
            .0;
        for exact_cleanup in [
            ".retire_agent_session(agent_session_id)",
            ".delete_agent_session(owner_id, agent_session_id, &bindings)",
            ".close_agent_session(owner_id, agent_session_id)",
            ".delete_jobs_by_agent_session(owner_id, agent_session_id)",
            ".clear_owner_for_session(",
            ".record_resource_cleanup_started(&session_id, \"ssh\")",
            ".close_conversation(agent_session_id)",
            ".record_resource_cleanup_succeeded(&session_id, \"ssh\")",
        ] {
            assert!(
                cleanup_owner.contains(exact_cleanup),
                "missing exact AgentSession cleanup {exact_cleanup}"
            );
        }
        assert!(
            !cleanup_owner.contains("delete_binding(\"conversation\"")
                && !cleanup_owner.contains("knowledge_service"),
            "canonical AgentSession deletion must not depend on the retired mutable Knowledge binding side channel"
        );
        assert!(cleanup_owner.contains("record_resource_cleanup_uncertain"));
        assert!(cleanup_owner.contains("acknowledge_persisted_agent_session_teardowns"));
    }

    #[test]
    fn pending_cleanup_reenters_owner_but_terminal_gate_stays_closed() {
        let pending = nomifun_agent_session::AgentSessionDeleteBlockers {
            effects: Vec::new(),
            resource_cleanup_pending: vec!["ssh".to_owned()],
            resource_cleanup_uncertainties: Vec::new(),
        };
        assert!(!delete_cleanup_requires_reconciliation(&pending));
        assert!(!pending.is_empty(), "pending cleanup must still block tombstone");

        let uncertain = nomifun_agent_session::AgentSessionDeleteBlockers {
            effects: Vec::new(),
            resource_cleanup_pending: Vec::new(),
            resource_cleanup_uncertainties: vec![
                nomifun_agent_session::ResourceCleanupUncertainty {
                    owner_domain: "ssh".to_owned(),
                    recorded_at: 1,
                },
            ],
        };
        assert!(delete_cleanup_requires_reconciliation(&uncertain));
    }

    #[test]
    fn canonical_autowork_config_uses_store_revision_and_operation_identity() {
        let snapshot = canonical_autowork_config_snapshot(
            nomifun_agent_session::AgentSessionAutomationConfig {
                enabled: true,
                tag: Some("release".to_owned()),
                max_requirements: Some(3),
                revision: 7,
                operation_id: Some("gateway:request-7".to_owned()),
            },
        )
        .unwrap();
        assert_eq!(snapshot.revision, "agent-session:7");
        assert_eq!(
            snapshot.operation_id.as_deref(),
            Some("gateway:request-7")
        );
        assert_eq!(parse_canonical_autowork_revision(&snapshot.revision).unwrap(), 7);
        assert!(parse_canonical_autowork_revision("conversation:7").is_err());
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
            reasoning_effort: None,
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
    runtime_sessions: &Arc<dyn AgentRuntimeSessions>,
    session_id: &str,
) -> Option<broadcast::Receiver<AgentStreamEvent>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(handle) = runtime_sessions.get_runtime(session_id) {
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

/// Namespace for compatibility-shaped metadata derived from the immutable
/// canonical binding. It is a projection only, never a second persistence or
/// lifecycle authority.
const NOMI_CORE_SESSION_METADATA_KEY: &str = "nomi_core_session";
const NOMI_CORE_SESSION_METADATA_VERSION: u64 = 1;
const NOMI_CORE_SESSION_KIND: &str = "agent_session";
const NOMI_CORE_REMOTE_KIND: &str = "remote_session";
const NOMI_CORE_MESSAGE_PAGE_SIZE: u32 = 100;
const NOMI_CORE_REMOTE_TURN_FINALIZER_TIMEOUT: Duration = Duration::from_secs(90);
const NOMI_CORE_REMOTE_TURN_FINALIZER_POLL: Duration = Duration::from_millis(100);
const NOMI_CORE_REMOTE_INITIAL_COMMAND_TIMEOUT: Duration = Duration::from_secs(150);
const NOMI_CORE_REMOTE_TURN_COMMAND_TIMEOUT: Duration = Duration::from_secs(150);
const NOMI_CORE_REMOTE_CANCEL_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// State shared by the app-local Agent Settings, AgentSession, and Remote
/// route builders.
///
/// `session_owner` is the same object that the desktop, Channel,
/// Cron, AutoWork, Companion, and AgentExecution wiring receives.  The
/// adapter never constructs a second Runtime registry or Session Store.
#[derive(Clone)]
pub(crate) struct NomiCoreAgentApiState {
    authoritative_user_id: Arc<str>,
    product_agent_resolver: Arc<NomiCoreProductAgentResolver>,
    skill_discovery: Arc<NomiCorePluginToolSessionProvider>,
    pub(crate) session_owner: Arc<NomiCoreSessionOwner>,
    pub(crate) control_plane: Arc<AgentControlPlane>,
    pub(crate) remote_repository: Arc<dyn IRemoteBindingRepository>,
    pub(crate) remote_runtime: super::remote_runtime::NomiCoreRemoteRuntimeCoordinator,
    pub(crate) resource_bindings:
        super::nomi_core_resource_bindings::NomiCoreResourceBindingResolverRegistry,
    pub(crate) mcp_server_repository: Arc<dyn nomifun_db::IMcpServerRepository>,
    pub(crate) wave4_owners: Arc<super::nomi_core_wave4::NomiCoreWave4Owners>,
    pub(crate) wave5_owner: Arc<super::agent_wave5_host::NomiCoreWave5Host>,
    ssh_pool: nomifun_ssh::SshConnectionPool,
    cron_cleanup_owner:
        Arc<std::sync::OnceLock<Arc<nomifun_cron::service::CronService>>>,
    requirement_cleanup_owner:
        Arc<std::sync::OnceLock<Arc<nomifun_requirement::RequirementService>>>,
    autowork_cleanup_owner:
        Arc<std::sync::OnceLock<Arc<nomifun_requirement::AutoWorkRunner>>>,
    #[cfg(feature = "browser-use")]
    browser_resources:
        Option<Arc<nomifun_browser_platform::workspace::BrowserResourceService>>,
    #[cfg(feature = "browser-use")]
    attached_chrome: Option<Arc<crate::AttachedChromeProviderService>>,
    delete_cleanup_locks:
        Arc<DashMap<(String, String), Arc<tokio::sync::Mutex<()>>>>,
}

impl NomiCoreAgentApiState {
    pub(crate) fn new(
        authoritative_user_id: Arc<str>,
        session_owner: Arc<NomiCoreSessionOwner>,
        control_plane: Arc<AgentControlPlane>,
        remote_repository: Arc<dyn IRemoteBindingRepository>,
        remote_runtime: super::remote_runtime::NomiCoreRemoteRuntimeCoordinator,
        resource_bindings: super::nomi_core_resource_bindings::NomiCoreResourceBindingResolverRegistry,
        mcp_server_repository: Arc<dyn nomifun_db::IMcpServerRepository>,
        wave4_owners: Arc<super::nomi_core_wave4::NomiCoreWave4Owners>,
        wave5_owner: Arc<super::agent_wave5_host::NomiCoreWave5Host>,
        product_agent_resolver: Arc<NomiCoreProductAgentResolver>,
        skill_discovery: Arc<NomiCorePluginToolSessionProvider>,
        ssh_pool: nomifun_ssh::SshConnectionPool,
        #[cfg(feature = "browser-use")]
        browser_resources: Option<
            Arc<nomifun_browser_platform::workspace::BrowserResourceService>,
        >,
        #[cfg(feature = "browser-use")]
        attached_chrome: Option<Arc<crate::AttachedChromeProviderService>>,
    ) -> Self {
        Self {
            authoritative_user_id,
            product_agent_resolver,
            skill_discovery,
            session_owner,
            control_plane,
            remote_repository,
            remote_runtime,
            resource_bindings,
            mcp_server_repository,
            wave4_owners,
            wave5_owner,
            ssh_pool,
            cron_cleanup_owner: Arc::new(std::sync::OnceLock::new()),
            requirement_cleanup_owner: Arc::new(std::sync::OnceLock::new()),
            autowork_cleanup_owner: Arc::new(std::sync::OnceLock::new()),
            #[cfg(feature = "browser-use")]
            browser_resources,
            #[cfg(feature = "browser-use")]
            attached_chrome,
            delete_cleanup_locks: Arc::new(DashMap::new()),
        }
    }

    pub(crate) fn install_cron_cleanup_owner(
        &self,
        service: Arc<nomifun_cron::service::CronService>,
    ) -> Result<(), &'static str> {
        self.cron_cleanup_owner
            .set(service)
            .map_err(|_| "Cron cleanup owner is already installed")
    }

    pub(crate) fn install_requirement_cleanup_owner(
        &self,
        service: Arc<nomifun_requirement::RequirementService>,
        runner: Arc<nomifun_requirement::AutoWorkRunner>,
    ) -> Result<(), &'static str> {
        self.requirement_cleanup_owner
            .set(service)
            .map_err(|_| "Requirement cleanup owner is already installed")?;
        self.autowork_cleanup_owner
            .set(runner)
            .map_err(|_| "AutoWork cleanup owner is already installed")
    }

    async fn quiesce_agent_session_execution_before_delete(
        &self,
        agent_session_id: &str,
    ) -> Result<(), NomiCoreApiError> {
        let runner = self.autowork_cleanup_owner.get().ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "AGENT_SESSION_AUTOWORK_CLEANUP_UNAVAILABLE",
                "AutoWork cleanup owner is not installed",
            )
        })?;
        runner
            .stop_for_session_delete(agent_session_id)
            .await
            .map_err(|error| {
                NomiCoreApiError::new(
                    StatusCode::CONFLICT,
                    "AGENT_SESSION_AUTOWORK_CLEANUP_FAILED",
                    format!(
                        "AutoWork/AgentExecution cleanup failed after AgentSession deletion was fenced: {error}"
                    ),
                )
            })
    }

    async fn terminate_agent_session_runtime_before_delete_fence(
        &self,
        agent_session_id: &str,
    ) -> Result<(), NomiCoreApiError> {
        if self
            .session_owner
            .runtime_sessions
            .has_owned_runtime(agent_session_id)
        {
            self.session_owner
                .runtime_sessions
                .terminate_and_wait_result(
                    agent_session_id,
                    Some(AgentKillReason::ConversationDeleted),
                )
                .await
                .map_err(|error| {
                    NomiCoreApiError::new(
                        StatusCode::CONFLICT,
                        "AGENT_SESSION_RUNTIME_CLEANUP_FAILED",
                        format!(
                            "Agent Runtime teardown failed before AgentSession deletion could be fenced: {error}"
                        ),
                    )
                })?;
        }
        Ok(())
    }

    fn delete_cleanup_lock(
        &self,
        owner_id: &str,
        agent_session_id: &str,
    ) -> Arc<tokio::sync::Mutex<()>> {
        self.delete_cleanup_locks
            .entry((owner_id.to_owned(), agent_session_id.to_owned()))
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    async fn cleanup_agent_session_resources_before_delete(
        &self,
        owner_id: &str,
        agent_session_id: &str,
    ) -> Result<(), NomiCoreApiError> {
        // This synchronous pool fence must be the first operation after the
        // durable Store fence. A cancelled HTTP future cannot leave direct
        // SshBackend holders able to admit new work against a deleting Session.
        self.ssh_pool.retire_agent_session(agent_session_id);
        let session_id = AgentSessionId::from(agent_session_id.to_owned());
        let deleting_session = self
            .session_owner
            .canonical()
            .store()
            .get_deleting_session(&session_id)
            .await
            .map_err(agent_session_store_error)?;
        if deleting_session.owner_ref.principal_id != owner_id {
            return Err(AppError::Forbidden(
                "deleting AgentSession belongs to another owner".to_owned(),
            )
            .into());
        }
        let existing_blockers = self
            .session_owner
            .canonical()
            .store()
            .delete_blockers(&session_id)
            .await
            .map_err(|error| AppError::Internal(format!(
                "inspect AgentSession delete blockers: {error}"
            )))?;
        let ssh_cleanup_overridden = self
            .session_owner
            .canonical()
            .store()
            .deletion_audits(&deleting_session.owner_ref, &session_id)
            .await
            .map_err(agent_session_store_error)?
            .iter()
            .any(|audit| {
                audit.target_kind == "resource_cleanup" && audit.target_id == "ssh"
            });
        if existing_blockers
            .resource_cleanup_uncertainties
            .iter()
            .any(|uncertainty| uncertainty.owner_domain == "ssh")
        {
            self.ssh_pool
                .acknowledge_persisted_agent_session_teardowns(agent_session_id);
            return Err(agent_session_cleanup_uncertain(agent_session_id));
        }

        #[cfg(feature = "browser-use")]
        {
            let bindings = managed_browser_profile_bindings(
                owner_id,
                &deleting_session.agent_binding,
            )?;
            if let Some(resources) = &self.browser_resources {
                resources
                    .delete_agent_session(owner_id, agent_session_id, &bindings)
                    .await
                    .map_err(|error| {
                        NomiCoreApiError::new(
                            StatusCode::CONFLICT,
                            "AGENT_SESSION_BROWSER_CLEANUP_FAILED",
                            format!(
                                "Browser Resource cleanup failed before AgentSession deletion: {error}"
                            ),
                        )
                    })?;
            } else if !bindings.is_empty() {
                return Err(NomiCoreApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "AGENT_SESSION_BROWSER_CLEANUP_UNAVAILABLE",
                    "Managed Browser profile cleanup owner is unavailable",
                ));
            }
        }
        #[cfg(not(feature = "browser-use"))]
        if deleting_session
            .agent_binding
            .typed_resource_bindings
            .iter()
            .any(|resource| resource.resource_kind.as_ref() == "browser")
        {
            return Err(NomiCoreApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "AGENT_SESSION_BROWSER_CLEANUP_UNAVAILABLE",
                "Browser cleanup is unavailable in this host build",
            ));
        }
        #[cfg(feature = "browser-use")]
        {
            let has_attached = has_attached_browser_binding(
                owner_id,
                &deleting_session.agent_binding,
            )?;
            if let Some(attached) = &self.attached_chrome {
                attached
                    .close_agent_session(owner_id, agent_session_id)
                    .await
                    .map_err(|error| {
                        NomiCoreApiError::new(
                            StatusCode::CONFLICT,
                            "AGENT_SESSION_ATTACHED_BROWSER_CLEANUP_FAILED",
                            format!(
                                "Attached Browser cleanup failed before AgentSession deletion: {error}"
                            ),
                        )
                    })?;
            } else if has_attached {
                return Err(NomiCoreApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "AGENT_SESSION_ATTACHED_BROWSER_CLEANUP_UNAVAILABLE",
                    "Attached Browser cleanup owner is unavailable",
                ));
            }
        }

        let cron = self.cron_cleanup_owner.get().ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "AGENT_SESSION_SCHEDULE_CLEANUP_UNAVAILABLE",
                "Schedule cleanup owner is not installed",
            )
        })?;
        cron.delete_jobs_by_agent_session(owner_id, agent_session_id)
            .await
            .map_err(|error| {
                NomiCoreApiError::new(
                    StatusCode::CONFLICT,
                    "AGENT_SESSION_SCHEDULE_CLEANUP_FAILED",
                    format!(
                        "Schedule cleanup failed after AgentSession deletion was fenced: {error}"
                    ),
                )
            })?;

        let requirements = self.requirement_cleanup_owner.get().ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "AGENT_SESSION_REQUIREMENT_CLEANUP_UNAVAILABLE",
                "Requirement cleanup owner is not installed",
            )
        })?;
        requirements
            .clear_owner_for_session(
                agent_session_id,
                nomifun_api_types::AutoWorkTargetKind::Conversation,
            )
            .await
            .map_err(|error| {
                NomiCoreApiError::new(
                    StatusCode::CONFLICT,
                    "AGENT_SESSION_REQUIREMENT_CLEANUP_FAILED",
                    format!(
                        "Requirement owner cleanup failed after AgentSession deletion was fenced: {error}"
                    ),
                )
            })?;

        if ssh_cleanup_overridden {
            // Manual risk acceptance is not physical cleanup proof. Do not
            // rewrite the retained unknown outcome as `succeeded`; the durable
            // non-private audit fact is the sole authority allowing deletion.
            self.session_owner
                .remove_workspace_for_binding(
                    owner_id,
                    &session_id,
                    &deleting_session.agent_binding,
                )
                .await?;
            self.ssh_pool
                .acknowledge_persisted_agent_session_teardowns(agent_session_id);
            return Ok(());
        }

        self.session_owner
            .canonical()
            .store()
            .record_resource_cleanup_started(&session_id, "ssh")
            .await
            .map_err(|error| AppError::Internal(format!(
                "persist SSH cleanup start: {error}"
            )))?;
        let teardowns = self.ssh_pool.close_conversation(agent_session_id).await;
        if ssh_teardown_loss(&teardowns).is_some() {
            self.session_owner
                .canonical()
                .store()
                .record_resource_cleanup_uncertain(&session_id, "ssh", now_ms())
                .await
                .map_err(|error| AppError::Internal(format!(
                    "persist SSH cleanup uncertainty: {error}"
                )))?;
            self.ssh_pool
                .acknowledge_persisted_agent_session_teardowns(agent_session_id);
            return Err(agent_session_cleanup_uncertain(agent_session_id));
        }
        self.session_owner
            .canonical()
            .store()
            .record_resource_cleanup_succeeded(&session_id, "ssh")
            .await
            .map_err(|error| AppError::Internal(format!(
                "persist SSH cleanup success: {error}"
            )))?;
        self.ssh_pool
            .acknowledge_persisted_agent_session_teardowns(agent_session_id);
        self.session_owner
            .remove_workspace_for_binding(
                owner_id,
                &session_id,
                &deleting_session.agent_binding,
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn recover_deleting_agent_sessions(&self) -> Result<(), AppError> {
        let sessions = self
            .session_owner
            .canonical()
            .store()
            .list_deleting_sessions()
            .await
            .map_err(agent_session_store_error)?;
        for session in sessions {
            let session_id = session.agent_session_id;
            let owner = session.owner_ref;
            let cleanup_lock =
                self.delete_cleanup_lock(&owner.principal_id, session_id.as_ref());
            let _cleanup = cleanup_lock.lock().await;
            let _operation_fence = self
                .session_owner
                .session_operation_lock(session_id.as_ref())
                .write_owned()
                .await;

            // No request future owns startup recovery. Retire direct SSH
            // holders before the first awaited Store classification.
            self.ssh_pool.retire_agent_session(session_id.as_ref());
            if let Err(error) = self
                .quiesce_agent_session_execution_before_delete(
                    session_id.as_ref(),
                )
                .await
            {
                tracing::warn!(
                    agent_session_id = session_id.as_ref(),
                    code = %error.code,
                    message = %error.message,
                    "canonical AgentSession execution cleanup remains fenced"
                );
                continue;
            }
            self.session_owner
                .canonical()
                .store()
                .quarantine_pending_effects_for_delete(&owner, &session_id, now_ms())
                .await
                .map_err(agent_session_store_error)?;
            self.session_owner
                .canonical()
                .store()
                .quarantine_pending_resource_cleanups_for_delete(
                    &owner,
                    &session_id,
                    now_ms(),
                )
                .await
                .map_err(agent_session_store_error)?;

            let blockers = self
                .session_owner
                .canonical()
                .store()
                .delete_blockers(&session_id)
                .await
                .map_err(agent_session_store_error)?;
            if !blockers.is_empty() {
                tracing::warn!(
                    agent_session_id = session_id.as_ref(),
                    effect_blockers = blockers.effects.len(),
                    pending_resource_cleanups = blockers.resource_cleanup_pending.len(),
                    unknown_resource_cleanups = blockers.resource_cleanup_uncertainties.len(),
                    "canonical AgentSession delete recovery awaits reconciliation"
                );
                continue;
            }

            if let Err(error) = self
                .cleanup_agent_session_resources_before_delete(
                    &owner.principal_id,
                    session_id.as_ref(),
                )
                .await
            {
                tracing::warn!(
                    agent_session_id = session_id.as_ref(),
                    code = %error.code,
                    message = %error.message,
                    "canonical AgentSession delete recovery remains fenced"
                );
                continue;
            }
            let blockers = self
                .session_owner
                .canonical()
                .store()
                .delete_blockers(&session_id)
                .await
                .map_err(agent_session_store_error)?;
            if !blockers.is_empty() {
                tracing::warn!(
                    agent_session_id = session_id.as_ref(),
                    effect_blockers = blockers.effects.len(),
                    pending_resource_cleanups = blockers.resource_cleanup_pending.len(),
                    unknown_resource_cleanups = blockers.resource_cleanup_uncertainties.len(),
                    "canonical AgentSession delete recovery awaits reconciliation"
                );
                continue;
            }
            let command = DeleteAgentSessionCommand {
                operation_id: OperationId::from(format!(
                    "delete-recovery:{}",
                    session_id.as_ref()
                )),
                agent_session_id: session_id.clone(),
                owner_ref: owner.clone(),
                requested_at: 0,
            };
            self.session_owner
                .canonical()
                .complete_fenced_delete(&command, now_ms())
                .await?;
            self.session_owner
                .remove_idmm_state(session_id.as_ref())
                .await?;
            if let Err(error) = self
                .wave4_owners
                .release_session(&owner.principal_id, session_id.as_ref())
                .await
            {
                tracing::warn!(
                    agent_session_id = session_id.as_ref(),
                    code = error.code,
                    "Wave 4 recovery cleanup deferred to orphan reconciliation"
                );
            }
        }
        Ok(())
    }

}

/// Gateway adapter over the one canonical AgentSession owner. The historical
/// Gateway capability names remain temporarily registered until UARC-053, but
/// no operation can read or mutate the retired Conversation Agent store.
#[derive(Clone)]
pub(crate) struct GatewayAgentSessionCapabilityPort {
    state: NomiCoreAgentApiState,
}

impl GatewayAgentSessionCapabilityPort {
    pub(crate) fn new(state: NomiCoreAgentApiState) -> Self {
        Self { state }
    }

    async fn projected_messages(
        &self,
        user_id: &str,
        session_id: &str,
        query: ListMessagesQuery,
    ) -> Result<MessageListResponse, AppError> {
        nomifun_channel::ChannelSessionPort::list_messages(
            self.state.session_owner.as_ref(),
            user_id,
            session_id,
            query,
        )
        .await
    }
}

fn gateway_delivery(
    delivery: IdempotentMessageDelivery,
) -> nomifun_gateway::ConversationDeliveryReceipt {
    nomifun_gateway::ConversationDeliveryReceipt {
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

#[async_trait]
impl nomifun_gateway::ConversationCapabilityPort for GatewayAgentSessionCapabilityPort {
    async fn list(
        &self,
        user_id: &str,
        query: nomifun_api_types::ListConversationsQuery,
        exclude_companion_companion: bool,
    ) -> Result<ConversationListResponse, AppError> {
        if user_id != self.state.authoritative_user_id.as_ref() {
            return Err(AppError::Forbidden("AgentSession owner mismatch".to_owned()));
        }
        let limit = query.limit.unwrap_or(50).clamp(1, 10_000);
        let page = self
            .state
            .session_owner
            .canonical()
            .store()
            .list_live_sessions(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: user_id.to_owned(),
                },
                query.cursor.as_deref(),
                10_000,
            )
            .await
            .map_err(agent_session_store_error)?;
        let mut projected = Vec::new();
        for item in page.items {
            let Some(conversation) = self
                .state
                .session_owner
                .canonical_conversation_projection(
                    user_id,
                    &item.session.agent_session_id,
                )
                .await?
            else {
                continue;
            };
            if query.source.as_deref().is_some_and(|source| {
                conversation.source.as_ref().map(|value| match value {
                    nomifun_common::ConversationSource::Nomifun => "nomifun",
                    nomifun_common::ConversationSource::Telegram => "telegram",
                    nomifun_common::ConversationSource::Lark => "lark",
                    nomifun_common::ConversationSource::Dingtalk => "dingtalk",
                    nomifun_common::ConversationSource::Weixin => "weixin",
                }) != Some(source)
            }) || query.cron_job_id.as_deref().is_some_and(|job| {
                conversation.extra.get("cron_job_id").and_then(Value::as_str) != Some(job)
            }) || query.pinned.is_some_and(|pinned| conversation.pinned != pinned)
                || (exclude_companion_companion
                    && conversation
                        .extra
                        .get("companion_session")
                        .and_then(Value::as_bool)
                        == Some(true))
            {
                continue;
            }
            projected.push(conversation);
        }
        let total = projected.len() as u64;
        let has_more = projected.len() > limit as usize;
        projected.truncate(limit as usize);
        Ok(PaginatedResult {
            items: projected,
            total,
            has_more,
        })
    }

    async fn runtime_summary_for(&self, conversation_id: &str) -> ConversationRuntimeSummary {
        self.state
            .session_owner
            .get_session(self.state.authoritative_user_id.as_ref(), conversation_id)
            .await
            .ok()
            .and_then(|conversation| conversation.runtime)
            .unwrap_or(ConversationRuntimeSummary {
                state: ConversationRuntimeStateKind::Idle,
                can_send_message: true,
                has_runtime: false,
                runtime_status: Some(ConversationStatus::Finished),
                is_processing: false,
                active_turn_id: None,
                processing_started_at: None,
            })
    }

    async fn latest_completed_turn_receipt(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<Option<nomifun_gateway::ConversationDeliveryReceipt>, AppError> {
        self.state
            .session_owner
            .get_session(user_id, conversation_id)
            .await?;
        let operation: Option<String> = sqlx::query_scalar(
            "SELECT turn.operation_id FROM agent_turns turn \
             JOIN agent_events started ON started.event_id = turn.started_event_id \
             WHERE turn.session_id = ? AND turn.state IN ('completed','failed','cancelled','interrupted') \
             ORDER BY started.seq DESC LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_optional(&self.state.session_owner.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        let Some(operation) = operation else { return Ok(None) };
        match self
            .state
            .session_owner
            .canonical_delivery_state_for_operation(
                user_id,
                &AgentSessionId::from(conversation_id.to_owned()),
                &OperationId::from(operation),
                false,
            )
            .await?
        {
            PublicTurnDeliveryState::Completed(delivery) => {
                Ok(Some(gateway_delivery(delivery)))
            }
            PublicTurnDeliveryState::Missing | PublicTurnDeliveryState::Accepted { .. } => Ok(None),
        }
    }

    async fn list_messages(
        &self,
        user_id: &str,
        conversation_id: &str,
        query: ListMessagesQuery,
    ) -> Result<MessageListResponse, AppError> {
        self.projected_messages(user_id, conversation_id, query).await
    }

    async fn register_delivery_notify(
        &self,
        user_id: &str,
        target_conversation_id: &str,
        _idempotency_key: &str,
        requester_conversation_id: &str,
    ) -> Result<nomifun_gateway::DeliveryNotifyRegistration, AppError> {
        self.state
            .session_owner
            .get_session(user_id, target_conversation_id)
            .await?;
        self.state
            .session_owner
            .get_session(user_id, requester_conversation_id)
            .await?;
        // Delivery-notify is a retired Conversation observer. The canonical
        // delegation surface uses AgentExecution receipts instead.
        Ok(nomifun_gateway::DeliveryNotifyRegistration::RefusedDeliveryNotifyOrigin)
    }

    async fn send_message_with_idempotency_key(
        &self,
        user_id: &str,
        conversation_id: &str,
        idempotency_key: &str,
        request: SendMessageRequest,
    ) -> Result<nomifun_gateway::ConversationDeliveryReceipt, AppError> {
        self.state
            .session_owner
            .send_session_message_idempotent(
                user_id,
                conversation_id,
                idempotency_key,
                request,
            )
            .await
            .map(gateway_delivery)
    }

    async fn get(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.state
            .session_owner
            .get_session(user_id, conversation_id)
            .await
    }

    async fn create(
        &self,
        user_id: &str,
        request: nomifun_gateway::ConversationCreateSpec,
    ) -> Result<ConversationResponse, AppError> {
        if user_id != self.state.authoritative_user_id.as_ref() {
            return Err(AppError::Forbidden("AgentSession owner mismatch".to_owned()));
        }
        if request
            .extra
            .get("workspace")
            .and_then(Value::as_str)
            .is_some_and(|workspace| !workspace.trim().is_empty())
        {
            return Err(AppError::Conflict(
                "Gateway-created AgentSessions use the configured Workspace resource; create a bound Agent from the Workbench for another path"
                    .to_owned(),
            ));
        }
        let owner = UserId::from(user_id.to_owned());
        let model = request.model.as_ref().map(|model| AgentChatModelSelectionDto {
            provider_id: model.provider_id.clone(),
            model: model.use_model.clone().unwrap_or_else(|| model.model.clone()),
        });
        let editor = self
            .state
            .control_plane
            .create_from_template(
                &owner,
                "chat.minimal",
                CreateAgentPresetFromTemplateRequest {
                    model: model.clone(),
                    reuse_existing: true,
                    display_name: request
                        .name
                        .clone()
                        .unwrap_or_else(|| "Assistant".to_owned()),
                    description: None,
                    model_route_refs: BTreeMap::new(),
                    chat_route_records: BTreeMap::new(),
                },
            )
            .await
            .map_err(control_plane_error_to_app)?;
        let mut binding = self
            .state
            .control_plane
            .resolve_agent_session_binding_with_model(
                &owner,
                editor.preset.preset_id.as_ref(),
                model.as_ref(),
            )
            .await
            .map_err(control_plane_error_to_app)?;
        let (_, _, snapshot) = self
            .state
            .control_plane
            .saved_binding_artifacts(&owner, &binding)
            .await
            .map_err(control_plane_error_to_app)?;
        let selections = snapshot
            .content
            .required_resource_kinds
            .iter()
            .filter_map(|kind| {
                (kind.as_ref() == "workspace").then(|| AgentResourceSelectionDto {
                    resource_kind: "workspace".to_owned(),
                    resource_id:
                        super::nomi_core_resource_bindings::DEFAULT_WORKSPACE_RESOURCE_ID
                            .to_owned(),
                })
            })
            .collect::<Vec<_>>();
        binding = self
            .state
            .resource_bindings
            .resolve_for_saved_binding(&self.state.control_plane, &owner, binding, &selections)
            .await
            .map_err(|error| AppError::UnprocessableEntity(error.message().to_owned()))?;
        let (_, _, snapshot) = self
            .state
            .control_plane
            .saved_binding_artifacts(&owner, &binding)
            .await
            .map_err(control_plane_error_to_app)?;
        let active_capabilities = snapshot
            .content
            .enabled_capabilities
            .iter()
            .filter(|capability| capability.consumption.is_contribution())
            .map(|capability| capability.capability.id.as_ref().to_owned())
            .collect();
        let binding: AgentBindingValue = serde_json::to_value(binding)
            .and_then(serde_json::from_value)
            .map_err(|error| AppError::Conflict(error.to_string()))?;
        let opened = self
            .state
            .session_owner
            .canonical()
            .open(
                PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: user_id.to_owned(),
                },
                binding,
                request.name,
                active_capabilities,
                &format!("gateway-create:{}", Uuid::now_v7()),
                now_ms(),
            )
            .await?;
        self.state
            .session_owner
            .materialize_workspace_for_binding(
                user_id,
                &opened.session.agent_session_id,
                &opened.session.agent_binding,
            )
            .await?;
        self.state
            .session_owner
            .canonical_conversation_projection(user_id, &opened.session.agent_session_id)
            .await?
            .ok_or_else(|| AppError::Internal("created AgentSession has no projection".to_owned()))
    }

    async fn update(
        &self,
        user_id: &str,
        conversation_id: &str,
        request: UpdateConversationRequest,
    ) -> Result<ConversationResponse, AppError> {
        let session_id = AgentSessionId::from(conversation_id.to_owned());
        self.state
            .session_owner
            .canonical()
            .store()
            .update_session_metadata(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: user_id.to_owned(),
                },
                &session_id,
                nomifun_agent_session::UpdateAgentSessionMetadata {
                    title: request.name,
                    pinned: request.pinned,
                    archived: None,
                },
            )
            .await
            .map_err(agent_session_store_error)?;
        self.state
            .session_owner
            .get_session(user_id, conversation_id)
            .await
    }

    async fn delete(&self, user_id: &str, conversation_id: &str) -> Result<(), AppError> {
        execute_nomi_core_agent_session_delete(
            self.state.clone(),
            AuthenticatedOwner(UserId::from(user_id.to_owned())),
            parse_agent_session_id(conversation_id)
                .map_err(|error| AppError::BadRequest(error.message))?,
            format!("gateway-delete:{}", Uuid::now_v7()),
        )
        .await
        .map(|_| ())
        .map_err(|error| AppError::Conflict(error.message))
    }

    async fn cancel(&self, user_id: &str, conversation_id: &str) -> Result<(), AppError> {
        self.state
            .session_owner
            .cancel_session(user_id, conversation_id)
            .await
    }

    async fn supports_scheduled_model_reconciliation(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<bool, AppError> {
        self.state
            .session_owner
            .get_session(user_id, conversation_id)
            .await?;
        Ok(false)
    }
}

fn ssh_teardown_loss(teardowns: &[nomifun_ssh::SshTeardown]) -> Option<String> {
    let losses = teardowns
        .iter()
        .filter_map(|teardown| match teardown {
            nomifun_ssh::SshTeardown::Lost { detail } => Some(detail.as_str()),
            nomifun_ssh::SshTeardown::Reaped { .. }
            | nomifun_ssh::SshTeardown::AlreadyDown { .. } => None,
        })
        .collect::<Vec<_>>();
    (!losses.is_empty()).then(|| losses.join("; "))
}

fn delete_cleanup_requires_reconciliation(
    blockers: &nomifun_agent_session::AgentSessionDeleteBlockers,
) -> bool {
    !blockers.effects.is_empty() || !blockers.resource_cleanup_uncertainties.is_empty()
}

fn agent_session_cleanup_uncertain(
    agent_session_id: &str,
) -> NomiCoreApiError {
    NomiCoreApiError::with_details(
        StatusCode::CONFLICT,
        "AGENT_SESSION_SSH_CLEANUP_UNCERTAIN",
        "AgentSession deletion was deferred because SSH teardown could not be proven.",
        json!({
            "agent_session_id": agent_session_id,
            "owner_domain": "ssh",
            "outcome": "unknown",
            "recovery": "domain_owner_reconciliation_or_explicit_manual_delete_override_required",
            "manual_override_confirmation": DELETE_OVERRIDE_CONFIRMATION,
        }),
    )
}

fn agent_session_delete_blocked(
    agent_session_id: &str,
    blockers: &nomifun_agent_session::AgentSessionDeleteBlockers,
) -> NomiCoreApiError {
    let effects = blockers
        .effects
        .iter()
        .map(|effect| {
            json!({
                "effect_id": effect.effect_id,
                "owner_domain": effect.owner_domain,
                "state": match effect.state {
                    nomifun_agent_session::AgentEffectState::Pending => "pending",
                    nomifun_agent_session::AgentEffectState::Unknown => "unknown",
                    nomifun_agent_session::AgentEffectState::Cancelled => "cancelled",
                    nomifun_agent_session::AgentEffectState::Returned => "returned",
                    nomifun_agent_session::AgentEffectState::Rejected => "rejected",
                },
            })
        })
        .collect::<Vec<_>>();
    NomiCoreApiError::with_details(
        StatusCode::CONFLICT,
        "AGENT_SESSION_DELETE_BLOCKED",
        "AgentSession deletion remains fenced until every admitted effect is terminal and every unknown outcome is explicitly reconciled.",
        json!({
            "agent_session_id": agent_session_id,
            "effects": effects,
            "resource_cleanup_pending": blockers.resource_cleanup_pending,
            "resource_cleanup_uncertainties": blockers.resource_cleanup_uncertainties,
            "recovery": "domain_owner_reconciliation_or_explicit_manual_delete_override_required",
            "manual_override_confirmation": DELETE_OVERRIDE_CONFIRMATION,
        }),
    )
}

/// Native Nomi tool owner bound to one exact authenticated AgentSession.
///
/// The model-facing tools have no owner/session selectors. This adapter keeps
/// those identities in host memory and revalidates canonical Store ownership
/// before every read or mutation.
struct NomiCoreSessionControlSink {
    session_owner: Arc<NomiCoreSessionOwner>,
    owner: AuthenticatedOwner,
    session_id: AgentSessionId,
}

#[async_trait]
impl SessionControlSink for NomiCoreSessionControlSink {
    async fn observe(&self, after_seq: u64, limit: u32) -> Result<Value, String> {
        let principal = authenticated_principal(&self.owner);
        self
            .session_owner
            .canonical()
            .get(&principal, &self.session_id)
            .await
            .map_err(|error| error.to_string())?;
        let after = (after_seq > 0).then(|| nomifun_agent_contracts::SessionEventCursor {
            agent_session_id: self.session_id.clone(),
            seq: after_seq,
        });
        let events = self.session_owner.canonical().events(
            &principal,
            &self.session_id,
            after.as_ref(),
            limit,
        ).await.map_err(|error| error.to_string())?;
        let messages = self.session_owner.canonical().messages(
            &principal,
            &self.session_id,
            after_seq,
        ).await.map_err(|error| error.to_string())?;
        serde_json::to_value(json!({
            "agent_session_id": self.session_id,
            "events": events.events,
            "messages": messages.into_iter().take(limit as usize).collect::<Vec<_>>(),
            "next_cursor": events.next_cursor,
        })).map_err(|error| error.to_string())
    }

    async fn steer(&self, message: &str, operation_id: &str) -> Result<Value, String> {
        let receipt = self
            .session_owner
            .canonical()
            .steer(
                &authenticated_principal(&self.owner),
                &self.session_id,
                operation_id,
                json!({"content": message}),
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "agent_session_id": self.session_id,
            "target_operation_id": receipt.target_operation_id,
            "replayed": receipt.duplicate,
            "status": "accepted",
            "cursor": receipt.cursor,
        }))
    }

    async fn fork(&self, title: Option<&str>, operation_id: &str) -> Result<Value, String> {
        let principal = authenticated_principal(&self.owner);
        self
            .session_owner
            .canonical()
            .get(&principal, &self.session_id)
            .await
            .map_err(|error| error.to_string())?;
        let through_seq = self.session_owner.canonical().store()
            .current_cursor(&self.session_id)
            .await
            .map_err(|error| error.to_string())?
            .seq;
        let result = self.session_owner.canonical().fork(
            &principal,
            &self.session_id,
            through_seq,
            title.map(str::to_owned),
            operation_id,
            now_ms(),
        )
        .await
        .map_err(|error| error.to_string())?;
        self.session_owner
            .inherit_idmm_state(
                self.session_id.as_ref(),
                result.child_session.agent_session_id.as_ref(),
            )
            .await
            .map_err(|error| error.to_string())?;
        serde_json::to_value(result).map_err(|error| error.to_string())
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
        .route(
            "/api/product-agent-bindings/{target_kind}/{target_id}",
            get(product_agent_options).put(select_product_agent_binding),
        )
        .route(
            "/api/agent-sessions",
            get(list_nomi_core_agent_sessions).post(create_nomi_core_agent_session),
        )
        .route(
            "/api/agent-session-messages/search",
            get(search_canonical_agent_session_messages),
        )
        .route(
            "/api/creative-studio/canvas-agent-sessions/resolve",
            post(resolve_canonical_creative_studio_canvas_agent_session),
        )
        .route("/api/agent-runtime", get(get_official_runtime))
        .route(
            "/api/agent-sessions/{agent_session_id}",
            get(get_nomi_core_agent_session)
                .patch(update_nomi_core_agent_session_metadata)
                .delete(delete_nomi_core_agent_session),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/model",
            put(switch_nomi_core_agent_session_model),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/reasoning-effort",
            put(update_nomi_core_agent_session_reasoning),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/agent-switch/preview",
            post(preview_nomi_core_agent_session_agent_switch),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/agent",
            put(apply_nomi_core_agent_session_agent_switch),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/knowledge",
            get(get_nomi_core_agent_session_knowledge)
                .put(update_nomi_core_agent_session_knowledge),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/projection",
            get(get_nomi_core_agent_session_projection),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/message-history",
            get(get_nomi_core_agent_session_message_history),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/message-history/{message_id}",
            get(get_nomi_core_agent_session_message),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/creation-tasks",
            get(list_canonical_creation_tasks).post(submit_canonical_creation_task),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/creation-tasks/{task_id}/cancel",
            post(cancel_canonical_creation_task),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/warmup",
            post(warm_nomi_core_agent_session),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/clear-context",
            post(clear_nomi_core_agent_session_context),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/side-question",
            post(ask_nomi_core_agent_session_side_question),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/workspace",
            get(browse_nomi_core_agent_session_workspace),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/delete-override",
            post(override_nomi_core_agent_session_delete),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/capabilities",
            get(get_nomi_core_agent_session_capabilities),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/slash-commands",
            get(get_nomi_core_agent_session_slash_commands),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/turns",
            post(start_nomi_core_agent_session_turn),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/turns/steer",
            post(steer_nomi_core_agent_session_turn),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/turns/cancel",
            post(cancel_nomi_core_agent_session_turn),
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectProductAgentBindingRequest {
    #[serde(default)]
    preset_id: Option<String>,
    #[serde(default)]
    selection: Option<ProductAgentSelection>,
    #[serde(default)]
    model: Option<AgentChatModelSelectionDto>,
    #[serde(default)]
    resource_selections: Option<Vec<AgentResourceSelectionDto>>,
    #[serde(default)]
    conversation_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProductAgentOptionsQuery {
    provider_id: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProductAgentOption {
    selection: ProductAgentSelection,
    display_name: String,
    available: bool,
    reason: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct ProductAgentOptions {
    selection: ProductAgentSelection,
    options: Vec<ProductAgentOption>,
    needs_model: bool,
}

fn product_option_reason(error: &ControlPlaneError) -> &'static str {
    let details = error.details().unwrap_or(Value::Null);
    let features = details["missing_features"].as_array();
    if error.code().as_ref() == "MODEL_ROUTE_FEATURES_MISSING" {
        if details["required_protocol"] == "openai.responses" || features.is_some_and(|items| items.iter().any(|item| item == "WebSearch")) {
            return "web_search";
        }
        if features.is_some_and(|items| items.iter().any(|item| item == "ImageInput")) { return "vision"; }
    }
    if error.code().as_ref().starts_with("MODEL_") { return "model"; }
    if error.code().as_ref() == "AGENT_PRESET_NOT_FOUND" { return "removed"; }
    "capability"
}

fn require_product_target(state: &NomiCoreAgentApiState, owner: &AuthenticatedOwner, kind: &str, id: &str) -> Result<&'static str, NomiCoreApiError> {
    if owner.as_ref() != state.product_agent_resolver.owner_id.as_ref() {
        return Err(NomiCoreApiError::new(StatusCode::FORBIDDEN, "RESOURCE_OWNER_MISMATCH", "product settings require the installation owner"));
    }
    if id.trim().is_empty() || id.len() > 512 || id.trim() != id {
        return Err(NomiCoreApiError::new(StatusCode::BAD_REQUEST, "PRESET_RESOURCE_NOT_BOUND", "invalid product target"));
    }
    product_default_template(kind).ok_or_else(|| NomiCoreApiError::new(StatusCode::UNPROCESSABLE_ENTITY,
        "CAPABILITY_UNAVAILABLE_ON_PLATFORM", "unsupported product target"))
}

async fn product_agent_options(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path((kind, id)): Path<(String, String)>,
    Query(query): Query<ProductAgentOptionsQuery>,
) -> Result<Json<ApiResponse<ProductAgentOptions>>, NomiCoreApiError> {
    let default = require_product_target(&state, &owner, &kind, &id)?;
    let model = match (query.provider_id, query.model) {
        (Some(provider_id), Some(model)) => Some(AgentChatModelSelectionDto { provider_id, model }),
        (None, None) => None,
        _ => return Err(NomiCoreApiError::new(StatusCode::BAD_REQUEST, "MODEL_ROUTE_NOT_FOUND", "incomplete model selection")),
    };
    let library = state.control_plane.library(&owner).await?;
    let fixed_selection = (kind == "companion").then(|| ProductAgentSelection::Template {
        template_key: default.to_owned(),
    });
    let mut candidates = if let Some(selection) = fixed_selection.as_ref() {
        vec![(selection.clone(), default.to_owned())]
    } else {
        let mut candidates = library.official_templates.iter().map(|template| {
            let key = serde_json::to_value(template.template_key).unwrap().as_str().unwrap().to_owned();
            (ProductAgentSelection::Template { template_key: key.clone() }, key)
        }).collect::<Vec<_>>();
        candidates.extend(library.user_presets.iter().map(|preset| (ProductAgentSelection::Preset { preset_id: preset.preset_id.clone() }, preset.display_name.clone())));
        candidates
    };
    let selection = match fixed_selection.or(state.product_agent_resolver.selection(&owner, &kind, &id).await?) {
        Some(selection) => selection,
        None => match state.control_plane.get_agent_binding(&owner, kind.clone(), id.clone()).await? {
            Some(record) => {
                let id = record.agent_binding.preset_revision_ref.preset_id;
                match state.control_plane.internal_official_template(&owner, &id).await.unwrap_or(None) {
                    Some(key) => ProductAgentSelection::Template { template_key: key.as_str().to_owned() },
                    None => ProductAgentSelection::Preset { preset_id: id },
                }
            }
            None => ProductAgentSelection::Template { template_key: default.to_owned() },
        },
    };
    if !candidates.iter().any(|(candidate, _)| candidate == &selection) {
        let name = match selection.preset_id() {
            Some(id) => state.control_plane.editor(&owner, id, None).await.map(|editor| editor.preset.display_name).unwrap_or_default(),
            None => String::new(),
        };
        candidates.push((selection.clone(), name));
    }
    let mut options = Vec::new();
    for (selection, display_name) in candidates {
        let result = state.control_plane.validate_product_selection(&owner, selection.template_id(), selection.preset_id(), model.as_ref()).await;
        options.push(ProductAgentOption { selection, display_name, available: result.is_ok(), reason: result.err().as_ref().map(product_option_reason) });
    }
    Ok(Json(ApiResponse::ok(ProductAgentOptions { selection, options, needs_model: model.is_none() })))
}

fn product_default_template(target_kind: &str) -> Option<&'static str> {
    match target_kind {
        "companion" => Some("companion.default"),
        "customer" => Some("customer-service.default"),
        "creative_studio_canvas" => Some("creative-studio.default"),
        _ => None,
    }
}

async fn select_product_agent_binding(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path((target_kind, target_id)): Path<(String, String)>,
    Json(request): Json<SelectProductAgentBindingRequest>,
) -> Result<Json<ApiResponse<Value>>, NomiCoreApiError> {
    let default = require_product_target(&state, &owner, &target_kind, &target_id)?;
    let selection = match (request.selection, request.preset_id) {
        (Some(selection), None) => selection,
        (None, Some(preset_id)) => ProductAgentSelection::Preset { preset_id },
        _ => return Err(NomiCoreApiError::new(StatusCode::BAD_REQUEST, "AGENT_PRESET_NOT_FOUND", "select exactly one Agent")),
    };
    if target_kind == "companion"
        && selection != (ProductAgentSelection::Template {
            template_key: default.to_owned(),
        })
    {
        return Err(NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "CAPABILITY_UNAVAILABLE_ON_PLATFORM",
            "Companion conversations always use companion.default",
        ));
    }
    let _guard = state.product_agent_resolver.default_binding_lock.lock().await;
    let model = request.model;
    let resource_selections = request.resource_selections;
    if let Some(id) = request.conversation_id.as_deref() {
        let current = state.session_owner.get_session(owner.as_ref(), id).await?;
        let session_id = parse_agent_session_id(id)?;
        let canonical = state
            .session_owner
            .canonical()
            .get(&authenticated_principal(&owner), &session_id)
            .await?;
        let resource_kind = match target_kind.as_str() {
            "creative_studio_canvas" => "canvas",
            other => other,
        };
        let belongs = canonical.session.agent_binding.typed_resource_bindings.iter().any(
            |resource| {
                resource.resource_kind.as_ref() == resource_kind
                    && resource.resource_id.as_ref() == target_id
            },
        );
        if !belongs { return Err(NomiCoreApiError::new(StatusCode::FORBIDDEN, "RESOURCE_OWNER_MISMATCH", "conversation belongs to another product target")); }
        if current.status == nomifun_common::ConversationStatus::Running {
            return Err(NomiCoreApiError::new(StatusCode::CONFLICT, "REMOTE_SESSION_BUSY", "wait for the current reply"));
        }
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_BINDING_IMMUTABLE",
            "Product Agent changes apply to future Sessions; fork or create a new Session",
        ));
    }
    // Validate before any binding/selection mutation, including stale UI requests.
    state.control_plane.validate_product_selection(&owner, selection.template_id(), selection.preset_id(), model.as_ref()).await?;
    let mut response = json!({ "selection": selection, "needs_model": model.is_none() });
    if model.is_some() {
        let mut binding = state.product_agent_resolver.materialize(&owner, &selection, model.as_ref()).await?;
        let existing = state.control_plane
            .get_agent_binding(&owner, target_kind.clone(), target_id.clone())
            .await?;
        let inherit_existing_resources = resource_selections.is_none()
            && existing.as_ref().is_some_and(|record|
                !record.agent_binding.typed_resource_bindings.is_empty());
        let inherited_selections;
        let selections = if let Some(selections) = resource_selections.as_deref() {
            Some(selections)
        } else if inherit_existing_resources {
            let (_, _, next_snapshot) = state.control_plane
                .saved_binding_artifacts(&owner, &binding)
                .await?;
            inherited_selections = existing.as_ref().into_iter()
                .flat_map(|record| record.agent_binding.typed_resource_bindings.iter())
                .filter(|resource| next_snapshot.content.required_resource_kinds.iter()
                    .any(|kind| kind.as_ref() == resource.resource_kind))
                .map(|resource| AgentResourceSelectionDto {
                    resource_kind: resource.resource_kind.clone(),
                    resource_id: resource.resource_id.clone(),
                })
                .collect::<Vec<_>>();
            Some(inherited_selections.as_slice())
        } else {
            None
        };
        if let Some(selections) = selections {
            binding = state.product_agent_resolver.resource_bindings
                .resolve_for_saved_binding(
                    &state.control_plane,
                    &owner,
                    binding,
                    selections,
                )
                .await?;
        }
        let previous = existing.as_ref().map(|record| record.agent_binding.binding_version);
        binding.binding_version = previous.unwrap_or(0) + 1;
        let stored = state.control_plane.put_agent_binding(&owner, target_kind.clone(), target_id.clone(),
            PutAgentBindingRequest { expected_binding_version: previous, agent_binding: binding }).await?;
        response["agent_binding"] = serde_json::to_value(stored.agent_binding)?;
    }
    state.product_agent_resolver.save_selection(&owner, &target_kind, &target_id, &selection).await?;
    Ok(Json(ApiResponse::ok(response)))
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
    pub(super) message: String,
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

impl From<super::nomi_core_resource_bindings::ResourceSelectionResolutionError>
    for NomiCoreApiError
{
    fn from(
        error: super::nomi_core_resource_bindings::ResourceSelectionResolutionError,
    ) -> Self {
        Self::with_details(
            StatusCode::UNPROCESSABLE_ENTITY,
            error.code(),
            error.message(),
            error.details().clone(),
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
pub(super) struct NomiCoreSessionMetadata {
    version: u64,
    kind: String,
    pub(super) binding: AgentBindingValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote: Option<RemoteBindingProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<AgentSessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fork_base_payload_id: Option<ArtifactId>,
}

#[derive(Debug, Serialize)]
struct NomiCoreAgentSessionCapabilityResponse {
    resolved_snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef,
    generation: u64,
    enabled_capabilities: Vec<String>,
    active_capabilities: Vec<String>,
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
#[serde(tag = "target", rename_all = "snake_case", deny_unknown_fields)]
enum NomiCoreDeleteOverrideRequest {
    Effect {
        effect_id: String,
        confirmation: String,
        reason: String,
    },
    ResourceCleanup {
        owner_domain: String,
        confirmation: String,
        reason: String,
    },
}

#[derive(Debug, Serialize)]
struct NomiCoreDeleteOverrideResponse {
    agent_session_id: String,
    remaining_blockers: nomifun_agent_session::AgentSessionDeleteBlockers,
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
struct NomiCoreSessionListQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default = "default_nomi_core_page_limit")]
    limit: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NomiCoreSessionMetadataUpdate {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    pinned: Option<bool>,
    #[serde(default)]
    archived: Option<bool>,
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

            let is_idle = session_owner
                .canonical()
                .store()
                .head(&session_id)
                .await
                .is_ok_and(|head| head.status != "running");
            if is_idle {
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

async fn get_official_runtime(
    State(state): State<NomiCoreAgentApiState>,
) -> Result<Json<ApiResponse<nomifun_api_types::RuntimeBuildDescriptor>>, NomiCoreApiError> {
    let host = state.session_owner.official_runtime.get()
        .ok_or_else(|| AppError::Conflict("Runtime host is not assembled".into()))?;
    Ok(Json(ApiResponse::ok(host.provider()?.descriptor().clone())))
}

async fn list_nomi_core_agent_sessions(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Query(query): Query<NomiCoreSessionListQuery>,
) -> Result<Json<ApiResponse<PaginatedResult<ConversationResponse>>>, NomiCoreApiError> {
    if query.limit == 0 || query.limit > 10_000 {
        return Err(NomiCoreApiError::new(
            StatusCode::BAD_REQUEST,
            "AGENT_SESSION_LIST_LIMIT_INVALID",
            "AgentSession list limit must be between 1 and 10000",
        ));
    }
    let page = state
        .session_owner
        .canonical()
        .store()
        .list_live_sessions(
            &authenticated_principal(&owner),
            query.cursor.as_deref(),
            query.limit,
        )
        .await
        .map_err(agent_session_store_error)?;
    let mut items = Vec::with_capacity(page.items.len());
    for item in page.items {
        let projection = state
            .session_owner
            .canonical_conversation_projection(
                owner.as_ref(),
                &item.session.agent_session_id,
            )
            .await?
            .ok_or_else(|| {
                AppError::Conflict(
                    "listed AgentSession has no canonical projection".to_owned(),
                )
            })?;
        items.push(projection);
    }
    Ok(Json(ApiResponse::ok(PaginatedResult {
        items,
        total: page.total,
        has_more: page.has_more,
    })))
}

async fn search_canonical_agent_session_messages(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Query(query): Query<SearchMessagesQuery>,
) -> Result<Json<ApiResponse<MessageSearchResponse>>, NomiCoreApiError> {
    let keyword = query.keyword.trim();
    if keyword.is_empty() {
        return Err(AppError::BadRequest("keyword must not be empty".to_owned()).into());
    }
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let page = query.page.unwrap_or(0);
    let offset = i64::from(page).saturating_mul(i64::from(page_size));
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) \
         FROM agent_messages message \
         JOIN agent_sessions session ON session.agent_session_id = message.session_id \
         WHERE session.principal_kind = 'user' AND session.principal_id = ? \
           AND session.deleted_at IS NULL AND message.presentation_intent = 'message' \
           AND instr(lower(COALESCE(json_extract(message.projection_json, '$.content'), '')), lower(?)) > 0",
    )
    .bind(owner.as_ref())
    .bind(keyword)
    .fetch_one(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let rows = sqlx::query_as::<_, (String, String, i64, i64, String, String, String)>(
        "SELECT message.session_id, message.projection_id, message.first_seq, message.last_seq, \
                message.presentation_intent, message.projection_json, message.semantic_digest \
         FROM agent_messages message \
         JOIN agent_sessions session ON session.agent_session_id = message.session_id \
         WHERE session.principal_kind = 'user' AND session.principal_id = ? \
           AND session.deleted_at IS NULL AND message.presentation_intent = 'message' \
           AND instr(lower(COALESCE(json_extract(message.projection_json, '$.content'), '')), lower(?)) > 0 \
         ORDER BY session.created_at DESC, message.first_seq DESC, message.projection_id DESC \
         LIMIT ? OFFSET ?",
    )
    .bind(owner.as_ref())
    .bind(keyword)
    .bind(i64::from(page_size) + 1)
    .bind(offset)
    .fetch_all(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let has_more = rows.len() > page_size as usize;
    let mut items = Vec::with_capacity(rows.len().min(page_size as usize));
    for (session, projection_id, first_seq, last_seq, intent, document, digest) in
        rows.into_iter().take(page_size as usize)
    {
        let session_id = parse_agent_session_id(&session)?;
        let created_at = state
            .session_owner
            .canonical()
            .store()
            .session_created_at(&session_id)
            .await
            .map_err(agent_session_store_error)?;
        let projection = MessageProjection {
            session_id: session_id.clone(),
            projection_id,
            first_seq: u64::try_from(first_seq)
                .map_err(|_| AppError::Internal("negative message sequence".to_owned()))?,
            last_seq: u64::try_from(last_seq)
                .map_err(|_| AppError::Internal("negative message sequence".to_owned()))?,
            presentation_intent: intent,
            message_type: None,
            message_status: None,
            projection: serde_json::from_str(&document)
                .map_err(|error| AppError::Internal(error.to_string()))?,
            semantic_digest: digest,
        };
        let Some(message) = canonical_message_response(&session_id, created_at, projection)? else {
            continue;
        };
        let Some(conversation) = state
            .session_owner
            .canonical_conversation_projection(owner.as_ref(), &session_id)
            .await?
        else {
            continue;
        };
        let text = message
            .content
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let preview_text = text.chars().take(240).collect::<String>();
        items.push(MessageSearchItem {
            message_id: message.message_id,
            message_type: "text".to_owned(),
            message_created_at: message.created_at,
            preview_text,
            conversation,
        });
    }
    Ok(Json(ApiResponse::ok(PaginatedResult {
        items,
        total: u64::try_from(total)
            .map_err(|_| AppError::Internal("negative message search total".to_owned()))?,
        has_more,
    })))
}

fn validate_creative_studio_session_request(
    request: &ResolveCreativeStudioCanvasAgentSessionRequest,
) -> Result<(), AppError> {
    nomifun_common::CreativeStudioCanvasId::parse(&request.canvas_id)
        .map_err(|error| AppError::BadRequest(format!("invalid Creative Studio canvas_id: {error}")))?;
    nomifun_common::validate_uuidv7(&request.session_id)
        .map_err(|error| AppError::BadRequest(format!("invalid Creative Studio session_id: {error}")))?;
    nomifun_common::ProviderId::parse(&request.model.provider_id)
        .map_err(|error| AppError::BadRequest(format!("invalid Creative Studio provider_id: {error}")))?;
    if request.model.model.trim().is_empty() || request.model.model.trim() != request.model.model {
        return Err(AppError::BadRequest(
            "Creative Studio Agent model must be trimmed and non-empty".to_owned(),
        ));
    }
    if let Some(key) = request.pending_turn_idempotency_key.as_deref() {
        nomifun_common::validate_uuidv7(key).map_err(|error| {
            AppError::BadRequest(format!(
                "invalid Creative Studio pending_turn_idempotency_key: {error}"
            ))
        })?;
    }
    Ok(())
}

fn creative_studio_visible_user_text(content: &str) -> Result<String, AppError> {
    let Ok(envelope) = serde_json::from_str::<Value>(content) else {
        return Ok(content.to_owned());
    };
    if envelope.get("kind").and_then(Value::as_str)
        != Some("nomifun.creative-studio.planning-turn")
    {
        return Ok(content.to_owned());
    }
    if envelope.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(AppError::Conflict(
            "Creative Studio planning message has an unsupported version".to_owned(),
        ));
    }
    envelope
        .get("userRequest")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError::Conflict(
                "Creative Studio planning message has no visible userRequest".to_owned(),
            )
        })
}

async fn canonical_creative_studio_history(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
) -> Result<Vec<CreativeStudioAgentHistoryMessage>, NomiCoreApiError> {
    state
        .session_owner
        .canonical()
        .get(&authenticated_principal(owner), session_id)
        .await?;
    let created_at = state
        .session_owner
        .canonical()
        .store()
        .session_created_at(session_id)
        .await
        .map_err(agent_session_store_error)?;
    let (mut projections, _, _) = state
        .session_owner
        .canonical()
        .store()
        .messages_before(session_id, None, 500)
        .await
        .map_err(agent_session_store_error)?;
    projections.reverse();
    let mut history = Vec::new();
    for projection in projections {
        let Some(message) = canonical_message_response(session_id, created_at, projection)? else {
            continue;
        };
        if message.r#type != MessageType::Text
            || message.status != Some(MessageStatus::Finish)
            || message.hidden
        {
            continue;
        }
        let Some(position) = message.position else { continue };
        let text = message
            .content
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (role, text) = match position {
            MessagePosition::Right => (
                CreativeStudioAgentHistoryRole::User,
                creative_studio_visible_user_text(text)?,
            ),
            MessagePosition::Left => (
                CreativeStudioAgentHistoryRole::Assistant,
                text.to_owned(),
            ),
            _ => continue,
        };
        history.push(CreativeStudioAgentHistoryMessage {
            id: message.message_id,
            role,
            status: CreativeStudioAgentHistoryStatus::Complete,
            text,
            activity_label: None,
            error_message: None,
        });
    }
    Ok(history)
}

async fn resolve_canonical_creative_studio_canvas_agent_session(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Json(request): Json<ResolveCreativeStudioCanvasAgentSessionRequest>,
) -> Result<
    (
        StatusCode,
        Json<ApiResponse<ResolveCreativeStudioCanvasAgentSessionResponse>>,
    ),
    NomiCoreApiError,
> {
    validate_creative_studio_session_request(&request)?;
    let project: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT CAST(json_extract(session.value, '$.model.providerId') AS TEXT), \
                CAST(json_extract(session.value, '$.model.model') AS TEXT), \
                CAST(json_extract(session.value, '$.pendingTurn.idempotencyKey') AS TEXT) \
         FROM creative_studio_projects project \
         JOIN installation_identity identity \
           ON identity.singleton_key = 'installation' AND identity.owner_user_id = ? \
         JOIN json_each(project.document_json, '$.chatSessions') session \
         WHERE project.project_id = ? AND json_extract(session.value, '$.id') = ?",
    )
    .bind(owner.as_ref())
    .bind(&request.canvas_id)
    .bind(&request.session_id)
    .fetch_optional(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let Some((provider_id, model, pending_key)) = project else {
        return Err(AppError::NotFound(
            "Creative Studio canvas/session is not owned by the installation user or does not exist"
                .to_owned(),
        )
        .into());
    };
    if provider_id.as_deref() != Some(request.model.provider_id.as_str())
        || model.as_deref() != Some(request.model.model.as_str())
        || pending_key != request.pending_turn_idempotency_key
    {
        return Err(AppError::Conflict(
            "Creative Studio canvas Session facts changed during AgentSession resolution".to_owned(),
        )
        .into());
    }

    let lock = state.session_owner.session_operation_lock(&format!(
        "creative-studio:{}:{}:{}",
        owner.as_ref(), request.canvas_id, request.session_id
    ));
    let _guard = lock.write().await;
    let mut bound_session: Option<String> = sqlx::query_scalar(
        "SELECT conversation_id FROM creative_studio_agent_sessions \
         WHERE owner_id = ? AND project_id = ? AND session_id = ?",
    )
    .bind(owner.as_ref())
    .bind(&request.canvas_id)
    .bind(&request.session_id)
    .fetch_optional(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let mut created = false;
    if bound_session.is_none() {
        let pending_key = request.pending_turn_idempotency_key.as_deref().ok_or_else(|| {
            AppError::Conflict(
                "Creative Studio AgentSession creation requires a durable pending Turn fence"
                    .to_owned(),
            )
        })?;
        let projection = state
            .session_owner
            .create_session_idempotent(
                owner.as_ref(),
                CreateConversationRequest {
                    r#type: nomifun_common::AgentType::Nomi,
                    name: Some("Creative Studio Agent".to_owned()),
                    model: Some(nomifun_common::ProviderWithModel {
                        provider_id: request.model.provider_id.clone(),
                        model: request.model.model.clone(),
                        use_model: Some(request.model.model.clone()),
                    }),
                    source: Some(nomifun_common::ConversationSource::Nomifun),
                    channel_chat_id: None,
                    preset_id: None,
                    delegation_policy: Default::default(),
                    execution_model_pool: None,
                    decision_policy: Default::default(),
                    execution_template_id: None,
                    extra: json!({
                        "product_agent_target_kind": "creative_studio_canvas",
                        "product_agent_target_id": request.canvas_id,
                    }),
                },
                None,
                &format!(
                    "creative-studio-session:{}:{}:{pending_key}",
                    request.canvas_id, request.session_id
                ),
            )
            .await?;
        let inserted = sqlx::query(
            "INSERT INTO creative_studio_agent_sessions \
                (owner_id, project_id, session_id, conversation_id, created_at, updated_at) \
             SELECT ?, ?, ?, ?, ?, ? \
             WHERE EXISTS ( \
                 SELECT 1 FROM creative_studio_projects project \
                 JOIN installation_identity identity \
                   ON identity.singleton_key = 'installation' AND identity.owner_user_id = ? \
                 JOIN json_each(project.document_json, '$.chatSessions') session \
                 WHERE project.project_id = ? \
                   AND json_extract(session.value, '$.id') = ? \
                   AND json_extract(session.value, '$.model.providerId') = ? \
                   AND json_extract(session.value, '$.model.model') = ? \
                   AND json_extract(session.value, '$.pendingTurn.idempotencyKey') = ? \
             ) ON CONFLICT(owner_id, project_id, session_id) DO NOTHING",
        )
        .bind(owner.as_ref())
        .bind(&request.canvas_id)
        .bind(&request.session_id)
        .bind(&projection.conversation_id)
        .bind(projection.created_at)
        .bind(projection.modified_at)
        .bind(owner.as_ref())
        .bind(&request.canvas_id)
        .bind(&request.session_id)
        .bind(&request.model.provider_id)
        .bind(&request.model.model)
        .bind(pending_key)
        .execute(&state.session_owner.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        created = inserted.rows_affected() == 1;
        bound_session = sqlx::query_scalar(
            "SELECT conversation_id FROM creative_studio_agent_sessions \
             WHERE owner_id = ? AND project_id = ? AND session_id = ?",
        )
        .bind(owner.as_ref())
        .bind(&request.canvas_id)
        .bind(&request.session_id)
        .fetch_optional(&state.session_owner.pool)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
        if bound_session.as_deref() != Some(projection.conversation_id.as_str()) {
            return Err(AppError::Conflict(
                "Creative Studio AgentSession binding winner changed during resolution".to_owned(),
            )
            .into());
        }
    }
    let session_id = parse_agent_session_id(bound_session.as_deref().ok_or_else(|| {
        AppError::Conflict("Creative Studio AgentSession binding was not committed".to_owned())
    })?)?;
    let projection = state
        .session_owner
        .canonical_conversation_projection(owner.as_ref(), &session_id)
        .await?
        .ok_or_else(|| {
            AppError::Conflict(
                "Creative Studio binding points to a missing canonical AgentSession".to_owned(),
            )
        })?;
    if let Some(model) = projection.model.as_ref()
        && (model.provider_id != request.model.provider_id
            || model.use_model.as_deref().unwrap_or(&model.model) != request.model.model)
    {
        return Err(AppError::Conflict(
            "Creative Studio AgentSession model is immutable and differs from the request"
                .to_owned(),
        )
        .into());
    }
    let history = canonical_creative_studio_history(&state, &owner, &session_id).await?;
    let project_message_ids = sqlx::query_scalar::<_, String>(
        "SELECT CAST(message.value AS TEXT) \
         FROM creative_studio_projects project, \
              json_each(project.document_json, '$.chatSessions') session, \
              json_each(session.value, '$.messageIds') message \
         WHERE project.project_id = ? AND json_extract(session.value, '$.id') = ? \
         ORDER BY CAST(message.key AS INTEGER)",
    )
    .bind(&request.canvas_id)
    .bind(&request.session_id)
    .fetch_all(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    if history.len() < project_message_ids.len()
        || history
            .iter()
            .map(|message| message.id.as_str())
            .take(project_message_ids.len())
            .ne(project_message_ids.iter().map(String::as_str))
    {
        return Err(AppError::Conflict(
            "Creative Studio canvas history is not a prefix of its canonical AgentSession"
                .to_owned(),
        )
        .into());
    }
    let assistant_ids = history
        .iter()
        .filter(|message| message.role == CreativeStudioAgentHistoryRole::Assistant)
        .map(|message| message.id.as_str())
        .collect::<HashSet<_>>();
    let applied_proposal_message_ids = sqlx::query_scalar::<_, String>(
        "SELECT receipt.assistant_message_id \
         FROM creative_studio_agent_proposal_receipts receipt \
         JOIN creative_studio_projects project ON project.project_id = receipt.project_id \
         CROSS JOIN json_each(project.document_json, '$.chatSessions') session \
         CROSS JOIN json_each(session.value, '$.messageIds') message \
         WHERE receipt.project_id = ? \
           AND json_extract(session.value, '$.id') = ? \
           AND CAST(message.key AS INTEGER) % 2 = 1 \
           AND CAST(message.value AS TEXT) = receipt.assistant_message_id \
         ORDER BY CAST(message.key AS INTEGER)",
    )
    .bind(&request.canvas_id)
    .bind(&request.session_id)
    .fetch_all(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
    .into_iter()
    .filter(|id| assistant_ids.contains(id.as_str()))
    .collect::<Vec<_>>();
    let history_key = serde_json::to_string(&history)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let status = if created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((
        status,
        Json(ApiResponse::ok(ResolveCreativeStudioCanvasAgentSessionResponse {
            binding: CreativeStudioCanvasAgentSessionBindingResponse {
                ownership: "creative-studio-exclusive",
                canvas_id: request.canvas_id,
                session_id: request.session_id,
                conversation_id: session_id.as_ref().to_owned(),
                model: CreativeStudioAgentModelRef {
                    provider_id: request.model.provider_id,
                    model: request.model.model,
                },
                history_key,
            },
            history,
            applied_proposal_message_ids,
            created,
        })),
    ))
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
    let mut binding = state
        .resource_bindings
        .resolve_for_saved_binding(
            &state.control_plane,
            &owner.0,
            binding,
            &request.resource_selections,
        )
        .await?;
    freeze_agent_session_knowledge_policy(
        &mut binding,
        request.knowledge_policy.as_ref(),
    )?;
    if let Some(workspace) = request.workspace.as_deref() {
        freeze_selected_workspace(
            &mut binding,
            owner.as_ref(),
            workspace,
            WorkspaceDirectoryCheck::Create,
        )?;
    }
    let agent_name = state
        .control_plane
        .editor(
            &owner.0,
            &binding.preset_revision_ref.preset_id,
            Some(binding.preset_revision_ref.revision),
        )
        .await?
        .preset
        .display_name;
    let projection =
        resolve_saved_binding_projection(&state, &owner, &binding, request.title.as_deref())
            .await?;
    if let Some(effort) = request.reasoning_effort {
        if !saved_binding_supports_reasoning_effort(
            &state,
            &owner,
            &binding,
            contract_reasoning_effort(effort),
        )
        .await?
        {
            return Err(NomiCoreApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "AGENT_SESSION_REASONING_UNSUPPORTED",
                "The selected Chat model protocol does not support this reasoning effort",
            ));
        }
    }
    let idmm_config = idmm_config_from_runtime_policy(&projection.runtime_policy)?;
    let creation_key = request_idempotency_key(
        &headers,
        "nomi-core-agent-session-create",
    )?;
    let binding_contract: AgentBindingValue = serde_json::to_value(&binding)
        .and_then(serde_json::from_value)
        .map_err(|error| AppError::Conflict(format!("Invalid Agent binding: {error}")))?;
    let active_capabilities = projection
        .snapshot
        .content
        .enabled_capabilities
        .iter()
        .filter(|capability| capability.consumption.is_contribution())
        .map(|capability| capability.capability.id.as_ref().to_owned())
        .collect();
    state.session_owner.validate_idmm_state(&idmm_config).await?;
    let opened = state
        .session_owner
        .canonical()
        .open_with_reasoning_effort(
            authenticated_principal(&owner),
            binding_contract,
            request.title.or(Some(agent_name)),
            active_capabilities,
            request.reasoning_effort.map(contract_reasoning_effort),
            &creation_key,
            now_ms(),
        )
        .await?;
    state
        .session_owner
        .materialize_workspace_for_binding(
            owner.as_ref(),
            &opened.session.agent_session_id,
            &opened.session.agent_binding,
        )
        .await?;
    state
        .session_owner
        .initialize_idmm_state(opened.session.agent_session_id.as_ref(), idmm_config)
        .await?;
    Ok(Json(ApiResponse::ok(CreateAgentSessionResponseDto {
        agent_session_id: opened.session.agent_session_id.as_ref().to_owned(),
        agent_binding: binding,
        state: "ready".to_owned(),
        cursor: session_cursor(&opened.session.agent_session_id, opened.cursor.seq),
    })))
}

fn freeze_agent_session_knowledge_policy(
    binding: &mut AgentBindingValueDto,
    requested: Option<&AgentSessionKnowledgePolicyDto>,
) -> Result<(), NomiCoreApiError> {
    let knowledge = binding
        .typed_resource_bindings
        .iter_mut()
        .filter(|resource| {
            resource.resource_kind
                == nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
        })
        .collect::<Vec<_>>();
    if knowledge.is_empty() {
        if requested.is_some() {
            return Err(NomiCoreApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "KNOWLEDGE_RESOURCE_NOT_BOUND",
                "Knowledge write-back policy requires at least one selected Knowledge base",
            ));
        }
        return Ok(());
    }
    let policy = requested.cloned().unwrap_or_default();
    if policy.writeback
        && knowledge
            .iter()
            .any(|resource| !resource.operations.contains("write"))
    {
        return Err(NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "KNOWLEDGE_WRITEBACK_NOT_AVAILABLE",
            "the selected Agent or Knowledge base does not grant write-back",
        ));
    }
    let writeback = if policy.writeback { "true" } else { "false" };
    let eagerness = match policy.writeback_eagerness {
        AgentSessionKnowledgeWritebackEagernessDto::Manual => "manual",
        AgentSessionKnowledgeWritebackEagernessDto::Auto => "auto",
    };
    for resource in knowledge {
        resource.typed_parameters.insert(
            nomifun_agent_domain_wave1::KNOWLEDGE_ENABLED_PARAMETER.to_owned(),
            "true".to_owned(),
        );
        resource.typed_parameters.insert(
            nomifun_agent_domain_wave1::KNOWLEDGE_WRITEBACK_PARAMETER.to_owned(),
            writeback.to_owned(),
        );
        resource.typed_parameters.insert(
            nomifun_agent_domain_wave1::KNOWLEDGE_WRITEBACK_EAGERNESS_PARAMETER.to_owned(),
            eagerness.to_owned(),
        );
    }
    Ok(())
}

fn agent_session_knowledge_binding(
    binding: &AgentBindingValue,
) -> Result<AgentSessionKnowledgeBindingDto, NomiCoreApiError> {
    let resources = binding
        .typed_resource_bindings
        .iter()
        .filter(|resource| {
            resource.resource_kind.as_ref()
                == nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
        })
        .collect::<Vec<_>>();
    let Some(first) = resources.first() else {
        return Ok(AgentSessionKnowledgeBindingDto::default());
    };
    let enabled = nomifun_agent_domain_wave1::agent_knowledge_enabled(first)
        .map_err(AppError::Conflict)?;
    let (writeback, eagerness) =
        nomifun_agent_domain_wave1::agent_knowledge_writeback_policy(first)
            .map_err(AppError::Conflict)?;
    for resource in resources.iter().skip(1) {
        if nomifun_agent_domain_wave1::agent_knowledge_enabled(resource)
            .map_err(AppError::Conflict)?
            != enabled
            || nomifun_agent_domain_wave1::agent_knowledge_writeback_policy(resource)
                .map_err(AppError::Conflict)?
                != (writeback, eagerness)
        {
            return Err(AppError::Conflict(
                "AgentSession Knowledge resources carry inconsistent live policies".to_owned(),
            )
            .into());
        }
    }
    Ok(AgentSessionKnowledgeBindingDto {
        enabled,
        writeback: enabled && writeback,
        writeback_eagerness: if enabled && writeback && eagerness == "auto" {
            AgentSessionKnowledgeWritebackEagernessDto::Auto
        } else {
            AgentSessionKnowledgeWritebackEagernessDto::Manual
        },
        kb_ids: resources
            .iter()
            .map(|resource| {
                nomifun_common::KnowledgeBaseId::parse(
                    resource.resource_id.as_ref().to_owned(),
                )
                .map_err(|error| {
                    AppError::Conflict(format!(
                        "AgentSession Knowledge resource identity is invalid: {error}"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

async fn get_nomi_core_agent_session_knowledge(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<AgentSessionKnowledgeBindingDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let observation = state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
    Ok(Json(ApiResponse::ok(agent_session_knowledge_binding(
        &observation.session.agent_binding,
    )?)))
}

async fn update_nomi_core_agent_session_knowledge(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(mut requested): Json<AgentSessionKnowledgeBindingDto>,
) -> Result<Json<ApiResponse<AgentSessionKnowledgeBindingDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    if requested.enabled && requested.kb_ids.is_empty() {
        return Err(NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "KNOWLEDGE_RESOURCE_NOT_BOUND",
            "enable Knowledge only after selecting at least one Knowledge base",
        ));
    }
    if !requested.enabled || !requested.writeback {
        requested.writeback = false;
        requested.writeback_eagerness =
            AgentSessionKnowledgeWritebackEagernessDto::Manual;
    }

    // Serialize admission, runtime recycling and the binding CAS against new
    // turns. A turn admitted first makes the Store reject this mutation; a
    // mutation admitted first tears down the old runtime before any successor
    // can observe the new binding.
    let _operation_fence = state
        .session_owner
        .session_operation_lock(session_id.as_ref())
        .write_owned()
        .await;
    let principal = authenticated_principal(&owner);
    let observation = state
        .session_owner
        .canonical()
        .get(&principal, &session_id)
        .await?;
    if observation.session.remote_binding_provenance.is_some() {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_KNOWLEDGE_IS_REMOTE_FROZEN",
            "Remote AgentSession Knowledge resources are fixed by its Remote binding",
        ));
    }
    if observation.head.status == "running" || observation.head.active_turn_id.is_some() {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_TURN_ACTIVE",
            "wait for the active Turn before changing Knowledge",
        ));
    }
    let attempt_transcript: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM conversation_execution_links \
         WHERE conversation_id = ? AND relation = 'attempt')",
    )
    .bind(session_id.as_ref())
    .fetch_one(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    if attempt_transcript != 0 {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_EXECUTION_ATTEMPT_READ_ONLY",
            "AgentExecution Attempt transcripts cannot change their Knowledge binding",
        ));
    }
    if agent_session_knowledge_binding(&observation.session.agent_binding)? == requested {
        return Ok(Json(ApiResponse::ok(requested)));
    }

    let current_dto = agent_binding_dto(&observation.session.agent_binding)?;
    let requested_ids = requested
        .kb_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut knowledge = state
        .resource_bindings
        .resolve_knowledge_for_saved_binding(
            &state.control_plane,
            &owner.0,
            &current_dto,
            &requested_ids,
        )
        .await?;
    if requested.writeback
        && knowledge
            .iter()
            .any(|resource| !resource.operations.contains("write"))
    {
        return Err(NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "KNOWLEDGE_WRITEBACK_NOT_AVAILABLE",
            "every selected Knowledge base must be editable before write-back can be enabled",
        ));
    }
    let enabled = if requested.enabled { "true" } else { "false" };
    let writeback = if requested.writeback { "true" } else { "false" };
    let eagerness = requested.writeback_eagerness.as_str();
    for resource in &mut knowledge {
        resource.typed_parameters.insert(
            nomifun_agent_domain_wave1::KNOWLEDGE_ENABLED_PARAMETER.to_owned(),
            enabled.to_owned(),
        );
        resource.typed_parameters.insert(
            nomifun_agent_domain_wave1::KNOWLEDGE_WRITEBACK_PARAMETER.to_owned(),
            writeback.to_owned(),
        );
        resource.typed_parameters.insert(
            nomifun_agent_domain_wave1::KNOWLEDGE_WRITEBACK_EAGERNESS_PARAMETER.to_owned(),
            eagerness.to_owned(),
        );
    }
    let mut replacement_dto = current_dto;
    replacement_dto.typed_resource_bindings.retain(|resource| {
        resource.resource_kind
            != nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
    });
    replacement_dto.typed_resource_bindings.extend(knowledge);
    replacement_dto
        .typed_resource_bindings
        .sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
    replacement_dto.binding_version = replacement_dto
        .binding_version
        .checked_add(1)
        .ok_or_else(|| AppError::Conflict("AgentSession binding version overflow".to_owned()))?;
    let replacement: AgentBindingValue = serde_json::to_value(&replacement_dto)
        .and_then(serde_json::from_value)
        .map_err(|error| {
            AppError::Conflict(format!(
                "resolved Session Knowledge binding is invalid: {error}"
            ))
        })?;

    state
        .session_owner
        .runtime_sessions
        .terminate_and_wait_result(
            session_id.as_ref(),
            Some(AgentKillReason::ConfigurationChanged),
        )
        .await?;
    let updated = state
        .session_owner
        .canonical()
        .store()
        .replace_session_resource_bindings(
            &principal,
            &session_id,
            &observation.session.agent_binding,
            replacement,
            nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND,
        )
        .await
        .map_err(agent_session_store_error)?;
    let updated_binding = agent_session_knowledge_binding(&updated.agent_binding)?;
    state.session_owner.user_events.send_to_user(
        owner.as_ref(),
        WebSocketMessage::new(
            "agentSession.knowledgeChanged",
            json!({
                "agent_session_id": session_id,
                "binding": updated_binding.clone(),
            }),
        ),
    );
    Ok(Json(ApiResponse::ok(updated_binding)))
}

#[derive(Clone)]
struct PreparedAgentSessionSwitch {
    preview: PreviewAgentSessionSwitchResponseDto,
    expected: AgentBindingValue,
    replacement: AgentBindingValue,
    source_label: String,
    target_label: String,
    handoff: Option<AgentHandoffEnvelopeV1>,
    active_capability_ids: Vec<String>,
}

fn handoff_mode(mode: AgentHandoffModeDto) -> AgentHandoffMode {
    match mode {
        AgentHandoffModeDto::ContinueTask => AgentHandoffMode::ContinueTask,
        AgentHandoffModeDto::ContextOnly => AgentHandoffMode::ContextOnly,
    }
}

fn handoff_citation(source: &AgentInputCitation) -> AgentHandoffInputCitationV1 {
    AgentHandoffInputCitationV1 {
        input: source.input,
        quote: source.quote.clone(),
    }
}

fn handoff_requirement(requirement: &AgentTaskRequirement) -> AgentHandoffRequirementV1 {
    AgentHandoffRequirementV1 {
        id: requirement.id.clone(),
        description: requirement.description.clone(),
        source: handoff_citation(&requirement.source),
        origin: requirement
            .origin
            .as_ref()
            .map(|origin| AgentHandoffRequirementOriginV1 {
                turn_operation_id: origin.turn_operation_id.clone(),
                requirement_id: origin.requirement_id.clone(),
                source: handoff_citation(&origin.source),
            }),
    }
}

fn handoff_plan(plan: &AgentPlan) -> AgentHandoffPlanV1 {
    AgentHandoffPlanV1 {
        revision: plan.revision,
        explanation: plan.explanation.clone(),
        steps: plan
            .steps
            .iter()
            .map(|step| AgentHandoffPlanStepV1 {
                step: step.step.clone(),
                status: match step.status {
                    AgentPlanStatus::Pending => "pending",
                    AgentPlanStatus::InProgress => "in_progress",
                    AgentPlanStatus::Completed => "completed",
                    AgentPlanStatus::Blocked => "blocked",
                }
                .to_owned(),
            })
            .collect(),
        needs_replan: plan.needs_replan,
    }
}

fn handoff_completion(report: &AgentCompletionReport) -> AgentHandoffCompletionAccountV1 {
    AgentHandoffCompletionAccountV1 {
        plan_revision: report.plan_revision,
        observation_revision: report.observation_revision,
        workspace_epoch: report.workspace_epoch,
        summary: report.summary.clone(),
        criteria: report
            .criteria
            .iter()
            .map(|criterion| AgentHandoffCompletionCriterionV1 {
                step: criterion.step.clone(),
                disposition: match criterion.disposition {
                    AgentCriterionDisposition::Supported => "supported",
                    AgentCriterionDisposition::Unverified => "unverified",
                    AgentCriterionDisposition::Blocked => "blocked",
                    AgentCriterionDisposition::ScopeChanged => "scope_changed",
                }
                .to_owned(),
                evidence_call_ids: criterion.evidence_call_ids.clone(),
                rationale: criterion.rationale.clone(),
                requirement_ids: criterion.requirement_ids.clone(),
                scope_change: criterion.scope_change.as_ref().map(handoff_citation),
            })
            .collect(),
    }
}

async fn deterministic_agent_handoff(
    state: &NomiCoreAgentApiState,
    session_id: &AgentSessionId,
    source: &AgentBindingValue,
    target: &AgentBindingValue,
) -> Result<(Option<AgentHandoffEnvelopeV1>, bool), NomiCoreApiError> {
    let operation_id: Option<String> = sqlx::query_scalar(
        "SELECT operation_id FROM agent_turns \
         WHERE session_id = ? AND state IN ('completed', 'failed', 'cancelled') \
         ORDER BY finished_at DESC, rowid DESC LIMIT 1",
    )
    .bind(session_id.as_ref())
    .fetch_optional(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(format!("read latest closed Agent Turn: {error}")))?;
    let Some(operation_id) = operation_id else {
        return Ok((None, false));
    };
    let operation = OperationId::from(operation_id.clone());
    let facts = state
        .session_owner
        .canonical()
        .store()
        .chat_causality_facts(session_id, &operation)
        .await
        .map_err(agent_session_store_error)?;
    let terminal = facts
        .events
        .iter()
        .filter(|event| {
            event.correlation_id.as_ref() == operation_id
                && matches!(
                    event.kind.0.as_str(),
                    "turn/completed" | "turn/failed" | "turn/cancelled"
                )
        })
        .min_by_key(|event| event.seq)
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::CONFLICT,
                "AGENT_SESSION_HANDOFF_SOURCE_INVALID",
                "latest closed Turn has no canonical terminal boundary",
            )
        })?;
    let mut engine_events = Vec::new();
    for event in facts.events.iter().filter(|event| {
        event.kind.0 == "runtime/progress-recorded"
            && event.correlation_id.as_ref() == operation_id
            && event.seq < terminal.seq
    }) {
        let Some(value) = facts
            .event_payloads
            .get(event.event_id.as_ref())
            .and_then(|payload| payload.get("event"))
        else {
            continue;
        };
        if value
            .get("event")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.starts_with("host_"))
        {
            continue;
        }
        let parsed = serde_json::from_value::<AgentEngineEvent>(value.clone()).map_err(|error| {
            NomiCoreApiError::with_details(
                StatusCode::CONFLICT,
                "AGENT_SESSION_HANDOFF_SOURCE_INVALID",
                "latest closed Turn contains an invalid Runtime event",
                json!({ "operation_id": operation_id, "error": error.to_string() }),
            )
        })?;
        engine_events.push(parsed);
    }
    let exact_source = engine_events.first().is_some_and(|event| match event {
        AgentEngineEvent::TurnStarted {
            binding,
            turn_operation_id,
        } => {
            binding.agent_session_id() == session_id
                && binding.resolved_snapshot_ref() == &source.resolved_snapshot_ref
                && turn_operation_id.as_ref() == operation_id
        }
        _ => false,
    });
    if !exact_source {
        return Ok((None, false));
    }
    if !matches!(
        engine_events.last(),
        Some(
            AgentEngineEvent::TurnCompleted { .. }
                | AgentEngineEvent::TurnFailed { .. }
                | AgentEngineEvent::TurnCancelled { .. }
        )
    ) {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_HANDOFF_SOURCE_INVALID",
            "latest closed Turn has no exact Runtime terminal record",
        ));
    }
    let plan_entry = engine_events
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, event)| match event {
            AgentEngineEvent::PlanUpdated { plan } => Some((index, plan.clone())),
            _ => None,
        });
    let completion_entry = engine_events
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, event)| match event {
            AgentEngineEvent::CompletionReported { report } => Some((index, report.clone())),
            _ => None,
        });
    let plan = plan_entry.as_ref().map(|(_, plan)| plan.clone());
    let completion = completion_entry.and_then(|(completion_index, report)| {
        plan_entry
            .as_ref()
            .is_some_and(|(plan_index, plan)| {
                completion_index > *plan_index
                    && report.plan_revision == plan.revision
                    && report.requirements == plan.requirements
            })
            .then_some(report)
    });
    let running_processes = engine_events.iter().rev().find_map(|event| match event {
        AgentEngineEvent::WorkStatus { status } => Some(!status.running_processes.is_empty()),
        _ => None,
    }).unwrap_or(false);

    let mut publish_calls = BTreeSet::new();
    let mut verified_artifacts = Vec::new();
    for event in &engine_events {
        match event {
            AgentEngineEvent::ToolStarted {
                call_id, action_id, ..
            } if action_id.as_ref() == "workspace.artifacts/publish" => {
                publish_calls.insert(call_id.as_ref().to_owned());
            }
            AgentEngineEvent::ToolCompleted { result, .. }
                if !result.is_error && publish_calls.contains(result.call_id.as_ref()) =>
            {
                if let Ok(artifact) =
                    serde_json::from_str::<nomifun_file::PublishedWorkspaceArtifact>(
                        &result.output_text(),
                    )
                {
                    verified_artifacts.push(AgentHandoffVerifiedArtifactV1 {
                        artifact_id: artifact.artifact_id,
                        source_path: artifact.source_path,
                        relative_path: artifact.relative_path,
                        mime_type: artifact.mime_type,
                        size_bytes: artifact.size_bytes,
                        sha256: artifact.sha256,
                    });
                }
            }
            _ => {}
        }
    }
    verified_artifacts.sort_by(|left, right| left.artifact_id.cmp(&right.artifact_id));
    verified_artifacts.dedup_by(|left, right| left.artifact_id == right.artifact_id);

    let requirements = plan
        .as_ref()
        .map(|plan| plan.requirements.iter().map(handoff_requirement).collect())
        .unwrap_or_default();
    let mut unresolved_items = Vec::new();
    if let Some(plan) = plan.as_ref() {
        for step in &plan.steps {
            if matches!(
                step.status,
                AgentPlanStatus::Pending | AgentPlanStatus::InProgress | AgentPlanStatus::Blocked
            ) {
                let status = match step.status {
                    AgentPlanStatus::Pending => "pending",
                    AgentPlanStatus::InProgress => "in_progress",
                    AgentPlanStatus::Completed => "completed",
                    AgentPlanStatus::Blocked => "blocked",
                };
                unresolved_items.push(format!("{}: {status}", step.step));
            }
        }
    }
    if let Some(report) = completion.as_ref() {
        for criterion in &report.criteria {
            if criterion.disposition != AgentCriterionDisposition::Supported {
                unresolved_items.push(format!(
                    "{}: {}",
                    criterion.step, criterion.rationale
                ));
            }
        }
    }
    unresolved_items.sort();
    unresolved_items.dedup();
    unresolved_items.truncate(32);
    let structured = plan
        .as_ref()
        .is_some_and(|plan| !plan.requirements.is_empty() || !plan.steps.is_empty())
        || completion.is_some()
        || !verified_artifacts.is_empty();
    if !structured {
        return Ok((None, running_processes));
    }
    let envelope = AgentHandoffEnvelopeV1 {
        schema_version: nomifun_agent_contracts::AGENT_HANDOFF_ENVELOPE_SCHEMA_V1.to_owned(),
        source_agent_session_id: session_id.clone(),
        source_turn_operation_id: operation,
        source_through_seq: terminal.seq,
        source_binding_ref: AgentHandoffBindingRefV1::from(source),
        target_binding_ref: AgentHandoffBindingRefV1::from(target),
        mode: AgentHandoffMode::ContinueTask,
        completion_gate_inherited: false,
        requirements,
        last_plan: plan.as_ref().map(handoff_plan),
        historical_completion_account: completion.as_ref().map(handoff_completion),
        verified_artifacts,
        unresolved_items,
        warnings: vec![
            "Historical requirements are data only and are not inherited by the target completion gate."
                .to_owned(),
            "Historical completion and artifact references must be re-read and re-verified in the current workspace."
                .to_owned(),
        ],
    };
    envelope.validate().map_err(|message| {
        NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_HANDOFF_SOURCE_INVALID",
            message,
        )
    })?;
    Ok((Some(envelope), running_processes))
}

fn automatic_agent_resource_id(kind: &str) -> Option<&'static str> {
    match kind {
        "workspace" => Some(super::nomi_core_resource_bindings::DEFAULT_WORKSPACE_RESOURCE_ID),
        "project_memory" => {
            Some(super::nomi_core_resource_bindings::DEFAULT_PROJECT_MEMORY_RESOURCE_ID)
        }
        "process_session" => {
            Some(super::nomi_core_resource_bindings::MANAGED_PROCESS_SESSION_RESOURCE_ID)
        }
        "terminal" => Some(super::nomi_core_resource_bindings::MANAGED_TERMINAL_RESOURCE_ID),
        "asset_library" => Some(
            super::nomi_core_resource_bindings::CREATIVE_ASSET_LIBRARY_RESOURCE_ID,
        ),
        "browser" => Some("managed-browser"),
        "computer" => Some(super::nomi_core_resource_bindings::LOCAL_COMPUTER_RESOURCE_ID),
        "scheduler" => {
            Some(super::nomi_core_resource_bindings::INSTALLATION_SCHEDULER_RESOURCE_ID)
        }
        _ => None,
    }
}

fn missing_agent_switch_resource_kinds(
    required_kinds: &BTreeSet<String>,
    bound_kinds: &BTreeSet<String>,
) -> Vec<String> {
    required_kinds
        .iter()
        .filter(|kind| {
            !bound_kinds.contains(*kind)
                && !super::nomi_core_resource_bindings::optional_unbound_resource_kind(kind)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod agent_switch_resource_tests {
    use super::missing_agent_switch_resource_kinds;
    use std::collections::BTreeSet;

    #[test]
    fn unavailable_browser_is_not_a_switch_blocker_when_unbound() {
        let required = BTreeSet::from([
            "browser".to_owned(),
            "knowledge_base".to_owned(),
            "workspace".to_owned(),
        ]);
        let bound = BTreeSet::from(["workspace".to_owned()]);
        assert!(missing_agent_switch_resource_kinds(&required, &bound).is_empty());
    }

    #[test]
    fn computer_and_workspace_still_require_bindings() {
        let required = BTreeSet::from([
            "browser".to_owned(),
            "computer".to_owned(),
            "workspace".to_owned(),
        ]);
        let bound = BTreeSet::from(["workspace".to_owned()]);
        assert_eq!(
            missing_agent_switch_resource_kinds(&required, &bound),
            vec!["computer".to_owned()]
        );
        assert_eq!(
            missing_agent_switch_resource_kinds(&required, &BTreeSet::new()),
            vec!["computer".to_owned(), "workspace".to_owned()]
        );
    }
}

async fn agent_switch_recovery_blocker(
    state: &NomiCoreAgentApiState,
    session_id: &AgentSessionId,
) -> Result<Option<AgentSwitchBlockerDto>, NomiCoreApiError> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT seq, inline_json FROM agent_events \
         WHERE session_id = ? AND kind = 'runtime/progress-recorded' \
           AND seq > COALESCE((SELECT MAX(seq) FROM agent_events \
             WHERE session_id = ? AND kind = 'session/agent-binding-changed'), 0) \
         ORDER BY seq ASC",
    )
    .bind(session_id.as_ref())
    .bind(session_id.as_ref())
    .fetch_all(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(format!("read Agent patch recovery: {error}")))?;
    agent_switch_recovery_blocker_from_rows(rows)
}

fn agent_switch_recovery_blocker_from_rows(
    rows: Vec<(i64, String)>,
) -> Result<Option<AgentSwitchBlockerDto>, NomiCoreApiError> {
    let mut latest_state: Option<(i64, nomifun_agent_runtime::AgentPatchRecoveryState)> = None;
    let mut latest_patch_dispatch = 0_i64;
    for (seq, raw) in rows {
        let payload: Value = serde_json::from_str(&raw).map_err(|error| {
            AppError::Internal(format!("decode Agent patch recovery event: {error}"))
        })?;
        let Some(event) = payload.get("event") else {
            continue;
        };
        if event.get("event").and_then(Value::as_str) == Some("host_tool_dispatch")
            && event
                .get("dispatch")
                .and_then(|dispatch| dispatch.get("action_id"))
                .and_then(Value::as_str)
                == Some("workspace.files/patch")
        {
            latest_patch_dispatch = seq;
        }
        if let Ok(AgentEngineEvent::PatchRecoveryUpdated { state }) =
            serde_json::from_value::<AgentEngineEvent>(event.clone())
        {
            state.validate().map_err(|error| {
                NomiCoreApiError::new(
                    StatusCode::CONFLICT,
                    "AGENT_SESSION_HANDOFF_RECOVERY_PENDING",
                    error.to_string(),
                )
            })?;
            latest_state = Some((seq, state));
        }
    }
    let pending = latest_state
        .as_ref()
        .is_some_and(|(_, state)| state.has_pending());
    let uncovered = latest_patch_dispatch > latest_state.as_ref().map_or(0, |(seq, _)| *seq);
    Ok((pending || uncovered).then(|| AgentSwitchBlockerDto {
        code: "AGENT_SESSION_HANDOFF_RECOVERY_PENDING".to_owned(),
        message: "Resolve the pending Patch recovery before switching Agents".to_owned(),
        details: Some(json!({
            "pending": pending,
            "uncovered_patch_dispatch": uncovered,
            "recovery": "complete_or_explicitly_clear_patch_recovery",
        })),
    }))
}

async fn agent_switch_blockers(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    observation: &SessionObservation,
    source_has_running_processes: bool,
) -> Result<Vec<AgentSwitchBlockerDto>, NomiCoreApiError> {
    let session_id = &observation.session.agent_session_id;
    let mut blockers = Vec::new();
    if observation.session.remote_binding_provenance.is_some() {
        blockers.push(AgentSwitchBlockerDto {
            code: "AGENT_SESSION_AGENT_IS_REMOTE_FROZEN".to_owned(),
            message: "Remote AgentSession Agent bindings cannot be changed locally".to_owned(),
            details: None,
        });
    }
    if observation.head.status == "running" || observation.head.active_turn_id.is_some() {
        blockers.push(AgentSwitchBlockerDto {
            code: "AGENT_SESSION_TURN_ACTIVE".to_owned(),
            message: "Wait for the active Turn before switching Agents".to_owned(),
            details: observation
                .head
                .active_turn_id
                .as_ref()
                .map(|turn_id| json!({ "active_turn_id": turn_id })),
        });
    }
    let attempt: Option<String> = sqlx::query_scalar(
        "SELECT execution_id FROM conversation_execution_links \
         WHERE conversation_id = ? AND relation IN ('attempt', 'automation') \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(session_id.as_ref())
    .fetch_optional(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    if let Some(execution_id) = attempt {
        blockers.push(AgentSwitchBlockerDto {
            code: "AGENT_EXECUTION_ATTEMPT_READ_ONLY".to_owned(),
            message: "AgentExecution Attempt audit Sessions cannot switch Agents".to_owned(),
            details: Some(json!({ "execution_id": execution_id })),
        });
    }
    let active_execution: Option<(String, String)> = sqlx::query_as(
        "SELECT execution.execution_id, execution.status \
         FROM conversation_execution_links link \
         JOIN agent_executions execution ON execution.execution_id = link.execution_id \
         WHERE link.conversation_id = ? AND link.relation = 'lead' AND link.active = 1 \
           AND execution.user_id = ? AND execution.deleted_at IS NULL \
           AND execution.status NOT IN ('completed', 'completed_with_failures', 'failed', 'cancelled') \
         ORDER BY link.updated_at DESC, link.id DESC LIMIT 1",
    )
    .bind(session_id.as_ref())
    .bind(owner.as_ref())
    .fetch_optional(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    if let Some((execution_id, status)) = active_execution {
        blockers.push(AgentSwitchBlockerDto {
            code: "AGENT_EXECUTION_ACTIVE".to_owned(),
            message: "Wait for or cancel the active AgentExecution before switching Agents"
                .to_owned(),
            details: Some(json!({ "execution_id": execution_id, "status": status })),
        });
    }
    let product_resource = observation
        .session
        .agent_binding
        .typed_resource_bindings
        .iter()
        .find(|binding| {
            matches!(
                binding.resource_kind.as_ref(),
                "companion" | "companion_memory" | "customer" | "canvas" | "channel"
            )
        });
    let cron_bound: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE conversation_id = ?)",
    )
    .bind(session_id.as_ref())
    .fetch_one(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    if product_resource.is_some() || cron_bound != 0 {
        blockers.push(AgentSwitchBlockerDto {
            code: "AGENT_SESSION_AGENT_SWITCH_UNSUPPORTED".to_owned(),
            message: "This product-bound Session owns a fixed Agent identity".to_owned(),
            details: Some(json!({
                "resource_kind": product_resource.map(|binding| binding.resource_kind.as_ref()),
                "cron_bound": cron_bound != 0,
            })),
        });
    }
    if state
        .session_owner
        .canonical()
        .store()
        .has_unsettled_effects(session_id)
        .await
        .map_err(agent_session_store_error)?
        || source_has_running_processes
    {
        blockers.push(AgentSwitchBlockerDto {
            code: "AGENT_SESSION_EFFECTS_UNSETTLED".to_owned(),
            message: "Settle effects and descendant processes before switching Agents".to_owned(),
            details: Some(json!({ "source_has_running_processes": source_has_running_processes })),
        });
    }
    if let Some(blocker) = agent_switch_recovery_blocker(state, session_id).await? {
        blockers.push(blocker);
    }
    Ok(blockers)
}

async fn prepare_agent_session_switch(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
    selection: &AgentSwitchSelectionDto,
    explicit_model: Option<&AgentChatModelSelectionDto>,
) -> Result<PreparedAgentSessionSwitch, NomiCoreApiError> {
    let principal = authenticated_principal(owner);
    let observation = state
        .session_owner
        .canonical()
        .get(&principal, session_id)
        .await?;
    let current_dto = agent_binding_dto(&observation.session.agent_binding)?;
    let (_, current_revision, current_snapshot) = state
        .control_plane
        .saved_binding_artifacts(&owner.0, &current_dto)
        .await?;
    let current_knowledge =
        agent_session_knowledge_binding(&observation.session.agent_binding)?;
    let current_model = current_revision
        .payload
        .chat_route_records
        .get(nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT)
        .map(|route| AgentChatModelSelectionDto {
            provider_id: route.primary.provider_id.clone(),
            model: route.primary.model.clone(),
        });
    let target_preset_id = match selection {
        AgentSwitchSelectionDto::Preset { preset_id } => preset_id.clone(),
        AgentSwitchSelectionDto::Template { template_key } => state
            .control_plane
            .create_from_template(
                &owner.0,
                template_key,
                CreateAgentPresetFromTemplateRequest {
                    model: explicit_model.cloned().or_else(|| current_model.clone()),
                    reuse_existing: true,
                    display_name: template_key.clone(),
                    description: None,
                    model_route_refs: BTreeMap::new(),
                    chat_route_records: BTreeMap::new(),
                },
            )
            .await?
            .preset
            .preset_id,
    };
    let raw_target = state
        .control_plane
        .resolve_agent_session_agent_binding(
            &owner.0,
            &current_dto,
            &target_preset_id,
            explicit_model,
        )
        .await
        .map_err(|error| {
            if error.code().as_ref().starts_with("MODEL_") {
                NomiCoreApiError::with_details(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "AGENT_SESSION_MODEL_INCOMPATIBLE",
                    "The current model cannot satisfy the target Agent",
                    json!({
                        "control_plane_code": error.code().as_ref(),
                        "control_plane_details": error.details(),
                        "recovery": "select_compatible_model",
                        "settings_section": "chat",
                    }),
                )
            } else {
                error.into()
            }
        })?;
    let (_, target_revision, target_snapshot) = state
        .control_plane
        .saved_binding_artifacts(&owner.0, &raw_target)
        .await?;
    state
        .product_agent_resolver
        .official_runtime
        .validate_agent(&target_snapshot)?;
    let current_label = state
        .control_plane
        .editor(
            &owner.0,
            current_revision.reference.preset_id.as_ref(),
            Some(current_revision.reference.revision),
        )
        .await?
        .preset
        .display_name;
    let target_label = state
        .control_plane
        .editor(
            &owner.0,
            target_revision.reference.preset_id.as_ref(),
            Some(target_revision.reference.revision),
        )
        .await?
        .preset
        .display_name;

    let target_kinds = target_snapshot
        .content
        .required_resource_kinds
        .iter()
        .map(|kind| kind.as_ref().to_owned())
        .collect::<BTreeSet<_>>();
    let mut selections = Vec::new();
    let mut selected_keys = BTreeSet::new();
    let selected_workspace = observation
        .session
        .agent_binding
        .typed_resource_bindings
        .iter()
        .find(|binding| binding.resource_kind.as_ref() == "workspace")
        .and_then(|binding| binding.typed_parameters.get("workspace_root"))
        .cloned();
    for binding in &observation.session.agent_binding.typed_resource_bindings {
        let kind = binding.resource_kind.as_ref();
        if !target_kinds.contains(kind) {
            continue;
        }
        let resource_id = if kind == "workspace" && selected_workspace.is_some() {
            super::nomi_core_resource_bindings::DEFAULT_WORKSPACE_RESOURCE_ID.to_owned()
        } else {
            binding.resource_id.as_ref().to_owned()
        };
        if selected_keys.insert((kind.to_owned(), resource_id.clone())) {
            selections.push(AgentResourceSelectionDto {
                resource_kind: kind.to_owned(),
                resource_id,
            });
        }
    }
    for kind in &target_kinds {
        if selections
            .iter()
            .any(|selection| selection.resource_kind == *kind)
        {
            continue;
        }
        if let Some(resource_id) = automatic_agent_resource_id(kind) {
            selections.push(AgentResourceSelectionDto {
                resource_kind: kind.clone(),
                resource_id: resource_id.to_owned(),
            });
        }
    }
    selections.sort_by(|left, right| {
        left.resource_kind
            .cmp(&right.resource_kind)
            .then_with(|| left.resource_id.cmp(&right.resource_id))
    });

    let mut blockers = Vec::new();
    let mut replacement_dto = match state
        .resource_bindings
        .resolve_for_saved_binding(
            &state.control_plane,
            &owner.0,
            raw_target.clone(),
            &selections,
        )
        .await
    {
        Ok(binding) => binding,
        Err(error) => {
            blockers.push(AgentSwitchBlockerDto {
                code: if matches!(
                    error.code(),
                    "PRESET_RESOURCE_NOT_BOUND" | "RESOURCE_SELECTION_REQUIRED"
                ) {
                    "AGENT_SESSION_RESOURCE_REQUIRED".to_owned()
                } else {
                    error.code().to_owned()
                },
                message: error.message().to_owned(),
                details: Some(json!({
                    "resource_details": error.details(),
                    "settings_section": "agents",
                    "recovery": "configure_target_agent_resources",
                })),
            });
            raw_target
        }
    };
    if let Some(workspace) = selected_workspace.as_deref() {
        freeze_selected_workspace(
            &mut replacement_dto,
            owner.as_ref(),
            workspace,
            WorkspaceDirectoryCheck::Runtime,
        )?;
    }
    if replacement_dto.typed_resource_bindings.iter().any(|resource| {
        resource.resource_kind == nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
    }) {
        if current_knowledge.enabled {
            freeze_agent_session_knowledge_policy(
                &mut replacement_dto,
                Some(&AgentSessionKnowledgePolicyDto {
                    writeback: current_knowledge.writeback,
                    writeback_eagerness: current_knowledge.writeback_eagerness,
                }),
            )?;
        } else {
            for resource in replacement_dto.typed_resource_bindings.iter_mut().filter(
                |resource| {
                    resource.resource_kind
                        == nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
                },
            ) {
                resource.typed_parameters.insert(
                    nomifun_agent_domain_wave1::KNOWLEDGE_ENABLED_PARAMETER.to_owned(),
                    "false".to_owned(),
                );
                resource.typed_parameters.insert(
                    nomifun_agent_domain_wave1::KNOWLEDGE_WRITEBACK_PARAMETER.to_owned(),
                    "false".to_owned(),
                );
                resource.typed_parameters.insert(
                    nomifun_agent_domain_wave1::KNOWLEDGE_WRITEBACK_EAGERNESS_PARAMETER.to_owned(),
                    "manual".to_owned(),
                );
            }
        }
    }
    replacement_dto.binding_version = observation
        .session
        .agent_binding
        .binding_version
        .checked_add(1)
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::CONFLICT,
                "AGENT_SESSION_BINDING_CHANGED",
                "AgentSession binding version cannot advance",
            )
        })?;
    let replacement: AgentBindingValue = serde_json::to_value(&replacement_dto)
        .and_then(serde_json::from_value)
        .map_err(|error| {
            AppError::Conflict(format!("resolved Session Agent binding is invalid: {error}"))
        })?;
    let (handoff, running_processes) = deterministic_agent_handoff(
        state,
        session_id,
        &observation.session.agent_binding,
        &replacement,
    )
    .await?;
    blockers.extend(
        agent_switch_blockers(state, owner, &observation, running_processes).await?,
    );

    let source_capabilities = current_snapshot
        .content
        .contributions()
        .map(|capability| capability.capability.id.as_ref().to_owned())
        .collect::<BTreeSet<_>>();
    let target_capabilities = target_snapshot
        .content
        .contributions()
        .map(|capability| capability.capability.id.as_ref().to_owned())
        .collect::<BTreeSet<_>>();
    let active_capability_ids = target_snapshot
        .content
        .enabled_capabilities
        .iter()
        .filter(|capability| capability.consumption.is_contribution())
        .map(|capability| capability.capability.id.as_ref().to_owned())
        .collect::<Vec<_>>();
    let current_resources = observation
        .session
        .agent_binding
        .typed_resource_bindings
        .iter()
        .map(|resource| AgentResourceSelectionDto {
            resource_kind: resource.resource_kind.as_ref().to_owned(),
            resource_id: resource.resource_id.as_ref().to_owned(),
        })
        .collect::<BTreeSet<_>>();
    let target_resources = replacement
        .typed_resource_bindings
        .iter()
        .map(|resource| AgentResourceSelectionDto {
            resource_kind: resource.resource_kind.as_ref().to_owned(),
            resource_id: resource.resource_id.as_ref().to_owned(),
        })
        .collect::<BTreeSet<_>>();
    let bound_kinds = replacement
        .typed_resource_bindings
        .iter()
        .map(|binding| binding.resource_kind.as_ref().to_owned())
        .collect::<BTreeSet<_>>();
    let missing_kinds = missing_agent_switch_resource_kinds(&target_kinds, &bound_kinds);
    if !missing_kinds.is_empty()
        && !blockers
            .iter()
            .any(|blocker| blocker.code == "AGENT_SESSION_RESOURCE_REQUIRED")
    {
        blockers.push(AgentSwitchBlockerDto {
            code: "AGENT_SESSION_RESOURCE_REQUIRED".to_owned(),
            message: "The target Agent requires additional configured resources".to_owned(),
            details: Some(json!({
                "missing_resource_kinds": missing_kinds,
                "settings_section": "agents",
            })),
        });
    }
    let target_model = target_revision
        .payload
        .chat_route_records
        .get(nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT)
        .map(|route| AgentChatModelSelectionDto {
            provider_id: route.primary.provider_id.clone(),
            model: route.primary.model.clone(),
        })
        .or_else(|| explicit_model.cloned())
        .or_else(|| current_model.clone())
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "AGENT_SESSION_MODEL_INCOMPATIBLE",
                "The target Agent has no compatible Chat model",
            )
        })?;
    let handoff_view = AgentHandoffAvailabilityDto {
        available: handoff.is_some(),
        requirement_count: handoff.as_ref().map_or(0, |handoff| handoff.requirements.len()),
        verified_artifact_count: handoff
            .as_ref()
            .map_or(0, |handoff| handoff.verified_artifacts.len()),
        unresolved_item_count: handoff
            .as_ref()
            .map_or(0, |handoff| handoff.unresolved_items.len()),
        completion_gate_inherited: false,
    };
    let preview = PreviewAgentSessionSwitchResponseDto {
        current: AgentSwitchIdentityDto {
            label: current_label.clone(),
            preset_id: current_revision.reference.preset_id.as_ref().to_owned(),
            preset_revision: current_revision.reference.revision,
            resolved_snapshot_ref: serde_json::to_value(&current_snapshot.snapshot_ref)
                .and_then(serde_json::from_value)?,
            binding_version: observation.session.agent_binding.binding_version,
        },
        target: AgentSwitchIdentityDto {
            label: target_label.clone(),
            preset_id: target_revision.reference.preset_id.as_ref().to_owned(),
            preset_revision: target_revision.reference.revision,
            resolved_snapshot_ref: serde_json::to_value(&target_snapshot.snapshot_ref)
                .and_then(serde_json::from_value)?,
            binding_version: replacement.binding_version,
        },
        model: AgentSwitchModelPreviewDto {
            provider_id: target_model.provider_id.clone(),
            model: target_model.model.clone(),
            preserved: explicit_model.is_none()
                && current_model.as_ref().is_some_and(|current| current == &target_model),
            compatible: true,
            missing_features: Vec::new(),
        },
        resources: AgentSwitchResourceDiffDto {
            retained: current_resources
                .intersection(&target_resources)
                .cloned()
                .collect(),
            dropped: current_resources
                .difference(&target_resources)
                .cloned()
                .collect(),
            missing_kinds,
        },
        capabilities: AgentSwitchCapabilityDiffDto {
            gained: target_capabilities
                .difference(&source_capabilities)
                .cloned()
                .collect(),
            lost: source_capabilities
                .difference(&target_capabilities)
                .cloned()
                .collect(),
        },
        handoff: handoff_view,
        expected_binding_version: observation.session.agent_binding.binding_version,
        can_apply: blockers.is_empty(),
        blockers,
    };
    Ok(PreparedAgentSessionSwitch {
        preview,
        expected: observation.session.agent_binding,
        replacement,
        source_label: current_label,
        target_label,
        handoff,
        active_capability_ids,
    })
}

async fn preview_nomi_core_agent_session_agent_switch(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(request): Json<PreviewAgentSessionSwitchRequestDto>,
) -> Result<Json<ApiResponse<PreviewAgentSessionSwitchResponseDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let prepared = prepare_agent_session_switch(
        &state,
        &owner,
        &session_id,
        &request.selection,
        request.model.as_ref(),
    )
    .await?;
    Ok(Json(ApiResponse::ok(prepared.preview)))
}

fn agent_switch_blocker_error(blocker: &AgentSwitchBlockerDto) -> NomiCoreApiError {
    let status = if matches!(
        blocker.code.as_str(),
        "AGENT_SESSION_MODEL_INCOMPATIBLE" | "AGENT_SESSION_RESOURCE_REQUIRED"
    ) {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::CONFLICT
    };
    NomiCoreApiError::with_details(
        status,
        blocker.code.clone(),
        blocker.message.clone(),
        blocker.details.clone().unwrap_or(Value::Null),
    )
}

fn agent_switch_store_error(error: nomifun_agent_session::SessionStoreError) -> NomiCoreApiError {
    let message = error.to_string();
    let code = match &error {
        nomifun_agent_session::SessionStoreError::Conflict(reason)
            if reason.contains("active Turn") =>
        {
            "AGENT_SESSION_TURN_ACTIVE"
        }
        nomifun_agent_session::SessionStoreError::Conflict(reason)
            if reason.contains("Remote") =>
        {
            "AGENT_SESSION_AGENT_IS_REMOTE_FROZEN"
        }
        nomifun_agent_session::SessionStoreError::Conflict(reason)
            if reason.contains("unsettled effects") =>
        {
            "AGENT_SESSION_EFFECTS_UNSETTLED"
        }
        nomifun_agent_session::SessionStoreError::Conflict(reason)
            if reason.contains("binding changed") || reason.contains("compare-and-swap") =>
        {
            "AGENT_SESSION_BINDING_CHANGED"
        }
        nomifun_agent_session::SessionStoreError::IdempotencyConflict(_) => {
            "IDEMPOTENCY_CONFLICT"
        }
        nomifun_agent_session::SessionStoreError::InvalidPayload(_) => {
            "AGENT_SESSION_HANDOFF_SOURCE_INVALID"
        }
        _ => "AGENT_SESSION_AGENT_SWITCH_UNSUPPORTED",
    };
    NomiCoreApiError::with_details(StatusCode::CONFLICT, code, message, Value::Null)
}

fn agent_switch_idempotency_key(headers: &HeaderMap) -> Result<String, NomiCoreApiError> {
    let value = headers
        .get("Idempotency-Key")
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::BAD_REQUEST,
                "NOMI_CORE_INVALID_REQUEST",
                "Agent switching requires a UUIDv7 Idempotency-Key",
            )
        })?
        .to_str()
        .map_err(|_| {
            NomiCoreApiError::new(
                StatusCode::BAD_REQUEST,
                "NOMI_CORE_INVALID_REQUEST",
                "Idempotency-Key must be visible ASCII",
            )
        })?;
    let value = canonical_nonempty(value, "Idempotency-Key")?;
    let uuid = Uuid::parse_str(&value).map_err(|_| {
        NomiCoreApiError::new(
            StatusCode::BAD_REQUEST,
            "NOMI_CORE_INVALID_REQUEST",
            "Agent switching Idempotency-Key must be a canonical UUIDv7",
        )
    })?;
    if uuid.get_version_num() != 7 || uuid.hyphenated().to_string() != value {
        return Err(NomiCoreApiError::new(
            StatusCode::BAD_REQUEST,
            "NOMI_CORE_INVALID_REQUEST",
            "Agent switching Idempotency-Key must be a canonical UUIDv7",
        ));
    }
    Ok(value)
}

async fn replay_agent_session_switch(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
    idempotency_key: &str,
    request_digest: &nomifun_agent_contracts::DigestHex,
) -> Result<Option<ApplyAgentSessionSwitchResponseDto>, NomiCoreApiError> {
    let existing: Option<(String, String, String)> = sqlx::query_as(
        "SELECT session_id, kind, inline_json FROM agent_events \
         WHERE producer_id = 'session_api' AND idempotency_key = ? LIMIT 1",
    )
    .bind(idempotency_key)
    .fetch_optional(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(format!("read Agent switch replay: {error}")))?;
    let Some((existing_session_id, kind, raw)) = existing else {
        return Ok(None);
    };
    if existing_session_id != session_id.as_ref() || kind != "session/agent-binding-changed" {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "IDEMPOTENCY_CONFLICT",
            "Idempotency-Key was already used by another command",
        ));
    }
    let transition: nomifun_agent_contracts::AgentBindingChangedPayloadV1 =
        serde_json::from_str(&raw).map_err(|error| {
            AppError::Internal(format!("decode Agent switch replay: {error}"))
        })?;
    if &transition.request_digest != request_digest {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "IDEMPOTENCY_CONFLICT",
            "Idempotency-Key was replayed with a different Agent switch request",
        ));
    }
    let handoff = if let Some(payload_id) = transition.handoff_payload_id.as_ref() {
        let stored: Option<(Vec<u8>, String)> = sqlx::query_as(
            "SELECT body, digest FROM agent_payloads WHERE payload_id = ? AND session_id = ?",
        )
        .bind(payload_id.as_ref())
        .bind(session_id.as_ref())
        .fetch_optional(&state.session_owner.pool)
        .await
        .map_err(|error| AppError::Internal(format!("read replayed Agent handoff: {error}")))?;
        let (body, stored_digest) = stored.ok_or_else(|| {
            AppError::Conflict("replayed Agent switch lost its handoff payload".to_owned())
        })?;
        if transition.handoff_payload_digest.as_ref().map(|digest| digest.as_ref())
            != Some(stored_digest.as_str())
        {
            return Err(AppError::Conflict(
                "replayed Agent handoff digest differs from its transition".to_owned(),
            )
            .into());
        }
        let body: SessionPayloadBody = serde_json::from_slice(&body).map_err(|error| {
            AppError::Internal(format!("decode replayed Agent handoff: {error}"))
        })?;
        match body {
            SessionPayloadBody::Json(value) => {
                let bytes = nomifun_agent_contracts::canonical_json_bytes(&value.0)
                    .map_err(|error| AppError::Internal(error.to_string()))?;
                if digest_bytes(&bytes).as_ref() != stored_digest {
                    return Err(AppError::Conflict(
                        "replayed Agent handoff body failed digest verification".to_owned(),
                    )
                    .into());
                }
                let envelope = serde_json::from_value::<AgentHandoffEnvelopeV1>(value.0)
                    .map_err(|error| {
                        AppError::Internal(format!("decode replayed Agent handoff: {error}"))
                    })?;
                envelope.validate().map_err(AppError::Conflict)?;
                Some(envelope)
            }
            _ => {
                return Err(AppError::Conflict(
                    "replayed Agent handoff is not canonical JSON".to_owned(),
                )
                .into());
            }
        }
    } else {
        None
    };
    let conversation = state
        .session_owner
        .canonical_conversation_projection(owner.as_ref(), session_id)
        .await?
        .ok_or_else(|| {
            AppError::Conflict(
                "replayed Agent-switched Session has no canonical projection".to_owned(),
            )
        })?;
    Ok(Some(ApplyAgentSessionSwitchResponseDto {
        conversation,
        transition_id: transition.transition_id.as_ref().to_owned(),
        previous_agent_label: transition.previous_agent_label,
        current_agent_label: transition.next_agent_label,
        binding_version: transition.next_binding_ref.binding_version,
        effective_from: "next_turn".to_owned(),
        handoff: AgentHandoffAvailabilityDto {
            available: handoff.is_some(),
            requirement_count: handoff
                .as_ref()
                .map_or(0, |handoff| handoff.requirements.len()),
            verified_artifact_count: handoff
                .as_ref()
                .map_or(0, |handoff| handoff.verified_artifacts.len()),
            unresolved_item_count: handoff
                .as_ref()
                .map_or(0, |handoff| handoff.unresolved_items.len()),
            completion_gate_inherited: false,
        },
        warnings: handoff
            .is_some()
            .then(|| {
                vec![
                    "Task facts were handed off as data only; source requirements were not inherited by the target completion gate."
                        .to_owned(),
                ]
            })
            .unwrap_or_default(),
    }))
}

async fn apply_nomi_core_agent_session_agent_switch(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ApplyAgentSessionSwitchRequestDto>,
) -> Result<Json<ApiResponse<ApplyAgentSessionSwitchResponseDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let idempotency_key = agent_switch_idempotency_key(&headers)?;
    let request_digest = digest_payload(&request)
        .map_err(|error| AppError::Internal(format!("digest Agent switch request: {error}")))?;
    let _operation_fence = state
        .session_owner
        .session_operation_lock(session_id.as_ref())
        .write_owned()
        .await;
    if let Some(replayed) = replay_agent_session_switch(
        &state,
        &owner,
        &session_id,
        &idempotency_key,
        &request_digest,
    )
    .await?
    {
        return Ok(Json(ApiResponse::ok(replayed)));
    }
    let current = state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
    if request.expected_binding_version != current.session.agent_binding.binding_version {
        return Err(NomiCoreApiError::with_details(
            StatusCode::CONFLICT,
            "AGENT_SESSION_BINDING_CHANGED",
            "AgentSession binding changed after preview",
            json!({
                "expected_binding_version": request.expected_binding_version,
                "actual_binding_version": current.session.agent_binding.binding_version,
            }),
        ));
    }
    let prepared = prepare_agent_session_switch(
        &state,
        &owner,
        &session_id,
        &request.selection,
        request.model.as_ref(),
    )
    .await?;
    if request.expected_binding_version != prepared.expected.binding_version {
        return Err(NomiCoreApiError::with_details(
            StatusCode::CONFLICT,
            "AGENT_SESSION_BINDING_CHANGED",
            "AgentSession binding changed after preview",
            json!({
                "expected_binding_version": request.expected_binding_version,
                "actual_binding_version": prepared.expected.binding_version,
            }),
        ));
    }
    if let Some(blocker) = prepared.preview.blockers.first() {
        return Err(agent_switch_blocker_error(blocker));
    }
    let mode = handoff_mode(request.handoff_mode);
    if mode == AgentHandoffMode::ContinueTask && prepared.handoff.is_none() {
        return Err(NomiCoreApiError::with_details(
            StatusCode::CONFLICT,
            "AGENT_SESSION_AGENT_SWITCH_UNSUPPORTED",
            "No structured latest-Turn task state is available for continue_task",
            json!({
                "supported_handoff_modes": ["context_only"],
                "completion_gate_inherited": false,
            }),
        ));
    }
    let transition_id = OperationId::from(idempotency_key.clone());
    let clear_reasoning_effort = if let Some(effort) = current.session.metadata.reasoning_effort {
        !saved_binding_supports_reasoning_effort(
            &state,
            &owner,
            &agent_binding_dto(&prepared.replacement)?,
            effort,
        )
        .await?
    } else {
        false
    };
    state
        .session_owner
        .runtime_sessions
        .terminate_and_wait_result(
            session_id.as_ref(),
            Some(AgentKillReason::ConfigurationChanged),
        )
        .await
        .map_err(|error| {
            NomiCoreApiError::with_details(
                StatusCode::CONFLICT,
                "AGENT_SESSION_AGENT_SWITCH_UNSUPPORTED",
                "The old Agent Runtime could not prove a complete teardown",
                json!({
                    "stage": "runtime_teardown",
                    "reason": error.to_string(),
                    "recovery": "retry_after_runtime_is_idle",
                }),
            )
        })?;
    super::hosted_effect_receipts::HostedEffectReceipts::new(state.session_owner.pool.clone())
        .ensure_settled(owner.as_ref(), session_id.as_ref())
        .await
        .map_err(|error| {
            NomiCoreApiError::with_details(
                StatusCode::CONFLICT,
                "AGENT_SESSION_EFFECTS_UNSETTLED",
                "AgentSession effects are not fully settled",
                json!({
                    "stage": "effect_settlement",
                    "reason": error.to_string(),
                    "recovery": "wait_or_reconcile_effects",
                }),
            )
        })?;
    let transitioned = state
        .session_owner
        .canonical()
        .store()
        .replace_session_agent_binding(
            &authenticated_principal(&owner),
            &session_id,
            nomifun_agent_session::ReplaceSessionAgentBinding {
                expected: prepared.expected,
                replacement: prepared.replacement,
                previous_agent_label: prepared.source_label.clone(),
                next_agent_label: prepared.target_label.clone(),
                transition_id: transition_id.clone(),
                request_digest,
                idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(idempotency_key),
                handoff_mode: mode,
                handoff: (mode == AgentHandoffMode::ContinueTask)
                    .then_some(prepared.handoff.clone())
                    .flatten(),
                initial_active_capability_ids: prepared.active_capability_ids,
            },
        )
        .await
        .map_err(agent_switch_store_error)?;
    let mut conversation = state
        .session_owner
        .canonical_conversation_projection(owner.as_ref(), &session_id)
        .await?
        .ok_or_else(|| {
            AppError::Conflict(
                "Agent-switched AgentSession has no canonical projection".to_owned(),
            )
        })?;
    if clear_reasoning_effort {
        state
            .session_owner
            .canonical()
            .store()
            .update_session_reasoning_effort(
                &authenticated_principal(&owner),
                &session_id,
                None,
            )
            .await
            .map_err(agent_session_store_error)?;
        conversation.reasoning_effort = None;
    }
    let handoff_view = AgentHandoffAvailabilityDto {
        available: mode == AgentHandoffMode::ContinueTask,
        requirement_count: prepared
            .handoff
            .as_ref()
            .map_or(0, |handoff| handoff.requirements.len()),
        verified_artifact_count: prepared
            .handoff
            .as_ref()
            .map_or(0, |handoff| handoff.verified_artifacts.len()),
        unresolved_item_count: prepared
            .handoff
            .as_ref()
            .map_or(0, |handoff| handoff.unresolved_items.len()),
        completion_gate_inherited: false,
    };
    let warnings = (mode == AgentHandoffMode::ContinueTask)
        .then(|| {
            vec![
                "Task facts were handed off as data only; source requirements were not inherited by the target completion gate."
                    .to_owned(),
            ]
        })
        .unwrap_or_default();
    let response = ApplyAgentSessionSwitchResponseDto {
        conversation: conversation.clone(),
        transition_id: transitioned.transition.transition_id.as_ref().to_owned(),
        previous_agent_label: prepared.source_label,
        current_agent_label: prepared.target_label,
        binding_version: transitioned.session.agent_binding.binding_version,
        effective_from: "next_turn".to_owned(),
        handoff: handoff_view,
        warnings,
    };
    state.session_owner.user_events.send_to_user(
        owner.as_ref(),
        WebSocketMessage::new(
            "agentSession.agentChanged",
            json!({
                "agent_session_id": session_id,
                "transition_id": response.transition_id,
                "previous_agent_label": response.previous_agent_label,
                "current_agent_label": response.current_agent_label,
                "binding_version": response.binding_version,
                "effective_from": response.effective_from,
                "conversation": conversation,
            }),
        ),
    );
    Ok(Json(ApiResponse::ok(response)))
}

async fn get_nomi_core_agent_session(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<SessionObservation>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let observation = state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
    Ok(Json(ApiResponse::ok(observation)))
}

async fn get_nomi_core_agent_session_projection(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<ConversationResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let projection = state
        .session_owner
        .canonical_conversation_projection(owner.as_ref(), &session_id)
        .await?
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::NOT_FOUND,
                "NOMI_CORE_AGENT_SESSION_NOT_FOUND",
                "AgentSession does not exist",
            )
        })?;
    Ok(Json(ApiResponse::ok(projection)))
}

async fn update_nomi_core_agent_session_metadata(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(update): Json<NomiCoreSessionMetadataUpdate>,
) -> Result<Json<ApiResponse<ConversationResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    state
        .session_owner
        .canonical()
        .store()
        .update_session_metadata(
            &authenticated_principal(&owner),
            &session_id,
            nomifun_agent_session::UpdateAgentSessionMetadata {
                title: update.name,
                archived: update.archived,
                pinned: update.pinned,
            },
        )
        .await
        .map_err(agent_session_store_error)?;
    let projection = state
        .session_owner
        .canonical_conversation_projection(owner.as_ref(), &session_id)
        .await?
        .ok_or_else(|| AppError::Conflict(
            "updated AgentSession has no canonical projection".to_owned(),
        ))?;
    Ok(Json(ApiResponse::ok(projection)))
}

async fn switch_nomi_core_agent_session_model(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(model): Json<AgentChatModelSelectionDto>,
) -> Result<Json<ApiResponse<ConversationResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let _operation_fence = state
        .session_owner
        .session_operation_lock(session_id.as_ref())
        .write_owned()
        .await;
    let observation = state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
    if observation.session.remote_binding_provenance.is_some() {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_MODEL_IS_REMOTE_FROZEN",
            "Remote AgentSession model selection is fixed by its Remote binding",
        ));
    }
    if observation.head.status == "running" || observation.head.active_turn_id.is_some() {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_TURN_ACTIVE",
            "wait for the active Turn before switching models",
        ));
    }
    let attempt_transcript: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM conversation_execution_links \
         WHERE conversation_id = ? AND relation = 'attempt')",
    )
    .bind(session_id.as_ref())
    .fetch_one(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    if attempt_transcript != 0 {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_EXECUTION_ATTEMPT_READ_ONLY",
            "AgentExecution Attempt transcripts cannot change their model binding",
        ));
    }

    let current_dto = agent_binding_dto(&observation.session.agent_binding)?;
    let replacement_dto = state
        .control_plane
        .resolve_agent_session_model_binding(&owner.0, &current_dto, &model)
        .await?;
    let replacement_supports_reasoning = if let Some(effort) =
        observation.session.metadata.reasoning_effort
    {
        saved_binding_supports_reasoning_effort(
            &state,
            &owner,
            &replacement_dto,
            effort,
        )
        .await?
    } else {
        true
    };
    let replacement: AgentBindingValue = serde_json::to_value(&replacement_dto)
        .and_then(serde_json::from_value)
        .map_err(|error| AppError::Conflict(format!(
            "resolved Session model binding is invalid: {error}"
        )))?;
    if replacement != observation.session.agent_binding {
        state
            .session_owner
            .canonical()
            .store()
            .replace_session_model_binding(
                &authenticated_principal(&owner),
                &session_id,
                &observation.session.agent_binding,
                replacement,
            )
            .await
            .map_err(agent_session_store_error)?;
    }
    if observation.session.metadata.reasoning_effort.is_some()
        && !replacement_supports_reasoning
    {
        state
            .session_owner
            .canonical()
            .store()
            .update_session_reasoning_effort(
                &authenticated_principal(&owner),
                &session_id,
                None,
            )
            .await
            .map_err(agent_session_store_error)?;
    }
    let projection = state
        .session_owner
        .canonical_conversation_projection(owner.as_ref(), &session_id)
        .await?
        .ok_or_else(|| {
            AppError::Conflict("model-switched AgentSession has no canonical projection".to_owned())
        })?;
    Ok(Json(ApiResponse::ok(projection)))
}

fn contract_reasoning_effort(value: SessionReasoningEffortDto) -> ReasoningEffort {
    match value {
        SessionReasoningEffortDto::Low => ReasoningEffort::Low,
        SessionReasoningEffortDto::Medium => ReasoningEffort::Medium,
        SessionReasoningEffortDto::High => ReasoningEffort::High,
        SessionReasoningEffortDto::XHigh => ReasoningEffort::XHigh,
        SessionReasoningEffortDto::Max => ReasoningEffort::Max,
        SessionReasoningEffortDto::Ultra => ReasoningEffort::Ultra,
    }
}

fn session_reasoning_effort_dto(value: ReasoningEffort) -> SessionReasoningEffortDto {
    match value {
        ReasoningEffort::Low => SessionReasoningEffortDto::Low,
        ReasoningEffort::Medium => SessionReasoningEffortDto::Medium,
        ReasoningEffort::High => SessionReasoningEffortDto::High,
        ReasoningEffort::XHigh => SessionReasoningEffortDto::XHigh,
        ReasoningEffort::Max => SessionReasoningEffortDto::Max,
        ReasoningEffort::Ultra => SessionReasoningEffortDto::Ultra,
    }
}

async fn saved_binding_supports_reasoning_effort(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    binding: &AgentBindingValueDto,
    effort: ReasoningEffort,
) -> Result<bool, NomiCoreApiError> {
    let (_, revision, snapshot) = state
        .control_plane
        .saved_binding_artifacts(&owner.0, binding)
        .await?;
    let Some(identity) = snapshot.content.chat_route_identity.as_ref() else {
        return Ok(false);
    };
    let record = revision
        .payload
        .chat_route_records
        .get(&identity.model_task)
        .ok_or_else(|| {
            AppError::Conflict(
                "AgentSession reasoning route is missing from its frozen Preset Revision"
                    .to_owned(),
            )
        })?;
    record.validate_for(identity).map_err(|error| {
        AppError::Conflict(format!(
            "AgentSession reasoning route differs from its frozen identity: {error}"
        ))
    })?;
    if !record.primary.features.contains(&ChatRouteFeature::Reasoning) {
        return Ok(false);
    }
    Ok(match record.primary.protocol {
        ChatRouteProtocol::OpenaiChat | ChatRouteProtocol::OpenaiResponses => true,
        ChatRouteProtocol::Gemini => matches!(
            effort,
            ReasoningEffort::Low | ReasoningEffort::Medium | ReasoningEffort::High
        ),
        _ => false,
    })
}

async fn update_nomi_core_agent_session_reasoning(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(update): Json<UpdateAgentSessionReasoningRequestDto>,
) -> Result<Json<ApiResponse<UpdateAgentSessionReasoningResponseDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let _operation_fence = state
        .session_owner
        .session_operation_lock(session_id.as_ref())
        .write_owned()
        .await;
    let observation = state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
    if observation.session.remote_binding_provenance.is_some() {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_REASONING_IS_REMOTE_FROZEN",
            "Remote AgentSession reasoning is fixed by its Remote binding",
        ));
    }
    if observation.head.status == "running" || observation.head.active_turn_id.is_some() {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_TURN_ACTIVE",
            "wait for the active Turn before changing reasoning effort",
        ));
    }
    let attempt_transcript: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM conversation_execution_links \
         WHERE conversation_id = ? AND relation = 'attempt')",
    )
    .bind(session_id.as_ref())
    .fetch_one(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    if attempt_transcript != 0 {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_EXECUTION_ATTEMPT_READ_ONLY",
            "AgentExecution Attempt transcripts cannot change reasoning effort",
        ));
    }
    if let Some(effort) = update.reasoning_effort {
        let binding = agent_binding_dto(&observation.session.agent_binding)?;
        if !saved_binding_supports_reasoning_effort(
            &state,
            &owner,
            &binding,
            contract_reasoning_effort(effort),
        )
        .await?
        {
            return Err(NomiCoreApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "AGENT_SESSION_REASONING_UNSUPPORTED",
                "The selected Chat model protocol does not support this reasoning effort",
            ));
        }
    }
    let reasoning_effort = update.reasoning_effort.map(contract_reasoning_effort);
    state
        .session_owner
        .canonical()
        .store()
        .update_session_reasoning_effort(
            &authenticated_principal(&owner),
            &session_id,
            reasoning_effort,
        )
        .await
        .map_err(agent_session_store_error)?;
    Ok(Json(ApiResponse::ok(
        UpdateAgentSessionReasoningResponseDto {
            reasoning_effort: reasoning_effort.map(session_reasoning_effort_dto),
        },
    )))
}

fn canonical_message_response(
    session_id: &AgentSessionId,
    created_at: i64,
    projection: MessageProjection,
) -> Result<Option<MessageResponse>, NomiCoreApiError> {
    canonical_message_response_with_observation(session_id, created_at, projection, None)
}

fn canonical_message_response_with_observation(
    session_id: &AgentSessionId,
    created_at: i64,
    projection: MessageProjection,
    observation: Option<&HistoricalToolObservation>,
) -> Result<Option<MessageResponse>, NomiCoreApiError> {
    let document = projection.projection.as_object().ok_or_else(|| {
        NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_MESSAGE_PROJECTION_INVALID",
            "canonical message projection is not an object",
        )
    })?;
    let state = document
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    if projection.presentation_intent == "turn_summary" {
        let Some(summary_message_id) = document.get("correlation_id").and_then(Value::as_str)
        else {
            return Ok(None);
        };
        let Some(root_message_id) = document
            .get("source_message_id")
            .and_then(Value::as_str)
        else {
            return Ok(None);
        };
        if [summary_message_id, root_message_id].into_iter().any(|value| {
            Uuid::parse_str(value)
                .ok()
                .is_none_or(|uuid| uuid.get_version_num() != 7)
        })
        {
            return Ok(None);
        }
        let stream_message_id =
            super::engine_journal::canonical_assistant_message_id(root_message_id)?;
        let started_at_ms = document.get("started_at_ms").cloned().unwrap_or(Value::Null);
        let finished_at_ms = document.get("finished_at_ms").cloned().unwrap_or(Value::Null);
        if matches!(state, "failed" | "interrupted") {
            let error = document.get("error").cloned().unwrap_or_else(|| {
                json!({
                    "message": "The upstream Agent failed while handling the request",
                    "code": "UNKNOWN_UPSTREAM_ERROR",
                    "ownership": "unknown_upstream",
                    "retryable": true,
                    "feedback_recommended": true,
                    "resolution": {
                        "kind": "send_feedback",
                        "target": "feedback"
                    }
                })
            });
            let content = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("The upstream Agent failed while handling the request");
            return Ok(Some(MessageResponse {
                message_id: summary_message_id.to_owned(),
                conversation_id: session_id.as_ref().to_owned(),
                msg_id: Some(stream_message_id),
                r#type: MessageType::Tips,
                content: json!({
                    "content": content,
                    "type": "error",
                    "error": error,
                    "turn_id": root_message_id,
                    "started_at_ms": started_at_ms,
                    "finished_at_ms": finished_at_ms,
                }),
                position: Some(MessagePosition::Center),
                status: Some(MessageStatus::Error),
                hidden: false,
                created_at: created_at.saturating_add(
                    i64::try_from(projection.first_seq).unwrap_or(i64::MAX),
                ),
            }));
        }
        let (activity_status, status) = match state {
            "running" => ("preparing", MessageStatus::Work),
            "completed" => ("prepared", MessageStatus::Finish),
            "cancelled" => ("error", MessageStatus::Error),
            _ => return Ok(None),
        };
        return Ok(Some(MessageResponse {
            message_id: summary_message_id.to_owned(),
            conversation_id: session_id.as_ref().to_owned(),
            // Match the live AgentStatus stream key so terminal hydration
            // replaces that transient row instead of rendering a duplicate.
            msg_id: Some(stream_message_id),
            r#type: MessageType::AgentStatus,
            content: json!({
                "backend": "nomi",
                "status": activity_status,
                "agent_name": "Nomi",
                "turn_id": root_message_id,
                "turn_summary": true,
                "started_seq": document.get("started_seq").cloned().unwrap_or(Value::Null),
                "finished_seq": document.get("finished_seq").cloned().unwrap_or(Value::Null),
                "started_at_ms": started_at_ms,
                "finished_at_ms": finished_at_ms,
            }),
            position: Some(MessagePosition::Center),
            status: Some(status),
            hidden: false,
            created_at: created_at.saturating_add(
                i64::try_from(projection.first_seq).unwrap_or(i64::MAX),
            ),
        }));
    }

    let Some(message_id) = document.get("correlation_id").and_then(Value::as_str) else {
        return Ok(None);
    };
    if Uuid::parse_str(message_id)
        .ok()
        .is_none_or(|uuid| uuid.get_version_num() != 7)
    {
        return Ok(None);
    }
    let (message_type, content, position) = match projection.presentation_intent.as_str() {
        "message" => (
            MessageType::Text,
            json!({
                "content": document.get("content").and_then(Value::as_str).unwrap_or_default(),
            }),
            if state == "accepted" {
                MessagePosition::Right
            } else {
                MessagePosition::Left
            },
        ),
        "thinking" => {
            let Some(turn_id) = document.get("turn_id").and_then(Value::as_str) else {
                return Ok(None);
            };
            if Uuid::parse_str(turn_id)
                .ok()
                .is_none_or(|uuid| uuid.get_version_num() != 7)
            {
                return Ok(None);
            }
            let content = document.get("content").and_then(Value::as_str).unwrap_or_default();
            if content.trim().is_empty() {
                return Ok(None);
            }
            (
                MessageType::Thinking,
                json!({ "content": content, "status": "done", "turn_id": turn_id }),
                MessagePosition::Left,
            )
        }
        "tool" => (
            MessageType::ToolCall,
            {
                let mut summary = document
                .get("tool_summary")
                .cloned()
                .unwrap_or_else(|| json!({
                    "call_id": message_id,
                    "name": "tool",
                    "args": {},
                }));
                if let Some(summary) = summary.as_object_mut() {
                    let recorded_error = summary.get("error").and_then(Value::as_str).map(str::to_owned);
                    summary.entry("status".to_owned()).or_insert_with(|| json!(
                        if recorded_error.is_some() { "error" }
                        else if state == "recorded" { "completed" }
                        else { "running" }
                    ));
                    if let Some(error) = recorded_error {
                        summary.entry("output".to_owned()).or_insert_with(|| json!(error));
                    }
                    if let Some(observation) = observation {
                        if let Some(turn_id) = &observation.turn_id {
                            summary.insert("turn_id".to_owned(), json!(turn_id));
                        }
                        if let Some(args) = &observation.args {
                            summary.insert("args".to_owned(), args.clone());
                        }
                        if let Some(output) = &observation.output {
                            summary.insert("output".to_owned(), json!(output));
                        }
                        if let Some(is_error) = observation.is_error {
                            summary.insert("status".to_owned(), json!(
                                if is_error { "error" } else { "completed" }
                            ));
                        }
                    }
                }
                summary
            },
            MessagePosition::Left,
        ),
        "agent_transition" => {
            let reference = document
                .get("reference")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    NomiCoreApiError::new(
                        StatusCode::CONFLICT,
                        "AGENT_SESSION_MESSAGE_PROJECTION_INVALID",
                        "Agent transition projection lost its canonical reference",
                    )
                })?;
            (
                MessageType::Tips,
                json!({
                    "type": "success",
                    "content": "",
                    "agent_transition": {
                        "transition_id": reference.get("transition_id"),
                        "previous_agent_label": reference.get("previous_agent_label"),
                        "next_agent_label": reference.get("next_agent_label"),
                        "previous_preset_id": reference.get("previous_binding_ref").and_then(|value| value.pointer("/preset_revision_ref/preset_id")),
                        "next_preset_id": reference.get("next_binding_ref").and_then(|value| value.pointer("/preset_revision_ref/preset_id")),
                        "effective_from": "next_turn",
                        "handoff_mode": reference.get("handoff_mode"),
                        "completion_gate_inherited": false,
                    }
                }),
                MessagePosition::Center,
            )
        }
        _ => return Ok(None),
    };
    let status = match state {
        "streaming" | "started" => MessageStatus::Work,
        "failed" | "error" | "uncertain" => MessageStatus::Error,
        "accepted" | "completed" | "recorded" => MessageStatus::Finish,
        _ => MessageStatus::Pending,
    };
    let status = if message_type == MessageType::ToolCall && content.get("status").and_then(Value::as_str) == Some("error") {
        MessageStatus::Error
    } else {
        status
    };
    Ok(Some(MessageResponse {
        message_id: message_id.to_owned(),
        conversation_id: session_id.as_ref().to_owned(),
        // The renderer's live stream and durable refresh share this canonical
        // projection identity. Without it, a completed assistant row cannot
        // replace its live counterpart after history hydration.
        msg_id: Some(message_id.to_owned()),
        r#type: message_type,
        content,
        position: Some(position),
        status: Some(status),
        hidden: false,
        created_at: created_at.saturating_add(
            i64::try_from(projection.first_seq).unwrap_or(i64::MAX),
        ),
    }))
}

/** Resolve presentation provenance for both sides of an old or new Agent boundary. */
async fn decorate_agent_transition_template_keys(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    message: &mut MessageResponse,
    cache: &mut HashMap<String, Option<String>>,
) {
    let Some(transition) = message
        .content
        .get_mut("agent_transition")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    for side in ["previous", "next"] {
        let label_field = format!("{side}_agent_label");
        let Some(label) = transition.get(&label_field).and_then(Value::as_str) else {
            continue;
        };
        if !nomifun_agent_contracts::OfficialPresetKey::ALL
            .iter()
            .any(|key| key.as_str() == label)
        {
            continue;
        }
        let preset_field = format!("{side}_preset_id");
        let Some(preset_id) = transition
            .get(&preset_field)
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let template_key = if let Some(cached) = cache.get(&preset_id) {
            cached.clone()
        } else {
            let resolved = match state
                .control_plane
                .internal_official_template(&owner.0, &preset_id)
                .await
            {
                Ok(key) => key.map(|key| key.as_str().to_owned()),
                Err(error) => {
                    tracing::warn!(preset_id, code = %error.code().as_ref(), "Agent transition template presentation unavailable");
                    None
                }
            };
            cache.insert(preset_id, resolved.clone());
            resolved
        };
        if let Some(template_key) = template_key {
            transition.insert(format!("{side}_template_key"), Value::String(template_key));
        }
    }
}

async fn get_nomi_core_agent_session_message_history(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Json<ApiResponse<MessageListResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
    let created_at = state
        .session_owner
        .canonical()
        .store()
        .session_created_at(&session_id)
        .await
        .map_err(agent_session_store_error)?;
    let page_size = query.page_size.unwrap_or(50).clamp(1, 500);
    let before_seq = query
        .cursor
        .as_deref()
        .filter(|cursor| !cursor.is_empty())
        .map(|cursor| {
            let (timestamp, _) = cursor.split_once(':').ok_or_else(|| {
                NomiCoreApiError::new(
                    StatusCode::BAD_REQUEST,
                    "AGENT_SESSION_MESSAGE_CURSOR_INVALID",
                    "message cursor is invalid",
                )
            })?;
            let timestamp = timestamp.parse::<i64>().map_err(|_| {
                NomiCoreApiError::new(
                    StatusCode::BAD_REQUEST,
                    "AGENT_SESSION_MESSAGE_CURSOR_INVALID",
                    "message cursor timestamp is invalid",
                )
            })?;
            u64::try_from(timestamp.saturating_sub(created_at)).map_err(|_| {
                NomiCoreApiError::new(
                    StatusCode::BAD_REQUEST,
                    "AGENT_SESSION_MESSAGE_CURSOR_INVALID",
                    "message cursor precedes the Session",
                )
            })
        })
        .transpose()?;
    let (projections, has_more, total) = state
        .session_owner
        .canonical()
        .store()
        .message_history_before(&session_id, before_seq, page_size)
        .await
        .map_err(agent_session_store_error)?;
    let observations = load_historical_tool_observations(
        &state.session_owner.pool,
        &session_id,
        &projections,
    ).await?;
    let mut items = Vec::new();
    let mut template_cache = HashMap::new();
    for projection in projections {
        let observation = observations.get(&projection.projection_id);
        if let Some(mut message) = canonical_message_response_with_observation(
            &session_id, created_at, projection, observation,
        )? {
            decorate_agent_transition_template_keys(&state, &owner, &mut message, &mut template_cache).await;
            items.push(message);
        }
    }
    Ok(Json(ApiResponse::ok(PaginatedResult {
        items,
        total,
        has_more,
    })))
}

async fn get_nomi_core_agent_session_message(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path((agent_session_id, message_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<MessageResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let message_uuid = Uuid::parse_str(&message_id).map_err(|_| {
        NomiCoreApiError::new(
            StatusCode::NOT_FOUND,
            "AGENT_SESSION_MESSAGE_NOT_FOUND",
            "message_id must be a canonical UUIDv7",
        )
    })?;
    if message_uuid.get_version_num() != 7 {
        return Err(NomiCoreApiError::new(
            StatusCode::NOT_FOUND,
            "AGENT_SESSION_MESSAGE_NOT_FOUND",
            "message_id must be a canonical UUIDv7",
        ));
    }
    state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
    let created_at = state
        .session_owner
        .canonical()
        .store()
        .session_created_at(&session_id)
        .await
        .map_err(agent_session_store_error)?;
    let projection = state
        .session_owner
        .canonical()
        .store()
        .messages_after(&session_id, 0)
        .await
        .map_err(agent_session_store_error)?
        .into_iter()
        .find(|projection| {
            projection
                .projection
                .get("correlation_id")
                .and_then(Value::as_str)
                == Some(message_id.as_str())
        })
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::NOT_FOUND,
                "AGENT_SESSION_MESSAGE_NOT_FOUND",
                "message projection does not exist",
            )
        })?;
    let observations = load_historical_tool_observations(
        &state.session_owner.pool, &session_id, std::slice::from_ref(&projection),
    ).await?;
    let observation = observations.get(&projection.projection_id);
    let mut message = canonical_message_response_with_observation(&session_id, created_at, projection, observation)?
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::NOT_FOUND,
                "AGENT_SESSION_MESSAGE_NOT_FOUND",
                "message projection is not user-visible",
            )
        })?;
    decorate_agent_transition_template_keys(&state, &owner, &mut message, &mut HashMap::new()).await;
    Ok(Json(ApiResponse::ok(message)))
}

fn required_creation_action(operation: &str) -> Result<&'static str, AppError> {
    match operation {
        "t2i" => Ok("creation.media/image"),
        "i2i" | "inpaint" => Ok("creation.media/image_edit"),
        "t2v" | "i2v" => Ok("creation.media/video"),
        "music" => Ok("creation.media/music"),
        "tts" => Ok("creation.media/audio"),
        _ => Err(AppError::BadRequest(
            "Unsupported AgentSession generation task".to_owned(),
        )),
    }
}

async fn require_creation_session(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
) -> Result<nomifun_agent_session::SessionObservation, NomiCoreApiError> {
    let observation = state
        .session_owner
        .canonical()
        .get(&authenticated_principal(owner), session_id)
        .await?;
    let binding = agent_binding_dto(&observation.session.agent_binding)?;
    let (_, _, snapshot) = state
        .control_plane
        .saved_binding_artifacts(&owner.0, &binding)
        .await?;
    if !snapshot
        .content
        .enabled_capabilities
        .iter()
        .any(|capability| capability.capability.id.as_ref() == "creation.media")
    {
        return Err(NomiCoreApiError::new(
            StatusCode::FORBIDDEN,
            "CAPABILITY_ACTION_NOT_GRANTED",
            "this AgentSession does not enable creation.media",
        ));
    }
    Ok(observation)
}

async fn append_canonical_creation_message(
    state: &NomiCoreAgentApiState,
    session_id: &AgentSessionId,
    message_id: &str,
    content: &str,
) -> Result<(), NomiCoreApiError> {
    let cause: String = sqlx::query_scalar(
        "SELECT event_id FROM agent_events \
         WHERE session_id = ? AND kind = 'session/ready' ORDER BY seq ASC LIMIT 1",
    )
    .bind(session_id.as_ref())
    .fetch_one(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let identity = format!("creation-message:{}:{message_id}", session_id.as_ref());
    state
        .session_owner
        .canonical()
        .store()
        .append_event(&nomifun_agent_contracts::SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: nomifun_agent_contracts::EventId::from(message_id.to_owned()),
            producer_id: nomifun_agent_contracts::EventProducerId::from("session_api"),
            idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: nomifun_agent_contracts::SessionEventKind(
                    "message/user-accepted".to_owned(),
                ),
                kind_version: 1,
                correlation_id: nomifun_agent_contracts::CorrelationId::from(
                    message_id.to_owned(),
                ),
                causation_event_id: Some(nomifun_agent_contracts::EventId::from(cause)),
                payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                    StrictJsonValue(json!({ "content": content })),
                ),
            },
        })
        .await
        .map_err(agent_session_store_error)?;
    Ok(())
}

async fn finish_canonical_creation_batch(
    state: &NomiCoreAgentApiState,
    session_id: &AgentSessionId,
    key: &str,
    root: nomifun_creation::CreationTask,
) -> Result<Vec<nomifun_creation::CreativeCreationTask>, NomiCoreApiError> {
    let ids: Vec<String> = serde_json::from_value(
        root.params
            .get("_nomifun_creation_batch")
            .cloned()
            .unwrap_or_else(|| json!([key])),
    )
    .map_err(|error| AppError::Internal(format!("Invalid persisted generation batch: {error}")))?;
    let mut tasks = vec![nomifun_creation::CreativeCreationTask::try_from(root.clone())?];
    for task_id in ids.into_iter().skip(1) {
        let task = state
            .session_owner
            .creation_service
            .create_creative_task(
                nomifun_creation::CreativeTaskOwner::ConversationTurn {
                    conversation_id: session_id.as_ref().to_owned(),
                    message_id: key.to_owned(),
                },
                task_id,
                nomifun_creation::NewCreationTask {
                    provider_id: root.provider_id.clone(),
                    model: root.model.clone(),
                    capability: root.capability.clone(),
                    params: root.params.clone(),
                    inputs: root.inputs.clone().unwrap_or_default(),
                },
            )
            .await?;
        tasks.push(nomifun_creation::CreativeCreationTask::try_from(task)?);
    }
    Ok(tasks)
}

async fn submit_canonical_creation_task(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    headers: HeaderMap,
    Json(mut request): Json<
        nomifun_conversation::SubmitConversationCreation,
    >,
) -> Result<
    (
        StatusCode,
        Json<ApiResponse<
            nomifun_conversation::ConversationCreationResponse,
        >>,
    ),
    NomiCoreApiError,
> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let observation = require_creation_session(&state, &owner, &session_id).await?;
    if observation.head.status == "running" {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_TURN_ACTIVE",
            "wait for the active Turn before submitting a generation task",
        ));
    }
    let key = request_idempotency_key(&headers, "agent-session-creation")?;
    nomifun_common::CreationTaskId::parse(&key)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    let original_request = serde_json::to_value(&request)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    match state.session_owner.creation_service.get_task(&key).await {
        Ok(task) => {
            if task.conversation_id.as_deref() != Some(session_id.as_ref())
                || task.message_id.as_deref() != Some(key.as_str())
                || task.params.get("_nomifun_creation_request") != Some(&original_request)
            {
                return Err(AppError::Conflict(
                    "this generation key already belongs to a different immutable request"
                        .to_owned(),
                )
                .into());
            }
            let tasks = finish_canonical_creation_batch(&state, &session_id, &key, task).await?;
            return Ok((
                StatusCode::ACCEPTED,
                Json(ApiResponse::ok(
                    nomifun_conversation::ConversationCreationResponse {
                        message_id: key,
                        tasks,
                    },
                )),
            ));
        }
        Err(AppError::NotFound(_)) => {}
        Err(error) => return Err(error.into()),
    }
    let required_action = required_creation_action(&request.capability)?;
    let binding = agent_binding_dto(&observation.session.agent_binding)?;
    let (_, _, snapshot) = state
        .control_plane
        .saved_binding_artifacts(&owner.0, &binding)
        .await?;
    if !snapshot
        .content
        .enabled_capabilities
        .iter()
        .any(|capability| {
            capability.capability.id.as_ref() == "creation.media"
                && capability
                    .action_allowlist
                    .iter()
                    .any(|action| action.as_ref() == required_action)
        })
    {
        return Err(NomiCoreApiError::new(
            StatusCode::FORBIDDEN,
            "CAPABILITY_ACTION_NOT_GRANTED",
            format!("this AgentSession does not enable {required_action}"),
        ));
    }
    let params = request.params.as_object_mut().ok_or_else(|| {
        AppError::BadRequest("Generation parameters must be an object".to_owned())
    })?;
    if params.keys().any(|key| key.starts_with("_nomifun")) {
        return Err(AppError::BadRequest(
            "Generation metadata is server-owned".to_owned(),
        )
        .into());
    }
    let prompt = params
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
        .ok_or_else(|| AppError::BadRequest("Describe the work to generate".to_owned()))?
        .to_owned();
    let max_count = if matches!(request.capability.as_str(), "t2v" | "i2v") {
        8
    } else {
        10
    };
    let count = params
        .get("count")
        .map(|value| {
            value
                .as_u64()
                .filter(|count| (1..=max_count).contains(count))
                .ok_or_else(|| {
                    AppError::BadRequest(format!(
                        "Generation count must be between 1 and {max_count}"
                    ))
                })
        })
        .transpose()?
        .unwrap_or(1);
    let batch_size = if matches!(request.capability.as_str(), "t2v" | "i2v") {
        count
    } else {
        1
    };
    let batch_ids = std::iter::once(key.clone())
        .chain((1..batch_size).map(|_| nomifun_common::CreationTaskId::new().into_string()))
        .collect::<Vec<_>>();
    if batch_size > 1 {
        params.insert("count".to_owned(), json!(1));
    }
    params.insert("_nomifun_creation_request".to_owned(), original_request);
    params.insert("_nomifun_creation_batch".to_owned(), json!(batch_ids));
    params.insert(
        "_nomifun_creation_agent".to_owned(),
        serde_json::to_value(&snapshot)
            .map_err(|error| AppError::Internal(error.to_string()))?,
    );
    let references = nomifun_conversation::import_creation_files(
        &state.session_owner.creation_service,
        session_id.as_ref(),
        &request.files,
        &request.capability,
        &request.inputs,
        false,
    )
    .await?;
    request.inputs.extend(references);
    let mut generation = nomifun_creation::NewCreationTask {
        provider_id: request.provider_id,
        model: request.model,
        capability: request.capability,
        params: request.params,
        inputs: request.inputs,
    };
    state
        .session_owner
        .creation_service
        .capture_model_config(&mut generation)
        .await?;
    let task = state
        .session_owner
        .creation_service
        .create_creative_task(
            nomifun_creation::CreativeTaskOwner::ConversationTurn {
                conversation_id: session_id.as_ref().to_owned(),
                message_id: key.clone(),
            },
            key.clone(),
            generation,
        )
        .await?;
    append_canonical_creation_message(&state, &session_id, &key, &prompt).await?;
    let tasks = finish_canonical_creation_batch(&state, &session_id, &key, task).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(ApiResponse::ok(
            nomifun_conversation::ConversationCreationResponse {
                message_id: key,
                tasks,
            },
        )),
    ))
}

async fn list_canonical_creation_tasks(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<
    Json<ApiResponse<
        nomifun_conversation::ConversationCreationPage,
    >>,
    NomiCoreApiError,
> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    require_creation_session(&state, &owner, &session_id).await?;
    let items = state
        .session_owner
        .creation_service
        .list_conversation_tasks(session_id.as_ref())
        .await?
        .into_iter()
        .map(nomifun_creation::CreativeCreationTask::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ApiResponse::ok(
        nomifun_conversation::ConversationCreationPage {
            items,
        },
    )))
}

async fn cancel_canonical_creation_task(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path((agent_session_id, task_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<nomifun_creation::CreativeCreationTask>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    require_creation_session(&state, &owner, &session_id).await?;
    let task = state.session_owner.creation_service.get_task(&task_id).await?;
    if task.conversation_id.as_deref() != Some(session_id.as_ref()) {
        return Err(AppError::NotFound("Generation task not found".to_owned()).into());
    }
    let task = state
        .session_owner
        .creation_service
        .cancel_task(&task_id)
        .await?;
    Ok(Json(ApiResponse::ok(
        nomifun_creation::CreativeCreationTask::try_from(task)?,
    )))
}

async fn warm_nomi_core_agent_session(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let _operation_fence = state
        .session_owner
        .session_operation_lock(session_id.as_ref())
        .read_owned()
        .await;
    state
        .session_owner
        .materialize_session_workspace(owner.as_ref(), &session_id)
        .await?;
    let mut projection = state
        .session_owner
        .canonical_conversation_projection(owner.as_ref(), &session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!(
            "AgentSession {} not found",
            session_id.as_ref(),
        )))?;
    if projection
        .extra
        .get("workspace")
        .and_then(Value::as_str)
        .is_none_or(|workspace| workspace.trim().is_empty())
    {
        let fallback = materialize_managed_session_workspace(
            &state.session_owner.managed_workspace_root,
            &session_id,
        )
        .await?;
        projection.extra["workspace"] = Value::String(fallback);
        projection.extra["custom_workspace"] = Value::Bool(false);
        projection.extra["is_temporary_workspace"] = Value::Bool(true);
        projection.extra["temp_workspace_id"] =
            Value::String(session_id.as_ref().to_owned());
    }
    let (options, _) = runtime_options_from_session(owner.as_ref(), projection, None)?;
    state
        .session_owner
        .runtime_sessions
        .get_or_create_runtime_for_preparation(
            session_id.as_ref(),
            tokio_util::sync::CancellationToken::new(),
            options,
        )
        .await?;
    Ok(Json(ApiResponse::success()))
}

async fn clear_nomi_core_agent_session_context(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let observation = state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
    if observation.head.status == "running" {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_SESSION_TURN_ACTIVE",
            "wait for the active Turn before clearing model context",
        ));
    }
    if let Some(runtime) = state
        .session_owner
        .runtime_sessions
        .get_runtime(session_id.as_ref())
    {
        runtime.clear_context().await?;
    }
    let cause: String = sqlx::query_scalar(
        "SELECT event_id FROM agent_events WHERE session_id = ? ORDER BY seq DESC LIMIT 1",
    )
    .bind(session_id.as_ref())
    .fetch_one(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    let identity = format!("context-clear:{}", Uuid::now_v7());
    state
        .session_owner
        .canonical()
        .store()
        .append_event(&nomifun_agent_contracts::SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: nomifun_agent_contracts::EventId::from(Uuid::now_v7().to_string()),
            producer_id: nomifun_agent_contracts::EventProducerId::from("session_api"),
            idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(identity),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: nomifun_agent_contracts::SessionEventKind("context/cleared".to_owned()),
                kind_version: 1,
                correlation_id: nomifun_agent_contracts::CorrelationId::from(
                    session_id.as_ref().to_owned(),
                ),
                causation_event_id: Some(nomifun_agent_contracts::EventId::from(cause)),
                payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                    StrictJsonValue(json!({ "reason": "user_requested" })),
                ),
            },
        })
        .await
        .map_err(agent_session_store_error)?;
    Ok(Json(ApiResponse::success()))
}

async fn ask_nomi_core_agent_session_side_question(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(request): Json<SideQuestionRequest>,
) -> Result<Json<ApiResponse<SideQuestionResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
    let runtime = state
        .session_owner
        .runtime_sessions
        .get_runtime(session_id.as_ref())
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::CONFLICT,
                "AGENT_SESSION_RUNTIME_NOT_ACTIVE",
                "side questions require an active Session Runtime",
            )
        })?;
    Ok(Json(ApiResponse::ok(
        runtime.handle_side_question(request).await?,
    )))
}

async fn browse_nomi_core_agent_session_workspace(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Query(query): Query<WorkspaceBrowseQuery>,
) -> Result<Json<ApiResponse<Vec<WorkspaceEntry>>>, NomiCoreApiError> {
    if query.path.trim().is_empty() {
        return Err(AppError::BadRequest("path must not be empty".to_owned()).into());
    }
    let session_id = parse_agent_session_id(&agent_session_id)?;
    state
        .session_owner
        .materialize_session_workspace(owner.as_ref(), &session_id)
        .await?;
    let projection = state
        .session_owner
        .canonical_conversation_projection(owner.as_ref(), &session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!(
            "AgentSession {} not found",
            session_id.as_ref(),
        )))?;
    let workspace = projection
        .extra
        .get("workspace")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|workspace| !workspace.is_empty())
        .ok_or_else(|| AppError::BadRequest(
            "AgentSession has no Workspace resource".to_owned(),
        ))?;
    let entries = nomifun_file::list_workspace_level(
        std::path::Path::new(workspace),
        &query.path,
        query.search.as_deref(),
    )?;
    Ok(Json(ApiResponse::ok(entries)))
}

async fn get_nomi_core_agent_session_capabilities(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<NomiCoreAgentSessionCapabilityResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let observation = state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
    let binding_dto = agent_binding_dto(&observation.session.agent_binding)?;
    let (_, revision, snapshot) = state
        .control_plane
        .saved_binding_artifacts(&owner.0, &binding_dto)
        .await?;
    if snapshot.snapshot_ref != observation.session.agent_binding.resolved_snapshot_ref {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_SNAPSHOT_IDENTITY_CONFLICT",
            "the saved Snapshot differs from the canonical AgentSession binding",
        ));
    }
    let enabled_capabilities = revision
        .payload
        .enabled_capabilities
        .iter()
        .map(|selection| selection.capability.id.as_ref().to_owned())
        .collect::<Vec<_>>();
    let active_capabilities = state
        .session_owner
        .canonical()
        .active_capability_ids(&authenticated_principal(&owner), &session_id)
        .await?;
    Ok(Json(ApiResponse::ok(
        NomiCoreAgentSessionCapabilityResponse {
            resolved_snapshot_ref: observation.session.agent_binding.resolved_snapshot_ref,
            generation: observation.head.active_set_generation,
            enabled_capabilities,
            active_capabilities,
            state_source: "canonical_agent_store",
        },
    )))
}

async fn get_nomi_core_agent_session_slash_commands(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<Vec<nomifun_api_types::SlashCommandItem>>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let commands = state
        .skill_discovery
        .discover_canonical_skill_commands(&owner, &session_id)
        .await?;
    Ok(Json(ApiResponse::ok(commands)))
}

async fn start_nomi_core_agent_session_turn(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CreateAgentSessionTurnRequestDto>,
) -> Result<Json<ApiResponse<CreateAgentSessionTurnResponseDto>>, NomiCoreApiError> {
    let initial_only = initial_delivery_requested(&headers)?;
    let result = start_owned_session_turn(
        &state.session_owner,
        &owner,
        &agent_session_id,
        request,
        initial_only,
    )
    .await?;
    Ok(Json(ApiResponse::ok(result)))
}

fn initial_delivery_requested(headers: &HeaderMap) -> Result<bool, NomiCoreApiError> {
    let mut values = headers.get_all("x-nomifun-initial-delivery").iter();
    let Some(value) = values.next() else {
        return Ok(false);
    };
    if values.next().is_some() || value.as_bytes() != b"1" {
        return Err(NomiCoreApiError::new(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
            "X-Nomifun-Initial-Delivery must appear exactly once with value 1",
        ));
    }
    Ok(true)
}

async fn steer_nomi_core_agent_session_turn(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(request): Json<SteerAgentSessionTurnRequestDto>,
) -> Result<Json<ApiResponse<AgentSessionTurnMutationResponseDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let key = canonical_nonempty(&request.idempotency_key, "idempotency_key")?;
    let turn = bounded_turn_input(request.input)?;
    let input = canonical_turn_input(&turn);
    let receipt = state
        .session_owner
        .canonical()
        .steer(
            &authenticated_principal(&owner),
            &session_id,
            &key,
            input,
        )
        .await?;
    if !receipt.duplicate {
        let turn_receipt = state
            .session_owner
            .canonical()
            .store()
            .read_turn_receipt(&session_id, &receipt.target_operation_id)
            .await
            .map_err(agent_session_store_error)?;
        let started = turn_receipt.started_event.ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::CONFLICT,
                "AGENT_SESSION_TURN_RECEIPT_INVALID",
                "active canonical Turn has no start fact",
            )
        })?;
        let root = match &started.payload {
            nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) => payload
                .0
                .get("source_message_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        }
        .ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::CONFLICT,
                "AGENT_SESSION_TURN_RECEIPT_INVALID",
                "active canonical Turn has no source message",
            )
        })?;
        let runtime = state
            .session_owner
            .runtime_sessions
            .get_runtime(session_id.as_ref())
            .ok_or_else(|| {
                NomiCoreApiError::new(
                    StatusCode::CONFLICT,
                    "AGENT_SESSION_RUNTIME_NOT_ACTIVE",
                    "steering requires the active Session Runtime",
                )
            })?;
        let queued = runtime
            .steer_with_receipt(nomifun_ai_agent::RuntimeSteerDelivery {
                receipt_operation_id: receipt.event_id.as_ref().to_owned(),
                wire_turn_id: root,
                turn_generation: started.seq,
                text: turn.content,
                files: turn.files,
                inject_skills: turn.inject_skills,
            })
            .await?;
        if !queued {
            return Err(NomiCoreApiError::new(
                StatusCode::CONFLICT,
                "AGENT_SESSION_STEER_NOT_QUEUED",
                "the active Runtime closed steering before this input was queued",
            ));
        }
    }
    Ok(Json(ApiResponse::ok(AgentSessionTurnMutationResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        target_operation_id: receipt.target_operation_id.as_ref().to_owned(),
        message_id: receipt.event_id.as_ref().to_owned(),
        cursor: session_cursor(&session_id, receipt.cursor.seq),
        status: "running".to_owned(),
        duplicate: receipt.duplicate,
    })))
}

async fn cancel_nomi_core_agent_session_turn(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(request): Json<CancelAgentSessionTurnRequestDto>,
) -> Result<Json<ApiResponse<AgentSessionTurnMutationResponseDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let key = canonical_nonempty(&request.idempotency_key, "idempotency_key")?;
    let principal = authenticated_principal(&owner);
    let receipt = state
        .session_owner
        .cancel_turn(
            &principal.principal_id,
            &session_id,
            &key,
            nomifun_common::AgentKillReason::UserCancelled,
        )
        .await?;
    Ok(Json(ApiResponse::ok(AgentSessionTurnMutationResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        target_operation_id: receipt.target_operation_id.as_ref().to_owned(),
        message_id: receipt.event_id.as_ref().to_owned(),
        cursor: session_cursor(&session_id, receipt.cursor.seq),
        status: "cancelled".to_owned(),
        duplicate: receipt.duplicate,
    })))
}

/// Both the built-in page and a scoped plugin UI use the same admission path.
async fn start_owned_session_turn(
    session_owner: &Arc<NomiCoreSessionOwner>,
    owner: &AuthenticatedOwner,
    agent_session_id: &str,
    request: CreateAgentSessionTurnRequestDto,
    initial_only: bool,
) -> Result<CreateAgentSessionTurnResponseDto, NomiCoreApiError> {
    let session_id = parse_agent_session_id(agent_session_id)?;
    let input = bounded_turn_input(request.input)?;
    let idempotency_key = canonical_nonempty(&request.idempotency_key, "idempotency_key")?;
    let delivery = session_owner
        .dispatch_canonical_turn(
            owner.as_ref(),
            &session_id,
            &idempotency_key,
            input,
            initial_only,
        )
        .await?;
    let operation_id = NomiCoreSessionOwner::turn_operation_id(
        owner.as_ref(),
        session_id.as_ref(),
        &idempotency_key,
    );
    let cursor = session_owner
        .canonical()
        .store()
        .current_cursor(&session_id)
        .await
        .map_err(agent_session_store_error)?;
    Ok(CreateAgentSessionTurnResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        operation_id: operation_id.as_ref().to_owned(),
        message_id: delivery.message_id,
        cursor: session_cursor(&session_id, cursor.seq),
        status: if delivery.completed { "completed" } else { "running" }.to_owned(),
        replayed: delivery.replayed,
        completed: delivery.completed,
        result_ok: delivery.result_ok,
        result_text: delivery.result_text,
        result_error: delivery.result_error,
        result_error_code: delivery.result_error_code,
        result_error_retryable: delivery.result_error_retryable,
    })
}

async fn get_nomi_core_agent_session_messages(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Query(query): Query<NomiCoreSessionPageQuery>,
) -> Result<Json<ApiResponse<NomiCoreAgentSessionMessagePageResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    validate_page_limit(query.limit)?;
    let messages = state
        .session_owner
        .canonical()
        .messages(&authenticated_principal(&owner), &session_id, query.after_seq)
        .await?
        .into_iter()
        .take(query.limit as usize)
        .collect::<Vec<_>>();
    // Projections are ordered by first_seq, while an earlier projection may
    // receive a later terminal update. The page cursor must cover every row
    // returned, not merely the last projection in first-seen order.
    let next_seq = messages
        .iter()
        .map(|message| message.last_seq)
        .max()
        .unwrap_or(query.after_seq);
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
) -> Result<Json<ApiResponse<nomifun_agent_session::SessionEventPage>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    validate_page_limit(query.limit)?;
    let after = (query.after_seq > 0).then(|| nomifun_agent_contracts::SessionEventCursor {
        agent_session_id: session_id.clone(),
        seq: query.after_seq,
    });
    let page = state
        .session_owner
        .canonical()
        .events(
            &authenticated_principal(&owner),
            &session_id,
            after.as_ref(),
            query.limit,
        )
        .await?;
    Ok(Json(ApiResponse::ok(page)))
}

async fn fork_nomi_core_agent_session(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ForkAgentSessionRequestDto>,
) -> Result<Json<ApiResponse<ForkAgentSessionResponseDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let principal = authenticated_principal(&owner);
    let parent = state
        .session_owner
        .canonical()
        .get(&principal, &session_id)
        .await?;
    let target_binding: AgentBindingValue = serde_json::from_value(
        serde_json::to_value(&request.target_agent_binding)?,
    )?;
    if target_binding != parent.session.agent_binding {
        return Err(NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "NOMI_CORE_FORK_BINDING_UNSUPPORTED",
            "fork must reuse the parent Session's server-resolved Agent binding",
        ));
    }
    let operation_id = request_idempotency_key(
        &headers,
        "nomi-core-agent-session-fork",
    )?;
    let fork = state
        .session_owner
        .canonical()
        .fork(
            &principal,
            &session_id,
            request.parent_through_seq,
            request.title,
            &operation_id,
            now_ms(),
        )
        .await?;
    state
        .session_owner
        .materialize_workspace_for_binding(
            owner.as_ref(),
            &fork.child_session.agent_session_id,
            &fork.child_session.agent_binding,
        )
        .await?;
    state
        .session_owner
        .inherit_idmm_state(
            session_id.as_ref(),
            fork.child_session.agent_session_id.as_ref(),
        )
        .await?;
    Ok(Json(ApiResponse::ok(ForkAgentSessionResponseDto {
        parent_agent_session_id: session_id.as_ref().to_owned(),
        child_agent_session_id: fork.child_session.agent_session_id.as_ref().to_owned(),
        child_agent_binding: agent_binding_dto(&fork.child_session.agent_binding)?,
        parent_through_seq: fork.contract.fork.parent_through_seq,
        child_base_is_self_contained: fork.contract.child_base_is_self_contained,
        copies_full_transcript: fork.contract.copies_full_transcript,
        migrates_runtime_private_handles: fork.contract.migrates_runtime_private_handles,
        replays_tool_or_effect: fork.contract.replays_tool_or_effect,
    })))
}

async fn delete_nomi_core_agent_session(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<NomiCoreAgentSessionDeleteResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let key = request_idempotency_key(&headers, "nomi-core-agent-session-delete")?;
    let task_state = state.clone();
    let task_owner = owner.clone();
    let task_session_id = session_id.clone();
    let deleted = tokio::spawn(async move {
        execute_nomi_core_agent_session_delete(
            task_state,
            task_owner,
            task_session_id,
            key,
        )
        .await
    })
    .await
    .map_err(|error| {
        NomiCoreApiError::with_details(
            StatusCode::SERVICE_UNAVAILABLE,
            "AGENT_SESSION_DELETE_OUTCOME_UNKNOWN",
            "AgentSession delete owner stopped before publishing its terminal result.",
            json!({
                "agent_session_id": session_id,
                "outcome": "unknown",
                "recovery": "retry_same_delete_request",
                "detail": error.to_string(),
            }),
        )
    })??;
    Ok(Json(ApiResponse::ok(NomiCoreAgentSessionDeleteResponse {
        agent_session_id: session_id.as_ref().to_owned(),
        state: "deleted",
        deleted_at: deleted.tombstone.deleted_at,
    })))
}

async fn execute_nomi_core_agent_session_delete(
    state: NomiCoreAgentApiState,
    owner: AuthenticatedOwner,
    session_id: AgentSessionId,
    key: String,
) -> Result<nomifun_agent_session::DeleteResult, NomiCoreApiError> {
    let cleanup_lock = state.delete_cleanup_lock(owner.as_ref(), session_id.as_ref());
    let _cleanup = cleanup_lock.lock().await;
    let _operation_fence = state
        .session_owner
        .session_operation_lock(session_id.as_ref())
        .write_owned()
        .await;
    let deleted_at = now_ms();
    let principal = authenticated_principal(&owner);
    match state
        .session_owner
        .canonical()
        .get(&principal, &session_id)
        .await
    {
        Ok(_) => {
            state
                .terminate_agent_session_runtime_before_delete_fence(session_id.as_ref())
                .await?;
        }
        // A deleting/deleted Session is replayed by `fence_delete`; a truly
        // missing Session receives the same authoritative error there.
        Err(AppError::NotFound(_)) => {}
        Err(error) => return Err(error.into()),
    }
    let prepared = state
        .session_owner
        .canonical()
        .fence_delete(&principal, &session_id, &key, deleted_at)
        .await?;
    let deleted = match prepared {
        PreparedAgentSessionDelete::AlreadyDeleted(deleted) => deleted,
        PreparedAgentSessionDelete::Fenced(command) => {
            // From this point the process-owned task, not the request future,
            // owns cleanup. Direct SSH holders are synchronously retired before
            // any further await.
            state.ssh_pool.retire_agent_session(session_id.as_ref());
            state
                .quiesce_agent_session_execution_before_delete(
                    session_id.as_ref(),
                )
                .await?;
            let blockers = state
                .session_owner
                .canonical()
                .store()
                .delete_blockers(&session_id)
                .await
                .map_err(agent_session_store_error)?;
            if delete_cleanup_requires_reconciliation(&blockers) {
                return Err(agent_session_delete_blocked(
                    session_id.as_ref(),
                    &blockers,
                ));
            }
            state
                .cleanup_agent_session_resources_before_delete(
                    owner.as_ref(),
                    session_id.as_ref(),
                )
                .await?;
            let blockers = state
                .session_owner
                .canonical()
                .store()
                .delete_blockers(&session_id)
                .await
                .map_err(agent_session_store_error)?;
            if !blockers.is_empty() {
                return Err(agent_session_delete_blocked(
                    session_id.as_ref(),
                    &blockers,
                ));
            }
            state
                .session_owner
                .canonical()
                .complete_fenced_delete(&command, now_ms())
                .await?
        }
    };
    state
        .session_owner
        .remove_idmm_state(session_id.as_ref())
        .await?;
    if let Err(error) = state
        .wave4_owners
        .release_session(owner.as_ref(), session_id.as_ref())
        .await
    {
        tracing::warn!(
            agent_session_id = session_id.as_ref(),
            code = error.code,
            "Wave 4 resource and receipt cleanup deferred to orphan reconciliation"
        );
    }
    Ok(deleted)
}

const DELETE_OVERRIDE_CONFIRMATION: &str =
    "DELETE DESPITE UNRESOLVED EXTERNAL EFFECTS";

async fn override_nomi_core_agent_session_delete(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(request): Json<NomiCoreDeleteOverrideRequest>,
) -> Result<Json<ApiResponse<NomiCoreDeleteOverrideResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    if owner.as_ref() != state.authoritative_user_id.as_ref() {
        return Err(AppError::Forbidden(
            "manual delete override requires the installation owner".to_owned(),
        )
        .into());
    }
    let (confirmation, reason) = match &request {
        NomiCoreDeleteOverrideRequest::Effect {
            confirmation,
            reason,
            ..
        }
        | NomiCoreDeleteOverrideRequest::ResourceCleanup {
            confirmation,
            reason,
            ..
        } => (confirmation, reason),
    };
    if confirmation != DELETE_OVERRIDE_CONFIRMATION
        || reason.len() < 20
        || reason.trim() != reason
        || reason.len() > 4096
        || reason.chars().any(char::is_control)
    {
        return Err(NomiCoreApiError::new(
            StatusCode::BAD_REQUEST,
            "AGENT_SESSION_DELETE_OVERRIDE_CONFIRMATION_INVALID",
            "manual delete override requires the exact risk confirmation and a 20-4096 character audit reason",
        ));
    }
    let reason_digest = nomifun_agent_contracts::DigestHex::from(format!(
        "{:x}",
        Sha256::digest(reason.as_bytes())
    ));
    let cleanup_lock = state.delete_cleanup_lock(owner.as_ref(), session_id.as_ref());
    let _cleanup = cleanup_lock.lock().await;
    let _operation_fence = state
        .session_owner
        .session_operation_lock(session_id.as_ref())
        .write_owned()
        .await;
    let deleting = state
        .session_owner
        .canonical()
        .store()
        .get_deleting_session(&session_id)
        .await
        .map_err(agent_session_store_error)?;
    let principal = authenticated_principal(&owner);
    if deleting.owner_ref != principal {
        return Err(AppError::Forbidden(
            "AgentSession belongs to another owner".to_owned(),
        )
        .into());
    }
    match request {
        NomiCoreDeleteOverrideRequest::Effect {
            effect_id,
            ..
        } => {
            state
                .session_owner
                .canonical()
                .store()
                .override_unknown_effect_for_delete(
                    &principal,
                    &session_id,
                    &effect_id,
                    &reason_digest,
                    now_ms(),
                )
                .await
                .map_err(agent_session_store_error)?;
        }
        NomiCoreDeleteOverrideRequest::ResourceCleanup {
            owner_domain,
            ..
        } => {
            state
                .session_owner
                .canonical()
                .store()
                .override_resource_cleanup_for_delete(
                    &principal,
                    &session_id,
                    &owner_domain,
                    &reason_digest,
                    now_ms(),
                )
                .await
                .map_err(agent_session_store_error)?;
            if owner_domain == "ssh" {
                state
                    .ssh_pool
                    .acknowledge_persisted_agent_session_teardowns(
                        session_id.as_ref(),
                    );
            }
        }
    }
    let remaining_blockers = state
        .session_owner
        .canonical()
        .store()
        .delete_blockers(&session_id)
        .await
        .map_err(agent_session_store_error)?;
    Ok(Json(ApiResponse::ok(
        NomiCoreDeleteOverrideResponse {
            agent_session_id,
            remaining_blockers,
        },
    )))
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
    let mcp_selection =
        exact_session_mcp_selection(&state.mcp_server_repository, &owner, &projection.binding)
            .await?;
    let mut create_request = projection.projection.request;
    install_creation_mcp_selection(&mut create_request.extra, &mcp_selection)?;
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
                // The creation key and Remote projection key must identify one
                // logical Session. Never retain an unlinked canonical Session.
                let _ = execute_nomi_core_agent_session_delete(
                    state.clone(),
                    owner.clone(),
                    session_id.clone(),
                    format!("remote-open-conflict-cleanup:{idempotency_key}"),
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

    // A Remote Session is ready to accept subsequent turns once its canonical
    // Store aggregate and immutable binding projection are committed. Runtime
    // materialization remains lazy under the unified owner.
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
            | AppError::WorkspaceDirectoryUnavailable(_)
            | AppError::WorkspaceDirectoryRuntimeUnavailable(_)
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
    use super::{
        agent_switch_recovery_blocker_from_rows, cancel_error_is_known_rejection,
        remote_delivery_terminal_event_type,
    };
    use nomifun_common::AppError;
    use serde_json::json;

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

    #[test]
    fn pending_or_uncovered_patch_recovery_blocks_agent_switching() {
        let pending = nomifun_agent_runtime::AgentEngineEvent::PatchRecoveryUpdated {
            state: nomifun_agent_runtime::AgentPatchRecoveryState {
                version: 1,
                targets: vec!["src/lib.rs".to_owned()],
                target_budget_exceeded: false,
            },
        };
        let pending = serde_json::to_string(&json!({ "event": pending })).unwrap();
        let blocker = agent_switch_recovery_blocker_from_rows(vec![(1, pending)])
            .unwrap()
            .expect("pending recovery blocker");
        assert_eq!(blocker.code, "AGENT_SESSION_HANDOFF_RECOVERY_PENDING");
        assert_eq!(blocker.details.unwrap()["pending"], true);

        let cleared = nomifun_agent_runtime::AgentEngineEvent::PatchRecoveryUpdated {
            state: nomifun_agent_runtime::AgentPatchRecoveryState::default(),
        };
        let cleared = serde_json::to_string(&json!({ "event": cleared })).unwrap();
        assert!(
            agent_switch_recovery_blocker_from_rows(vec![(2, cleared.clone())])
                .unwrap()
                .is_none()
        );
        let dispatch = json!({
            "event": {
                "event": "host_tool_dispatch",
                "dispatch": { "action_id": "workspace.files/patch" }
            }
        })
        .to_string();
        let blocker = agent_switch_recovery_blocker_from_rows(vec![(2, cleared), (3, dispatch)])
            .unwrap()
            .expect("uncovered patch dispatch blocker");
        assert_eq!(blocker.details.unwrap()["uncovered_patch_dispatch"], true);
    }
}

async fn resolve_saved_binding_projection(
    state: &NomiCoreAgentApiState,
    owner: &AuthenticatedOwner,
    binding: &AgentBindingValueDto,
    title: Option<&str>,
) -> Result<
    super::agent_binding_projection::SavedAgentBindingProjection,
    NomiCoreApiError,
> {
    let (binding, revision, snapshot) = state
        .control_plane
        .saved_binding_artifacts(&owner.0, binding)
        .await?;
    super::agent_binding_projection::project_saved_artifacts(
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
    load_session_from_owner(&state.session_owner, owner, session_id).await
}

/// The HTTP and plugin view adapters must enforce the same identity invariant.
async fn load_session_from_owner(
    session_owner: &Arc<NomiCoreSessionOwner>,
    owner: &AuthenticatedOwner,
    session_id: &AgentSessionId,
) -> Result<ConversationResponse, NomiCoreApiError> {
    let response = session_owner
        .get_session(owner.as_ref(), session_id.as_ref())
        .await
        .map_err(NomiCoreApiError::from)?;
    if parse_agent_session_id(&response.conversation_id)? != *session_id {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "NOMI_CORE_SESSION_IDENTITY_CONFLICT",
            "canonical Session owner returned a different Session identity",
        ));
    }
    Ok(response)
}

pub(super) fn session_metadata(
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
    attach_session_metadata_with_fork(extra, binding, remote, None, None)
}

fn attach_session_metadata_with_fork(
    extra: &mut Value,
    binding: &AgentBindingValue,
    remote: Option<RemoteBindingProvenance>,
    parent_session_id: Option<AgentSessionId>,
    fork_base_payload_id: Option<ArtifactId>,
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
            parent_session_id,
            fork_base_payload_id,
        })?,
    );
    Ok(())
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
    let mut found = HashMap::<String, MessageProjection>::new();
    for projection in owner
        .canonical()
        .store()
        .messages_after(session_id, 0)
        .await
        .map_err(agent_session_store_error)?
    {
        if let Some(message_id) = projection
            .projection
            .get("correlation_id")
            .and_then(Value::as_str)
            && wanted.contains(message_id)
        {
            found.entry(message_id.to_owned()).or_insert(projection);
        }
    }
    message_ids
        .iter()
        .filter_map(|id| found.get(id))
        .map(|projection| serde_json::to_value(projection).map_err(Into::into))
        .collect()
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

fn authenticated_principal(owner: &AuthenticatedOwner) -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: owner.as_ref().to_owned(),
    }
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

fn canonical_turn_input(request: &SendMessageRequest) -> Value {
    json!({
        "content": request.content,
        "files": request.files,
        "inject_skills": request.inject_skills,
        "hidden": request.hidden,
        "origin": request.origin,
        "channel_platform": request.channel_platform,
    })
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
