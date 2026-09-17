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
use nomifun_ai_agent::types::AgentRuntimeBuildOptions;
use nomifun_ai_agent::{
    AgentRuntimeRegistry, AgentStreamEvent, KernelNomiPluginToolSession,
    NomiPluginProductToolInvocation, NomiPluginProductToolInvoker,
    NomiPluginProductToolSchemaResolver, NomiPluginToolError,
    NomiPluginToolSchemaResolver, NomiPluginToolSession,
    NomiPluginToolSessionProvider, NomiPluginToolSessionRequest,
    NomiPlatformBuiltinContextAdmission,
    NomiPlatformBuiltinLifecycleAdmission,
    NomiPlatformBuiltinToolAdmission,
    SessionControlSink,
};
use nomifun_agent_contracts::{
    AgentBindingValue, AgentSessionId, ArtifactId, ContributionSourceKind,
    DeleteAgentSessionCommand, OperationId, PrincipalRef, RemoteBindingProvenance,
    ResolvedCapability, ScopeKey, StrictJsonValue, UserId,
};
use nomifun_agent_control_plane::{
    AgentControlPlane, AuthenticatedOwner, ControlPlaneError,
};
use super::nomi_core_control_plane::control_plane_router_without_legacy_skills;
use nomifun_api_types::{
    AgentBindingValueDto, AgentResourceSelectionDto,
    ApiResponse, ConversationResponse,
    ConversationRuntimeStateKind, CreateAgentSessionRequestDto, CreateConversationRequest,
    CreateAgentSessionResponseDto, CreateAgentSessionTurnRequestDto,
    CreateAgentSessionTurnResponseDto, AgentSessionTurnMutationResponseDto,
    CancelAgentSessionTurnRequestDto, SteerAgentSessionTurnRequestDto,
    ErrorResponse, ForkAgentSessionRequestDto,
    ForkAgentSessionResponseDto, ListMessagesQuery, MessageListResponse, MessageResponse,
    RemoteCancelRequestDto, RemoteMutationResponseDto, RemoteObserveRequestDto,
    RemoteObserveResponseDto, RemoteOpenRequestDto, RemoteOpenResponseDto,
    RemoteOpenStateViewDto, RemoteTurnRequestDto,
    AgentChatModelSelectionDto, AgentResolvedSnapshot, SessionCursorDto,
    McpServerId,
    SendMessageRequest, UpdateConversationRequest,
    SwitchAgentSessionPresetRequestDto, SwitchAgentSessionPresetResponseDto,
    UpdateAgentSessionCapabilitySelectionRequestDto,
    UpdateAgentSessionCapabilitySelectionResponseDto,
    CreateAgentPresetFromTemplateRequest, PutAgentBindingRequest,
};
use nomifun_common::{AppError, ConversationStatus, MessagePosition, MessageType};
use nomifun_conversation::service::{
    BackgroundTurnReconciliationDisposition,
    BackgroundTurnRuntimePreparation, IdempotentMessageDelivery,
    PublicTurnDeliveryState,
};
use nomifun_conversation::{
    AgentExecutionConversationPort, CanonicalAgentSessionOwner, ConversationService,
    PreparedAgentSessionDelete, ProductAgentResolution, ProductAgentSnapshotResolver,
    ProductAgentTarget,
};
use nomifun_db::{
    AgentExecutionTurnAuthority, AppendNomiRemoteEventParams, GetOrCreateRemoteSessionParams,
    IRemoteBindingRepository, RemoteOpenResult, SortOrder, TransitionNomiRemoteSessionParams,
};
use nomifun_db::models::{MessageRow, NomiRemoteEventRow, NomiRemoteSessionRow};
use nomifun_agent_session::{MessageProjection, SessionObservation};
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
/// receipts, resources, forks and deletion. `ConversationService` is a
/// temporary legacy runtime adapter with separate Wave 6 deletion ownership.
pub(crate) struct NomiCoreSessionOwner {
    runtime_engines: std::sync::OnceLock<Arc<super::runtime_engines::RuntimeEngineHost>>,
    runtime_control_plane: std::sync::OnceLock<std::sync::Weak<AgentControlPlane>>,
    service: ConversationService,
    canonical: CanonicalAgentSessionOwner,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    execution: AgentExecutionConversationPort,
    session_operation_locks:
        Arc<DashMap<String, Arc<tokio::sync::RwLock<()>>>>,
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
    runtime_engines: Arc<super::runtime_engines::RuntimeEngineHost>,
    resource_bindings: super::nomi_core_resource_bindings::NomiCoreResourceBindingResolverRegistry,
    owner_id: Arc<str>,
    pool: nomifun_db::SqlitePool,
    default_binding_lock: tokio::sync::Mutex<()>,
}

impl NomiCoreProductAgentResolver {
    pub(crate) fn new(control_plane: Arc<AgentControlPlane>, owner_id: Arc<str>, pool: nomifun_db::SqlitePool, runtime_engines: Arc<super::runtime_engines::RuntimeEngineHost>, resource_bindings: super::nomi_core_resource_bindings::NomiCoreResourceBindingResolverRegistry) -> Self {
        Self {
            control_plane,
            runtime_engines,
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
    async fn resolve_preset(
        &self,
        owner_id: &str,
        preset_id: &str,
        requested_model: Option<&nomifun_common::ProviderWithModel>,
        current_binding: Option<&AgentBindingValueDto>,
    ) -> Result<ProductAgentResolution, AppError> {
        let owner = UserId::from(owner_id.to_owned());
        let model = requested_model.map(|model| AgentChatModelSelectionDto {
            provider_id: model.provider_id.clone(), model: model.model.clone(),
        });
        let mut binding = self.control_plane.resolve_agent_session_binding_with_model(&owner, preset_id, model.as_ref())
            .await.map_err(control_plane_error_to_app)?;
        if let Some(current) = current_binding {
            let (_, _, target) = self.control_plane.saved_binding_artifacts(&owner, &binding)
                .await.map_err(control_plane_error_to_app)?;
            // Reuse only the user's selected resource IDs. The new revision
            // determines operations, and product authorities validate them anew.
            let selections = current.typed_resource_bindings.iter()
                .filter(|resource| target.content.required_resource_kinds.iter().any(|kind| kind.as_ref() == resource.resource_kind))
                .map(|resource| AgentResourceSelectionDto { resource_kind: resource.resource_kind.clone(), resource_id: resource.resource_id.clone() })
                .collect::<Vec<_>>();
            binding = self.resource_bindings.resolve_for_saved_binding(&self.control_plane, &owner, binding, &selections)
                .await.map_err(|error| AppError::UnprocessableEntity(format!("{}: {}", error.code(), error.message())))?;
        }
        let (binding, revision, snapshot) = self.control_plane.saved_binding_artifacts(&owner, &binding)
            .await.map_err(control_plane_error_to_app)?;
        let target_engine = self.runtime_engines.validate_agent(&snapshot)?;
        let editor = self.control_plane.editor(&owner, revision.reference.preset_id.as_ref(), Some(revision.reference.revision))
            .await.map_err(control_plane_error_to_app)?;
        let common_owner = nomifun_common::UserId::parse(owner_id.to_owned())
            .map_err(|error| AppError::Forbidden(format!("invalid Agent owner: {error}")))?;
        let mut projected = super::nomi_core_agent_projection::project_saved_artifacts(
            &common_owner, binding, revision, snapshot, Some(&editor.preset.display_name),
        )?;
        if current_binding.is_some() {
            let repository: Arc<dyn nomifun_db::IMcpServerRepository> = Arc::new(nomifun_db::SqliteMcpServerRepository::new(self.pool.clone()));
            let selection = exact_session_mcp_selection(&repository, &AuthenticatedOwner(owner.clone()), &projected.binding)
                .await.map_err(|error| AppError::Conflict(error.message))?;
            install_runtime_mcp_selection(&mut projected.projection.request.extra, &selection)
                .map_err(|error| AppError::Conflict(error.message))?;
        }
        // A next-turn preset selection rebuilds the runtime just like the
        // explicit switch endpoint. Its exact Kernel binding must accompany
        // the projected snapshot; the UI snapshot alone cannot open a session.
        attach_session_metadata(&mut projected.projection.request.extra, &projected.binding, None)
            .map_err(|error| AppError::Conflict(error.message))?;
        projected.projection.request.extra[nomifun_api_types::RUNTIME_ENGINE_BINDING_KEY] = serde_json::to_value(&target_engine)
            .map_err(|error| AppError::Internal(error.to_string()))?;
        self.runtime_engines.catalog()?.validate_session_extra(&target_engine, &projected.projection.request.extra)?;
        Ok(ProductAgentResolution { snapshot: projected.projection.snapshot, runtime_extra: projected.projection.request.extra })
    }

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
        let mut selection = self.selection(&owner, &target.target_kind, &target.target_id).await?;
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
            let mut binding = self
                .control_plane
                .resolve_agent_session_binding(&owner, &editor.preset.preset_id)
                .await
                .map_err(control_plane_error_to_app)?;
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
        let target_engine = self.runtime_engines.validate_agent(&snapshot)?;
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
        let mut projected = super::nomi_core_agent_projection::project_saved_artifacts(
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
        projected.projection.request.extra[nomifun_api_types::RUNTIME_ENGINE_BINDING_KEY] =
            serde_json::to_value(&target_engine)
                .map_err(|error| AppError::Internal(error.to_string()))?;
        self.runtime_engines
            .catalog()?
            .validate_session_extra(&target_engine, &projected.projection.request.extra)?;
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
        service: ConversationService,
        canonical: CanonicalAgentSessionOwner,
        runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    ) -> Self {
        let execution = service.agent_execution_port(runtime_registry.clone());
        Self {
            service,
            canonical,
            runtime_engines: std::sync::OnceLock::new(),
            runtime_control_plane: std::sync::OnceLock::new(),
            runtime_registry,
            execution,
            session_operation_locks: Arc::new(DashMap::new()),
        }
    }

    pub(crate) fn service(&self) -> &ConversationService {
        &self.service
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

    pub(crate) fn install_runtime_engines(&self, host: Arc<super::runtime_engines::RuntimeEngineHost>, control_plane: std::sync::Weak<AgentControlPlane>) -> Result<(), AppError> {
        self.runtime_control_plane.set(control_plane).map_err(|_| AppError::Conflict("Session control plane already installed".into()))?;
        self.runtime_engines.set(host).map_err(|_| AppError::Conflict("Session runtime host already installed".into()))
    }

    /// Idempotent counterpart of [`Self::create_session`].
    ///
    /// The creation key is interpreted and durably owned by
    /// `ConversationService`; this facade does not keep a second identity map.
    pub(crate) async fn create_session_idempotent(
        &self,
        owner_id: &str,
        mut request: CreateConversationRequest,
        snapshot: Option<AgentResolvedSnapshot>,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        // Consumer snapshots retain the immutable binding, not an Engine
        // override. Reconstruct only host metadata here; saved artifacts below
        // must authenticate the reference and the complete projected snapshot.
        let consumer_projection = request.extra.get(NOMI_CORE_SESSION_METADATA_KEY).is_none()
            && snapshot.as_ref().is_some_and(|snapshot| snapshot.canonical_binding.is_some());
        nomifun_api_types::ExecutionConstraints::from_extra(&request.extra)?;
        if let Some(canonical) = snapshot.as_ref().and_then(|snapshot| snapshot.canonical_binding.as_ref()) {
            if self.runtime_engines.get().is_none() {
                return Err(AppError::Conflict("Canonical Agent consumer requires the assembled Engine host".into()));
            }
            if consumer_projection {
                if request.extra.get(nomifun_api_types::RUNTIME_ENGINE_BINDING_KEY).is_some() {
                    return Err(AppError::Conflict("A consumer cannot override the saved Agent Engine".into()));
                }
                let binding: AgentBindingValue = serde_json::to_value(canonical).and_then(serde_json::from_value)
                    .map_err(|error| AppError::Conflict(format!("Invalid consumer Agent binding: {error}")))?;
                attach_session_metadata(&mut request.extra, &binding, None)
                    .map_err(|error| AppError::Conflict(error.message))?;
            }
        }
        if request.extra.get(NOMI_CORE_SESSION_METADATA_KEY).is_some()
            && let Some(host) = self.runtime_engines.get()
        {
            // All consumers inherit the exact saved Agent revision, including
            // workbench tests, remote sessions and automation. No composer override.
            let metadata: NomiCoreSessionMetadata = serde_json::from_value(
                request.extra[NOMI_CORE_SESSION_METADATA_KEY].clone(),
            ).map_err(|error| AppError::Internal(error.to_string()))?;
            let control_plane = self.runtime_control_plane.get().and_then(|value| value.upgrade())
                .ok_or_else(|| AppError::Conflict("Session control plane is unavailable".into()))?;
            let binding = serde_json::to_value(&metadata.binding).and_then(serde_json::from_value)
                .map_err(|error| AppError::Internal(error.to_string()))?;
            let (saved_binding, revision, resolved) = control_plane.saved_binding_artifacts(
                &nomifun_agent_contracts::UserId::from(owner_id.to_owned()), &binding,
            ).await.map_err(super::state::control_plane_error_to_app)?;
            if let Some(snapshot) = snapshot.as_ref().filter(|snapshot| snapshot.canonical_binding.is_some()) {
                let canonical = snapshot.canonical_binding.as_ref().expect("filtered canonical binding");
                if canonical != &binding {
                    return Err(AppError::Conflict("Consumer snapshot and Session metadata name different Agent bindings".into()));
                }
                let common_owner = nomifun_common::UserId::parse(owner_id.to_owned())
                    .map_err(|error| AppError::Forbidden(format!("Invalid consumer owner: {error}")))?;
                let projected = super::nomi_core_agent_projection::project_saved_artifacts(
                    &common_owner, saved_binding.clone(), revision.clone(), resolved.clone(), Some(&snapshot.preset_name),
                )?.projection;
                if &projected.snapshot != snapshot {
                    return Err(AppError::Conflict("Consumer Agent snapshot differs from its saved immutable artifacts".into()));
                }
                if consumer_projection {
                    if request.r#type != projected.request.r#type {
                        return Err(AppError::Conflict("Consumer Agent type differs from the saved Agent".into()));
                    }
                    let extra = request.extra.as_object_mut().ok_or_else(|| AppError::Conflict("Consumer Session extra must be an object".into()))?;
                    let projected_extra = projected.request.extra.as_object().ok_or_else(|| AppError::Internal("Agent projection extra must be an object".into()))?;
                    for (key, value) in projected_extra {
                        if extra.get(key).is_some_and(|existing| existing != value) {
                            return Err(AppError::Conflict(format!("Consumer overlay conflicts with the saved Agent field {key}")));
                        }
                        extra.insert(key.clone(), value.clone());
                    }
                    // Only the saved binding contributes an initial MCP
                    // selection. Runtime catalog/resource admission remains
                    // authoritative; no inferred name or extra server grant.
                    let ids = saved_binding.typed_resource_bindings.iter()
                        .filter(|resource| resource.resource_kind.as_ref() == "mcp_server")
                        .map(|resource| resource.resource_id.as_ref().to_owned()).collect::<BTreeSet<_>>();
                    let selected = serde_json::to_value(ids).map_err(|error| AppError::Internal(error.to_string()))?;
                    if extra.get("selected_mcp_server_ids").is_some_and(|value| value != &selected) {
                        return Err(AppError::Conflict("Consumer MCP selection differs from its saved Agent binding".into()));
                    }
                    extra.insert("selected_mcp_server_ids".into(), selected);
                }
            }
            // A replay keeps the already-created Session's exact build even
            // if an Agent channel changed since the first creation. The
            // Conversation repository still owns creation-key arbitration.
            let prior_engine = match self.service.conversation_repo().find_by_creation_key(owner_id, creation_key).await? {
                Some(row) => {
                    if row.user_id != owner_id {
                        return Err(AppError::Conflict("Consumer creation key crossed its owner boundary".into()));
                    }
                    let extra: Value = serde_json::from_str(&row.extra).map_err(|error| AppError::Conflict(error.to_string()))?;
                    let prior: NomiCoreSessionMetadata = serde_json::from_value(extra.get(NOMI_CORE_SESSION_METADATA_KEY).cloned()
                        .ok_or_else(|| AppError::Conflict("Creation key already belongs to a legacy unbound Session".into()))?)
                        .map_err(|error| AppError::Conflict(error.to_string()))?;
                    if prior.binding != saved_binding {
                        return Err(AppError::Conflict("Creation key already belongs to another immutable Agent binding".into()));
                    }
                    if extra.get(nomifun_api_types::EXECUTION_CONSTRAINTS_KEY) != request.extra.get(nomifun_api_types::EXECUTION_CONSTRAINTS_KEY) {
                        return Err(AppError::Conflict("Creation replay changed the execution constraints".into()));
                    }
                    Some(super::runtime_engines::binding_from_extra(&extra)?
                        .ok_or_else(|| AppError::Conflict("Created Agent Session has no exact Engine binding".into()))?)
                }
                None => None,
            };
            // Forks carry their parent's exact build; neither replay nor Fork
            // may resolve a different build through the current channel.
            let engine = match (super::runtime_engines::binding_from_extra(&request.extra)?, prior_engine) {
                (Some(requested), Some(prior)) if requested != prior => return Err(AppError::Conflict("Creation replay requested a different Engine".into())),
                (Some(engine), _) | (None, Some(engine)) => engine,
                (None, None) => host.agent_binding()?,
            };
            host.catalog()?.validate_snapshot(&engine, &resolved)?;
            host.catalog()?.validate_session_extra(&engine, &request.extra)?;
            super::nomi_core_mcp_catalog::validate_product_session_selection(&resolved, &saved_binding.typed_resource_bindings, &request.extra)?;
            request.extra[nomifun_api_types::RUNTIME_ENGINE_BINDING_KEY] = serde_json::to_value(engine)
                .map_err(|error| AppError::Internal(error.to_string()))?;
        }
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

    /// Materialize the retiring Conversation-shaped consumer projection from
    /// a Store-only canonical AgentSession. `None` means there is no canonical
    /// row and permits an explicit legacy fallback; every other canonical
    /// state (foreign owner, deleting/tombstoned row, invalid saved artifacts)
    /// fails closed.
    async fn canonical_conversation_projection(
        &self,
        owner_id: &str,
        session_id: &AgentSessionId,
    ) -> Result<Option<ConversationResponse>, AppError> {
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: owner_id.to_owned(),
        };
        let observed = match self.canonical.get(&principal, session_id).await {
            Ok(observed) => observed,
            Err(AppError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
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
        let common_owner = nomifun_common::UserId::parse(owner_id.to_owned()).map_err(|error| {
            AppError::Forbidden(format!("invalid canonical AgentSession owner: {error}"))
        })?;
        let projected = super::nomi_core_agent_projection::project_saved_artifacts(
            &common_owner,
            binding,
            revision,
            snapshot,
            observed.session.metadata.title.as_deref(),
        )?;
        let workspace = frozen_workspace_root(
            owner_id,
            session_id,
            &projected.binding,
        )?;
        Ok(Some(canonical_conversation_response(
            observed,
            projected,
            workspace,
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
    skill_artifacts: Arc<nomifun_plugin_platform::application::FsPluginArtifactStore>,
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
    robot_owner: Option<Arc<super::nomi_core_robot::NomiCoreRobotWave4Owner>>,
    plugin_runtime:
        Arc<nomifun_plugin_platform::runtime::PluginRuntimeApplicationService>,
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
        robot_owner: Option<Arc<super::nomi_core_robot::NomiCoreRobotWave4Owner>>,
        plugin_runtime: Arc<
            nomifun_plugin_platform::runtime::PluginRuntimeApplicationService,
        >,
        wave2_owner: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
        skill_artifacts: Arc<nomifun_plugin_platform::application::FsPluginArtifactStore>,
        pool: nomifun_db::SqlitePool,
    ) -> Self {
        Self {
            hosted_effects: super::hosted_effect_receipts::HostedEffectReceipts::new(pool),
            skill_artifacts,
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
            plugin_runtime,
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
        let mut runtime_binding = binding;
        if wave2_workspace_selected {
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
            let server_workspace = response
                .extra
                .get("workspace")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::Conflict(
                        "Nomi Wave 2 AgentSession has no server-resolved workspace"
                            .to_owned(),
                    )
                })?;
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
            Arc::clone(&self.skill_artifacts),
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
        let skills = super::engine_skills::compile(&compiled, &registry, self.skill_artifacts.clone()).await?;
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
                skills.instructions, skills.resources,
            ).map_err(|error| AppError::Conflict(error.to_string()))?)
                .map_err(|error| AppError::Conflict(error.to_string()))?
        };
        let plugin_session = if constraints.restricted() { plugin_session } else {
            plugin_session.with_verified_skill_commands(
                Arc::clone(&self.kernel), Arc::clone(&compiled), skills.commands,
            ).map_err(|error| AppError::Conflict(format!("Nomi Skill command materialization failed: {error}")))?
        };
        let robot_capability_ids = super::nomi_core_robot::tool_capability_ids();
        let enabled_robot_ids = compiled
            .content()
            .enabled_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .filter(|capability_id| robot_capability_ids.contains(capability_id))
            .filter(|capability_id| constraints.allows_capability(capability_id.as_ref()))
            .collect::<BTreeSet<_>>();
        let dynamic = if enabled_robot_ids.is_empty()
        {
            None
        } else {
            let owner = self.robot_owner.as_ref().ok_or_else(|| {
                AppError::Conflict(
                    "Nomi Robot Tool owner is unavailable for this Session".to_owned(),
                )
            })?;
            let robot_bindings = compiled
                .target_resource_bindings
                .iter()
                .filter(|binding| binding.resource_kind.as_ref() == "robot")
                .collect::<Vec<_>>();
            let [robot_binding] = robot_bindings.as_slice() else {
                return Err(AppError::Conflict(
                    "Nomi Robot Tools require one exact server-resolved Robot binding"
                        .to_owned(),
                ));
            };
            let (descriptors, invoker) = owner
                .resolve_session_tools(
                    &principal,
                    &session_id,
                    robot_binding,
                    &enabled_robot_ids,
                )
                .await
                .map_err(AppError::Conflict)?;
            Some((descriptors, Arc::new(super::hosted_effect_receipts::RobotReceiptInvoker {
                    receipts: self.hosted_effects.clone(), user: principal.principal_id.clone(),
                    session: session_id.as_ref().to_owned(), delegate: invoker,
                }) as Arc<dyn nomifun_ai_agent::NomiHostDynamicToolInvoker>))
        };
        let plugin_product_actions = if constraints.restricted() { Vec::new() } else {
            KernelNomiPluginToolSession::materialize_plugin_product_actions(
            &compiled,
            &principal,
            &session_id,
            &ScopeKey::from(format!("session:{}", session_id.as_ref())),
            Arc::new(NomiCorePluginProductSchemaResolver {
                application: Arc::clone(&self.plugin_runtime),
            }),
        )
            .await
            .map_err(|error| {
                AppError::Conflict(format!(
                    "Nomi Plugin Tool session materialization failed: {error}"
                ))
            })? };
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
            product: Some((
                plugin_product_actions,
                Arc::new(NomiCorePluginProductToolInvoker {
                    application: Arc::clone(&self.plugin_runtime),
                    owner_user_id: principal.principal_id.clone(),
                    session_id: session_id.as_ref().to_owned(),
                    receipts: self.hosted_effects.clone(),
                }),
            )),
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
    names: Vec<String>,
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
    let mut selection = ExactSessionMcpSelection { ids: Vec::new(), names: Vec::new() };
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
        selection.names.push(server.name);
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

fn install_runtime_mcp_selection(
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
        "mcp_server_ids".to_owned(),
        serde_json::to_value(&selection.ids)?,
    );
    object.insert(
        "mcp_servers".to_owned(),
        Value::Array(
            selection
                .names
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    object.remove("selected_session_mcp_servers");
    object.remove("session_mcp_servers");
    Ok(())
}

struct NomiCorePluginProductSchemaResolver {
    application:
        Arc<nomifun_plugin_platform::runtime::PluginRuntimeApplicationService>,
}

#[async_trait]
impl NomiPluginProductToolSchemaResolver for NomiCorePluginProductSchemaResolver {
    async fn resolve(
        &self,
        owner: &PrincipalRef,
        capability: &ResolvedCapability,
        reference: &nomifun_agent_contracts::CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, String> {
        if owner.principal_kind != "user" {
            return Err("Plugin Agent Tool owner must be a user principal".to_owned());
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

struct NomiCorePluginProductToolInvoker {
    receipts: super::hosted_effect_receipts::HostedEffectReceipts,
    session_id: String,
    application:
        Arc<nomifun_plugin_platform::runtime::PluginRuntimeApplicationService>,
    owner_user_id: String,
}

#[async_trait]
impl NomiPluginProductToolInvoker for NomiCorePluginProductToolInvoker {
    async fn preflight(&self, request: NomiPluginProductToolInvocation) -> Result<(), NomiPluginToolError> {
        let owner = super::engine_plugin_product_tools::PluginProductOwner {
            application: self.application.clone(), receipts: self.receipts.clone(),
        };
        owner.preflight(&self.owner_user_id, request.capability(), &request.action().action_id,
            request.operation_id().clone(), request.input().clone()).await.map_err(|error| match error {
                super::engine_plugin_product_tools::PluginProductCallError::Rejected(message) => NomiPluginToolError::Contract(message),
                super::engine_plugin_product_tools::PluginProductCallError::Unknown(message) => NomiPluginToolError::OutcomeUnknown(message),
            })
    }
    async fn invoke(
        &self,
        request: NomiPluginProductToolInvocation,
    ) -> Result<StrictJsonValue, NomiPluginToolError> {
        let owner = super::engine_plugin_product_tools::PluginProductOwner {
            application: self.application.clone(), receipts: self.receipts.clone(),
        };
        owner.invoke(&self.owner_user_id, &self.session_id, request.capability(),
            &request.action().action_id, request.operation_id().clone(), request.input().clone(),
            nomifun_plugin_platform::runtime::PluginRuntimeCallCancellation::from_shared_flag(request.cancellation().shared_flag()))
            .await.map_err(|error| match error {
                super::engine_plugin_product_tools::PluginProductCallError::Rejected(message) => NomiPluginToolError::Contract(message),
                super::engine_plugin_product_tools::PluginProductCallError::Unknown(message) => NomiPluginToolError::OutcomeUnknown(message),
            })
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
            plugin_product_capabilities: persisted
                .content
                .enabled_capabilities
                .iter()
                .filter(|capability| {
                    capability.contribution_lock.source_kind
                        == ContributionSourceKind::PluginProductActiveRelease
                })
                .cloned()
                .collect(),
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

#[async_trait]
impl nomifun_cron::CronSessionPort for NomiCoreSessionOwner {
    async fn get_session(
        &self,
        query: &nomifun_cron::CronSessionLookup,
    ) -> Result<nomifun_cron::CronSessionProjection, AppError> {
        if let Some(response) = self
            .canonical_conversation_projection(&query.owner_id, &query.agent_session_id)
            .await?
        {
            return cron_session_projection_from_response(&query.owner_id, response, None);
        }
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
        if self
            .canonical_conversation_projection(&request.owner_id, &request.agent_session_id)
            .await?
            .is_some()
        {
            // The Cron repository already committed and CAS-checked the
            // canonical relation in `cron_jobs.conversation_id`. Canonical
            // AgentSession records intentionally carry no mutable Cron
            // back-reference; this port only revalidates exact ownership and
            // saved artifacts after the durable write.
            return Ok(());
        }
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
        let response = self.create_session_idempotent(user_id, request, snapshot, creation_key).await?;
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
                    companion_device_turn: None,
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
        self.service
            .refresh_product_agent_for_existing(owner_id, session_id)
            .await?;
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
        let workspace = frozen_workspace_root(
            owner_id,
            &session_id,
            &observed.session.agent_binding,
        )?
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
    async fn refresh_product_agent(&self, owner_id: &str, session_id: &str) -> Result<ConversationResponse, AppError> {
        self.service.refresh_product_agent_for_existing(owner_id, session_id).await
    }
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
        let agent_session_id = AgentSessionId::from(conversation_id.to_owned());
        nomifun_common::validate_uuidv7(agent_session_id.as_ref()).map_err(|error| {
            AppError::NotFound(format!(
                "AgentSession identity is not canonical UUIDv7: {error}"
            ))
        })?;
        if let Some(response) = self
            .canonical_conversation_projection(owner_id, &agent_session_id)
            .await?
        {
            return Ok(response);
        }
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
        preset_id: None,
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

fn frozen_workspace_root(
    owner_id: &str,
    _session_id: &AgentSessionId,
    binding: &AgentBindingValue,
) -> Result<Option<String>, AppError> {
    let binding: AgentBindingValueDto = serde_json::to_value(binding)
        .and_then(serde_json::from_value)
        .map_err(|error| {
            AppError::Conflict(format!(
                "canonical AgentSession has an invalid frozen binding: {error}"
            ))
        })?;
    nomifun_agent_execution::resolve_frozen_automation_workspace(owner_id, &binding)
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

fn canonical_conversation_response(
    observed: SessionObservation,
    projected: super::nomi_core_agent_projection::NomiCoreSavedBindingProjection,
    workspace: Option<String>,
) -> Result<ConversationResponse, AppError> {
    let SessionObservation { session, head, .. } = observed;
    let super::nomi_core_agent_projection::NomiCoreSavedBindingProjection {
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
    if let Some(workspace) = workspace {
        extra.insert("workspace".to_owned(), Value::String(workspace));
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
        _ => ConversationStatus::Pending,
    };
    let name = request
        .name
        .unwrap_or_else(|| snapshot.preset_name.clone());
    Ok(ConversationResponse {
        conversation_id: session.agent_session_id.as_ref().to_owned(),
        name,
        r#type: request.r#type,
        model: request.model,
        status,
        runtime: None,
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
        linked_execution_id: None,
        execution_step_id: None,
        execution_attempt_id: None,
        // Canonical Session metadata currently has no public timestamp field.
        // Keep the retiring DTO deterministic rather than manufacturing wall
        // clock facts; no canonical consumer treats this projection as time
        // authority.
        created_at: 0,
        modified_at: 0,
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
            workspace_binding_lease: None,
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
        canonical_autowork_config_snapshot, companion_archive_message,
        cron_session_projection_from_response, delete_cleanup_requires_reconciliation,
        frozen_workspace_root,
        parse_canonical_autowork_revision, session_projection_revision, ssh_teardown_loss,
    };
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_agent_contracts::{
        AgentBindingValue, AgentPresetId, AgentSessionId, DigestHex,
        PresetRevisionRef, ResolvedSnapshotId, ResolvedSnapshotRef,
        ResourceBindingId, ResourceId, ResourceKind, TypedResourceBinding,
    };
    use nomifun_api_types::{AgentKnowledgePolicy, AgentResolvedSnapshot, ExecutionModelRef, MessageResponse};
    use nomifun_common::{
        AgentType, ConversationSource, ConversationStatus, DecisionPolicy, DelegationPolicy,
        MessagePosition, MessageType, ProviderWithModel,
    };
    use serde_json::json;

    const SESSION_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
    const OWNER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";

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
            "session_metadata(",
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
    fn domain_session_ports_try_the_canonical_store_before_legacy_fallback() {
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
        assert!(
            cron_get.find("canonical_conversation_projection").unwrap()
                < cron_get.find(".service").unwrap()
        );

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
        assert!(get.contains("self.service.get"));
    }

    #[test]
    fn frozen_workspace_projection_is_exact_and_owner_scoped() {
        let workspace = std::env::temp_dir().join("uarc-canonical-workspace");
        let workspace = workspace.to_string_lossy().into_owned();
        let session_id = AgentSessionId::from(SESSION_ID);
        assert_eq!(
            frozen_workspace_root(
                OWNER_ID,
                &session_id,
                &frozen_binding(&workspace, OWNER_ID),
            )
            .unwrap(),
            Some(workspace)
        );
        let error = frozen_workspace_root(
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
            frozen_workspace_root(OWNER_ID, &session_id, &no_workspace).unwrap(),
            None
        );
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
            .expect("canonical delete must fence Store admission first");
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
const NOMI_CORE_REMOTE_TURN_FINALIZER_TIMEOUT: Duration = Duration::from_secs(90);
const NOMI_CORE_REMOTE_TURN_FINALIZER_POLL: Duration = Duration::from_millis(100);
const NOMI_CORE_REMOTE_INITIAL_COMMAND_TIMEOUT: Duration = Duration::from_secs(150);
const NOMI_CORE_REMOTE_TURN_COMMAND_TIMEOUT: Duration = Duration::from_secs(150);
const NOMI_CORE_REMOTE_CANCEL_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// State shared by the app-local Agent Settings, AgentSession, and Remote
/// route builders.
///
/// `session_owner` is the same object that the normal Conversation, Channel,
/// Cron, AutoWork, Companion, and AgentExecution wiring receives.  The
/// adapter never constructs an AgentRuntimeRegistry or a ConversationService.
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
        .route("/api/agent-sessions", post(create_nomi_core_agent_session))
        .route("/api/runtime-engines", get(list_runtime_engines))
        .route(
            "/api/agent-sessions/{agent_session_id}",
            get(get_nomi_core_agent_session).delete(delete_nomi_core_agent_session),
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
            "/api/agent-sessions/{agent_session_id}/preset",
            put(switch_nomi_core_agent_session_preset),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/capability-selection",
            put(update_nomi_core_agent_session_capability_selection),
        )
        .route(
            "/api/agent-sessions/{agent_session_id}/mcp-selection",
            put(update_nomi_core_agent_session_mcp_selection),
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
    let mut candidates = library.official_templates.iter().map(|template| {
        let key = serde_json::to_value(template.template_key).unwrap().as_str().unwrap().to_owned();
        (ProductAgentSelection::Template { template_key: key.clone() }, key)
    }).collect::<Vec<_>>();
    candidates.extend(library.user_presets.iter().map(|preset| (ProductAgentSelection::Preset { preset_id: preset.preset_id.clone() }, preset.display_name.clone())));
    let selection = match state.product_agent_resolver.selection(&owner, &kind, &id).await? {
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
    let _guard = state.product_agent_resolver.default_binding_lock.lock().await;
    let mut model = request.model;
    if let Some(id) = request.conversation_id.as_deref() {
        let current = state.session_owner.get_session(owner.as_ref(), id).await?;
        let belongs = (current.extra["product_agent_target_kind"] == target_kind && current.extra["product_agent_target_id"] == target_id)
            || (target_kind == "companion" && current.extra["companion_id"] == target_id);
        if !belongs { return Err(NomiCoreApiError::new(StatusCode::FORBIDDEN, "RESOURCE_OWNER_MISMATCH", "conversation belongs to another product target")); }
        if current.status == nomifun_common::ConversationStatus::Running {
            return Err(NomiCoreApiError::new(StatusCode::CONFLICT, "REMOTE_SESSION_BUSY", "wait for the current reply"));
        }
        if let Some(current_model) = current.model {
            model = Some(AgentChatModelSelectionDto { provider_id: current_model.provider_id, model: current_model.model });
        }
    }
    // Validate before any binding/selection mutation, including stale UI requests.
    state.control_plane.validate_product_selection(&owner, selection.template_id(), selection.preset_id(), model.as_ref()).await?;
    let mut response = json!({ "selection": selection, "needs_model": model.is_none() });
    if model.is_some() {
        let mut binding = state.product_agent_resolver.materialize(&owner, &selection, model.as_ref()).await?;
        if let Some(id) = request.conversation_id.as_deref() {
            let (value, revision, snapshot) = state.control_plane.saved_binding_artifacts(&owner, &binding).await?;
            let name = state.control_plane.editor(&owner, &binding.preset_revision_ref.preset_id, None).await?.preset.display_name;
            let projected = super::nomi_core_agent_projection::project_saved_artifacts(&common_owner_id(&owner)?, value, revision, snapshot, Some(&name))?;
            state.session_owner.service().replace_product_agent_resolution(owner.as_ref(), id,
                &ProductAgentTarget { target_kind: target_kind.clone(), target_id: target_id.clone(), default_template_key: default.to_owned() },
                ProductAgentResolution { snapshot: projected.projection.snapshot, runtime_extra: projected.projection.request.extra }).await?;
        }
        let existing = state.control_plane.get_agent_binding(&owner, target_kind.clone(), target_id.clone()).await?;
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

async fn list_runtime_engines(
    State(state): State<NomiCoreAgentApiState>,
) -> Result<Json<ApiResponse<Vec<nomifun_api_types::RuntimeEngineDescriptor>>>, NomiCoreApiError> {
    let host = state.session_owner.runtime_engines.get()
        .ok_or_else(|| AppError::Conflict("Runtime host is not assembled".into()))?;
    Ok(Json(ApiResponse::ok(host.catalog()?.list())))
}

async fn create_nomi_core_agent_session(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    headers: HeaderMap,
    Json(request): Json<CreateAgentSessionRequestDto>,
) -> Result<Json<ApiResponse<CreateAgentSessionResponseDto>>, NomiCoreApiError> {
    if request.capability_selection.as_ref().is_some_and(|selection| {
        !selection.enabled_skills.is_empty()
            || !selection.excluded_auto_skills.is_empty()
            || !selection.mcp_server_ids.is_empty()
    }) {
        return Err(NomiCoreApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "AGENT_SESSION_SELECTION_IS_FROZEN",
            "Session capabilities and resources must be part of the saved Agent binding",
        ));
    }
    let binding = state
        .control_plane
        .resolve_agent_session_binding_with_model(&owner.0, &request.preset_id, request.model.as_ref())
        .await?;
    let binding = state
        .resource_bindings
        .resolve_for_saved_binding(
            &state.control_plane,
            &owner.0,
            binding,
            &request.resource_selections,
        )
        .await?;
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
    let opened = state
        .session_owner
        .canonical()
        .open(
            authenticated_principal(&owner),
            binding_contract,
            request.title.or(Some(agent_name)),
            active_capabilities,
            &creation_key,
            now_ms(),
        )
        .await?;
    Ok(Json(ApiResponse::ok(CreateAgentSessionResponseDto {
        agent_session_id: opened.session.agent_session_id.as_ref().to_owned(),
        agent_binding: binding,
        state: "ready".to_owned(),
        cursor: session_cursor(&opened.session.agent_session_id, opened.cursor.seq),
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionMcpSelectionRequest {
    #[serde(rename = "mcp_server_ids")]
    _mcp_server_ids: Vec<String>,
}

async fn update_nomi_core_agent_session_mcp_selection(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(_request): Json<SessionMcpSelectionRequest>,
) -> Result<Json<ApiResponse<ConversationResponse>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    state.session_owner.canonical().get(&authenticated_principal(&owner), &session_id).await?;
    Err(NomiCoreApiError::new(
        StatusCode::CONFLICT,
        "AGENT_SESSION_BINDING_IMMUTABLE",
        "MCP resources are frozen in the AgentSession binding; fork or create a new Session",
    ))
}

async fn update_nomi_core_agent_session_capability_selection(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(_request): Json<UpdateAgentSessionCapabilitySelectionRequestDto>,
) -> Result<Json<ApiResponse<UpdateAgentSessionCapabilitySelectionResponseDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    state.session_owner.canonical().get(&authenticated_principal(&owner), &session_id).await?;
    Err(NomiCoreApiError::new(
        StatusCode::CONFLICT,
        "AGENT_SESSION_BINDING_IMMUTABLE",
        "Capability grants are frozen in the AgentSession binding; fork or create a new Session",
    ))
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

async fn switch_nomi_core_agent_session_preset(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(_request): Json<SwitchAgentSessionPresetRequestDto>,
) -> Result<Json<ApiResponse<SwitchAgentSessionPresetResponseDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    state.session_owner.canonical().get(&authenticated_principal(&owner), &session_id).await?;
    Err(NomiCoreApiError::new(
        StatusCode::CONFLICT,
        "AGENT_SESSION_BINDING_IMMUTABLE",
        "AgentPreset is frozen for this Session; fork or create a new Session",
    ))
}

async fn start_nomi_core_agent_session_turn(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(request): Json<CreateAgentSessionTurnRequestDto>,
) -> Result<Json<ApiResponse<CreateAgentSessionTurnResponseDto>>, NomiCoreApiError> {
    let result = start_owned_session_turn(&state.session_owner, &owner, &agent_session_id, request).await?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn steer_nomi_core_agent_session_turn(
    State(state): State<NomiCoreAgentApiState>,
    Extension(owner): Extension<AuthenticatedOwner>,
    Path(agent_session_id): Path<String>,
    Json(request): Json<SteerAgentSessionTurnRequestDto>,
) -> Result<Json<ApiResponse<AgentSessionTurnMutationResponseDto>>, NomiCoreApiError> {
    let session_id = parse_agent_session_id(&agent_session_id)?;
    let key = canonical_nonempty(&request.idempotency_key, "idempotency_key")?;
    let input = canonical_turn_input(&bounded_turn_input(request.input)?);
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
    Ok(Json(ApiResponse::ok(AgentSessionTurnMutationResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        target_operation_id: receipt.target_operation_id.as_ref().to_owned(),
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
    let receipt = state
        .session_owner
        .canonical()
        .cancel(&authenticated_principal(&owner), &session_id, &key)
        .await?;
    Ok(Json(ApiResponse::ok(AgentSessionTurnMutationResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        target_operation_id: receipt.target_operation_id.as_ref().to_owned(),
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
) -> Result<CreateAgentSessionTurnResponseDto, NomiCoreApiError> {
    let session_id = parse_agent_session_id(agent_session_id)?;
    let input = canonical_turn_input(&bounded_turn_input(request.input)?);
    let idempotency_key = canonical_nonempty(&request.idempotency_key, "idempotency_key")?;
    let receipt = session_owner
        .canonical()
        .start_turn(
            &authenticated_principal(owner),
            &session_id,
            &idempotency_key,
            input,
        )
        .await?;
    Ok(CreateAgentSessionTurnResponseDto {
        agent_session_id: session_id.as_ref().to_owned(),
        operation_id: receipt.operation_id.as_ref().to_owned(),
        cursor: session_cursor(&session_id, receipt.cursor.seq),
        status: "running".to_owned(),
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
    let next_seq = messages.last().map_or(query.after_seq, |message| message.last_seq);
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
            "ConversationService returned a different Session identity",
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
        message_type: Some(row.r#type.clone()),
        message_status: row.status.clone(),
        projection,
        semantic_digest,
    })
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
    if let Some(preset_id) = object.get("preset_id") {
        request.preset_id = serde_json::from_value(preset_id.clone())?;
        if request.preset_id.as_deref().is_some_and(|value| value.trim().is_empty()) {
            return Err(NomiCoreApiError::new(StatusCode::BAD_REQUEST, "NOMI_CORE_INVALID_REQUEST", "turn preset_id must be non-empty"));
        }
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

fn canonical_turn_input(request: &SendMessageRequest) -> Value {
    json!({
        "preset_id": request.preset_id,
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
        preset_id: None,
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
