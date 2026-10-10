//! One host-owned typed Session facade for the Nomi-core product.
//!
//! All product and domain consumers use the canonical AgentSession Store.
//! Conversation DTOs project the same Session identity and Turn authority.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

#[path = "native_turn_recovery.rs"]
mod native_turn_recovery;
#[path = "session_capability_selection.rs"]
mod session_capability_selection;
#[path = "session_contract_evolution.rs"]
mod session_contract_evolution;
#[path = "native_execution_control.rs"]
pub(super) mod native_execution_control;

#[cfg(test)]
#[path = "idmm_message_tests.rs"]
mod idmm_message_tests;

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
    AgentRuntimeSessions, AgentSendError, AgentStreamEvent,
};
use nomifun_agent_contracts::{
    IdmmDecisionExplanation, IdmmDecisionNotice,
    AgentBindingValue, AgentHandoffBindingRefV1, AgentHandoffCompletionAccountV1,
    AgentHandoffCompletionCriterionV1, AgentHandoffEnvelopeV1, AgentHandoffInputCitationV1,
    AgentHandoffMode, AgentHandoffPlanStepV1, AgentHandoffPlanV1,
    AgentHandoffRequirementOriginV1, AgentHandoffRequirementV1,
    AgentHandoffVerifiedArtifactV1, AgentSessionId, ArtifactId, ChatRouteFeature,
    ChatRouteProtocol,
    DeleteAgentSessionCommand, OperationId, PrincipalRef, RemoteBindingProvenance,
    ReasoningEffort, SessionPayloadBody, StrictJsonValue, UserId, digest_bytes,
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
#[path = "history_text_continuation.rs"]
mod history_text_continuation;
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
    native_engines: std::sync::OnceLock<std::sync::Weak<super::engine_session_host::EngineSessionHost>>,
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

/// Presentation context captured from the host-admitted binding. Error
/// diagnostics must not borrow a later Agent or model selection in this Session.
#[derive(Clone, Default)]
pub(super) struct TurnErrorContext {
    pub(super) agent_label: Option<String>,
    pub(super) agent_template_key: Option<String>,
    pub(super) model_name: Option<String>,
    pub(super) workspace_path: Option<String>,
}

impl TurnErrorContext {
    pub(super) fn from_response(response: &ConversationResponse, workspace: Option<&str>) -> Self {
        let snapshot = response.agent_snapshot.as_ref();
        Self {
            agent_label: snapshot.map(|snapshot| snapshot.preset_name.clone())
                .filter(|label| !label.trim().is_empty()),
            agent_template_key: response.extra.get("official_template_key")
                .and_then(Value::as_str)
                .filter(|value| nomifun_agent_contracts::OfficialPresetKey::ALL.iter()
                    .any(|key| key.as_str() == *value))
                .map(str::to_owned),
            model_name: snapshot.and_then(|snapshot| snapshot.resolved_model.as_ref())
                .map(|model| model.model.clone()).filter(|model| !model.trim().is_empty()),
            workspace_path: workspace.filter(|path| !path.is_empty()).map(str::to_owned),
        }
    }

    pub(super) fn apply(&self, error: &mut nomifun_api_types::AgentStreamErrorData) {
        error.agent_label = self.agent_label.clone();
        error.agent_template_key = self.agent_template_key.clone();
        error.model_name = self.model_name.clone();
        // A path-specific failure can identify the rejected path more precisely.
        if error.workspace_path.is_none() {
            error.workspace_path = self.workspace_path.clone();
        }
    }
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
            let (_, _, next_snapshot) = self.control_plane
                .saved_binding_artifacts(&owner, &binding)
                .await
                .map_err(control_plane_error_to_app)?;
            // Product identity resources belong to the target, never to an
            // older saved choice for that target. Other resource choices are
            // inherited only if the next Snapshot still requires their kind.
            let exact_target_resources = match target.target_kind.as_str() {
                "companion" => vec![
                    ("companion", target.target_id.as_str()),
                    ("companion_memory", target.target_id.as_str()),
                    ("scheduler", super::nomi_core_resource_bindings::INSTALLATION_SCHEDULER_RESOURCE_ID),
                ],
                "creative_studio_canvas" => vec![
                    ("canvas", target.target_id.as_str()),
                    ("asset_library", super::nomi_core_resource_bindings::CREATIVE_ASSET_LIBRARY_RESOURCE_ID),
                ],
                _ => Vec::new(),
            }.into_iter().filter(|(kind, _)| next_snapshot.content.required_resource_kinds.iter()
                .any(|required| required.as_ref() == *kind)).collect::<Vec<_>>();
            let target_resources_ready = existing.as_ref().is_some_and(|record| {
                exact_target_resources.iter().all(|(kind, id)| {
                    let mut bindings = record.agent_binding.typed_resource_bindings.iter()
                        .filter(|resource| resource.resource_kind == *kind);
                    matches!(bindings.next(), Some(resource) if resource.resource_id == *id)
                        && bindings.next().is_none()
                })
            });
            if target_resources_ready && existing.as_ref().is_some_and(|record|
                record.agent_binding.preset_revision_ref == binding.preset_revision_ref
                && record.agent_binding.resolved_snapshot_ref == binding.resolved_snapshot_ref) {
                existing.unwrap().agent_binding
            } else {
                if !exact_target_resources.is_empty() || existing.as_ref().is_some_and(|previous|
                    !previous.agent_binding.typed_resource_bindings.is_empty()) {
                    let mut selections = existing.as_ref().into_iter()
                        .flat_map(|previous| previous.agent_binding.typed_resource_bindings.iter())
                        .filter(|resource| next_snapshot.content.required_resource_kinds.iter()
                            .any(|kind| kind.as_ref() == resource.resource_kind))
                        .map(|resource| AgentResourceSelectionDto {
                            resource_kind: resource.resource_kind.clone(),
                            resource_id: resource.resource_id.clone(),
                        })
                        .collect::<Vec<_>>();
                    for (kind, id) in &exact_target_resources {
                        selections.retain(|resource| resource.resource_kind != *kind);
                        selections.push(AgentResourceSelectionDto {
                            resource_kind: (*kind).to_owned(),
                            resource_id: (*id).to_owned(),
                        });
                    }
                    if target.target_kind == "creative_studio_canvas"
                        && matches!(&selection, ProductAgentSelection::Template { template_key }
                            if template_key == "creative-studio.default")
                        && existing.as_ref().is_none_or(|record|
                            record.agent_binding.typed_resource_bindings.is_empty())
                    {
                        // A model-first Canvas selection may have no prior
                        // binding. Supply only the same deterministic owner
                        // infrastructure used by the implicit product entry.
                        for (kind, id) in [
                            ("workspace", super::nomi_core_resource_bindings::DEFAULT_WORKSPACE_RESOURCE_ID),
                            ("process_session", super::nomi_core_resource_bindings::MANAGED_PROCESS_SESSION_RESOURCE_ID),
                            ("project_memory", super::nomi_core_resource_bindings::DEFAULT_PROJECT_MEMORY_RESOURCE_ID),
                        ] {
                            if next_snapshot.content.required_resource_kinds.iter()
                                .any(|required| required.as_ref() == kind)
                                && !selections.iter().any(|resource| resource.resource_kind == kind) {
                                selections.push(AgentResourceSelectionDto {
                                    resource_kind: kind.to_owned(),
                                    resource_id: id.to_owned(),
                                });
                            }
                        }
                    }
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
            native_engines: std::sync::OnceLock::new(),
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

    pub(super) fn domain_pool(&self) -> &nomifun_db::SqlitePool {
        &self.pool
    }

    #[cfg(feature = "browser-use")]
    pub(crate) fn session_operation_locks(&self) -> Arc<DashMap<String, Arc<tokio::sync::RwLock<()>>>> {
        self.session_operation_locks.clone()
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

    /// A chat-only Agent still needs the runtime's managed working directory.
    /// This does not select or grant a Workspace resource in its frozen binding.
    async fn runtime_options_for_projection(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        mut projection: ConversationResponse,
    ) -> Result<AgentRuntimeBuildOptions, AppError> {
        if projection.conversation_id != session_id.as_ref() {
            return Err(AppError::Conflict("Runtime projection belongs to another Session".into()));
        }
        if projection.extra.get("workspace").and_then(Value::as_str)
            .is_none_or(|workspace| workspace.trim().is_empty())
        {
            let fallback = materialize_managed_session_workspace(&self.managed_workspace_root, session_id).await?;
            projection.extra["workspace"] = Value::String(fallback);
            projection.extra["custom_workspace"] = Value::Bool(false);
            projection.extra["is_temporary_workspace"] = Value::Bool(true);
            projection.extra["temp_workspace_id"] = Value::String(session_id.as_ref().to_owned());
        }
        runtime_options_from_session(owner_id, projection, None).map(|(options, _)| options)
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
                // A saved Canvas selection can point at another Canvas (or omit
                // its target resource). Resolve that choice before freezing a
                // new Session; existing Sessions keep their immutable binding.
                let needs_canvas_rebind = target.target_kind == "creative_studio_canvas"
                    && [
                        ("canvas", target.target_id.as_str()),
                        (
                            "asset_library",
                            super::nomi_core_resource_bindings::CREATIVE_ASSET_LIBRARY_RESOURCE_ID,
                        ),
                    ]
                    .into_iter()
                    .any(|(kind, id)| {
                        let mut bindings = existing
                            .agent_binding
                            .typed_resource_bindings
                            .iter()
                            .filter(|resource| resource.resource_kind == kind);
                        !matches!(bindings.next(), Some(resource) if resource.resource_id == id)
                            || bindings.next().is_some()
                    });
                let resolution = if needs_canvas_rebind {
                    let resolver = self
                        .product_agent_resolver
                        .get()
                        .and_then(std::sync::Weak::upgrade)
                        .ok_or_else(|| AppError::Conflict(
                            "Session product Agent resolver is unavailable".to_owned(),
                        ))?;
                    resolver.resolve(owner_id, &target, request.model.as_ref()).await?
                } else {
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
                    ProductAgentResolution {
                        snapshot: projected.projection.snapshot,
                        runtime_extra: projected.projection.request.extra,
                    }
                };
                merge_product_agent_resolution(
                    &mut request.extra,
                    &target,
                    &resolution,
                )?;
                snapshot = Some(resolution.snapshot);
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
        operation_id: &OperationId,
        request: &SendMessageRequest,
    ) -> Result<Value, AppError> {
        self.canonical_turn_input_with_expectation(owner_id,session_id,operation_id,request,None).await
    }

    async fn canonical_turn_input_with_expectation(&self,owner_id:&str,session_id:&AgentSessionId,operation_id:&OperationId,
        request:&SendMessageRequest,expected_binding_version:Option<u64>)->Result<Value,AppError> {
        let mut session = self
            .canonical
            .get(
                &PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner_id.to_owned(),
                },
                session_id,
            )
            .await?;
        if expected_binding_version.is_some_and(|expected|session.session.agent_binding.binding_version!=expected) {
            return Err(AppError::Conflict("voice input no longer matches its frozen Agent binding".into()));
        }
        let receipt = self.canonical.store().read_turn_receipt(session_id, operation_id)
            .await.map_err(agent_session_store_error)?;
        if let Some(started) = receipt.started_event {
            // Exact-key redelivery belongs to the original admission, even
            // after a model/config edit. The store still validates full input.
            let mut input = canonical_turn_input(request);
            if let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) = started.payload {
                if let Some(admission) = payload.0.get("admission") {
                    input["admission"] = admission.clone();
                }
                return Ok(input);
            }
            return Err(AppError::Conflict("canonical Turn admission is unavailable".into()));
        }
        if expected_binding_version.is_none() {
            self.prepare_session_contract_evolution(owner_id, session_id).await?;
            session = self.canonical.get(&PrincipalRef {
                principal_kind: "user".into(), principal_id: owner_id.into(),
            }, session_id).await?;
        }
        let mut binding = agent_binding_dto(&session.session.agent_binding)
            .map_err(|error| AppError::Conflict(error.message))?;
        let control_plane = self
            .runtime_control_plane
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Session control plane is unavailable".into()))?;
        let (mut saved_binding, revision, mut snapshot) = control_plane
            .saved_binding_artifacts(&UserId::from(owner_id.to_owned()), &binding)
            .await
            .map_err(control_plane_error_to_app)?;
        if saved_binding != session.session.agent_binding {
            return Err(AppError::Conflict(
                "AgentSession binding differs from its saved immutable artifacts".to_owned(),
            ));
        }
        if expected_binding_version.is_none() && session.head.active_turn_id.is_none()
            && session.head.status != "running"
            && session.session.remote_binding_provenance.is_none()
        {
            let attempt: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM conversation_execution_links \
                 WHERE conversation_id = ? AND relation IN ('attempt', 'automation'))",
            )
            .bind(session_id.as_ref())
            .fetch_one(&self.pool)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
            // Refresh only the selected Chat model/configuration at an idle
            // turn boundary; all non-model Agent/resource contracts stay exact.
            if !attempt && let Some(route) = revision.payload.chat_route_records
                .get(nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT)
            {
                let owner = UserId::from(owner_id.to_owned());
                let principal = PrincipalRef {
                    principal_kind: "user".into(),
                    principal_id: owner_id.into(),
                };
                let model = AgentChatModelSelectionDto {
                    provider_id: route.primary.provider_id.clone(),
                    model: route.primary.model.clone(),
                };
                let refreshed = control_plane
                    .resolve_agent_session_model_binding(&owner, &binding, &model)
                    .await.map_err(control_plane_error_to_app)?;
                let clear_effort = if let Some(effort) = session.session.metadata.reasoning_effort {
                    !binding_supports_reasoning_effort(&control_plane, &owner, &refreshed, effort).await?
                } else {
                    false
                };
                let refreshed_value: AgentBindingValue = serde_json::to_value(&refreshed)
                    .and_then(serde_json::from_value)
                    .map_err(|error| AppError::Conflict(format!(
                        "refreshed model binding is invalid: {error}"
                    )))?;
                if refreshed_value != session.session.agent_binding {
                    // A view warmup may have built an idle host from the old
                    // canonical snapshot but the already-updated provider
                    // revision. The registry's provider cache cannot detect
                    // this snapshot-only change. Prove that exact idle owner
                    // closed before committing a new immutable binding.
                    self.settle_session_binding_runtime(owner_id, session_id).await?;
                    self.canonical.store().replace_session_model_binding(
                        &principal, session_id, &session.session.agent_binding, refreshed_value,
                    ).await.map_err(agent_session_store_error)?;
                    if clear_effort {
                        self.canonical.store().update_session_reasoning_effort(
                            &principal, session_id, None,
                        ).await.map_err(agent_session_store_error)?;
                    }
                    session = self.canonical.get(&principal, session_id).await?;
                    binding = agent_binding_dto(&session.session.agent_binding)
                        .map_err(|error| AppError::Conflict(error.message))?;
                    let current = control_plane.saved_binding_artifacts(&owner, &binding)
                        .await.map_err(control_plane_error_to_app)?;
                    saved_binding = current.0;
                    snapshot = current.2;
                }
            }
        }
        if request.plugin_delivery.is_some() {
            let module = snapshot.content.enabled_capabilities.iter().find(|capability|
                capability.consumption.is_contribution()
                    && capability.capability.id.as_ref() == nomifun_plugin_development::MODULE_ID);
            if module.is_none() || nomifun_plugin_development::CREATE_ACTIONS.iter().any(|action|
                !module.expect("checked module").action_allowlist.iter().any(|allowed| allowed.as_ref() == *action)) {
                return Err(AppError::UnprocessableEntity(
                    "AGENT_LAUNCH_MODULE_REQUIRED: plugin delivery requires the saved creation module and actions".into(),
                ));
            }
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
        let facts = self.canonical.store().turn_output_facts(session_id, operation_id)
            .await.map_err(agent_session_store_error)?;
        let delivery = nomifun_agent_execution::canonical_turn_delivery(&facts, &receipt, replayed)?;
        Ok(PublicTurnDeliveryState::Completed(IdempotentMessageDelivery {
            message_id: delivery.message_id,
            replayed: delivery.replayed,
            completed: delivery.completed,
            result_ok: delivery.result_ok,
            result_text: delivery.result_text,
            result_error: delivery.result_error,
            result_error_code: delivery.result_error_code,
            result_error_retryable: delivery.result_error_retryable,
        }))
    }

    async fn settle_dispatch_failure(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        operation_id: &OperationId,
        message: &str,
        mut error: nomifun_api_types::AgentStreamErrorData,
    ) -> Result<Option<nomifun_api_types::AgentStreamErrorData>, AppError> {
        let receipt = self
            .canonical
            .store()
            .read_turn_receipt(session_id, operation_id)
            .await
            .map_err(agent_session_store_error)?;
        if receipt.status != nomifun_agent_session::TurnReceiptStatus::Running {
            return Ok(None);
        }
        // Dispatch still holds the Session operation lock; this binding is the
        // one admitted for the failing Turn, before any subsequent selection.
        if let Ok(Some(response)) = self.canonical_conversation_projection(owner_id, session_id).await
            && let Some(binding) = response.agent_snapshot.as_ref().and_then(|snapshot| snapshot.canonical_binding.as_ref())
            && let Some(started) = receipt.started_event.as_ref()
            && let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) = &started.payload
            && payload.0.get("resolved_snapshot_ref") == serde_json::to_value(&binding.resolved_snapshot_ref).ok().as_ref()
        {
            let workspace = response.extra.get("workspace").and_then(Value::as_str);
            TurnErrorContext::from_response(&response, workspace).apply(&mut error);
        }
        let started = receipt.started_event.ok_or_else(|| {
            AppError::Conflict("failed Turn has no start fact".to_owned())
        })?;
        let identity = format!(
            "turn-dispatch-failed:{}:{}",
            session_id.as_ref(),
            operation_id.as_ref(),
        );
        let result = self.canonical
            .store()
            .append_turn_terminal(
                &nomifun_agent_contracts::SessionEventAppend {
                    agent_session_id: session_id.clone(),
                    event_id: nomifun_agent_contracts::EventId::from(identity.clone()),
                    producer_id: nomifun_agent_contracts::EventProducerId::from(
                        "runtime_supervisor",
                    ),
                    idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(identity),
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
            .await;
        match result {
            Ok(_) => {},
            Err(nomifun_agent_session::SessionStoreError::ExecutionLeaseActive) => return Ok(None),
            Err(error) => return Err(agent_session_store_error(error)),
        }
        Ok(Some(error))
    }

    /// A process restart cannot retain an in-memory Runtime owner. Reconcile
    /// every durable running Turn before publishing routes so the Session is
    /// immediately usable again instead of remaining permanently busy.
    pub(crate) async fn reconcile_orphaned_active_turns(self: &Arc<Self>, engine_sessions: Arc<super::engine_session_host::EngineSessionHost>) -> Result<usize, AppError> {
        let _ = self.native_engines.set(Arc::downgrade(&engine_sessions));
        let failed_pauses: Vec<(String, String)> = sqlx::query_as(
            "SELECT h.session_id,json_extract(s.owner_ref_json,'$.principal_id') FROM agent_session_heads h \
             JOIN agent_sessions s ON s.agent_session_id=h.session_id WHERE s.state='live' AND h.status='paused' \
             AND json_extract(s.owner_ref_json,'$.principal_kind')='user'")
            .fetch_all(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
        for (session, owner) in failed_pauses {
            // Existing current-generation model failures need no Runtime
            // restart or replay. Ordinary owner pauses remain recoverable.
            if let Err(error) = self.canonical_conversation_projection(&owner, &session.into()).await {
                tracing::warn!(%error, "existing native failure pause remains fenced during startup");
            }
        }
        self.schedule_native_recovery(engine_sessions).await
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
        if let AgentStreamEvent::Error(error) = event {
            // Structured error fields have their own public wire contract.
            event_data["data"] = serde_json::to_value(error).ok()?;
        }
        let step_message_id = match event {
            AgentStreamEvent::Text(data) => data.step.and_then(|step|
                super::engine_journal::canonical_assistant_step_message_id(root_message_id, step).ok()),
            AgentStreamEvent::Thinking(data) => data.step.and_then(|step|
                super::engine_journal::canonical_thinking_step_message_id(root_message_id, step).ok()),
            _ => None,
        };
        let assistant_message_id = step_message_id.as_deref().unwrap_or(assistant_message_id);
        Some(WebSocketMessage::new(
            "message.stream",
            json!({
                "conversation_id": session_id,
                "msg_id": assistant_message_id,
                "turn_id": root_message_id,
                "type": event_data.get("type").cloned().unwrap_or(json!("unknown")),
                "data": event_data.get("data").cloned().unwrap_or_else(|| json!({})),
                "hidden": false,
                "created_at": now_ms(),
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

    fn canonical_turn_paused_wire_event(
        session_id: &AgentSessionId,
        root_message_id: &str,
    ) -> WebSocketMessage<Value> {
        WebSocketMessage::new("turn.paused",json!({
            "conversation_id":session_id,"turn_id":root_message_id,"status":"paused","execution_phase":"paused",
            "state":"ai_waiting_input","detail":"Execution paused; owner authorization is required to continue.","can_send_message":false,
            "runtime":{"state":"idle","execution_phase":"paused","can_send_message":false,"has_runtime":true,
                "runtime_status":"finished","is_processing":false,"active_turn_id":root_message_id},
        }))
    }

    fn canonical_turn_completed_wire_event(
        session_id: &AgentSessionId,
        root_message_id: &str,
        terminal: &AgentStreamEvent,
    ) -> WebSocketMessage<Value> {
        if matches!(terminal, AgentStreamEvent::Finish(data) if data.stop_reason == Some(nomifun_ai_agent::protocol::events::TurnStopReason::Paused)) {
            return Self::canonical_turn_paused_wire_event(session_id,root_message_id);
        }
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
        processing_started_at: i64,
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
                    "processing_started_at": processing_started_at,
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
        operation_id: OperationId,
        cancellation: tokio_util::sync::CancellationToken,
        mut events: broadcast::Receiver<AgentStreamEvent>,
    ) {
        let sink = self.user_events.clone();
        let runtimes = self.runtime_sessions.clone();
        let relay_session_id = session_id.clone();
        let store = self.canonical.store().clone();
        let task = async move {
            loop {
                let mut event = tokio::select! {
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
                if terminal {
                    // A fenced old driver may emit a local error after a new
                    // producer owns the Turn. Only a matching canonical
                    // terminal may close the product stream.
                    if !Self::canonical_stream_terminal_agrees(&store, &session_id, &operation_id, turn_generation, &event).await { break; }
                    if matches!(event, AgentStreamEvent::Error(_))
                        && let Ok(receipt) = store.read_turn_receipt(&session_id, &operation_id).await
                        && let Some(terminal) = receipt.terminal_event
                        && let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) = terminal.payload
                        && let Some(error) = payload.0.get("error")
                        && let Ok(error) = serde_json::from_value(error.clone())
                    {
                        // The canonical terminal contains the captured Agent,
                        // model and workspace, shared by realtime and history.
                        event = AgentStreamEvent::Error(error);
                    }
                }
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

    async fn canonical_stream_terminal_agrees(
        store: &nomifun_agent_session::AgentSessionStore, session: &AgentSessionId,
        operation: &OperationId, generation: u64, event: &AgentStreamEvent,
    ) -> bool {
        // Read state and generation from one snapshot. An old relay must not
        // borrow a later generation's pause OR terminal for the same Turn.
        let state = match store.native_execution_notification_state(session, operation, generation).await {
            Ok(Some(state)) => state, _ => return false,
        };
        if matches!(event, AgentStreamEvent::Finish(data) if data.stop_reason == Some(nomifun_ai_agent::protocol::events::TurnStopReason::Paused)) {
            return state == "paused";
        }
        match event {
            AgentStreamEvent::Error(_) => state == "failed",
            AgentStreamEvent::Finish(_) => matches!(state.as_str(), "completed" | "cancelled" | "interrupted"),
            _ => false,
        }
    }

    async fn dispatch_canonical_turn(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        request: SendMessageRequest,
        initial_only: bool,
        idmm_decision: Option<IdmmDecisionExplanation>,
    ) -> Result<IdempotentMessageDelivery, AppError> {
        if self.canonical.store().head(session_id).await.map_err(agent_session_store_error)?.status == "paused" {
            // The same owner settlement used by reload also precedes a fresh
            // request from callers that do not first fetch the conversation.
            self.canonical_conversation_projection(owner_id, session_id).await?;
        }
        let _operation_fence = self
            .session_operation_lock(session_id.as_ref())
            .write_owned()
            .await;
        self.dispatch_canonical_turn_locked(owner_id,session_id,idempotency_key,request,initial_only,None,None,None,false,idmm_decision).await
    }

    async fn dispatch_canonical_turn_locked(&self,owner_id:&str,session_id:&AgentSessionId,idempotency_key:&str,request:SendMessageRequest,
        initial_only:bool,voice_binding:Option<u64>,voice_context_floor:Option<u64>,voice_shutdown:Option<&tokio_util::sync::CancellationToken>,voice_supersede:bool,idmm_decision:Option<IdmmDecisionExplanation>)->Result<IdempotentMessageDelivery,AppError> {
        let mut input = if let Some(version)=voice_binding {
            self.canonical_turn_input_with_expectation(owner_id,session_id,&Self::turn_operation_id(owner_id,session_id.as_ref(),idempotency_key),&request,Some(version)).await?
        }else {self
            .canonical_turn_input_with_admission(
                owner_id, session_id,
                &Self::turn_operation_id(owner_id, session_id.as_ref(), idempotency_key),
                &request,
            )
            .await?};
        if voice_shutdown.is_some_and(|stop|stop.is_cancelled()){return Err(AppError::Conflict("voice dispatcher stopped before admission".into()));}
        if let Some(decision) = &idmm_decision {
            decision.validate().map_err(|error| AppError::BadRequest(error.into()))?;
            if request.origin.as_deref() != Some("idmm") {
                return Err(AppError::BadRequest("IDMM source must be authored by the supervisor".into()));
            }
            input["idmm_decision"] = serde_json::to_value(decision)
                .map_err(|error| AppError::Internal(error.to_string()))?;
        }
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: owner_id.to_owned(),
        };
        let receipt = if let Some(binding)=voice_binding {
            self.canonical.start_voice_turn(&principal,session_id,idempotency_key,input,&nomifun_agent_session::NativeInputContextFence {binding_version:binding,
                context_floor:voice_context_floor.ok_or_else(||AppError::Conflict("voice input has no original context proof".into()))?,supersede_model_step:voice_supersede}).await?
        }else if initial_only {
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
        // All windows observe the same committed accepted input before Runtime
        // output starts, including input sent by another product surface.
        if let Err(error) = self.publish_canonical_accepted_input(
            owner_id, session_id, &operation_id, &root_message_id,
        ).await {
            tracing::warn!(agent_session_id=session_id.as_ref(),message_id=%root_message_id,%error,
                "canonical realtime input delivery unavailable; continuing the accepted Turn");
        }
        let projection = self
            .canonical_conversation_projection(owner_id, session_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!(
                "AgentSession {} not found",
                session_id.as_ref(),
            )))?;
        let processing_started_at = projection
            .runtime
            .as_ref()
            .filter(|runtime| runtime.active_turn_id.as_deref() == Some(root_message_id.as_str()))
            .and_then(|runtime| runtime.processing_started_at)
            .ok_or_else(|| {
                AppError::Conflict("accepted canonical Turn has no authoritative start time".into())
            })?;
        let options = self.runtime_options_for_projection(owner_id, session_id, projection).await?;
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
                let detail = error.to_string();
                let settled_error = self.settle_dispatch_failure(
                    owner_id, session_id, &operation_id, &detail,
                    AgentSendError::from_app_error_ref(&error).into_stream_error(),
                ).await?;
                if let Some(settled_error) = settled_error
                    && let Some(message) = Self::canonical_stream_wire_event(
                        session_id, &root_message_id,
                        &Self::canonical_assistant_stream_message_id(&root_message_id)?,
                        &AgentStreamEvent::Error(settled_error),
                    )
                {
                    self.user_events.send_to_user(owner_id, message);
                    self.user_events.send_to_user(owner_id, Self::canonical_turn_dispatch_failed_wire_event(
                        session_id, &root_message_id, &detail,
                    ));
                }
                return Err(error);
            }
        };
        let events = runtime.subscribe();
        let assistant_message_id =
            Self::canonical_assistant_stream_message_id(&root_message_id)?;
        self.user_events.send_to_user(
            owner_id,
            Self::canonical_turn_started_wire_event(
                session_id,
                &root_message_id,
                processing_started_at,
            ),
        );
        self.spawn_canonical_stream_relay(
            owner_id.to_owned(),
            session_id.clone(),
            root_message_id.clone(),
            assistant_message_id.clone(),
            generation,
            operation_id.clone(),
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
            let settled_error = self.settle_dispatch_failure(
                owner_id, session_id, &operation_id, &detail, error.stream_error().clone(),
            ).await?;
            if let Some(settled_error) = settled_error
                && let Some(message) = Self::canonical_stream_wire_event(
                    session_id, &root_message_id, &assistant_message_id,
                    &AgentStreamEvent::Error(settled_error),
                )
            {
                self.user_events.send_to_user(owner_id, message);
                self.user_events.send_to_user(
                    owner_id,
                    Self::canonical_turn_dispatch_failed_wire_event(
                        session_id, &root_message_id, &detail,
                    ),
                );
            }
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
            return Err(if error.code() == Some(nomifun_api_types::AgentErrorCode::NomifunSessionConfigurationChanged) {
                AppError::SessionConfigurationChanged(detail)
            } else { AppError::BadGateway(detail) });
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
        // Step revision and Attempt identity fence the invocation. Outbox
        // enqueue/ack may advance only the Attempt's metadata CAS revision.
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
               AND attempt.attempt_id = ? AND attempt.version >= ? \
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

    /// Materialize the Conversation consumer projection from the canonical
    /// AgentSession. `None` means there is no canonical row; foreign ownership,
    /// deletion and invalid saved artifacts fail closed.
    pub(crate) async fn canonical_conversation_projection(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
    ) -> Result<Option<ConversationResponse>, AppError> {
        let projection = self.canonical_conversation_projection_readonly(owner_id, session_id).await?;
        let Some(response) = projection.as_ref() else { return Ok(None); };
        if response.extra.get("execution_phase").and_then(Value::as_str) != Some("paused") {
            return Ok(projection);
        }
        // Projection is also called under admission/recovery locks. Those
        // callers cannot recursively acquire the writer lock; a fresh GET or
        // the pre-admission pass performs settlement instead.
        let Ok(_operation_guard) = self.session_operation_lock(session_id.as_ref()).try_write_owned() else {
            return Ok(projection);
        };
        // A Session could have resumed or selected a successor between the
        // first read and this lock. Only this fresh exact binding may release
        // the registry owner; the Store CAS alone would be too late for that.
        let current = self.canonical_conversation_projection_readonly(owner_id, session_id).await?;
        let Some(current) = current.as_ref() else { return Ok(None); };
        self.settle_existing_model_failure_pause(owner_id, session_id, current).await?;
        self.canonical_conversation_projection_readonly(owner_id, session_id).await
    }

    async fn settle_existing_model_failure_pause(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        response: &ConversationResponse,
    ) -> Result<bool, AppError> {
        let principal = PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() };
        let store = self.canonical.store();
        let Some(execution) = store.inspect_latest_native_execution(&principal, session_id).await
            .map_err(agent_session_store_error)? else { return Ok(false); };
        let Some(pause) = execution.pause else { return Ok(false); };
        if execution.state != "paused" || !pause.cleanup_proven
            || execution.pending_effects != 0 || execution.unknown_effects != 0
            || execution.producer_lease_live || execution.pause_requested
            || execution.checkpoint_revision != pause.checkpoint_revision
            || execution.checkpoint_digest.as_deref() != pause.checkpoint_digest.as_ref().map(|digest| digest.as_ref())
        {
            return Ok(false);
        }
        let Some(failure) = AgentSendError::from_model_pause_reason(&pause.reason) else { return Ok(false); };
        let Some(binding) = response.agent_snapshot.as_ref().and_then(|snapshot| snapshot.canonical_binding.as_ref()) else {
            return Ok(false);
        };
        let snapshot: nomifun_agent_contracts::ResolvedSnapshotRef = serde_json::to_value(&binding.resolved_snapshot_ref)
            .and_then(serde_json::from_value).map_err(|error| AppError::Conflict(format!(
                "native failure pause has an invalid admitted Snapshot: {error}")))?;
        let operation = OperationId::from(execution.operation_id);
        if self.runtime_sessions.active_turn_generation(session_id.as_ref())
            .is_some_and(|generation| generation != execution.execution_generation)
        {
            return Ok(false);
        }
        let receipt = store.read_turn_receipt(session_id, &operation).await.map_err(agent_session_store_error)?;
        let Some(started) = receipt.started_event else { return Ok(false); };
        let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(started_payload) = &started.payload else {
            return Ok(false);
        };
        let Some(root) = started_payload.0.get("source_message_id").and_then(Value::as_str) else {
            return Ok(false);
        };
        let facts = store.chat_causality_facts(session_id, &operation).await.map_err(agent_session_store_error)?;
        if facts.head.status != "paused" || facts.head.active_turn_id.as_deref() != Some(operation.as_ref())
            || facts.session.owner_ref != principal
            || facts.session.agent_binding.resolved_snapshot_ref != snapshot
            || facts.execution_generation != execution.execution_generation
            || facts.execution_fence != pause.execution_fence
            || response.runtime.as_ref().and_then(|runtime| runtime.active_turn_id.as_deref()) != Some(root)
            || response.extra.pointer("/execution_pause/paused_at_ms").and_then(Value::as_i64) != Some(pause.paused_at_ms)
            || started_payload.0.get("resolved_snapshot_ref") != Some(&serde_json::to_value(&snapshot)
                .map_err(|error| AppError::Internal(error.to_string()))?)
        {
            return Ok(false);
        }
        // Preserve release failures as fences. A cleanup flag is not proof
        // that this process has joined and released its retained native owner.
        if let Err(error) = self.runtime_sessions.terminate_and_wait_result(session_id.as_ref(), None).await {
            tracing::warn!(agent_session_id=session_id.as_ref(), %error,
                "native model failure pause could not release its Runtime owner");
            return Ok(false);
        }
        self.runtime_sessions.release_runtime_turn(session_id.as_ref(), execution.execution_generation).await?;
        let mut error = failure.into_stream_error();
        TurnErrorContext::from_response(response, response.extra.get("workspace").and_then(Value::as_str)).apply(&mut error);
        let identity = format!("native-failure-settled:{}:{}:{}", session_id.as_ref(), operation.as_ref(), pause.revision);
        let failed = nomifun_agent_contracts::SessionEventAppend {
            agent_session_id: session_id.clone(), event_id: identity.clone().into(), producer_id: "runtime_supervisor".into(),
            idempotency_key: identity.into(),
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: nomifun_agent_contracts::SessionEventKind("turn/failed".into()), kind_version: 1, correlation_id: operation.as_ref().into(),
                causation_event_id: Some(started.event_id.clone()),
                payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "message":error.message,"error":error,"finished_at_ms":pause.paused_at_ms,
                    "native_failure_pause_revision":pause.revision,
                }))),
            },
        };
        let committed = store.settle_native_failure_pause(&principal, session_id, &operation,
            &snapshot, &pause, &[pause.reason.as_str()], &failed).await.map_err(agent_session_store_error)?;
        if committed {
            if let Some(message) = Self::canonical_stream_wire_event(session_id, root,
                &Self::canonical_assistant_stream_message_id(root)?, &AgentStreamEvent::Error(error.clone())) {
                self.user_events.send_to_user(owner_id, message);
            }
            self.user_events.send_to_user(owner_id,
                Self::canonical_turn_completed_wire_event(session_id, root, &AgentStreamEvent::Error(error)));
        }
        Ok(committed)
    }

    async fn canonical_conversation_projection_readonly(
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
        // A later pause of the same Turn may be outside the first event page,
        // even when that page already contains an older pause.
        if observed.head.status == "paused"
            && let Some(operation) = observed.head.active_turn_id.as_deref()
            && let Some(paused) = self.canonical.store().read_native_pause_event_at(
                session_id, &OperationId::from(operation), observed.head.last_seq,
            ).await.map_err(agent_session_store_error)?
            && !observed.events.iter().any(|event| event.event_id == paused.event_id)
        {
            observed.events.push(paused);
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
        let (agent_name, official_template) = control_plane
            .saved_binding_presentation(&owner, &binding_dto)
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
        if request.origin.as_deref() == Some("idmm") {
            return Err(AppError::BadRequest("IDMM origin is reserved for the supervisor".into()));
        }
        self.dispatch_canonical_turn(
            owner_id,
            &AgentSessionId::from(session_id.to_owned()),
            idempotency_key,
            request,
            false,
            None,
        )
        .await
    }

    pub(crate) async fn send_session_idmm_message_idempotent(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
        request: SendMessageRequest,
        decision: IdmmDecisionExplanation,
    ) -> Result<IdempotentMessageDelivery, AppError> {
        self.dispatch_canonical_turn(owner_id, &AgentSessionId::from(session_id.to_owned()),
            idempotency_key, request, false, Some(decision)).await
    }

    async fn publish_canonical_accepted_input(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        operation_id: &OperationId,
        message_id: &str,
    ) -> Result<(), AppError> {
        // This exact Turn snapshot resolves both inline and stored Payloads.
        // Before dispatch it contains only the committed input/start boundary;
        // no Message projection or current configuration supplies its content.
        let facts = self.canonical.store().native_recovery_facts(session_id, operation_id)
            .await.map_err(agent_session_store_error)?;
        let event = facts.events.iter().find(|event| event.event_id.as_ref() == message_id)
            .ok_or_else(|| AppError::Conflict("accepted input has no canonical event".into()))?;
        let input = facts.event_payloads.get(message_id)
            .ok_or_else(|| AppError::Conflict("accepted input has no resolved canonical payload".into()))?;
        let created_at = self.canonical.store().session_created_at(session_id)
            .await.map_err(agent_session_store_error)?;
        self.user_events.send_to_user(owner_id,
            Self::canonical_accepted_input_wire_event(session_id, created_at, event, input)?);
        Ok(())
    }

    fn canonical_accepted_input_wire_event(
        session_id: &AgentSessionId,
        created_at: i64,
        event: &nomifun_agent_contracts::SessionEventRecord,
        input: &Value,
    ) -> Result<WebSocketMessage<Value>, AppError> {
        if event.agent_session_id != *session_id || event.kind.0 != "message/user-accepted"
            || event.kind_version != 1 || event.correlation_id.as_ref() != event.event_id.as_ref()
        {
            return Err(AppError::Conflict("canonical input event identity is invalid".into()));
        }
        let message_uuid = Uuid::parse_str(event.event_id.as_ref())
            .ok().filter(|message| message.get_version_num() == 7)
            .ok_or_else(|| AppError::Conflict("accepted input identity must be UUIDv7".into()))?;
        let display_at_ms = message_uuid.as_bytes()[..6].iter()
            .fold(0_i64, |time, byte| (time << 8) | i64::from(*byte));
        let content = input.get("content").and_then(Value::as_str)
            .ok_or_else(|| AppError::Conflict("accepted input content is invalid".into()))?;
        let decision: Option<IdmmDecisionExplanation> = input.get("idmm_decision")
            .filter(|value| !value.is_null()).cloned().map(serde_json::from_value).transpose()
            .map_err(|error| AppError::Conflict(format!("accepted IDMM source is invalid: {error}")))?;
        if let Some(decision) = &decision {
            decision.validate().map_err(|error| AppError::Conflict(error.into()))?;
        }
        Ok(WebSocketMessage::new("message.userCreated", json!({
            "conversation_id":session_id,"msg_id":event.event_id,
            "content":content,"idmm_decision":decision,
            "position":"right","status":"finish","hidden":input["hidden"],
            "origin":input["origin"],"channel_platform":input["channel_platform"],
            "created_at":created_at.saturating_add(i64::try_from(event.seq).unwrap_or(i64::MAX)),
            "display_at_ms":display_at_ms,
        })))
    }

    pub(crate) async fn append_session_idmm_notice(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
        notice: IdmmDecisionNotice,
    ) -> Result<(), AppError> {
        let session_id = AgentSessionId::from(session_id.to_owned());
        let principal = PrincipalRef { principal_kind:"user".into(),principal_id:owner_id.into() };
        let result = self.canonical.store().append_idmm_notice(&principal, &session_id, idempotency_key, notice)
            .await.map_err(agent_session_store_error)?;
        if let Some(result) = result.filter(|result| !result.duplicate) {
            if let Some(record) = result.record {
                self.user_events.send_to_user(owner_id, WebSocketMessage::new("message.annotationUpdated", json!({
                    "conversation_id":session_id,"message_id":record.correlation_id,
                })));
            }
        }
        Ok(())
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

    /// Voice input waits on the original work boundary and does not take over
    /// Desktop queueing. None proves no new admission was attempted.
    pub(crate) async fn voice_start_message_with_policy(&self,owner:&str,session:&AgentSessionId,key:&str,request:SendMessageRequest,version:u64,context_floor:u64,stop:&tokio_util::sync::CancellationToken,supersede:bool)
        ->Result<Option<IdempotentMessageDelivery>,AppError> {
        let lock=self.session_operation_lock(session.as_ref());
        let _guard=tokio::select! {biased;_ = stop.cancelled()=>return Ok(None),guard=lock.write_owned()=>guard};
        let observation=self.canonical.get(&PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()},session).await?;
        if observation.session.agent_binding.binding_version!=version{return Err(AppError::Conflict("voice input binding changed before admission".into()));}
        if observation.head.status!="ready"||observation.head.active_turn_id.is_some(){return Ok(None);}
        let idle=self.runtime_sessions.active_turn_generation(session.as_ref()).is_none()&&match self.runtime_sessions.get_runtime(session.as_ref()) {
            // None is the original pristine HostedRuntime constructor state.
            // The registry/canonical/effect fences still prove no live Turn.
            Some(runtime)=>runtime.is_transport_healthy()&&matches!(runtime.status(),None|Some(nomifun_common::ConversationStatus::Pending|nomifun_common::ConversationStatus::Finished)),
            None=>!self.runtime_sessions.has_owned_runtime(session.as_ref()),
        };
        if !idle||self.canonical.store().has_unsettled_effects(session).await.map_err(agent_session_store_error)? {return Ok(None);}
        if stop.is_cancelled(){return Ok(None);}
        self.dispatch_canonical_turn_locked(owner,session,key,request,false,Some(version),Some(context_floor),Some(stop),supersede,None).await.map(Some)
    }

    pub(crate) async fn voice_operation_receipt(&self,owner:&str,session:&AgentSessionId,key:&str)->Result<nomifun_agent_session::TurnReceipt,AppError> {
        self.canonical.turn_receipt(&PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()},session,&Self::turn_operation_id(owner,session.as_ref(),key)).await
    }

    pub(crate) fn voice_register_task(&self,task:std::pin::Pin<Box<dyn std::future::Future<Output=()>+Send+'static>>)->bool {
        self.background_tasks.spawn(task)
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
        self.cancel_receipted_runtime_turn(session_id, &receipt, reason).await?;
        Ok(receipt)
    }

    async fn cancel_exact_turn(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        target_operation_id: &OperationId,
        reason: nomifun_common::AgentKillReason,
    ) -> Result<AgentMutationReceipt, AppError> {
        let receipt = self.canonical.cancel_exact_turn(
            &PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() },
            session_id, idempotency_key, target_operation_id,
        ).await?;
        self.cancel_receipted_runtime_turn(session_id, &receipt, reason).await?;
        Ok(receipt)
    }

    async fn cancel_accepted_input_turn(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
        idempotency_key: &str,
        source_message_id: &str,
    ) -> Result<AgentMutationReceipt, AppError> {
        let source = Uuid::parse_str(source_message_id)
            .ok().filter(|source| source.get_version_num() == 7)
            .ok_or_else(|| AppError::BadRequest("expected_turn_id must be a canonical UUIDv7".into()))?;
        self.canonical.get(
            &PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() },
            session_id,
        ).await?;
        // Source-to-operation identity is immutable. The existing exact Turn
        // mutation rechecks active/closed state atomically, so a successor
        // started between this lookup and cancellation cannot become a target.
        let operation: Option<String> = sqlx::query_scalar(
            "SELECT turn.operation_id FROM agent_turns turn \
             JOIN agent_events started ON started.event_id=turn.started_event_id \
               AND started.session_id=turn.session_id AND started.kind='turn/started' \
               AND started.correlation_id=turn.operation_id \
             JOIN agent_events input ON input.event_id=turn.source_message_id \
               AND input.session_id=turn.session_id AND input.kind='message/user-accepted' \
               AND started.causation_event_id=input.event_id \
             WHERE turn.session_id=? AND turn.source_message_id=?",
        ).bind(session_id.as_ref()).bind(source.to_string()).fetch_optional(&self.pool).await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let operation = operation.ok_or_else(|| AppError::Conflict("expected Turn has no canonical accepted input".into()))?;
        self.cancel_exact_turn(owner_id, session_id, idempotency_key, &OperationId::from(operation),
            nomifun_common::AgentKillReason::UserCancelled).await
    }

    /// Voice opt-in adds proof, then delegates runtime cleanup to the existing owner.
    pub(crate) async fn voice_cancel_exact_native_turn(&self,owner:&str,session:&AgentSessionId,key:&str,target:&OperationId,
        fence:&nomifun_agent_session::NativeTurnMutationFence)->Result<AgentMutationReceipt,AppError> {
        let receipt=self.canonical.cancel_exact_native_turn(&PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()},session,key,target,fence).await?;
        self.cancel_receipted_runtime_turn(session,&receipt,nomifun_common::AgentKillReason::UserCancelled).await?;
        Ok(receipt)
    }

    pub(crate) async fn voice_steer_exact_native_with_policy(&self,owner:&str,session:&AgentSessionId,key:&str,target:&OperationId,input:SendMessageRequest,
        fence:&nomifun_agent_session::NativeTurnMutationFence,supersede:bool)->Result<AgentMutationReceipt,AppError> {
        let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()};
        let receipt=if supersede {self.canonical.steer_exact_native_immediate_turn(&principal,session,key,target,canonical_turn_input(&input),fence).await?}
            else{self.canonical.steer_exact_native_turn(&principal,session,key,target,canonical_turn_input(&input),fence).await?};
        self.queue_receipted_steering(session,&receipt).await?;
        Ok(receipt)
    }

    async fn cancel_receipted_runtime_turn(
        &self,
        session_id: &AgentSessionId,
        receipt: &AgentMutationReceipt,
        reason: nomifun_common::AgentKillReason,
    ) -> Result<(), AppError> {
        // The mutation receipt fixes the cancelled Turn, including replays.
        // Native resume can advance that Turn's generation beyond its original
        // start sequence; resolve the exact generation from canonical storage.
        let generation = self.canonical.store().native_execution_generation(
            session_id, &receipt.target_operation_id,
        ).await.map_err(agent_session_store_error)?;
        self.runtime_sessions.cancel_runtime_turn(
            session_id.as_ref(),
            generation,
            Some(reason),
        )?;
        Ok(())
    }

    async fn queue_receipted_steering(
        &self,
        session_id: &AgentSessionId,
        receipt: &AgentMutationReceipt,
    ) -> Result<(), AppError> {
        let turn = self.canonical.store().read_turn_receipt(session_id, &receipt.target_operation_id)
            .await.map_err(agent_session_store_error)?;
        if matches!(turn.status, nomifun_agent_session::TurnReceiptStatus::Completed
            | nomifun_agent_session::TurnReceiptStatus::Failed | nomifun_agent_session::TurnReceiptStatus::Cancelled)
            && turn.terminal_event.is_some() {
            // A durable steering command for a closed target has expired. Its
            // original canonical fact is the acknowledgement, never a new input
            // for whichever Turn happens to be active now.
            return Ok(());
        }
        if turn.status != nomifun_agent_session::TurnReceiptStatus::Running || turn.terminal_event.is_some() {
            return Err(AppError::Conflict("steering target has no active canonical Turn receipt".into()));
        }
        let started = turn.started_event.ok_or_else(|| AppError::Conflict("steering target has no start fact".into()))?;
        let root = match &started.payload {
            nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) => payload.0.get("source_message_id")
                .and_then(Value::as_str).map(str::to_owned),
            _ => None,
        }.ok_or_else(|| AppError::Conflict("steering target has no source message".into()))?;
        let facts = self.canonical.store().turn_output_facts(session_id, &receipt.target_operation_id)
            .await.map_err(agent_session_store_error)?;
        if facts.head.active_turn_id.as_deref() != Some(receipt.target_operation_id.as_ref()) {
            if facts.events.iter().any(|event| event.correlation_id.as_ref() == receipt.target_operation_id.as_ref()
                && matches!(event.kind.0.as_str(), "turn/completed" | "turn/failed" | "turn/cancelled")) {
                return Ok(());
            }
            return Err(AppError::Conflict("steering target no longer owns the canonical active Turn".into()));
        }
        let steering = facts.events.iter().find(|event| event.event_id == receipt.event_id
            && event.kind.0 == "turn/steer-accepted" && event.correlation_id.as_ref() == receipt.target_operation_id.as_ref())
            .ok_or_else(|| AppError::Conflict("steering delivery has no exact canonical admission".into()))?;
        let payload = facts.event_payloads.get(steering.event_id.as_ref())
            .ok_or_else(|| AppError::Conflict("steering admission has no canonical payload".into()))?;
        if payload.get("target_operation_id").and_then(Value::as_str) != Some(receipt.target_operation_id.as_ref()) {
            return Err(AppError::Conflict("steering admission target differs from its receipt".into()));
        }
        // Recover original delivery data on every attempt, including a replay
        // after canonical commit but before the native queue acknowledgement.
        let input = payload.get("input").ok_or_else(|| AppError::Conflict("steering admission has no input".into()))?;
        let text = input.get("content").and_then(Value::as_str)
            .ok_or_else(|| AppError::Conflict("steering input has no text".into()))?.to_owned();
        let files = super::runtime_attachments::references(input)?;
        let inject_skills = super::runtime_attachments::selected_skills(input)?;
        let generation = self.canonical.store().native_execution_generation(session_id, &receipt.target_operation_id)
            .await.map_err(agent_session_store_error)?;
        if self.runtime_sessions.active_turn_generation(session_id.as_ref())
            .is_some_and(|current| current != generation) {
            return Err(AppError::Conflict("steering target differs from the exact Runtime generation".into()));
        }
        let runtime = self.runtime_sessions.get_runtime(session_id.as_ref())
            .ok_or_else(|| AppError::Conflict("steering requires the target Runtime".into()))?;
        let queued = runtime.steer_with_receipt(nomifun_ai_agent::RuntimeSteerDelivery {
            receipt_operation_id: receipt.event_id.as_ref().to_owned(), wire_turn_id: root, turn_generation: generation,
            text, files, inject_skills,
        }).await?;
        if queued { return Ok(()); }
        let current = self.canonical.store().read_turn_receipt(session_id, &receipt.target_operation_id)
            .await.map_err(agent_session_store_error)?;
        if current.terminal_event.is_some() && matches!(current.status,
            nomifun_agent_session::TurnReceiptStatus::Completed | nomifun_agent_session::TurnReceiptStatus::Failed
                | nomifun_agent_session::TurnReceiptStatus::Cancelled) {
            return Ok(());
        }
        Err(AppError::Conflict("the target Runtime closed steering before delivery".into()))
    }

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
    // Recompile on the saved Provider mounts using the same resolution path
    // as evolution admission. Exact persisted artifact checks still apply.
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
    let compiled = AgentPresetCompiler::compile_with_frozen_role_providers(
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
        &persisted,
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
    let message = format!("Nomi Plugin Tool Kernel admission failed: {error}");
    if matches!(error, nomifun_agent_kernel::KernelError::CapabilityProvenanceDrift { .. }
        | nomifun_agent_kernel::KernelError::SkillProvenanceDrift { .. }) {
        AppError::SessionConfigurationChanged(message)
    } else { AppError::Conflict(message) }
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
                if self.canonical.store().has_native_execution_owner(&request.agent_session_id, &operation)
                    .await.map_err(agent_session_store_error)? {
                    // Startup recovery, not Cron, owns leased native orphans.
                    // Do not race its takeover while a Runtime is attaching.
                    return Ok(nomifun_cron::CronTurnReconciliation::LiveExactOwnerWait);
                }
                self.settle_dispatch_failure(
                    &request.owner_id,
                    &request.agent_session_id,
                    &operation,
                    "Runtime owner was not recoverable after restart",
                    AgentSendError::from_engine_turn_failure(
                        "NATIVE_RECOVERY_RUNTIME_OWNER_NOT_RECOVERABLE: Runtime owner was not recoverable after restart"
                    ).into_stream_error(),
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
        let (mut projections, has_more, total) = self
            .canonical
            .store()
            .messages_before(&session_id, None, limit)
            .await
            .map_err(agent_session_store_error)?;
        super::history_thinking_display::hydrate_thinking_lifecycle(&self.pool, &session_id, &mut projections).await?;
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
        mut request: CreateConversationRequest,
        skill_names: Vec<String>,
    ) -> Result<ConversationResponse, AppError> {
        let companion_id = request
            .extra
            .get("companion_id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::BadRequest(
                "Companion Session requires companion_id".to_owned(),
            ))?
            .to_owned();
        let resolver = self.product_agent_resolver.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| AppError::Conflict("Companion Agent resolver is unavailable".into()))?;
        let target = ProductAgentTarget { target_kind: "companion".into(), target_id: companion_id.clone(), default_template_key: "companion.default".into() };
        let resolution = resolver.resolve(owner_id, &target, request.model.as_ref()).await?;
        merge_product_agent_resolution(&mut request.extra, &target, &resolution)?;
        let owner = UserId::from(owner_id.to_owned());
        let current = resolution.snapshot.canonical_binding.as_ref()
            .ok_or_else(|| AppError::Conflict("Companion Agent has no canonical binding".into()))?;
        let (_, _, snapshot) = resolver.control_plane.saved_binding_artifacts(&owner, current).await.map_err(control_plane_error_to_app)?;
        let mut selection = session_capability_selection::selection(&snapshot);
        selection.skill_names = skill_names;
        let mut binding = resolver.control_plane.resolve_agent_session_capabilities_binding(&owner, current, Some(&selection)).await.map_err(control_plane_error_to_app)?;
        let mcp = resolver.resource_bindings.resolve_mcp_for_saved_binding(&resolver.control_plane, &owner, &binding).await
            .map_err(|error| AppError::Conflict(format!("{}: {}", error.code(), error.message())))?;
        binding.typed_resource_bindings = current.typed_resource_bindings.iter().filter(|resource| resource.resource_kind != "mcp_server").cloned().chain(mcp).collect();
        let binding: AgentBindingValue = serde_json::to_value(binding).and_then(serde_json::from_value).map_err(|error| AppError::Conflict(error.to_string()))?;
        attach_session_metadata(&mut request.extra, &binding, None).map_err(|error| AppError::Conflict(error.message))?;
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
        let _operation_fence = self.session_operation_lock(session_id.as_ref()).write_owned().await;
        match self.canonical.get(&principal, &session_id).await {
            Ok(_) => self.runtime_sessions.terminate_and_wait_result(session_id.as_ref(), Some(nomifun_common::AgentKillReason::ConfigurationChanged)).await?,
            Err(AppError::NotFound(_)) => {},
            Err(error) => return Err(error),
        }
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
            PublicTurnDeliveryState::Accepted { message_id } => {
                let mut delivery = agent_execution_delivery_from_conversation(IdempotentMessageDelivery {
                    message_id,
                    replayed: true,
                    completed: false,
                    result_ok: None,
                    result_text: None,
                    result_error: None,
                    result_error_code: None,
                    result_error_retryable: None,
                });
                // Authorization and exact operation lookup above must precede
                // reading suspension. Runtime idleness is not a receipt.
                delivery.paused_reason = self.canonical.store().native_pause_state(
                    &AgentSessionId::from(conversation_id.to_owned()),
                    &Self::turn_operation_id(owner_id, conversation_id, operation_id),
                ).await.map_err(agent_session_store_error)?.map(|pause| pause.reason);
                Some(delivery)
            },
            PublicTurnDeliveryState::Completed(delivery) => {
                Some(agent_execution_delivery_from_conversation(delivery))
            }
        })
    }

    async fn read_turn_output(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: Option<&str>,
    ) -> Result<Option<nomifun_agent_execution::AgentExecutionTurnOutput>, AppError> {
        let session_id = AgentSessionId::from(conversation_id.to_owned());
        // Authorize before querying either operation identity or content.
        self.canonical.get(&PrincipalRef {
            principal_kind: "user".into(), principal_id: owner_id.into(),
        }, &session_id).await?;
        let operation_id = if let Some(operation) = operation_id {
            Self::turn_operation_id(owner_id, conversation_id, operation)
        } else {
            // Explicit adoption chooses the latest admitted Turn. A running
            // latest Turn cannot make an older closed Turn adoptable.
            let latest: Option<String> = sqlx::query_scalar(
                "SELECT operation_id FROM agent_turns WHERE session_id = ? ORDER BY rowid DESC LIMIT 1",
            ).bind(conversation_id).fetch_optional(&self.pool).await
                .map_err(|error| AppError::Internal(error.to_string()))?;
            let Some(latest) = latest else { return Ok(None); };
            OperationId::from(latest)
        };
        let receipt = self.canonical.store().read_turn_receipt(&session_id, &operation_id)
            .await.map_err(agent_session_store_error)?;
        if receipt.status == nomifun_agent_session::TurnReceiptStatus::NotFound { return Ok(None); }
        let facts = self.canonical.store().turn_output_facts(&session_id, &operation_id)
            .await.map_err(agent_session_store_error)?;
        let mut delivery = nomifun_agent_execution::canonical_turn_delivery(&facts, &receipt, true)?;
        if !delivery.completed {
            delivery.paused_reason = self.canonical.store().native_pause_state(&session_id, &operation_id)
                .await.map_err(agent_session_store_error)?.map(|pause| pause.reason);
        }
        let (output_files, integrity_ok) = if delivery.completed {
            let workspace = frozen_workspace_root(
                &self.managed_workspace_root, owner_id, &session_id, &facts.session.agent_binding,
            )?;
            // Hashing may read substantial output bytes; keep it off the
            // Runtime's async owner thread. Facts and paths remain read-only.
            let output_facts = facts;
            let output_receipt = receipt.clone();
            match tokio::task::spawn_blocking(move || {
                nomifun_agent_execution::canonical_turn_output_files(
                    &output_facts, &output_receipt, workspace.as_deref().map(std::path::Path::new),
                )
            }).await.map_err(|error| AppError::Internal(format!("verify canonical Turn output: {error}")))? {
                Ok(files) => (files, true),
                Err(error) => {
                    tracing::warn!(%error, %conversation_id, operation_id = %operation_id.as_ref(),
                        "canonical Turn output verification failed; automatic replay is disabled");
                    (Vec::new(), false)
                }
            }
        } else { (Vec::new(), true) };
        Ok(Some(nomifun_agent_execution::AgentExecutionTurnOutput {
            canonical_operation_id: operation_id.as_ref().to_owned(),
            terminal_event_id: receipt.terminal_event.as_ref().map(|event| event.event_id.as_ref().to_owned()),
            delivery, output_files, integrity_ok,
        }))
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

    async fn cancel_turn_for_execution(
        &self,
        owner_id: &str,
        conversation_id: &str,
        cancellation_operation_id: &str,
        target_operation_id: &str,
    ) -> Result<(), AppError> {
        self.cancel_exact_turn(
            owner_id, &AgentSessionId::from(conversation_id.to_owned()), cancellation_operation_id,
            &OperationId::from(target_operation_id.to_owned()), nomifun_common::AgentKillReason::UserCancelled,
        ).await.map(|_| ())
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
        self.queue_receipted_steering(&session_id, &receipt).await?;
        Ok(receipt.event_id.as_ref().to_owned())
    }

    async fn steer_turn_for_execution(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        target_operation_id: &str,
        request: SendMessageRequest,
    ) -> Result<String, AppError> {
        let session_id = AgentSessionId::from(conversation_id.to_owned());
        let receipt = self.canonical.steer_exact_turn(
            &PrincipalRef { principal_kind: "user".into(), principal_id: owner_id.into() },
            &session_id, operation_id, &OperationId::from(target_operation_id.to_owned()), canonical_turn_input(&request),
        ).await?;
        self.queue_receipted_steering(&session_id, &receipt).await?;
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
        paused_reason: None,
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
        plugin_delivery: None,
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
        if matches!(resource.resource_kind.as_str(), "workspace" | "process_session") {
            super::nomi_core_resource_bindings::freeze_resource_definition_id(resource)?;
        }
    }

    // A project directory is also the process cwd for Agents that do not expose
    // workspace file Actions. Keep a zero-operation workspace identity so the
    // Session can freeze that cwd without manufacturing any file authority.
    if !has_workspace_resource {
        let mut workspace = TypedResourceBindingDto {
            binding_id: String::new(),
            resource_kind: "workspace".to_owned(),
            resource_id,
            owner_id: owner_id.to_owned(),
            operations: BTreeSet::new(),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([(
                "workspace_root".to_owned(),
                canonical.to_owned(),
            )]),
        };
        super::nomi_core_resource_bindings::freeze_resource_definition_id(&mut workspace)?;
        binding.typed_resource_bindings.push(workspace);
    }
    binding.typed_resource_bindings.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));

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

fn canonical_pause_notice(status: &str, operation: Option<&str>, events: &[nomifun_agent_contracts::SessionEventRecord]) -> Option<Value> {
    if status != "paused" { return None; }
    let operation = operation?;
    let event = events.iter().rev().find(|event| event.kind.0 == "turn/paused" && event.correlation_id.as_ref() == operation)?;
    let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) = &event.payload else { return None; };
    let pause: nomifun_agent_session::NativePauseState = serde_json::from_value(payload.0.get("pause")?.clone()).ok()?;
    // Only bounded public reason codes belong in the conversation projection.
    // Arbitrary owner/provider prose remains in its original private record.
    let public_reason = pause.reason.len() <= 128 && (
        pause.reason.strip_prefix("EXECUTION_MODEL_").is_some_and(|code|
            serde_json::from_value::<nomifun_chat_model_broker::ChatModelErrorCode>(json!(code)).is_ok())
        || matches!(pause.reason.as_str(), "EXECUTION_USER_REQUESTED" | "EXECUTION_ATTACH_FAILED"
            | "EXECUTION_PREPARATION_BLOCKED" | "EXECUTION_SESSION_PAYLOAD_BUDGET"
            | "EXECUTION_MODEL_STREAM_ENDED_WITHOUT_TERMINAL" | "EXECUTION_MODEL_INVALID_EVENT")
    );
    let reason = if public_reason { pause.reason.as_str() } else { "EXECUTION_PAUSED" };
    Some(json!({"reason":reason,"cleanup_proven":pause.cleanup_proven,"paused_at_ms":pause.paused_at_ms}))
}

#[cfg(test)]
mod paused_projection_tests {
    use super::*;

    fn paused_event(operation: &str, reason: &str) -> nomifun_agent_contracts::SessionEventRecord {
        nomifun_agent_contracts::SessionEventRecord {
            agent_session_id:"pause-session".into(),seq:10,event_id:"pause-event".into(),
            producer_id:"runtime_supervisor".into(),idempotency_key:"pause-key".into(),
            kind:nomifun_agent_contracts::SessionEventKind("turn/paused".into()),
            kind_version:1,correlation_id:operation.into(),causation_event_id:None,
            payload:nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"pause":{
                "revision":1,"reason":reason,"checkpoint_revision":7,"checkpoint_digest":"a".repeat(64),
                "execution_fence":1,"cleanup_proven":true,"paused_at_ms":123,
            }}))),
        }
    }

    #[test]
    fn pause_notice_keeps_the_exact_active_turn_and_public_reason() {
        let events=vec![paused_event("active","EXECUTION_MODEL_PROVIDER_UNAVAILABLE"),paused_event("foreign","OWNER_REQUESTED")];
        let notice=canonical_pause_notice("paused",Some("active"),&events).unwrap();
        assert_eq!(notice,json!({"reason":"EXECUTION_MODEL_PROVIDER_UNAVAILABLE","cleanup_proven":true,"paused_at_ms":123}));
        assert!(canonical_pause_notice("running",Some("active"),&events).is_none());
        assert!(canonical_pause_notice("paused",Some("missing"),&events).is_none());
        assert!(canonical_pause_notice("paused",None,&events).is_none());
    }

    #[test]
    fn pause_notice_does_not_publish_arbitrary_prose_or_invent_cleanup() {
        for reason in ["sensitive provider or owner prose","FIXTURE_PROVIDER_SECRET","EXECUTION_MODEL_FIXTURE_SECRET"] {
            let mut event=paused_event("active",reason);
            let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload)=&mut event.payload else { unreachable!() };
            payload.0["pause"]["cleanup_proven"]=json!(false);
            let notice=canonical_pause_notice("paused",Some("active"),&[event]).unwrap();
            assert_eq!(notice,json!({"reason":"EXECUTION_PAUSED","cleanup_proven":false,"paused_at_ms":123}));
        }
    }
}

fn canonical_conversation_response(
    observed: SessionObservation,
    projected: super::agent_binding_projection::SavedAgentBindingProjection,
    workspace: Option<String>,
    created_at: i64,
    execution_link: Option<ConversationExecutionLinkProjection>,
    companion_id: Option<String>,
) -> Result<ConversationResponse, AppError> {
    let active_turn = canonical_active_turn_runtime(&observed.head, &observed.events)?;
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
    extra.insert("execution_phase".to_owned(),Value::String(head.status.clone()));
    extra.remove("execution_pause");
    if let Some(pause) = canonical_pause_notice(&head.status,head.active_turn_id.as_deref(),&events) {
        extra.insert("execution_pause".to_owned(),pause);
    }
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
        // The compatibility aggregate remains nonterminal while canonical
        // ownership is paused or awaiting reconciliation. Otherwise desktop
        // queue reconciliation interprets Finished as permission to dispatch.
        "running" | "paused" | "reconciliation" => ConversationStatus::Running,
        "failed" | "open_failed" => ConversationStatus::Finished,
        "opening" => ConversationStatus::Pending,
        _ => ConversationStatus::Finished,
    };
    let runtime = Some(ConversationRuntimeSummary {
        state: if head.status == "running" {
            ConversationRuntimeStateKind::Running
        } else {
            ConversationRuntimeStateKind::Idle
        },
        can_send_message: head.active_turn_id.is_none(),
        has_runtime: head.status == "running",
        runtime_status: Some(status),
        is_processing: head.status == "running",
        active_turn_id: active_turn.as_ref().map(|(message_id, _)| message_id.clone()),
        processing_started_at: active_turn.as_ref().map(|(_, started_at)| *started_at),
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
        session_purpose: session.metadata.purpose,
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

fn canonical_active_turn_runtime(
    head: &nomifun_agent_session::SessionHeadProjection,
    events: &[nomifun_agent_contracts::SessionEventRecord],
) -> Result<Option<(String, i64)>, AppError> {
    let Some(operation) = head.active_turn_id.as_deref() else {
        return Ok(None);
    };
    // The caller hydrates this exact owning Turn's receipt when its start is
    // outside the bounded event page. A Session cursor is ordering, never time.
    let started = events
        .iter()
        .rev()
        .find(|event| {
            event.agent_session_id == head.session_id
                && event.seq <= head.last_seq
                && event.kind.0 == "turn/started"
                && event.correlation_id.as_ref() == operation
        })
        .ok_or_else(|| AppError::Conflict("active canonical Turn has no started event".into()))?;
    let nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload) = &started.payload else {
        return Err(AppError::Conflict(
            "canonical Turn start requires its inline admission".into(),
        ));
    };
    let source = payload
        .0
        .get("source_message_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .filter(|uuid| uuid.get_version_num() == 7)
        .ok_or_else(|| AppError::Conflict("canonical Turn start has no source UUIDv7".into()))?;
    let started_uuid = Uuid::parse_str(started.event_id.as_ref())
        .ok()
        .filter(|uuid| uuid.get_version_num() == 7)
        .ok_or_else(|| AppError::Conflict("canonical Turn start event is not UUIDv7".into()))?;
    // Match the owning Turn's history projection: the first six UUIDv7 bytes
    // contain its committed wall-clock start, including after re-entry.
    let bytes = started_uuid.as_bytes();
    let started_at =
        u64::from_be_bytes([0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]]) as i64;
    Ok(Some((source.to_string(), started_at)))
}

#[cfg(test)]
mod active_turn_runtime_tests {
    use super::*;
    use nomifun_agent_contracts::{SessionEventKind, SessionEventPayloadRef, SessionEventRecord};
    use nomifun_agent_session::SessionHeadProjection;

    fn fixture() -> (SessionHeadProjection, SessionEventRecord) {
        let session = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let head = SessionHeadProjection {
            session_id: session.clone(),
            status: "running".into(),
            active_turn_id: Some("turn:current".into()),
            active_set_generation: 1,
            last_seq: 901,
            unread_count: 0,
        };
        let started = SessionEventRecord {
            agent_session_id: session,
            seq: 801,
            event_id: "01a11692-0c44-7db1-a359-09a4c9bf125e".into(),
            producer_id: "session_owner".into(),
            idempotency_key: "started".into(),
            kind: SessionEventKind("turn/started".into()),
            kind_version: 1,
            correlation_id: "turn:current".into(),
            causation_event_id: None,
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "source_message_id": "0190f5fe-7c00-7a00-8000-000000000003"
            }))),
        };
        (head, started)
    }

    #[test]
    fn active_turn_runtime_uses_its_committed_uuid_clock_after_a_bounded_page() {
        let (head, started) = fixture();
        // The readonly projection appends the exact receipt's start even when
        // it is outside the first 500 events. Later foreign starts cannot win.
        let mut foreign = started.clone();
        foreign.seq = 900;
        foreign.correlation_id = "turn:foreign".into();
        let runtime = canonical_active_turn_runtime(&head, &[started, foreign])
            .unwrap()
            .unwrap();
        assert_eq!(runtime.0, "0190f5fe-7c00-7a00-8000-000000000003");
        assert_eq!(runtime.1, 1_791_380_032_580);
        assert_ne!(runtime.1, 1_700_000_000_000 + 801);
    }

    #[test]
    fn active_turn_runtime_requires_the_owning_started_fact_and_uuid_clock() {
        let (mut head, mut started) = fixture();
        assert!(canonical_active_turn_runtime(&head, &[]).is_err());
        started.event_id = "cursor-is-not-time".into();
        assert!(canonical_active_turn_runtime(&head, &[started]).is_err());
        head.active_turn_id = None;
        assert_eq!(canonical_active_turn_runtime(&head, &[]).unwrap(), None);
    }

    #[tokio::test]
    async fn active_turn_runtime_and_cold_history_share_the_owning_turn_clock() {
        let (_journal, pool) = super::super::engine_journal::test_fixture().await;
        let store = nomifun_agent_session::AgentSessionStore::from_pool(pool)
            .await
            .unwrap();
        let session = AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let before = store.current_cursor(&session).await.unwrap();
        let mut observed = store.observe(&session, None, 1).await.unwrap();
        assert!(canonical_active_turn_runtime(&observed.head, &observed.events).is_err());
        let operation = OperationId::from(observed.head.active_turn_id.clone().unwrap());
        observed.events.push(
            store
                .read_turn_receipt(&session, &operation)
                .await
                .unwrap()
                .started_event
                .unwrap(),
        );
        let (source, started_at) = canonical_active_turn_runtime(&observed.head, &observed.events)
            .unwrap()
            .unwrap();
        let (history, _, _) = store
            .message_history_before(&session, None, 50)
            .await
            .unwrap();
        let summary = history
            .iter()
            .find(|row| row.presentation_intent == "turn_summary")
            .unwrap();
        assert_eq!(summary.projection["source_message_id"], source);
        assert_eq!(summary.projection["started_at_ms"], started_at);
        assert!(summary.projection["finished_at_ms"].is_null());
        assert_eq!(store.current_cursor(&session).await.unwrap(), before);
    }
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
            None => return Err(AppError::Conflict("canonical AgentSession has no frozen Snapshot projection".to_owned())),
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

    #[tokio::test]
    async fn execution_admission_keeps_invocation_authority_across_metadata_revisions() {
        use tower::ServiceExt;
        const TRUST: &str = "execution-authority-regression";
        async fn post(router: &axum::Router, path: &str, body: serde_json::Value) -> serde_json::Value {
            let response = router.clone().oneshot(axum::http::Request::builder().method("POST").uri(path)
                .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string())).unwrap()).await.unwrap();
            let status = response.status();
            let body: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(
                response.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
            assert!(status.is_success(), "{path}: {status}: {body}");
            body["data"].clone()
        }
        let root = tempfile::tempdir().unwrap();
        let config = crate::AppConfig {
            data_dir: root.path().join("data"), work_dir: root.path().join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken, local_trust_secret: Some(TRUST.into()),
            ..Default::default()
        };
        std::fs::create_dir_all(&config.data_dir).unwrap();
        let database = nomifun_db::init_database(&config.database_path()).await.unwrap();
        let services = crate::services::AppServices::from_config(database, &config).await.unwrap();
        let (states, _components) = super::super::state::try_build_module_states(&services).await.unwrap();
        let owner = states.nomi_core_agent_api.session_owner.clone();
        let router = super::super::create_router_with_states(&services, states);
        let provider = post(&router, "/api/providers", json!({
            "platform":"custom","name":"authority fixture","base_url":"http://127.0.0.1:9/v1",
            "auth_scheme":"bearer","credentials":{"api_keys":["test-only"]},"enabled":true,
            "initial_model":{"model":"authority-fixture","enabled":true,"capabilities":[{
                "task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default"
            }]}
        })).await;
        let model = json!({"provider_id":provider["provider_id"],"model":"authority-fixture"});
        let preset = post(&router, "/api/agent-presets/from-template/chat.minimal",
            json!({"reuse_existing":false,"display_name":"authority fixture","model":model})).await;
        let input = json!({"preset_id":preset["preset"]["preset_id"],"model":model});
        let child = post(&router, "/api/agent-sessions", input).await;
        let session = child["agent_session_id"].as_str().unwrap();
        let pool = services.database.pool();
        // Seed only Execution's business rows; the Session and its frozen
        // binding above come from the real owner. No model task is started.
        let execution_id = uuid::Uuid::now_v7().to_string();
        let step = uuid::Uuid::now_v7().to_string();
        let attempt = uuid::Uuid::now_v7().to_string();
        let now = super::now_ms();
        nomifun_db::sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,plan_gate,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,lease_owner,lease_expires_at,created_at,updated_at) VALUES(?,?,'authority fixture','running','automatic','fixed','automatic','automatic','{}','authority-fixture',?,?,?)")
            .bind(&execution_id).bind(services.authoritative_user_id.as_ref()).bind(now + 60_000)
            .bind(now).bind(now).execute(pool).await.unwrap();
        nomifun_db::sqlx::query("INSERT INTO agent_execution_steps(step_id,execution_id,title,spec,kind,status,version,introduced_in_revision,created_at,updated_at) VALUES(?,?,'fixture','Answer in chat','agent','running',3,1,?,?)")
            .bind(&step).bind(&execution_id).bind(now).bind(now).execute(pool).await.unwrap();
        nomifun_db::sqlx::query("INSERT INTO agent_execution_attempts(attempt_id,execution_id,step_id,attempt_no,status,trigger_reason,effective_config,version,created_at,updated_at) VALUES(?,?,?,0,'running','initial','{}',5,?,?)")
            .bind(&attempt).bind(&execution_id).bind(&step).bind(now).bind(now).execute(pool).await.unwrap();
        nomifun_db::sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) VALUES(?,?,'attempt',?,?,1,?,?)")
            .bind(session).bind(&execution_id).bind(&step).bind(&attempt).bind(now).bind(now).execute(pool).await.unwrap();
        let mut authority = nomifun_db::AgentExecutionTurnAuthority {
            execution_id, step_id: step.clone(), attempt_id: attempt,
            expected_step_version:3, expected_attempt_version:2, lease_owner:"authority-fixture".into(),
        };
        let user = services.authoritative_user_id.as_ref();
        owner.validate_agent_execution_turn_authority(user, session, &authority).await.unwrap();
        authority.expected_attempt_version = 6;
        assert!(owner.validate_agent_execution_turn_authority(user, session, &authority).await.is_err(),
            "metadata revision cannot move backwards relative to captured authority");
        authority.expected_attempt_version = 2;
        nomifun_db::sqlx::query("UPDATE agent_execution_steps SET version=4 WHERE step_id=?")
            .bind(&step).execute(pool).await.unwrap();
        assert!(owner.validate_agent_execution_turn_authority(user, session, &authority).await.is_err(),
            "a prior invocation cannot acquire a successor Step generation");
        services.shutdown_nomi_core_host().await.unwrap();
        services.database.close().await;
    }

    #[test]
    fn kernel_configuration_refusals_keep_typed_local_attribution() {
        for refusal in [
            nomifun_agent_kernel::KernelError::CapabilityProvenanceDrift {
                capability_id: nomifun_agent_contracts::CapabilityId::from("workspace.process"),
                reason: "Revision contribution lock does not match; nested provider error 503".into(),
            },
            nomifun_agent_kernel::KernelError::SkillProvenanceDrift {
                skill_id: nomifun_agent_contracts::SkillId::from("skill.one"), reason:"changed Skill content".into(),
            },
        ] {
            let error = super::kernel_error_to_app(refusal);
            assert!(matches!(&error, nomifun_common::AppError::SessionConfigurationChanged(_)));
            assert_eq!(error.status_code(), StatusCode::CONFLICT);
            assert_eq!(error.error_code(), "NOMIFUN_SESSION_CONFIGURATION_CHANGED");
            let classified = nomifun_ai_agent::AgentSendError::from_app_error_ref(&error).into_stream_error();
            assert_eq!(classified.code, Some(nomifun_api_types::AgentErrorCode::NomifunSessionConfigurationChanged));
            assert_eq!(classified.ownership, Some(nomifun_api_types::AgentErrorOwnership::Nomifun));
            assert_eq!(classified.retryable, Some(false));
        }
        let unrelated = super::kernel_error_to_app(nomifun_agent_kernel::KernelError::CapabilityNotActive {
            capability_id: nomifun_agent_contracts::CapabilityId::from("workspace.process"),
        });
        assert!(matches!(unrelated, nomifun_common::AppError::Conflict(_)));
    }

    #[test]
    fn configuration_error_rehydrates_with_original_ownership_and_recovery_advice() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let root = "0190f5fe-7c00-7a00-8abc-012345678911";
        let classified = nomifun_ai_agent::AgentSendError::session_configuration_changed(
            "Original local provenance diagnostic").into_stream_error();
        let error = serde_json::to_value(classified).unwrap();
        let message = canonical_message_response(&session_id, 1_000, MessageProjection {
            session_id:session_id.clone(),projection_id:"configuration-error".into(),first_seq:2,last_seq:3,
            presentation_intent:"turn_summary".into(),message_type:Some("agent_status".into()),message_status:Some("finish".into()),
            projection:json!({"correlation_id":"0190f5fe-7c00-7a00-8abc-012345678912","state":"failed",
                "source_message_id":root,"started_at_ms":4_000_000,"finished_at_ms":4_002_000,"error":error}),
            semantic_digest:"digest".into(),
        }).unwrap().unwrap();
        assert_eq!(message.r#type, MessageType::Tips);
        assert_eq!(message.content["type"], "error");
        assert_eq!(message.content["turn_id"], root);
        assert_eq!(message.content["error"], error);
        assert_eq!(message.content["error"]["retryable"], false);
        assert_eq!(message.content["error"]["resolution"]["kind"], "start_new_session");
    }

    #[test]
    fn error_context_keeps_the_same_public_fields_in_stream_and_history() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let root = "0190f5fe-7c00-7a00-8abc-012345678911";
        let mut error = nomifun_ai_agent::AgentSendError::from_engine_turn_failure(
            "model step limit of 32 exceeded").into_stream_error();
        let captured = super::TurnErrorContext {
            agent_label: Some("Original Agent".into()),
            agent_template_key: Some("chat.minimal".into()),
            model_name: Some("original-model".into()),
            workspace_path: Some("/tmp/original-workspace".into()),
        };
        captured.apply(&mut error);
        let expected = serde_json::to_value(&error).unwrap();
        let emitted_after = nomifun_common::now_ms();
        let stream = NomiCoreSessionOwner::canonical_stream_wire_event(
            &session_id, root, "0190f5fe-7c00-7a00-8abc-012345678910",
            &AgentStreamEvent::Error(error),
        ).unwrap();
        let history = canonical_message_response(&session_id, 1_000, MessageProjection {
            session_id: session_id.clone(), projection_id: "context-error".into(), first_seq: 2, last_seq: 3,
            presentation_intent: "turn_summary".into(), message_type: Some("tips".into()), message_status: Some("error".into()),
            projection: json!({"correlation_id":"0190f5fe-7c00-7a00-8abc-012345678912","state":"failed",
                "source_message_id":root,"started_at_ms":4_000_000,"finished_at_ms":4_002_000,"error":expected}),
            semantic_digest: "digest".into(),
        }).unwrap().unwrap();
        assert_eq!(stream.data["data"], expected);
        assert!(stream.data["created_at"].as_i64().is_some_and(|timestamp|
            timestamp >= emitted_after && timestamp <= nomifun_common::now_ms()));
        assert_eq!(history.content["error"], expected);
        assert_eq!(expected["agentLabel"], "Original Agent");
        assert_eq!(expected["taskIncompleteReason"], "step_limit");
        assert!(stream.data["data"].get("workspace_path").is_none());

        let mut path_error = nomifun_api_types::AgentStreamErrorData::legacy("invalid path", None);
        path_error.workspace_path = Some("/tmp/rejected-path".into());
        captured.apply(&mut path_error);
        assert_eq!(path_error.workspace_path.as_deref(), Some("/tmp/rejected-path"));
    }

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
    fn canonical_stream_steps_use_the_same_identity_as_durable_public_messages() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let root = "0190f5fe-7c00-7a00-8abc-012345678911";
        let fallback = NomiCoreSessionOwner::canonical_assistant_stream_message_id(root).unwrap();
        for step in [1_u16, 2, 300] {
            let event: AgentStreamEvent = serde_json::from_value(json!({
                "type": "content", "data": { "content": "progress", "step": step },
            })).unwrap();
            let wire = NomiCoreSessionOwner::canonical_stream_wire_event(
                &session_id, root, &fallback, &event,
            ).unwrap();
            assert_eq!(wire.data["msg_id"],
                super::super::engine_journal::canonical_assistant_step_message_id(root, step).unwrap());
            assert_eq!(wire.data["turn_id"], root);
            if step > 1 { assert_ne!(wire.data["msg_id"], fallback); }
        }
    }

    #[test]
    fn canonical_thinking_steps_keep_completion_scoped_to_the_same_durable_row() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let root = "0190f5fe-7c00-7a00-8abc-012345678911";
        let fallback = NomiCoreSessionOwner::canonical_assistant_stream_message_id(root).unwrap();
        let mut identities = std::collections::BTreeSet::new();
        for step in [1_u16, 2, 300] {
            let identity = super::super::engine_journal::canonical_thinking_step_message_id(root, step).unwrap();
            assert!(identities.insert(identity.clone()));
            assert_ne!(identity, root);
            assert_ne!(identity, super::super::engine_journal::canonical_assistant_step_message_id(root, step).unwrap());
            assert_eq!(uuid::Uuid::parse_str(&identity).unwrap().get_version_num(), 7);
            for status in ["thinking", "done"] {
                let event: AgentStreamEvent = serde_json::from_value(json!({
                    "type": "thinking", "data": { "content": "", "step": step, "status": status },
                })).unwrap();
                let wire = NomiCoreSessionOwner::canonical_stream_wire_event(
                    &session_id, root, &fallback, &event,
                ).unwrap();
                assert_eq!(wire.data["msg_id"], identity);
                assert_eq!(wire.data["turn_id"], root);
                assert_eq!(wire.data["data"]["status"], status);
            }
        }
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
    fn history_projects_confirmed_process_cancellation_without_hiding_other_errors() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let message_id = "0190f5fe-7c00-7a00-8abc-012345678912";
        for (capability, reaped, is_error, expected) in [
            ("workspace.process", true, true, "canceled"),
            ("workspace.process", true, false, "canceled"),
            ("workspace.process", false, true, "error"),
            ("workspace.files", true, true, "error"),
        ] {
            let output = json!({"state":"cancelled","cleanup":{"reaped":reaped},"output":{"text":"STARTED"}}).to_string();
            let message = canonical_message_response_with_observation(&session_id, 1000,
                MessageProjection {
                    session_id:session_id.clone(), projection_id:format!("tool:{message_id}"),
                    first_seq:4,last_seq:6,presentation_intent:"tool".into(),message_type:None,message_status:None,
                    projection:json!({"correlation_id":message_id,"state":"recorded",
                        "tool_summary":{"call_id":"call-1","name":"exec_command","capability_id":capability}}),
                    semantic_digest:"digest".into(),
                }, Some(&HistoricalToolObservation { turn_id:None,args:None,output:Some(output.clone()),is_error:Some(is_error) })
            ).unwrap().unwrap();
            assert_eq!(message.content["status"], expected);
            assert_eq!(message.content["output"], output);
            assert_eq!(message.status, Some(if expected == "canceled" { MessageStatus::Finish } else { MessageStatus::Error }));
        }
    }

    #[test]
    fn history_projects_public_step_text_with_its_turn_identity() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let root = "0190f5fe-7c00-7a00-8abc-012345678911";
        let message_id = "0190f5fe-7c00-7a00-8abc-012345678912";
        let message = canonical_message_response(
            &session_id,
            1_000,
            MessageProjection {
                session_id: session_id.clone(),
                projection_id: format!("message:{message_id}"),
                first_seq: 3,
                last_seq: 8,
                presentation_intent: "message".to_owned(),
                message_type: None,
                message_status: None,
                projection: json!({
                    "correlation_id": message_id,
                    "content": "I found the cause.",
                    "turn_id": root,
                    "state": "completed"
                }),
                semantic_digest: "digest".to_owned(),
            },
        ).unwrap().unwrap();

        assert_eq!(message.r#type, MessageType::Text);
        assert_eq!(message.content["content"], "I found the cause.");
        assert_eq!(message.content["turn_id"], root);
    }

    #[test]
    fn history_preserves_each_accepted_steer_identity_without_reopening_its_turn() {
        let session_id = AgentSessionId::from(SESSION_ID);
        let operation = format!("turn:user:{SESSION_ID}:root:operation");
        let ids = ["0190f5fe-7c00-7a00-8abc-012345678921", "0190f5fe-7c00-7a00-8abc-012345678922"];
        let mut messages = Vec::new();
        for (index, id) in ids.iter().enumerate() {
            let message = canonical_message_response(&session_id, 1_000, MessageProjection {
                session_id: session_id.clone(), projection_id: format!("message:{id}"),
                first_seq: 10 + index as u64, last_seq: 10 + index as u64,
                presentation_intent: "message".into(), message_type: None, message_status: None,
                projection: json!({"projection_id":format!("message:{id}"), "correlation_id":operation,
                    "presentation_intent":"message", "content":"改为结果/纠正结果.txt", "state":"accepted"}),
                semantic_digest: "digest".into(),
            }).unwrap().expect("an admitted steering message must survive cold history");
            assert_eq!(message.message_id, *id);
            assert_eq!(message.msg_id.as_deref(), Some(*id));
            assert_eq!(message.position, Some(MessagePosition::Right));
            assert_eq!(message.status, Some(MessageStatus::Finish));
            assert_eq!(message.content["content"], "改为结果/纠正结果.txt");
            assert!(message.content["turn_id"].is_null());
            messages.push(message);
        }
        assert_ne!(messages[0].message_id, messages[1].message_id);
        assert!(messages[0].created_at < messages[1].created_at);
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
            1_791_380_032_580,
        );

        assert_eq!(wire.name, "turn.started");
        assert_eq!(wire.data["turn_id"], root_message_id);
        assert_eq!(wire.data["status"], "running");
        assert_eq!(wire.data["runtime"]["state"], "running");
        assert_eq!(wire.data["runtime"]["is_processing"], true);
        assert_eq!(wire.data["runtime"]["active_turn_id"], root_message_id);
        assert_eq!(wire.data["runtime"]["processing_started_at"], 1_791_380_032_580_i64);
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
    fn selected_workspace_identity_includes_final_grants_and_is_stable_on_refreeze() {
        let directory = tempfile::tempdir().unwrap();
        let mut source: AgentBindingValueDto = serde_json::from_value(
            serde_json::to_value(frozen_binding(&std::env::temp_dir().to_string_lossy(), OWNER_ID)).unwrap(),
        ).unwrap();
        freeze_selected_workspace(&mut source, OWNER_ID, &directory.path().to_string_lossy(), WorkspaceDirectoryCheck::Create).unwrap();
        let mut target = source.clone();
        target.typed_resource_bindings[0].operations.remove("write");
        freeze_selected_workspace(&mut target, OWNER_ID, &directory.path().to_string_lossy(), WorkspaceDirectoryCheck::Runtime).unwrap();
        assert_eq!(source.typed_resource_bindings[0].resource_id, target.typed_resource_bindings[0].resource_id);
        assert_eq!(source.typed_resource_bindings[0].typed_parameters, target.typed_resource_bindings[0].typed_parameters);
        assert_ne!(source.typed_resource_bindings[0].binding_id, target.typed_resource_bindings[0].binding_id,
            "freezing the same physical root must not overwrite a narrower Agent's definition identity");
        let expected = target.typed_resource_bindings.clone();
        freeze_selected_workspace(&mut target, OWNER_ID, &directory.path().to_string_lossy(), WorkspaceDirectoryCheck::Runtime).unwrap();
        assert_eq!(target.typed_resource_bindings, expected, "identical final definitions reuse their identities");
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
        assert_eq!(std::fs::canonicalize(workspace.parent().unwrap()).unwrap(),
            std::fs::canonicalize(&managed_root).unwrap());
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
            session_purpose: Default::default(),
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
                "official_template_key": "chat.minimal",
                "custom_agent_id": "0190f5fe-7c00-7a00-8abc-012345678999",
            }),
        };
        let context = super::TurnErrorContext::from_response(&response, Some("/tmp/admitted-workspace"));
        assert_eq!(context.agent_label.as_deref(), Some("Frozen Agent"));
        assert_eq!(context.agent_template_key.as_deref(), Some("chat.minimal"));
        assert_eq!(context.model_name.as_deref(), Some("step-3.7-flash"));
        assert_eq!(context.workspace_path.as_deref(), Some("/tmp/admitted-workspace"));
        let mut changed_selection = response.clone();
        changed_selection.model.as_mut().unwrap().model = "later-renderer-selection".into();
        changed_selection.extra["agent_name"] = json!("Later Agent");
        let captured = super::TurnErrorContext::from_response(&changed_selection, None);
        assert_eq!(captured.agent_label, context.agent_label);
        assert_eq!(captured.model_name, context.model_name);
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
            ".delete_agent_session(owner_id, agent_session_id)",
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
            session_purpose: Default::default(),
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
    pub(crate) session_owner: Arc<NomiCoreSessionOwner>,
    pub(crate) control_plane: Arc<AgentControlPlane>,
    pub(crate) remote_repository: Arc<dyn IRemoteBindingRepository>,
    pub(crate) remote_runtime: super::remote_runtime::NomiCoreRemoteRuntimeCoordinator,
    pub(crate) resource_bindings:
        super::nomi_core_resource_bindings::NomiCoreResourceBindingResolverRegistry,
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
    #[cfg(feature = "browser-use")]
    browser_profile_store: nomifun_browser_platform::runtime::BrowserProfileStore,
    #[cfg(feature = "browser-use")]
    pub(crate) browser_user_close: Arc<dyn crate::browser_workspace_provider::BrowserUserClosePort>,
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
        wave4_owners: Arc<super::nomi_core_wave4::NomiCoreWave4Owners>,
        wave5_owner: Arc<super::agent_wave5_host::NomiCoreWave5Host>,
        product_agent_resolver: Arc<NomiCoreProductAgentResolver>,
            ssh_pool: nomifun_ssh::SshConnectionPool,
        #[cfg(feature = "browser-use")]
        browser_resources: Option<
            Arc<nomifun_browser_platform::workspace::BrowserResourceService>,
        >,
        #[cfg(feature = "browser-use")]
        attached_chrome: Option<Arc<crate::AttachedChromeProviderService>>,
        #[cfg(feature = "browser-use")]
        browser_profile_store: nomifun_browser_platform::runtime::BrowserProfileStore,
        #[cfg(feature = "browser-use")]
        browser_user_close: Arc<dyn crate::browser_workspace_provider::BrowserUserClosePort>,
    ) -> Self {
        Self {
            authoritative_user_id,
            product_agent_resolver,
            session_owner,
            control_plane,
            remote_repository,
            remote_runtime,
            resource_bindings,
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
            #[cfg(feature = "browser-use")]
            browser_profile_store,
            #[cfg(feature = "browser-use")]
            browser_user_close,
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
            if let Some(resources) = &self.browser_resources {
                resources.delete_agent_session(owner_id, agent_session_id).await
                    .map_err(|error| NomiCoreApiError::new(StatusCode::CONFLICT,
                        "AGENT_SESSION_BROWSER_CLEANUP_FAILED",
                        format!("Browser cleanup failed before AgentSession deletion: {error}")))?;
            } else {
                // Directory-host cleanup can run offline. A system-managed
                // WebKit profile requires its native removal port and fails
                // closed here; deleting a former directory is not sufficient.
                let store = self.browser_profile_store.clone();
                let key = nomifun_browser_platform::workspace::managed_workspace_key(owner_id, agent_session_id)
                    .map_err(|error| AppError::Conflict(error.to_string()))?;
                tokio::task::spawn_blocking(move || store.delete_persistent_profile(&key)).await
                    .map_err(|error| AppError::Conflict(error.to_string()))?
                    .map_err(|error| AppError::Conflict(error.to_string()))?;
            }
        }
        #[cfg(not(feature = "browser-use"))]
        // A human can own a managed browser without any Agent Browser grant.
        // On macOS only the native data-store port can prove cleanup; a host
        // lacking that port must retain the canonical identity for retry.
        if cfg!(target_os = "macos") || deleting_session
            .agent_binding
            .typed_resource_bindings
            .iter()
            .any(|resource| resource.resource_kind.as_ref() == "browser")
        {
            return Err(NomiCoreApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "AGENT_SESSION_BROWSER_CLEANUP_UNAVAILABLE",
                "Browser cleanup is unavailable in this host build; finish Session deletion in NomiFun Desktop",
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
                .await
                .map_err(|error| {
                    NomiCoreApiError::new(
                        StatusCode::CONFLICT,
                        "AGENT_SESSION_WORKSPACE_CLEANUP_FAILED",
                        format!(
                            "Managed Workspace cleanup failed after AgentSession deletion was fenced: {error}"
                        ),
                    )
                })?;
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
            .await
            .map_err(|error| {
                NomiCoreApiError::new(
                    StatusCode::CONFLICT,
                    "AGENT_SESSION_WORKSPACE_CLEANUP_FAILED",
                    format!(
                        "Managed Workspace cleanup failed after AgentSession deletion was fenced: {error}"
                    ),
                )
            })?;
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
            "/api/agent-sessions/{agent_session_id}/execution",
            get(get_nomi_core_agent_session_execution),
        )
        .route("/api/agent-sessions/{agent_session_id}/execution/pause", post(native_execution_control::pause))
        .route("/api/agent-sessions/{agent_session_id}/execution/resume", post(native_execution_control::resume))
        .route("/api/agent-sessions/{agent_session_id}/execution/effects", get(native_execution_control::effects))
        .route("/api/agent-sessions/{agent_session_id}/execution/reconcile", post(native_execution_control::reconcile))
        .route("/api/agent-sessions/{agent_session_id}/task-plan", get(conversation_task_plan::get))
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
            "/api/agent-sessions/{agent_session_id}/capability-selection",
            get(session_capability_selection::get_selection).put(session_capability_selection::update_selection),
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
    pub(super) fn new(
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
    include_product_sessions: bool,
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
    let store = state.session_owner.canonical().store();
    let principal = authenticated_principal(&owner);
    let page = if query.include_product_sessions {
        store.list_live_sessions(&principal, query.cursor.as_deref(), query.limit).await
    } else {
        store.list_live_sessions_for_purpose(&principal, query.cursor.as_deref(), query.limit,
            nomifun_agent_contracts::SessionPurpose::Conversation).await
    }.map_err(agent_session_store_error)?;
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
    let mut message_sources = HashMap::new();
    for projection in &projections {
        if projection.presentation_intent != "message" { continue; }
        let document = &projection.projection;
        let Some(id) = document.get("correlation_id").and_then(Value::as_str) else { continue; };
        let source = if document.get("state").and_then(Value::as_str) == Some("accepted") {
            id.to_owned()
        } else if let Some(source) = document.get("turn_id").and_then(Value::as_str) {
            source.to_owned()
        } else {
            // Older empty assistant projections have no content part carrying
            // turn_id. The first assistant ID uses the reversible XOR-1 mapping.
            super::engine_journal::canonical_assistant_message_id(id)?
        };
        message_sources.insert(id.to_owned(), source);
    }
    let sources = message_sources.values().cloned().collect::<BTreeSet<_>>()
        .into_iter().collect::<Vec<_>>();
    let summaries = state.session_owner.canonical().store()
        .turn_history_for_sources(session_id, &sources).await
        .map_err(agent_session_store_error)?;
    let mut outcomes = HashMap::new();
    for summary in summaries {
        let source = summary.projection["source_message_id"].as_str().unwrap().to_owned();
        let turn_state = summary.projection["state"].as_str().unwrap().to_owned();
        let error_message = if matches!(turn_state.as_str(), "failed" | "interrupted") {
            canonical_message_response(session_id, created_at, summary)?
                .and_then(|message| message.content.get("content").and_then(Value::as_str).map(str::to_owned))
        } else { None };
        outcomes.insert(source, (turn_state, error_message));
    }
    let mut history = Vec::new();
    let mut last_assistants = HashMap::new();
    for projection in projections {
        let Some(message) = canonical_message_response(session_id, created_at, projection)? else {
            continue;
        };
        if message.r#type != MessageType::Text || message.hidden {
            continue;
        }
        let source = message_sources.get(&message.message_id);
        let outcome = source.and_then(|source| outcomes.get(source));
        // Only terminal Turns belong to durable Canvas history. In particular,
        // assistant-complete can be journaled before the Turn itself settles.
        if outcome.is_some_and(|(state, _)| state == "running")
            || (outcome.is_none() && message.status != Some(MessageStatus::Finish)) {
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
        if matches!(role, CreativeStudioAgentHistoryRole::Assistant) {
            if let Some(source) = source {
                last_assistants.insert(source.clone(), history.len());
            }
        }
        history.push(CreativeStudioAgentHistoryMessage {
            id: message.message_id,
            role,
            status: CreativeStudioAgentHistoryStatus::Complete,
            text,
            activity_label: None,
            error_message: None,
        });
    }
    for (source, index) in last_assistants {
        let Some((state, error)) = outcomes.get(&source) else { continue; };
        match state.as_str() {
            "failed" | "interrupted" => {
                history[index].status = CreativeStudioAgentHistoryStatus::Failed;
                history[index].error_message = error.clone();
            }
            "cancelled" => history[index].status = CreativeStudioAgentHistoryStatus::Stopped,
            _ => {}
        }
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
        .filter(|message| message.role == CreativeStudioAgentHistoryRole::Assistant
            && message.status == CreativeStudioAgentHistoryStatus::Complete)
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
        .resolve_agent_session_binding_with_capabilities(&owner.0, &request.preset_id, request.model.as_ref(), request.session_capabilities.as_ref())
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
    let editor = state
        .control_plane
        .editor(
            &owner.0,
            &binding.preset_revision_ref.preset_id,
            Some(binding.preset_revision_ref.revision),
        )
        .await?;
    let document = &editor.revision.as_ref().ok_or_else(|| NomiCoreApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY, "AGENT_PRESET_REQUIRED", "A stable Agent revision is required",
    ))?.document;
    for required in &request.required_modules {
        let selection = document.enabled_capabilities.iter().find(|selection| &selection.capability.id == required);
        if selection.is_none()
            || (required == nomifun_plugin_development::MODULE_ID
                && nomifun_plugin_development::CREATE_ACTIONS.iter().any(|action|
                    !selection.expect("checked selection").action_allowlist.iter().any(|granted| granted == action)))
        {
            return Err(NomiCoreApiError::new(StatusCode::UNPROCESSABLE_ENTITY,
                "AGENT_LAUNCH_MODULE_REQUIRED", format!("The saved Agent cannot execute required module {required}")));
        }
    }
    let agent_name = editor.preset.display_name;
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
    state.session_owner.user_events.send_to_user(
        owner.as_ref(),
        WebSocketMessage::new(
            "conversation.listChanged",
            json!({
                "conversation_id": opened.session.agent_session_id,
                "action": "created",
            }),
        ),
    );
    Ok(Json(ApiResponse::ok(CreateAgentSessionResponseDto {
        agent_session_id: opened.session.agent_session_id.as_ref().to_owned(),
        agent_binding: binding,
        state: "ready".to_owned(),
        cursor: session_cursor(&opened.session.agent_session_id, opened.cursor.seq),
    })))
}

/// The plugin workbench shares the canonical owner and official Runtime, but
/// opens with an immutable product purpose and the user's resolved Agent binding.
pub(super) async fn create_plugin_authoring_session(
    state: &NomiCoreAgentApiState,
    owner: &UserId,
    binding: AgentBindingValueDto,
    reasoning_effort: Option<SessionReasoningEffortDto>,
    creation_key: &str,
) -> Result<String, NomiCoreApiError> {
    let (_, _, snapshot) = state.control_plane.saved_binding_artifacts(owner, &binding).await?;
    let selections = snapshot.content.required_resource_kinds.iter().filter_map(|kind| {
        automatic_agent_resource_id(kind.as_ref()).map(|resource_id| AgentResourceSelectionDto {
            resource_kind: kind.as_ref().to_owned(), resource_id: resource_id.to_owned(),
        })
    }).collect::<Vec<_>>();
    let binding = state.resource_bindings.resolve_for_saved_binding(
        &state.control_plane, owner, binding, &selections,
    ).await?;
    let authenticated = AuthenticatedOwner(owner.clone());
    let projection = resolve_saved_binding_projection(state, &authenticated, &binding, Some("Plugin authoring")).await?;
    super::plugin_authoring_sessions::validate_scope(&projection.snapshot)?;
    if let Some(effort) = reasoning_effort {
        if !saved_binding_supports_reasoning_effort(state, &authenticated, &binding, contract_reasoning_effort(effort)).await? {
            return Err(NomiCoreApiError::new(StatusCode::UNPROCESSABLE_ENTITY,
                "AGENT_SESSION_REASONING_UNSUPPORTED", "The selected model does not support this reasoning effort"));
        }
    }
    let idmm = idmm_config_from_runtime_policy(&projection.runtime_policy)?;
    state.session_owner.validate_idmm_state(&idmm).await?;
    let binding_contract: AgentBindingValue = serde_json::from_value(serde_json::to_value(&binding)?)?;
    let active = projection.snapshot.content.contributions().map(|value| value.capability.id.as_ref().to_owned()).collect();
    let opened = state.session_owner.canonical().open_with_purpose(authenticated_principal(&authenticated),
        binding_contract, Some("Plugin authoring".into()), active,
        reasoning_effort.map(contract_reasoning_effort), nomifun_agent_contracts::SessionPurpose::PluginAuthoring,
        creation_key, now_ms()).await?;
    // Materialize the selected binding's workspace, or its managed scratch directory.
    state.session_owner.materialize_workspace_for_binding(owner.as_ref(), &opened.session.agent_session_id,
        &opened.session.agent_binding).await?;
    state.session_owner.initialize_idmm_state(opened.session.agent_session_id.as_ref(), idmm).await?;
    Ok(opened.session.agent_session_id.as_ref().to_owned())
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
        super::nomi_core_resource_bindings::freeze_resource_definition_id(resource)?;
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
        super::nomi_core_resource_bindings::freeze_resource_definition_id(resource)?;
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

    #[test]
    fn agent_switch_storage_failure_is_not_an_unsupported_session() {
        let error = super::agent_switch_store_error(
            nomifun_agent_session::SessionStoreError::Sqlite(sqlx::Error::Protocol(
                "foreign key condition rejected the transition".to_owned(),
            )),
        );
        assert_eq!(error.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_ne!(error.code, "AGENT_SESSION_AGENT_SWITCH_UNSUPPORTED");
        assert!(error.message.contains("foreign key condition"));
    }

    #[test]
    fn agent_switch_active_turn_preserves_its_specific_blocker() {
        let error = super::agent_switch_store_error(
            nomifun_agent_session::SessionStoreError::Conflict("active Turn".to_owned()),
        );
        assert_eq!(error.status, axum::http::StatusCode::CONFLICT);
        assert_eq!(error.code, "AGENT_SESSION_TURN_ACTIVE");
    }
}

async fn agent_switch_recovery_blocker(
    pool: &nomifun_db::SqlitePool,
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
    .fetch_all(pool)
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
    if let Some(blocker) = agent_switch_recovery_blocker(&state.session_owner.pool, session_id).await? {
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
    let preserved_extensions = session_capability_selection::selection(&current_snapshot);
    let raw_target = state.control_plane.resolve_agent_session_capabilities_binding(&owner.0, &raw_target, Some(&preserved_extensions)).await?;
    if raw_target.preset_revision_ref == current_dto.preset_revision_ref && raw_target.resolved_snapshot_ref == current_dto.resolved_snapshot_ref {
        return Err(NomiCoreApiError::new(StatusCode::CONFLICT, "AGENT_SESSION_AGENT_UNCHANGED", "the selected Agent and Session extensions are already active"));
    }
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
                super::nomi_core_resource_bindings::freeze_resource_definition_id(resource)?;
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
        // An unexpected storage/contract failure is not evidence that this
        // Session kind cannot switch Agents. Preserve the common typed error
        // classification instead of translating an internal cause to a
        // misleading product blocker.
        _ => return agent_session_store_error(error).into(),
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
    let current = state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
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
    state.session_owner.canonical_conversation_projection(owner.as_ref(), &session_id).await?;
    let observation = state
        .session_owner
        .canonical()
        .get(&authenticated_principal(&owner), &session_id)
        .await?;
    Ok(Json(ApiResponse::ok(observation)))
}

async fn get_nomi_core_agent_session_execution(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
) -> Result<Json<ApiResponse<Option<nomifun_agent_session::NativeExecutionInspection>>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    state.session_owner.canonical_conversation_projection(owner.as_ref(), &session_id).await?;
    let inspection = state.session_owner.canonical().store()
        .inspect_latest_native_execution(&authenticated_principal(&owner), &session_id)
        .await.map_err(agent_session_store_error)?;
    Ok(Json(ApiResponse::ok(inspection)))
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
    state.session_owner.user_events.send_to_user(
        owner.as_ref(),
        WebSocketMessage::new(
            "conversation.listChanged",
            json!({
                "conversation_id": session_id,
                "action": "updated",
            }),
        ),
    );
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
         WHERE conversation_id = ? AND relation IN ('attempt', 'automation'))",
    )
    .bind(session_id.as_ref())
    .fetch_one(&state.session_owner.pool)
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    if attempt_transcript != 0 {
        return Err(NomiCoreApiError::new(
            StatusCode::CONFLICT,
            "AGENT_EXECUTION_ATTEMPT_READ_ONLY",
            "AgentExecution transcripts cannot change their frozen model binding",
        ));
    }

    state.session_owner.prepare_session_contract_evolution(owner.as_ref(), &session_id).await?;
    let observation = state.session_owner.canonical()
        .get(&authenticated_principal(&owner), &session_id).await?;
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
        state.session_owner.settle_session_binding_runtime(owner.as_ref(), &session_id).await?;
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
        SessionReasoningEffortDto::None => ReasoningEffort::None,
        SessionReasoningEffortDto::Minimal => ReasoningEffort::Minimal,
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
        ReasoningEffort::None => SessionReasoningEffortDto::None,
        ReasoningEffort::Minimal => SessionReasoningEffortDto::Minimal,
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
    Ok(binding_supports_reasoning_effort(&state.control_plane, &owner.0, binding, effort).await?)
}

async fn binding_supports_reasoning_effort(
    control_plane: &AgentControlPlane,
    owner: &UserId,
    binding: &AgentBindingValueDto,
    effort: ReasoningEffort,
) -> Result<bool, AppError> {
    let (_, revision, snapshot) = control_plane.saved_binding_artifacts(owner, binding)
        .await.map_err(control_plane_error_to_app)?;
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
    if effort != ReasoningEffort::None && !record.primary.features.contains(&ChatRouteFeature::Reasoning) {
        return Ok(false);
    }
    Ok(match record.primary.protocol {
        ChatRouteProtocol::OpenaiChat | ChatRouteProtocol::OpenaiResponses => true,
        ChatRouteProtocol::Gemini => matches!(
            effort,
            ReasoningEffort::Minimal | ReasoningEffort::Low | ReasoningEffort::Medium | ReasoningEffort::High
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
    let idmm_decision: Option<IdmmDecisionExplanation> = document.get("idmm_decision")
        .filter(|value| !value.is_null()).cloned().map(serde_json::from_value).transpose()?;
    if let Some(decision) = &idmm_decision {
        decision.validate().map_err(|error| NomiCoreApiError::new(StatusCode::CONFLICT,
            "AGENT_SESSION_MESSAGE_PROJECTION_INVALID", error))?;
    }
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
                "turn_state": state,
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

    // Steering has one message per admission event, while correlation_id
    // deliberately remains the owning Turn operation. Use the persisted
    // message identity for accepted user rows, including existing projections.
    let message_id = if projection.presentation_intent == "message" && state == "accepted" {
        projection.projection_id.strip_prefix("message:")
    } else {
        document.get("correlation_id").and_then(Value::as_str)
    };
    let Some(message_id) = message_id else {
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
                "turn_id": document.get("turn_id"),
                "idmm_decision": idmm_decision,
                "display_at_ms": document.get("display_at_ms").and_then(Value::as_i64).unwrap_or_else(|| {
                    // created_at remains the stable keyset/order cursor. The
                    // displayed clock comes from the message, not Session age.
                    let uuid = Uuid::parse_str(message_id).expect("validated above");
                    let bytes = uuid.as_bytes();
                    bytes[..6].iter().fold(0_i64, |time, byte| (time << 8) | i64::from(*byte))
                }),
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
                json!({ "content": content, "status": if state == "streaming" { "thinking" } else { "done" }, "turn_id": turn_id }),
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
                            let cancelled = summary.get("capability_id").and_then(Value::as_str) == Some("workspace.process")
                                && observation.output.as_deref().and_then(|output| serde_json::from_str::<Value>(output).ok())
                                    .is_some_and(|result| result.get("state").and_then(Value::as_str) == Some("cancelled")
                                        && result.pointer("/cleanup/reaped").and_then(Value::as_bool) == Some(true));
                            summary.insert("status".to_owned(), json!(
                                if cancelled { "canceled" } else if is_error { "error" } else { "completed" }
                            ));
                        }
                    }
                }
                summary
            },
            MessagePosition::Left,
        ),
        "idmm_notice" => {
            let notice: IdmmDecisionNotice = serde_json::from_value(document.get("reference").cloned()
                .ok_or_else(|| NomiCoreApiError::new(StatusCode::CONFLICT,
                    "AGENT_SESSION_MESSAGE_PROJECTION_INVALID", "IDMM notice lost its canonical reference"))?)?;
            notice.validate().map_err(|error| NomiCoreApiError::new(StatusCode::CONFLICT,
                "AGENT_SESSION_MESSAGE_PROJECTION_INVALID", error))?;
            (MessageType::Tips, json!({
                "content":notice.decision.rationale,
                "type":if notice.status == nomifun_agent_contracts::IdmmDecisionNoticeStatus::Failed { "error" } else { "warning" },
                "display_at_ms":notice.created_at,
                "idmm_notice":notice,
            }), MessagePosition::Center)
        }
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
        "accepted" | "completed" | "recorded" | "waiting_for_human" => MessageStatus::Finish,
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

#[path = "conversation_task_plan.rs"]
mod conversation_task_plan;

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
    let (mut projections, has_more, total) = state
        .session_owner
        .canonical()
        .store()
        .message_history_before(&session_id, before_seq, page_size)
        .await
        .map_err(agent_session_store_error)?;
    super::history_thinking_display::hydrate_thinking_lifecycle(&state.session_owner.pool, &session_id, &mut projections).await?;
    let observations = load_historical_tool_observations(
        &state.session_owner.pool,
        &session_id,
        &projections,
    ).await?;
    let continuations = history_text_continuation::load(&state.session_owner.pool,&session_id,&projections).await?;
    let mut items = Vec::new();
    let mut template_cache = HashMap::new();
    for projection in projections {
        let observation = observations.get(&projection.projection_id);
        if let Some(mut message) = canonical_message_response_with_observation(
            &session_id, created_at, projection, observation,
        )? {
            if let Some(previous)=continuations.get(&message.message_id) {
                message.content["continuation_of_message_id"]=json!(previous);
            }
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
        });
    let mut projection = match projection {
        Some(projection) => Some(projection),
        None => state.session_owner.canonical().store()
            .runtime_tool_history_message(&session_id, &message_id).await.map_err(agent_session_store_error)?,
    }.ok_or_else(|| {
            NomiCoreApiError::new(
                StatusCode::NOT_FOUND,
                "AGENT_SESSION_MESSAGE_NOT_FOUND",
                "message projection does not exist",
            )
        })?;
    super::history_thinking_display::hydrate_thinking_lifecycle(
        &state.session_owner.pool, &session_id, std::slice::from_mut(&mut projection),
    ).await?;
    let observations = load_historical_tool_observations(
        &state.session_owner.pool, &session_id, std::slice::from_ref(&projection),
    ).await?;
    let continuations=history_text_continuation::load(&state.session_owner.pool,&session_id,std::slice::from_ref(&projection)).await?;
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
    if let Some(previous)=continuations.get(&message.message_id) {
        message.content["continuation_of_message_id"]=json!(previous);
    }
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
        .write_owned()
        .await;
    state.session_owner.prepare_session_contract_evolution(owner.as_ref(), &session_id).await?;
    let observation = state.session_owner.canonical()
        .get(&authenticated_principal(&owner), &session_id).await?;
    state.session_owner.materialize_workspace_for_binding(
        owner.as_ref(), &session_id, &observation.session.agent_binding,
    ).await?;
    let projection = state
        .session_owner
        .canonical_conversation_projection(owner.as_ref(), &session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!(
            "AgentSession {} not found",
            session_id.as_ref(),
        )))?;
    let options = state.session_owner.runtime_options_for_projection(
        owner.as_ref(), &session_id, projection,
    ).await?;
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
    state.session_owner.queue_receipted_steering(&session_id, &receipt).await?;
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
    let receipt = if let Some(expected_turn_id) = request.expected_turn_id.as_deref() {
        state.session_owner.cancel_accepted_input_turn(
            &principal.principal_id, &session_id, &key, expected_turn_id,
        ).await?
    } else {
        state.session_owner.cancel_turn(
            &principal.principal_id,
            &session_id,
            &key,
            nomifun_common::AgentKillReason::UserCancelled,
        )
        .await?
    };
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
            None,
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
    state.session_owner.user_events.send_to_user(
        owner.as_ref(),
        WebSocketMessage::new(
            "conversation.listChanged",
            json!({
                "conversation_id": session_id,
                "action": "deleted",
            }),
        ),
    );
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
                unresolved_targets: Vec::new(),
                unresolved_before_input: None,
                target_budget_exceeded: false,
            },
        };
        let pending = serde_json::to_string(&json!({ "event": pending })).unwrap();
        let blocker = agent_switch_recovery_blocker_from_rows(vec![(1, pending)])
            .unwrap()
            .expect("pending recovery blocker");
        assert_eq!(blocker.code, "AGENT_SESSION_HANDOFF_RECOVERY_PENDING");
        assert_eq!(blocker.details.unwrap()["pending"], true);

        let unresolved = nomifun_agent_runtime::AgentEngineEvent::PatchRecoveryUpdated {
            state: nomifun_agent_runtime::AgentPatchRecoveryState {
                version: 2,
                targets: Vec::new(),
                unresolved_targets: vec!["src/unpublished.rs".to_owned()],
                unresolved_before_input: Some(1),
                target_budget_exceeded: false,
            },
        };
        let unresolved = serde_json::to_string(&json!({ "event": unresolved })).unwrap();
        let blocker = agent_switch_recovery_blocker_from_rows(vec![(1, unresolved)])
            .unwrap().expect("unresolved mutation blocker");
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

pub(super) fn bounded_turn_input(value: Value) -> Result<SendMessageRequest, NomiCoreApiError> {
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
    if ["idmm_decision", "idmm_notice", "input_source"].iter().any(|key| object.contains_key(*key))
        || object.get("origin").and_then(Value::as_str) == Some("idmm") {
        return Err(NomiCoreApiError::new(StatusCode::BAD_REQUEST,
            "NOMI_CORE_INVALID_REQUEST", "IDMM source metadata is owned by the supervisor"));
    }
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
    if let Some(requirement) = object.get("plugin_delivery").filter(|value| !value.is_null()) {
        let requirement: nomifun_api_types::PluginDeliveryRequirement = serde_json::from_value(requirement.clone())?;
        if let Some(draft) = &requirement.draft_id {
            nomifun_common::validate_uuidv7(draft).map_err(|_| NomiCoreApiError::new(
                StatusCode::BAD_REQUEST,"PLUGIN_INVALID_DRAFT_ID","plugin_delivery requires a canonical draft UUIDv7",
            ))?;
        }
        request.plugin_delivery = Some(requirement);
    }
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

pub(super) fn canonical_turn_input(request: &SendMessageRequest) -> Value {
    let mut input = json!({
        "content": request.content,
        "files": request.files,
        "inject_skills": request.inject_skills,
        "hidden": request.hidden,
        "origin": request.origin,
        "channel_platform": request.channel_platform,
    });
    if let Some(requirement) = &request.plugin_delivery {
        input["plugin_delivery"] = serde_json::to_value(requirement).expect("delivery requirement");
    }
    input
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
        plugin_delivery: None,
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

#[cfg(test)]
mod failure_pause_settlement_tests {
    use super::*;
    use nomifun_agent_contracts::SessionEventPayloadRef;
    use nomifun_agent_runtime::AgentEngineEvent;
    #[tokio::test]
    async fn reload_settles_a_proven_model_failure_pause_and_accepts_the_next_message() {
        use tower::ServiceExt;
        const TRUST: &str = "failure-pause-reload";
        async fn request(router: &axum::Router, method: &str, path: &str, body: Value) -> Value {
            let response = router.clone().oneshot(axum::http::Request::builder().method(method).uri(path)
                .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string())).unwrap()).await.unwrap();
            let status = response.status();
            let body: Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
                .await.unwrap()).unwrap();
            assert!(status.is_success(), "{path}: {status}: {body}");
            body["data"].clone()
        }
        let root = tempfile::tempdir().unwrap();
        let config = crate::AppConfig {
            data_dir:root.path().join("data"), work_dir:root.path().join("work"),
            auth_policy:nomifun_auth::AuthPolicy::TrustLocalToken, local_trust_secret:Some(TRUST.into()), ..Default::default()
        };
        std::fs::create_dir_all(&config.data_dir).unwrap();
        let database = nomifun_db::init_database(&config.database_path()).await.unwrap();
        let services = crate::services::AppServices::from_config(database, &config).await.unwrap();
        let (states, _components) = super::super::state::try_build_module_states(&services).await.unwrap();
        let owner = states.nomi_core_agent_api.session_owner.clone();
        let router = super::super::create_router_with_states(&services, states);
        let provider = request(&router, "POST", "/api/providers", json!({
            "platform":"custom","name":"failure fixture","base_url":"http://127.0.0.1:9/v1",
            "auth_scheme":"bearer","credentials":{"api_keys":["test-only"]},"enabled":true,
            "initial_model":{"model":"failure-fixture","enabled":true,"capabilities":[{
                "task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default"
            }]}
        })).await;
        let model = json!({"provider_id":provider["provider_id"],"model":"failure-fixture"});
        let preset = request(&router, "POST", "/api/agent-presets/from-template/chat.minimal",
            json!({"reuse_existing":false,"display_name":"Original failure Agent","model":model})).await;
        let opened = request(&router, "POST", "/api/agent-sessions",
            json!({"preset_id":preset["preset"]["preset_id"],"model":model})).await;
        let session = AgentSessionId::from(opened["agent_session_id"].as_str().unwrap());
        let principal = PrincipalRef { principal_kind:"user".into(), principal_id:services.authoritative_user_id.as_ref().into() };
        let send: SendMessageRequest = serde_json::from_value(json!({"content":"Produce the requested result"})).unwrap();
        let operation = NomiCoreSessionOwner::turn_operation_id(&principal.principal_id, session.as_ref(), "failed-model-turn");
        let input = owner.canonical_turn_input_with_admission(&principal.principal_id, &session, &operation, &send).await.unwrap();
        let admitted = owner.canonical.start_turn(&principal, &session, "failed-model-turn", input).await.unwrap();
        assert_eq!(admitted.operation_id, operation);
        let observed = owner.canonical.get(&principal, &session).await.unwrap();
        let store = owner.canonical.store();
        let started = store.read_turn_receipt(&session, &operation).await.unwrap().started_event.unwrap();
        let SessionEventPayloadRef::InlineJson(payload) = &started.payload else { panic!("start payload must be inline") };
        let source = payload.0["source_message_id"].as_str().unwrap();
        let lease = store.claim_native_execution(nomifun_agent_session::NativeExecutionClaim {
            owner:principal.clone(), agent_session_id:session.clone(), operation_id:operation.clone(),
            snapshot:observed.session.agent_binding.resolved_snapshot_ref.clone(),
            active_set_generation:observed.head.active_set_generation, holder:"paused-failure-owner".into(),
            expected_fence:0, checkpoint:None,
        }).await.unwrap();
        let runtime = services.official_runtime.binding().unwrap();
        let engine = nomifun_agent_runtime::EngineBinding::new(session.clone(),
            format!("conversation-runtime:{}", session.as_ref()).into(), runtime.build_id.into(), runtime.build_digest.into(),
            observed.session.agent_binding.resolved_snapshot_ref.clone()).unwrap();
        let historical_result = nomifun_agent_runtime::AgentToolResult::text("read-before-failure".into(), "ORIGINAL_OBSERVED_CONTENT", false);
        let model_operation = OperationId::from("fixture-model-operation");
        store.claim_native_chat_operation(&lease, nomifun_agent_session::ChatOperationClaimRequest {
            agent_session_id:session.clone(), operation_id:model_operation.clone(), turn_operation_id:operation.clone(),
            causation_event_id:source.into(), route_identity:serde_json::from_value(payload.0["route_identity"].clone()).unwrap(),
            resolved_snapshot_ref:observed.session.agent_binding.resolved_snapshot_ref.clone(),
        }).await.unwrap();
        let native_events = [
            serde_json::to_value(AgentEngineEvent::TurnStarted { binding:engine, turn_operation_id:operation.clone() }).unwrap(),
            serde_json::to_value(AgentEngineEvent::TurnInputScope { wire_turn_id:source.into() }).unwrap(),
            serde_json::to_value(AgentEngineEvent::ModelStepStarted { step:1, operation_id:model_operation }).unwrap(),
            serde_json::to_value(AgentEngineEvent::ToolCallCompleted { step:1, call:nomifun_chat_model_broker::ChatToolCall {
                call_id:"read-before-failure".into(), name:"read_file".into(), arguments:StrictJsonValue(json!({"path":"result.txt"})),
                provider_metadata:None,
            } }).unwrap(),
            serde_json::to_value(AgentEngineEvent::ToolCompleted { step:1, result:historical_result.clone() }).unwrap(),
            json!({"event":"host_cleanup_proven"}),
            serde_json::to_value(AgentEngineEvent::TurnPaused { model_steps:1, reason:"EXECUTION_MODEL_PROVIDER_UNAVAILABLE".into() }).unwrap(),
        ];
        for (index, event) in native_events.into_iter().enumerate() {
            let identity = format!("fixture-progress:{index}");
            store.append_native_event(&lease, &nomifun_agent_contracts::SessionEventAppend {
                agent_session_id:session.clone(), event_id:identity.clone().into(), producer_id:"runtime_supervisor".into(),
                idempotency_key:identity.into(), semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {
                    kind:nomifun_agent_contracts::SessionEventKind("runtime/progress-recorded".into()), kind_version:1,
                    correlation_id:operation.as_ref().into(), causation_event_id:Some(started.event_id.clone()),
                    payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"producer_seq":index+1,"event":event}))),
                },
            }, None).await.unwrap();
        }
        store.pause_native_execution(&lease, "EXECUTION_MODEL_PROVIDER_UNAVAILABLE", true).await.unwrap();
        assert_eq!(store.head(&session).await.unwrap().status, "paused");
        let projected = request(&router, "GET", &format!("/api/agent-sessions/{}/projection", session.as_ref()), Value::Null).await;
        assert_eq!(projected["runtime"]["can_send_message"], true);
        assert_eq!(projected["extra"]["execution_phase"], "ready");
        assert!(projected["runtime"]["active_turn_id"].is_null());
        let failed = store.read_turn_receipt(&session, &operation).await.unwrap();
        assert_eq!(failed.status, nomifun_agent_session::TurnReceiptStatus::Failed);
        let SessionEventPayloadRef::InlineJson(failure) = failed.terminal_event.unwrap().payload else { panic!("failure payload must be inline") };
        assert_eq!(failure.0["error"]["code"], "USER_LLM_PROVIDER_UNAVAILABLE");
        assert_eq!(failure.0["error"]["modelName"], "failure-fixture");
        assert_eq!(failure.0["error"]["detail"], "EXECUTION_MODEL_PROVIDER_UNAVAILABLE");
        let next_operation = NomiCoreSessionOwner::turn_operation_id(&principal.principal_id, session.as_ref(), "next-message");
        let next_input = owner.canonical_turn_input_with_admission(&principal.principal_id, &session, &next_operation, &send).await.unwrap();
        let next = owner.canonical.start_turn(&principal, &session, "next-message", next_input).await.unwrap();
        assert_eq!(store.head(&session).await.unwrap().active_turn_id.as_deref(), Some(next.operation_id.as_ref()));
        let mut projection = owner.canonical_conversation_projection(&principal.principal_id, &session).await.unwrap().unwrap();
        let workspace = match projection.extra.get("workspace").and_then(Value::as_str) {
            Some(workspace) => workspace.to_owned(),
            None => materialize_managed_session_workspace(&owner.managed_workspace_root, &session).await.unwrap(),
        };
        projection.extra["workspace"] = json!(workspace);
        // This receipt only reads canonical history. No Runtime or physical
        // Knowledge workspace owner is opened by the test.
        let options = AgentRuntimeBuildOptions {
            user_id:principal.principal_id.clone(), agent_type:projection.r#type, workspace,
            model:projection.model, conversation_id:session.as_ref().into(), delegation_policy:projection.delegation_policy,
            extra:projection.extra, conversation_created_at:Some(projection.created_at), workspace_binding_lease:None,
        };
        let successor_start = store.read_turn_receipt(&session, &next.operation_id).await.unwrap().started_event.unwrap();
        let SessionEventPayloadRef::InlineJson(successor_payload) = successor_start.payload else { panic!("successor must have inline admission") };
        let successor_root = successor_payload.0["source_message_id"].as_str().unwrap();
        let engines = owner.native_engines.get().unwrap().upgrade().unwrap();
        let receipt = engines.read_turn_receipt(&options, &services.official_runtime.binding().unwrap(),
            &observed.session.agent_binding.resolved_snapshot_ref, &SendMessageData {
                content:send.content.clone(), msg_id:successor_root.into(), source_message_id:Some(successor_root.into()),
                files:Vec::new(), inject_skills:Vec::new(), origin:None,
            }).await.unwrap();
        let window = engines.read_history(&receipt, 8).await.unwrap();
        assert_eq!(window.turns.len(), 1);
        assert_eq!(window.turns[0].receipt_status, "failed");
        let before_history_read = store.current_cursor(&session).await.unwrap();
        let history = super::super::unified_runtime_history::load(window, &engines, &receipt, 1024 * 1024).await.unwrap();
        assert!(history.messages.iter().flat_map(|message| &message.content).any(|content|
            matches!(content, nomifun_chat_model_broker::ChatContentPart::ToolResult { output, .. }
                if *output == historical_result.output)), "the original result stays historical data without tool replay");
        assert_eq!(store.current_cursor(&session).await.unwrap(), before_history_read,
            "history decoding grants no execution, checkpoint restoration or new facts");
        assert_eq!(store.read_turn_receipt(&session, &operation).await.unwrap().status, nomifun_agent_session::TurnReceiptStatus::Failed);
        owner.canonical.cancel_exact_turn(&principal, &session, "cancel-test-successor", &next.operation_id).await.unwrap();
        services.shutdown_nomi_core_host().await.unwrap();
        services.database.close().await;
    }
}
