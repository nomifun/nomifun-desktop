use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nomifun_agent_contracts::{
    CapabilityConsumer, CapabilityOperationLock, CapabilityRef,
    AgentBindingValue, AgentPreset, AgentPresetId, AgentPresetRevision, AgentPresetSource,
    CapabilitySelection, ExactVersionRef, OfficialPresetKey, PresetRevisionRef, RemoteBinding,
    RemoteBindingId, UserId, compare_revision_contribution_locks,
};
use nomifun_api_types::{
    AgentBindingRecordDto, AgentBindingSummaryDto, AgentBindingTargetDto, AgentBindingValueDto,
    AgentCatalogResponse, AgentPresetDraftDto, AgentPresetEditorResponse, AgentPresetLibraryResponse,
    AgentPresetRevisionImpactResponse, AgentPresetSummaryDto,
    CreateAgentPresetFromTemplateRequest, CreateAgentPresetRequest, CreateRemoteBindingRequest,
    EditorDraftStateDto, ExactCatalogRefDto, FreshStartPresentationDto, PutAgentBindingRequest,
    RemoteBindingDto, RevisionImpactConsumerDto, RevisionImpactConsumerKindDto,
    ResolveAgentPresetPreviewRequest, ResolveAgentPresetPreviewResponse,
    ResolveSavedRevisionPreviewRequest, SaveAgentPresetRevisionRequest,
    SaveAgentPresetRevisionResponse, UpdateRemoteBindingRequest,
};
use serde_json::json;
use uuid::Uuid;

use crate::catalog::{CatalogProvider, OfficialTemplateCatalog};
use crate::compiler::{PresetPreviewCompiler, revision_api};
use crate::continuation::editor_test_plan;
use crate::error::ControlPlaneError;
use crate::impact::{
    ControlPlaneRevisionImpactCatalogProvider, RevisionImpactCatalogProvider,
};
use crate::store::{
    AgentBindingTarget, ControlPlaneStore, StoredAgentBinding, StoredPreset,
};
use crate::wire::wire_cast;

const SETTINGS_SCENE: &str = "agent_settings";
const SETTINGS_SURFACE: &str = "desktop";
const SETTINGS_AUDIENCE: &str = "owner";
const CHAT_MODEL_TASK: &str = nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT;

/// Host-owned source for the initial Chat route shown by the product editor.
///
/// The control-plane contract deliberately does not know how a host stores
/// provider credentials or model capabilities.  A host may inject this
/// resolver to materialize a complete, opaque [`ChatRouteRecord`] when a new
/// preset is created.  Returning `None` is an honest "no usable Chat route is
/// configured" result; callers then receive the normal blocked preview rather
/// than a fabricated route or a raw JSON escape hatch.
#[async_trait::async_trait]
pub trait DefaultChatRouteResolver: Send + Sync {
    async fn resolve_default_chat_route(
        &self,
        owner: &UserId,
    ) -> Result<Option<nomifun_agent_contracts::ChatRouteRecord>, ControlPlaneError>;
}

pub struct AgentControlPlane {
    store: Arc<dyn ControlPlaneStore>,
    catalog: Arc<dyn CatalogProvider>,
    impact_catalog: Arc<dyn RevisionImpactCatalogProvider>,
    templates: OfficialTemplateCatalog,
    compiler: PresetPreviewCompiler,
    default_chat_route_resolver: Option<Arc<dyn DefaultChatRouteResolver>>,
}

impl AgentControlPlane {
    pub fn new(
        store: Arc<dyn ControlPlaneStore>,
        catalog: Arc<dyn CatalogProvider>,
        templates: OfficialTemplateCatalog,
        compiler: PresetPreviewCompiler,
    ) -> Self {
        let impact_catalog = Arc::new(ControlPlaneRevisionImpactCatalogProvider::new(
            Arc::clone(&catalog),
        ));
        Self {
            store,
            catalog,
            impact_catalog,
            templates,
            compiler,
            default_chat_route_resolver: None,
        }
    }

    /// Inject the host-owned Chat route materializer used for new product
    /// presets.  The resolver is consulted only when the request contains no
    /// Chat route at all; explicit route records remain caller-owned and are
    /// validated by the canonical revision/compiler contract.
    pub fn with_default_chat_route_resolver(
        mut self,
        resolver: Arc<dyn DefaultChatRouteResolver>,
    ) -> Self {
        self.default_chat_route_resolver = Some(resolver);
        self
    }

    /// Replace only the read-side lifecycle catalog used by AP-5 impact
    /// inspection. This cannot mutate a Revision and does not affect Compiler
    /// source resolution.
    pub fn with_revision_impact_catalog_provider(
        mut self,
        provider: Arc<dyn RevisionImpactCatalogProvider>,
    ) -> Self {
        self.impact_catalog = provider;
        self
    }

    pub async fn library(
        &self,
        owner: &UserId,
    ) -> Result<AgentPresetLibraryResponse, ControlPlaneError> {
        let presets = self.store.list_presets(owner).await?;
        let agent_bindings = self.store.list_agent_bindings(owner).await?;
        let remote_bindings = self.store.list_remote_bindings(owner).await?;
        let mut bound_counts: BTreeMap<AgentPresetId, u32> = BTreeMap::new();
        for binding in &agent_bindings {
            *bound_counts
                .entry(binding.value.preset_revision_ref.preset_id.clone())
                .or_default() += 1;
        }
        for binding in &remote_bindings {
            *bound_counts
                .entry(
                    binding
                        .agent_binding
                        .preset_revision_ref
                        .preset_id
                        .clone(),
                )
                .or_default() += 1;
        }

        let user_presets = presets
            .iter()
            .map(|preset| {
                preset_summary(
                    preset,
                    bound_counts
                        .get(&preset.preset.preset_id)
                        .copied()
                        .unwrap_or_default(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let active_bindings = agent_bindings
            .iter()
            .map(binding_summary)
            .collect::<Result<Vec<_>, _>>()?;
        let official_templates = self.templates.list()?;
        Ok(AgentPresetLibraryResponse {
            fresh_start: FreshStartPresentationDto {
                data_generation: 4,
                legacy_data_imported: false,
                official_template_count: official_templates.len() as u32,
                user_preset_count: user_presets.len() as u32,
            },
            official_templates,
            user_presets,
            active_bindings,
        })
    }

    pub fn catalog(&self) -> Result<AgentCatalogResponse, ControlPlaneError> {
        self.catalog.snapshot()?.as_api()
    }

    pub fn resolve_capability(
        &self,
        reference: &CapabilityRef,
        consumer: CapabilityConsumer,
    ) -> Result<CapabilityOperationLock, ControlPlaneError> {
        let snapshot = self.catalog.snapshot()?;
        let entry = snapshot
            .capability_catalog_entry(reference)?
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "CAPABILITY_NOT_MATERIALIZED",
                    axum::http::StatusCode::NOT_FOUND,
                    format!(
                        "capability {}@{} is not materialized",
                        reference.id.as_ref(),
                        reference.version.as_ref()
                    ),
                )
            })?;
        entry.operation_lock(consumer).map_err(|error| {
            ControlPlaneError::canonical(
                error.code(),
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                error.to_string(),
            )
        })
    }


    pub async fn create_preset(
        &self,
        owner: &UserId,
        request: CreateAgentPresetRequest,
    ) -> Result<AgentPresetEditorResponse, ControlPlaneError> {
        let display_name = nonempty_name(request.display_name)?;
        let preset_id = AgentPresetId::from(Uuid::now_v7().to_string());
        if request.document.is_some() && request.fork_from_revision.is_some() {
            return Err(ControlPlaneError::canonical(
                "PRESET_CREATE_INVALID",
                axum::http::StatusCode::BAD_REQUEST,
                "choose either an edited configuration or a saved revision to copy",
            ));
        }
        if let Some(reference) = request.fork_from_revision {
            let reference: PresetRevisionRef = wire_cast(&reference)?;
            let revision = self
                .store
                .get_revision(&reference)
                .await?
                .ok_or_else(|| not_found("AgentPresetRevision"))?;
            let source = self
                .store
                .get_preset(&reference.preset_id)
                .await?
                .ok_or_else(|| not_found("AgentPreset"))?;
            if source.preset.owner_user_id.as_ref() != Some(owner) {
                return Err(not_found("AgentPresetRevision"));
            }
            return self
                .create_with_initial_revision(
                    owner,
                    preset_id,
                    display_name,
                    request.description,
                    wire_cast(&revision.payload)?,
                    None,
                )
                .await;
        }
        let has_configuration = request.document.is_some();
        let document = self
            .materialize_default_chat_route(owner, request.document.unwrap_or_else(empty_document))
            .await?;
        if document.model_route_refs.is_empty() {
            if has_configuration {
                return Err(ControlPlaneError::canonical(
                    "MODEL_ROUTE_NOT_CONFIGURED",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "configure an available chat model before saving this Agent",
                ));
            }
            let stored = StoredPreset {
                preset: AgentPreset {
                    preset_id,
                    owner_user_id: Some(owner.clone()),
                    source: AgentPresetSource::User,
                    display_name,
                    description: request.description,
                    current_stable_revision: None,
                },
            };
            self.store.insert_preset(stored.clone()).await?;
            return editor_response(stored, None, document, None);
        }
        // A host-provided default route is a real immutable initial Revision,
        // not editor-only metadata.  This makes a newly-created product
        // preset immediately selectable without asking the user to type route
        // IDs or JSON.
        self.create_with_initial_revision(
            owner,
            preset_id,
            display_name,
            request.description,
            document,
            None,
        )
        .await
    }

    pub async fn retire_preset(
        &self,
        owner: &UserId,
        preset_id: &str,
    ) -> Result<(), ControlPlaneError> {
        self.store
            .retire_preset(owner, &AgentPresetId::from(preset_id.to_owned()))
            .await
    }

    pub async fn create_from_template(
        &self,
        owner: &UserId,
        template_id: &str,
        request: CreateAgentPresetFromTemplateRequest,
    ) -> Result<AgentPresetEditorResponse, ControlPlaneError> {
        let template_key = parse_official_key(template_id)
            .ok_or_else(|| not_found("OfficialPresetTemplate"))?;
        let seed = self
            .templates
            .seed(template_key)
            .ok_or_else(|| not_found("OfficialPresetTemplate"))?;
        let display_name = nonempty_name(request.display_name)?;
        let mut model_route_refs = request.model_route_refs;
        let mut chat_route_records = request.chat_route_records;
        if !model_route_refs.contains_key(CHAT_MODEL_TASK)
            && !chat_route_records.contains_key(CHAT_MODEL_TASK)
            && let Some(record) =
                self.resolve_default_chat_route(owner).await?
        {
            model_route_refs.insert(
                CHAT_MODEL_TASK.to_owned(),
                record.primary.model_route_id.as_ref().to_owned(),
            );
            chat_route_records.insert(
                CHAT_MODEL_TASK.to_owned(),
                serde_json::to_value(record)?,
            );
        }
        let document = nomifun_api_types::AgentPresetDocumentDto {
            schema_version: "1.0.0".into(),
            model_route_refs,
            chat_route_records,
            initial_capabilities: seed
                .initial_capabilities
                .iter()
                .map(selection_api)
                .collect::<Result<Vec<_>, _>>()?,
            on_demand_capabilities: seed
                .on_demand_capabilities
                .iter()
                .map(selection_api)
                .collect::<Result<Vec<_>, _>>()?,
            skill_bindings: seed
                .skill_bindings
                .iter()
                .map(exact_ref_api)
                .collect(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: String::new(),
            instructions: String::new(),
            starter_prompts: Vec::new(),
        };
        self.create_with_initial_revision(
            owner,
            AgentPresetId::from(Uuid::now_v7().to_string()),
            display_name,
            request.description,
            document,
            Some(template_key),
        )
        .await
    }

    async fn resolve_default_chat_route(
        &self,
        owner: &UserId,
    ) -> Result<Option<nomifun_agent_contracts::ChatRouteRecord>, ControlPlaneError> {
        let Some(resolver) = &self.default_chat_route_resolver else {
            return Ok(None);
        };
        let route = resolver.resolve_default_chat_route(owner).await?;
        if let Some(route) = &route {
            route.validate().map_err(|error| {
                ControlPlaneError::canonical(
                    "MODEL_ROUTE_RECORD_INVALID",
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    format!("host default Chat route is invalid: {error}"),
                )
            })?;
        }
        Ok(route)
    }

    async fn materialize_default_chat_route(
        &self,
        owner: &UserId,
        mut document: nomifun_api_types::AgentPresetDocumentDto,
    ) -> Result<nomifun_api_types::AgentPresetDocumentDto, ControlPlaneError> {
        if let Some(record) = self.resolve_default_chat_route(owner).await? {
            document.model_route_refs.insert(
                CHAT_MODEL_TASK.to_owned(),
                record.primary.model_route_id.as_ref().to_owned(),
            );
            document.chat_route_records.insert(
                CHAT_MODEL_TASK.to_owned(),
                serde_json::to_value(record)?,
            );
        }
        Ok(document)
    }

    pub async fn editor(
        &self,
        owner: &UserId,
        preset_id: &str,
        revision_number: Option<u64>,
    ) -> Result<AgentPresetEditorResponse, ControlPlaneError> {
        let stored = self.owned_preset(owner, preset_id).await?;
        let revision = match revision_number {
            Some(number) => self
                .store
                .get_revision_number(&stored.preset.preset_id, number)
                .await?,
            None => match stored.preset.current_stable_revision.as_ref() {
                Some(reference) => self.store.get_revision(reference).await?,
                None => None,
            },
        };
        let document = revision
            .as_ref()
            .map(|revision| wire_cast(&revision.payload))
            .transpose()?
            .unwrap_or_else(empty_document);
        editor_response(stored, revision, document, None)
    }

    pub async fn preview(
        &self,
        owner: &UserId,
        preset_id: &str,
        request: ResolveAgentPresetPreviewRequest,
    ) -> Result<ResolveAgentPresetPreviewResponse, ControlPlaneError> {
        if request.draft.preset_id != preset_id {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::BAD_REQUEST,
                "draft preset_id must match the route preset_id",
            ));
        }
        let stored = self.owned_preset(owner, preset_id).await?;
        let current = self.current_revision(&stored).await?;
        let current_snapshot = self.current_snapshot(current.as_ref()).await?;
        ensure_expected_current(
            stored.preset.current_stable_revision.as_ref(),
            request.expected_current_revision.as_ref(),
        )?;
        let catalog = self.catalog.snapshot()?;
        let transient_template_key = request
            .draft
            .source_template_key
            .map(|key| wire_cast(&key))
            .transpose()?;
        Ok(self
            .compiler
            .compile(
                owner,
                &request,
                current.as_ref(),
                current_snapshot.as_ref(),
                transient_template_key,
                &catalog,
            )?
            .response)
    }

    pub async fn save_revision(
        &self,
        owner: &UserId,
        preset_id: &str,
        request: SaveAgentPresetRevisionRequest,
    ) -> Result<SaveAgentPresetRevisionResponse, ControlPlaneError> {
        if request.draft.preset_id != preset_id {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::BAD_REQUEST,
                "draft preset_id must match the route preset_id",
            ));
        }
        let stored = self.owned_preset(owner, preset_id).await?;
        ensure_expected_current(
            stored.preset.current_stable_revision.as_ref(),
            request.expected_current_revision.as_ref(),
        )?;
        let current = self.current_revision(&stored).await?;
        let current_snapshot = self.current_snapshot(current.as_ref()).await?;
        let preview_request = ResolveAgentPresetPreviewRequest {
            expected_current_revision: request.expected_current_revision.clone(),
            draft: request.draft.clone(),
            scene: SETTINGS_SCENE.into(),
            surface: SETTINGS_SURFACE.into(),
            audience: SETTINGS_AUDIENCE.into(),
        };
        let catalog = self.catalog.snapshot()?;
        let transient_template_key = request
            .draft
            .source_template_key
            .map(|key| wire_cast(&key))
            .transpose()?;
        let compilation = self.compiler.compile(
            owner,
            &preview_request,
            current.as_ref(),
            current_snapshot.as_ref(),
            transient_template_key,
            &catalog,
        )?;
        if compilation.response.preview_digest != request.preview_digest {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::CONFLICT,
                "preview_digest is stale for the submitted draft",
            ));
        }
        let snapshot = compilation.snapshot.ok_or_else(|| {
            ControlPlaneError::with_details(
                "PRESET_REVISION_SAVE_FAILED",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "Preview is blocked; no immutable Revision or Session was created",
                json!({ "diagnostics": compilation.response.diagnostics }),
            )
        })?;
        if current
            .as_ref()
            .is_some_and(|revision| revision.reference == compilation.candidate_revision_ref)
        {
            let current = current.expect("clean compilation has a current Revision");
            return Ok(SaveAgentPresetRevisionResponse {
                preset: preset_summary(
                    &stored,
                    self.bound_count(owner, &stored.preset.preset_id).await?,
                )?,
                revision: revision_api(&current)?,
                resolved_snapshot_ref: wire_cast(&snapshot.snapshot_ref)?,
                preview_digest: compilation.response.preview_digest,
            });
        }
        let revision = AgentPresetRevision {
            reference: compilation.candidate_revision_ref,
            payload: compilation.payload,
            contribution_locks: compilation.contribution_locks,
            created_by: owner.clone(),
            created_at_ms: snapshot.created_at_ms,
            reason: request.reason,
        };
        revision.validate().map_err(|violation| {
            ControlPlaneError::canonical(
                violation.code,
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                violation.message,
            )
        })?;
        let stored = self
            .store
            .append_revision(
                stored.preset.current_stable_revision.as_ref(),
                revision.clone(),
                snapshot.clone(),
                request.draft.display_name,
                request.draft.description,
            )
            .await?;
        Ok(SaveAgentPresetRevisionResponse {
            preset: preset_summary(&stored, self.bound_count(owner, &stored.preset.preset_id).await?)?,
            revision: revision_api(&revision)?,
            resolved_snapshot_ref: wire_cast(&snapshot.snapshot_ref)?,
            preview_digest: compilation.response.preview_digest,
        })
    }

    pub async fn get_revision(
        &self,
        owner: &UserId,
        preset_id: &str,
        revision_number: u64,
    ) -> Result<nomifun_api_types::AgentPresetRevisionDto, ControlPlaneError> {
        let stored = self.owned_preset(owner, preset_id).await?;
        let revision = self
            .store
            .get_revision_number(&stored.preset.preset_id, revision_number)
            .await?
            .ok_or_else(|| not_found("AgentPresetRevision"))?;
        revision_api(&revision)
    }

    /// Compare one immutable, owner-scoped Revision's server-generated
    /// ContributionLock set with the current formal Catalog and list current
    /// owner-scoped bindings that still reference that Revision.
    pub async fn revision_impact(
        &self,
        owner: &UserId,
        preset_id: &str,
        revision_number: u64,
    ) -> Result<AgentPresetRevisionImpactResponse, ControlPlaneError> {
        let stored = self.owned_preset(owner, preset_id).await?;
        let revision = self
            .store
            .get_revision_number(&stored.preset.preset_id, revision_number)
            .await?
            .ok_or_else(|| not_found("AgentPresetRevision"))?;
        let current_catalog = self.impact_catalog.current_contributions()?;
        let diff = compare_revision_contribution_locks(
            &revision.contribution_locks,
            &current_catalog,
        )
        .map_err(|error| {
            ControlPlaneError::canonical(
                "PRESET_CONTRIBUTION_LOCK_INVALID",
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("revision impact comparison failed closed: {error}"),
            )
        })?;

        let mut affected_consumers = Vec::new();
        for binding in self.store.list_agent_bindings(owner).await? {
            if binding.value.preset_revision_ref == revision.reference {
                affected_consumers.push(RevisionImpactConsumerDto {
                    kind: RevisionImpactConsumerKindDto::AgentBinding,
                    consumer_id: format!(
                        "{}:{}",
                        binding.target.target_kind, binding.target.target_id
                    ),
                    target_kind: Some(binding.target.target_kind),
                    binding_version: binding.value.binding_version,
                });
            }
        }
        for binding in self.store.list_remote_bindings(owner).await? {
            if binding.agent_binding.preset_revision_ref == revision.reference {
                affected_consumers.push(RevisionImpactConsumerDto {
                    kind: RevisionImpactConsumerKindDto::RemoteBinding,
                    consumer_id: binding.remote_binding_id.as_ref().to_owned(),
                    target_kind: None,
                    binding_version: binding.agent_binding.binding_version,
                });
            }
        }
        affected_consumers.sort_by(|left, right| {
            let left_kind = match left.kind {
                RevisionImpactConsumerKindDto::AgentBinding => 0,
                RevisionImpactConsumerKindDto::RemoteBinding => 1,
            };
            let right_kind = match right.kind {
                RevisionImpactConsumerKindDto::AgentBinding => 0,
                RevisionImpactConsumerKindDto::RemoteBinding => 1,
            };
            (left_kind, left.consumer_id.as_str())
                .cmp(&(right_kind, right.consumer_id.as_str()))
        });

        Ok(AgentPresetRevisionImpactResponse {
            preset_revision_ref: wire_cast(&revision.reference)?,
            catalog_digest: diff.catalog_digest.as_ref().to_owned(),
            status: wire_cast(&diff.status)?,
            summary: wire_cast(&diff.summary)?,
            contributions: wire_cast(&diff.contributions)?,
            affected_consumers,
        })
    }

    /// Load the immutable Snapshot attached to one owner-scoped Preset
    /// revision.  This is a read-only control-plane operation used by
    /// Session/capability projections; it never resolves a newer catalog
    /// revision or creates a second Snapshot.
    pub async fn saved_snapshot(
        &self,
        owner: &UserId,
        preset_id: &str,
        revision_number: u64,
    ) -> Result<nomifun_agent_contracts::ResolvedSnapshotEnvelope, ControlPlaneError> {
        let stored = self.owned_preset(owner, preset_id).await?;
        let revision = self
            .store
            .get_revision_number(&stored.preset.preset_id, revision_number)
            .await?
            .ok_or_else(|| not_found("AgentPresetRevision"))?;
        self.store
            .get_snapshot(&revision.reference)
            .await?
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "CAPABILITY_NOT_MATERIALIZED",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "saved Agent Preset revision has no immutable Snapshot",
                )
            })
    }

    /// Load the immutable artifacts frozen into an existing target binding.
    ///
    /// Product retirement closes authoring and new-binding admission, but it
    /// must not erase or hide the exact Revision/Snapshot already referenced by
    /// a Session, Remote session, companion, or automation history record.
    pub async fn saved_binding_artifacts(
        &self,
        owner: &UserId,
        binding: &AgentBindingValueDto,
    ) -> Result<
        (
            AgentBindingValue,
            AgentPresetRevision,
            nomifun_agent_contracts::ResolvedSnapshotEnvelope,
        ),
        ControlPlaneError,
    > {
        let binding: AgentBindingValue = wire_cast(binding)?;
        let (revision, snapshot) = self
            .load_binding_artifacts(owner, &binding)
            .await?;
        Ok((binding, revision, snapshot))
    }

    /// Freeze the authenticated owner's current stable Preset revision into a
    /// Session binding using only the persisted immutable Snapshot.
    pub async fn resolve_agent_session_binding(
        &self,
        owner: &UserId,
        preset_id: &str,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let stored = self
            .store
            .get_preset(&AgentPresetId::from(preset_id.to_owned()))
            .await?
            .ok_or_else(|| not_found("AgentPreset"))?;
        if stored.preset.owner_user_id.as_ref() != Some(owner) {
            return Err(ControlPlaneError::canonical(
                "RESOURCE_OWNER_MISMATCH",
                axum::http::StatusCode::FORBIDDEN,
                "AgentPreset owner does not match the authenticated owner",
            ));
        }
        let stable = stored
            .preset
            .current_stable_revision
            .clone()
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "CAPABILITY_NOT_MATERIALIZED",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "AgentPreset has no current stable Revision for Session creation",
                )
            })?;
        let revision = self
            .store
            .get_revision(&stable)
            .await?;
        let snapshot = self
            .store
            .get_snapshot(&stable)
            .await?;
        let binding = session_binding_from_stable_artifacts(stable, revision, snapshot)?;
        self.validate_agent_binding(owner, &binding).await?;
        wire_cast(&binding)
    }

    pub async fn preview_saved_revision(
        &self,
        owner: &UserId,
        preset_id: &str,
        revision_number: u64,
        request: ResolveSavedRevisionPreviewRequest,
    ) -> Result<ResolveAgentPresetPreviewResponse, ControlPlaneError> {
        let stored = self.owned_preset(owner, preset_id).await?;
        let revision = self
            .store
            .get_revision_number(&stored.preset.preset_id, revision_number)
            .await?
            .ok_or_else(|| not_found("AgentPresetRevision"))?;
        let current_snapshot = self
            .store
            .get_snapshot(&revision.reference)
            .await?
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "CAPABILITY_NOT_MATERIALIZED",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "saved Revision has no persisted ResolvedSnapshotRef",
                )
            })?;
        let preview_request = ResolveAgentPresetPreviewRequest {
            expected_current_revision: Some(wire_cast(&revision.reference)?),
            draft: draft_api(&stored, Some(&revision), wire_cast(&revision.payload)?, None)?,
            scene: request.scene,
            surface: request.surface,
            audience: request.audience,
        };
        let catalog = self.catalog.snapshot()?;
        Ok(self
            .compiler
            .compile(
                owner,
                &preview_request,
                Some(&revision),
                Some(&current_snapshot),
                None,
                &catalog,
            )?
            .response)
    }

    pub async fn get_agent_binding(
        &self,
        owner: &UserId,
        target_kind: String,
        target_id: String,
    ) -> Result<Option<AgentBindingRecordDto>, ControlPlaneError> {
        let target = AgentBindingTarget {
            target_kind,
            target_id,
        };
        let binding = self.store.get_agent_binding(&target).await?;
        binding
            .filter(|binding| &binding.owner_user_id == owner)
            .map(binding_record_api)
            .transpose()
    }

    pub async fn put_agent_binding(
        &self,
        owner: &UserId,
        target_kind: String,
        target_id: String,
        request: PutAgentBindingRequest,
    ) -> Result<AgentBindingRecordDto, ControlPlaneError> {
        let value: AgentBindingValue = wire_cast(&request.agent_binding)?;
        self.validate_agent_binding(owner, &value).await?;
        let target = AgentBindingTarget {
            target_kind,
            target_id,
        };
        let existing = self.store.get_agent_binding(&target).await?;
        let next_version = existing
            .as_ref()
            .map(|binding| binding.value.binding_version + 1)
            .unwrap_or(1);
        if value.binding_version != next_version {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::CONFLICT,
                format!("next binding_version must be {next_version}"),
            ));
        }
        let stored = self
            .store
            .put_agent_binding(
                StoredAgentBinding {
                    target,
                    owner_user_id: owner.clone(),
                    value,
                },
                request.expected_binding_version,
            )
            .await?;
        binding_record_api(stored)
    }

    pub async fn list_remote_bindings(
        &self,
        owner: &UserId,
    ) -> Result<Vec<RemoteBindingDto>, ControlPlaneError> {
        self.store
            .list_remote_bindings(owner)
            .await?
            .iter()
            .map(wire_cast)
            .collect()
    }

    pub async fn get_remote_binding(
        &self,
        owner: &UserId,
        binding_id: &str,
    ) -> Result<Option<RemoteBindingDto>, ControlPlaneError> {
        self.owned_remote_binding(owner, binding_id)
            .await?
            .map(|binding| wire_cast(&binding))
            .transpose()
    }

    pub async fn create_remote_binding(
        &self,
        owner: &UserId,
        request: CreateRemoteBindingRequest,
    ) -> Result<RemoteBindingDto, ControlPlaneError> {
        let agent_binding: AgentBindingValue = wire_cast(&request.agent_binding)?;
        self.validate_agent_binding(owner, &agent_binding).await?;
        let binding = RemoteBinding {
            remote_binding_id: RemoteBindingId::from(Uuid::now_v7().to_string()),
            owner_user_id: owner.clone(),
            name: nonempty_name(request.name)?,
            agent_binding,
        };
        wire_cast(&self.store.insert_remote_binding(binding).await?)
    }

    pub async fn update_remote_binding(
        &self,
        owner: &UserId,
        binding_id: &str,
        request: UpdateRemoteBindingRequest,
    ) -> Result<RemoteBindingDto, ControlPlaneError> {
        let existing = self
            .owned_remote_binding(owner, binding_id)
            .await?
            .ok_or_else(|| not_found("RemoteBinding"))?;
        let agent_binding: AgentBindingValue = wire_cast(&request.agent_binding)?;
        self.validate_agent_binding(owner, &agent_binding).await?;
        let updated = RemoteBinding {
            remote_binding_id: existing.remote_binding_id,
            owner_user_id: owner.clone(),
            name: nonempty_name(request.name)?,
            agent_binding,
        };
        wire_cast(
            &self
                .store
                .update_remote_binding(
                    updated,
                    request.expected_binding_version,
                    &request.expected_agent_binding_digest,
                )
                .await?,
        )
    }

    pub async fn delete_remote_binding(
        &self,
        owner: &UserId,
        binding_id: &str,
    ) -> Result<(), ControlPlaneError> {
        self.store
            .delete_remote_binding(owner, &RemoteBindingId::from(binding_id.to_owned()))
            .await
    }

    pub fn build_editor_test_plan(
        &self,
        draft_state: EditorDraftStateDto,
        preview: ResolveAgentPresetPreviewResponse,
        draft: AgentPresetDraftDto,
        reason: Option<String>,
    ) -> Result<nomifun_api_types::AgentPresetEditorTestPlanDto, ControlPlaneError> {
        editor_test_plan(draft_state, preview, draft, reason)
    }

    async fn owned_remote_binding(
        &self,
        owner: &UserId,
        binding_id: &str,
    ) -> Result<Option<RemoteBinding>, ControlPlaneError> {
        Ok(self
            .store
            .get_remote_binding(&RemoteBindingId::from(binding_id.to_owned()))
            .await?
            .filter(|binding| &binding.owner_user_id == owner))
    }

    async fn create_with_initial_revision(
        &self,
        owner: &UserId,
        preset_id: AgentPresetId,
        display_name: String,
        description: Option<String>,
        document: nomifun_api_types::AgentPresetDocumentDto,
        transient_template_key: Option<OfficialPresetKey>,
    ) -> Result<AgentPresetEditorResponse, ControlPlaneError> {
        let draft = AgentPresetDraftDto {
            preset_id: preset_id.as_ref().to_owned(),
            display_name: display_name.clone(),
            description: description.clone(),
            source_template_key: transient_template_key
                .map(|key| wire_cast(&key))
                .transpose()?,
            current_revision: None,
            document: document.clone(),
        };
        let preview_request = ResolveAgentPresetPreviewRequest {
            expected_current_revision: None,
            draft,
            scene: SETTINGS_SCENE.into(),
            surface: SETTINGS_SURFACE.into(),
            audience: SETTINGS_AUDIENCE.into(),
        };
        let catalog = self.catalog.snapshot()?;
        let compilation = self.compiler.compile(
            owner,
            &preview_request,
            None,
            None,
            transient_template_key,
            &catalog,
        )?;
        let snapshot = compilation.snapshot.ok_or_else(|| {
            ControlPlaneError::with_details(
                "PRESET_REVISION_SAVE_FAILED",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "template expansion did not pass compiler validation",
                json!({ "diagnostics": compilation.response.diagnostics }),
            )
        })?;
        let revision = AgentPresetRevision {
            reference: compilation.candidate_revision_ref,
            payload: compilation.payload,
            contribution_locks: compilation.contribution_locks,
            created_by: owner.clone(),
            created_at_ms: snapshot.created_at_ms,
            reason: Some("Initial Revision".into()),
        };
        revision.validate().map_err(|violation| {
            ControlPlaneError::canonical(
                violation.code,
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                violation.message,
            )
        })?;
        let canonical_document: nomifun_api_types::AgentPresetDocumentDto =
            wire_cast(&revision.payload)?;
        let stored = StoredPreset {
            preset: AgentPreset {
                preset_id,
                owner_user_id: Some(owner.clone()),
                source: AgentPresetSource::User,
                display_name,
                description,
                current_stable_revision: Some(revision.reference.clone()),
            },
        };
        let stored = self
            .store
            .insert_preset_with_revision(stored, revision.clone(), snapshot)
            .await?;
        editor_response(
            stored,
            Some(revision),
            canonical_document,
            transient_template_key,
        )
    }

    async fn owned_preset(
        &self,
        owner: &UserId,
        preset_id: &str,
    ) -> Result<StoredPreset, ControlPlaneError> {
        let preset = self
            .store
            .get_preset(&AgentPresetId::from(preset_id.to_owned()))
            .await?
            .ok_or_else(|| not_found("AgentPreset"))?;
        if preset.preset.owner_user_id.as_ref() != Some(owner) {
            return Err(not_found("AgentPreset"));
        }
        Ok(preset)
    }

    async fn current_revision(
        &self,
        preset: &StoredPreset,
    ) -> Result<Option<AgentPresetRevision>, ControlPlaneError> {
        match preset.preset.current_stable_revision.as_ref() {
            Some(reference) => self.store.get_revision(reference).await,
            None => Ok(None),
        }
    }

    async fn current_snapshot(
        &self,
        revision: Option<&AgentPresetRevision>,
    ) -> Result<Option<nomifun_agent_contracts::ResolvedSnapshotEnvelope>, ControlPlaneError> {
        match revision {
            Some(revision) => self.store.get_snapshot(&revision.reference).await,
            None => Ok(None),
        }
    }

    async fn validate_agent_binding(
        &self,
        owner: &UserId,
        binding: &AgentBindingValue,
    ) -> Result<(), ControlPlaneError> {
        let _ = self.load_binding_artifacts(owner, binding).await?;
        let preset = self
            .store
            .get_preset(&binding.preset_revision_ref.preset_id)
            .await?
            .ok_or_else(|| not_found("AgentPreset"))?;
        if preset.preset.owner_user_id.as_ref() != Some(owner)
            || preset.preset.source != AgentPresetSource::User
        {
            return Err(not_found("AgentPreset"));
        }
        Ok(())
    }

    async fn load_binding_artifacts(
        &self,
        owner: &UserId,
        binding: &AgentBindingValue,
    ) -> Result<
        (
            AgentPresetRevision,
            nomifun_agent_contracts::ResolvedSnapshotEnvelope,
        ),
        ControlPlaneError,
    > {
        if binding
            .typed_resource_bindings
            .iter()
            .any(|resource| resource.owner_id != owner.as_ref())
        {
            return Err(ControlPlaneError::canonical(
                "RESOURCE_OWNER_MISMATCH",
                axum::http::StatusCode::FORBIDDEN,
                "typed resource binding owner does not match the authenticated owner",
            ));
        }
        let revision = self
            .store
            .get_revision(&binding.preset_revision_ref)
            .await?
            .ok_or_else(|| not_found("AgentPresetRevision"))?;
        if revision.created_by != *owner {
            return Err(not_found("AgentPresetRevision"));
        }
        revision.validate().map_err(|violation| {
            ControlPlaneError::canonical(
                violation.code,
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                violation.message,
            )
        })?;
        let snapshot = self
            .store
            .get_snapshot(&binding.preset_revision_ref)
            .await?
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "CAPABILITY_NOT_MATERIALIZED",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "resolved Snapshot is missing for the exact Preset revision",
                )
            })?;
        snapshot.validate().map_err(|violation| {
            ControlPlaneError::canonical(
                violation.code,
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                violation.message,
            )
        })?;
        if snapshot.actor.principal_kind != "user"
            || snapshot.actor.principal_id != owner.as_ref()
        {
            return Err(not_found("AgentPresetRevision"));
        }
        if snapshot.snapshot_ref != binding.resolved_snapshot_ref {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::CONFLICT,
                "ResolvedSnapshotRef does not match the saved exact revision",
            ));
        }
        Ok((revision, snapshot))
    }

    async fn bound_count(
        &self,
        owner: &UserId,
        preset_id: &AgentPresetId,
    ) -> Result<u32, ControlPlaneError> {
        let agent = self
            .store
            .list_agent_bindings(owner)
            .await?
            .iter()
            .filter(|binding| &binding.value.preset_revision_ref.preset_id == preset_id)
            .count();
        let remote = self
            .store
            .list_remote_bindings(owner)
            .await?
            .iter()
            .filter(|binding| &binding.agent_binding.preset_revision_ref.preset_id == preset_id)
            .count();
        Ok((agent + remote) as u32)
    }
}

fn empty_document() -> nomifun_api_types::AgentPresetDocumentDto {
    nomifun_api_types::AgentPresetDocumentDto {
        schema_version: "1.0.0".into(),
        model_route_refs: BTreeMap::new(),
        chat_route_records: BTreeMap::new(),
        initial_capabilities: Vec::new(),
        on_demand_capabilities: Vec::new(),
        skill_bindings: Vec::new(),
        system_role_provider_overrides: BTreeMap::new(),
        persona: String::new(),
        instructions: String::new(),
        starter_prompts: Vec::new(),
    }
}

fn editor_response(
    stored: StoredPreset,
    revision: Option<AgentPresetRevision>,
    document: nomifun_api_types::AgentPresetDocumentDto,
    transient_template_key: Option<OfficialPresetKey>,
) -> Result<AgentPresetEditorResponse, ControlPlaneError> {
    let draft = draft_api(
        &stored,
        revision.as_ref(),
        document,
        transient_template_key,
    )?;
    Ok(AgentPresetEditorResponse {
        preset: preset_summary(&stored, 0)?,
        revision: revision.as_ref().map(revision_api).transpose()?,
        draft,
    })
}

fn draft_api(
    stored: &StoredPreset,
    revision: Option<&AgentPresetRevision>,
    document: nomifun_api_types::AgentPresetDocumentDto,
    transient_template_key: Option<OfficialPresetKey>,
) -> Result<AgentPresetDraftDto, ControlPlaneError> {
    Ok(AgentPresetDraftDto {
        preset_id: stored.preset.preset_id.as_ref().to_owned(),
        display_name: stored.preset.display_name.clone(),
        description: stored.preset.description.clone(),
        source_template_key: transient_template_key
            .map(|key| wire_cast(&key))
            .transpose()?,
        current_revision: revision
            .map(|revision| wire_cast(&revision.reference))
            .transpose()?,
        document,
    })
}

fn preset_summary(
    stored: &StoredPreset,
    bound_target_count: u32,
) -> Result<AgentPresetSummaryDto, ControlPlaneError> {
    Ok(AgentPresetSummaryDto {
        preset_id: stored.preset.preset_id.as_ref().to_owned(),
        owner_user_id: stored
            .preset
            .owner_user_id
            .as_ref()
            .map(|owner| owner.as_ref().to_owned()),
        source: wire_cast(&stored.preset.source)?,
        display_name: stored.preset.display_name.clone(),
        description: stored.preset.description.clone(),
        current_stable_revision: stored
            .preset
            .current_stable_revision
            .as_ref()
            .map(wire_cast)
            .transpose()?,
        bound_target_count,
    })
}

fn binding_summary(
    binding: &StoredAgentBinding,
) -> Result<AgentBindingSummaryDto, ControlPlaneError> {
    Ok(AgentBindingSummaryDto {
        target_kind: binding.target.target_kind.clone(),
        target_id: binding.target.target_id.clone(),
        preset_revision_ref: wire_cast(&binding.value.preset_revision_ref)?,
        resolved_snapshot_ref: wire_cast(&binding.value.resolved_snapshot_ref)?,
        binding_version: binding.value.binding_version,
    })
}

fn binding_record_api(
    binding: StoredAgentBinding,
) -> Result<AgentBindingRecordDto, ControlPlaneError> {
    Ok(AgentBindingRecordDto {
        target: AgentBindingTargetDto {
            target_kind: binding.target.target_kind,
            target_id: binding.target.target_id,
        },
        owner_user_id: binding.owner_user_id.as_ref().to_owned(),
        agent_binding: wire_cast(&binding.value)?,
    })
}

fn selection_api(
    reference: &nomifun_agent_contracts::CapabilityRef,
) -> Result<nomifun_api_types::CapabilitySelectionDto, ControlPlaneError> {
    wire_cast(&CapabilitySelection {
        capability: reference.clone(),
        action_allowlist: BTreeSet::new(),
    })
}

fn exact_ref_api<T>(reference: &ExactVersionRef<T>) -> ExactCatalogRefDto
where
    T: AsRef<str>,
{
    ExactCatalogRefDto {
        id: reference.id.as_ref().to_owned(),
        version: reference.version.as_ref().to_owned(),
    }
}

fn session_binding_from_stable_artifacts(
    stable: PresetRevisionRef,
    revision: Option<AgentPresetRevision>,
    snapshot: Option<nomifun_agent_contracts::ResolvedSnapshotEnvelope>,
) -> Result<AgentBindingValue, ControlPlaneError> {
    let revision = revision.ok_or_else(|| {
        ControlPlaneError::canonical(
            "PRESET_REVISION_DIGEST_MISMATCH",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "AgentPreset current stable Revision is unavailable",
        )
    })?;
    revision.validate().map_err(|violation| {
        ControlPlaneError::canonical(
            violation.code,
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            violation.message,
        )
    })?;
    let snapshot = snapshot.ok_or_else(|| {
        ControlPlaneError::canonical(
            "CAPABILITY_NOT_MATERIALIZED",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "AgentPreset current stable Revision has no persisted Snapshot",
        )
    })?;
    snapshot.validate().map_err(|violation| {
        ControlPlaneError::canonical(
            violation.code,
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            violation.message,
        )
    })?;
    if revision.reference != stable || snapshot.content.preset_revision_ref != stable {
        return Err(ControlPlaneError::canonical(
            "PRESET_REVISION_DIGEST_MISMATCH",
            axum::http::StatusCode::CONFLICT,
            "persisted Revision and Snapshot do not match current_stable_revision",
        ));
    }

    Ok(AgentBindingValue {
        preset_revision_ref: stable,
        resolved_snapshot_ref: snapshot.snapshot_ref,
        typed_resource_bindings: Vec::new(),
        binding_version: 1,
    })
}

fn ensure_expected_current(
    current: Option<&PresetRevisionRef>,
    expected: Option<&nomifun_api_types::PresetRevisionRefDto>,
) -> Result<(), ControlPlaneError> {
    let expected = expected.map(wire_cast).transpose()?;
    if current != expected.as_ref() {
        return Err(ControlPlaneError::canonical(
            "PRESET_REVISION_DIGEST_MISMATCH",
            axum::http::StatusCode::CONFLICT,
            "expected_current_revision does not match the current immutable revision",
        ));
    }
    Ok(())
}

fn nonempty_name(value: String) -> Result<String, ControlPlaneError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(ControlPlaneError::canonical(
            "PRESET_REVISION_SAVE_FAILED",
            axum::http::StatusCode::BAD_REQUEST,
            "AgentPreset display name is required",
        ));
    }
    Ok(value)
}

fn parse_official_key(value: &str) -> Option<OfficialPresetKey> {
    OfficialPresetKey::ALL
        .into_iter()
        .find(|key| key.as_str() == value)
}

fn not_found(subject: &str) -> ControlPlaneError {
    let (code, status) = match subject {
        "OfficialPresetTemplate" => (
            "OFFICIAL_PRESET_KEY_SET_MISMATCH",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        ),
        "RemoteBinding" => (
            "REMOTE_BINDING_NOT_FOUND",
            axum::http::StatusCode::NOT_FOUND,
        ),
        "AgentPreset" => (
            "AGENT_PRESET_NOT_FOUND",
            axum::http::StatusCode::NOT_FOUND,
        ),
        _ => (
            "PRESET_REVISION_DIGEST_MISMATCH",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        ),
    };
    ControlPlaneError::canonical(code, status, format!("{subject} does not exist"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CatalogSnapshot, CompilerReleaseInputs, InMemoryControlPlaneStore,
        OfficialTemplateCatalog, PresetPreviewCompiler, StaticCatalogProvider,
        StaticRevisionImpactCatalogProvider,
    };
    use nomifun_agent_contracts::{
        CapabilityCatalogEntry, CapabilityCatalogMaterialization,
        CapabilityCatalogMaterializer, CapabilityContributions,
        CapabilityConsumer, CapabilityId, CapabilityKind,
        CapabilityManifest, CapabilityOwner, CapabilityProvenance,
        CapabilityRef, CapabilityReleaseState, CatalogAvailability,
        ContributionId, ContributionLock, ContributionSourceKind, DigestHex,
        LocalizedMetadata, PackageId, PackageRef, PlatformConstraint,
        PluginMountId, PluginSourceKind, PluginSourceMetadata,
        RuntimeProfileKind, RuntimeTarget, StableSourceIdentity,
        StrictJsonValue, VersionString, capability_surface_declarations,
        digest_payload,
    };
    use nomifun_agent_kernel::{
        CompilerEnvironment, MaterializedCapability, MaterializedRegistry,
    };
    use serde_json::json;

    fn test_compiler(templates: &OfficialTemplateCatalog) -> PresetPreviewCompiler {
        let release = CompilerReleaseInputs {
            resolver_version: VersionString::from("1.0.0"),
            runtime_protocol_version: VersionString::from("1.0.0"),
            runtime_feature_inventory_digest: DigestHex::from("runtime-features"),
            canonical_schema_manifest_digest: DigestHex::from("schema"),
            target_contribution_manifest_digest: DigestHex::from("contributions"),
            availability_evidence_revision: "fixture".into(),
        };
        PresetPreviewCompiler::new(release.clone(), templates.clone()).with_materialized_registry(
            Arc::new(MaterializedRegistry::empty()),
            CompilerEnvironment {
                resolver_version: release.resolver_version.clone(),
                required_runtime_protocol_version: release.runtime_protocol_version.clone(),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: release
                    .runtime_feature_inventory_digest
                    .clone(),
                available_runtime_features: BTreeSet::new(),
                installation_role_bindings: BTreeMap::new(),
                canonical_schema_manifest_digest: release
                    .canonical_schema_manifest_digest
                    .clone(),
                target_contribution_manifest_digest: release
                    .target_contribution_manifest_digest
                    .clone(),
                host_target: RuntimeTarget::from("test"),
                host_surface: "desktop".into(),
                availability_evidence_revision: release.availability_evidence_revision,
            },
        )
    }

    fn test_control_plane(
        store: Arc<InMemoryControlPlaneStore>,
    ) -> AgentControlPlane {
        let catalog = Arc::new(StaticCatalogProvider::new(Default::default()));
        let templates = OfficialTemplateCatalog::load().unwrap();
        let compiler = test_compiler(&templates);
        AgentControlPlane::new(store, catalog, templates, compiler)
    }

    #[tokio::test]
    async fn configured_creation_without_a_model_does_not_leave_an_empty_preset() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control_plane = test_control_plane(store.clone());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let error = control_plane.create_preset(&owner, CreateAgentPresetRequest {
            display_name: "Edited official preset".into(), description: None,
            fork_from_revision: None, document: Some(empty_document()),
        }).await.unwrap_err();
        assert_eq!(error.code().as_ref(), "MODEL_ROUTE_NOT_CONFIGURED");
        assert!(store.list_presets(&owner).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn configured_creation_rejects_an_ambiguous_revision_source() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control_plane = test_control_plane(store.clone());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let error = control_plane.create_preset(&owner, CreateAgentPresetRequest {
            display_name: "Ambiguous".into(), description: None,
            fork_from_revision: Some(nomifun_api_types::PresetRevisionRefDto {
                preset_id: "0190f5fe-7c00-7a00-8000-000000000002".into(),
                revision: 1, revision_digest: "a".repeat(64),
            }), document: Some(empty_document()),
        }).await.unwrap_err();
        assert_eq!(error.code().as_ref(), "PRESET_CREATE_INVALID");
        assert!(store.list_presets(&owner).await.unwrap().is_empty());
    }

    fn catalog_capability(
        id: &str,
        consumers: impl IntoIterator<Item = CapabilityConsumer>,
    ) -> (MaterializedCapability, CapabilityCatalogEntry) {
        let package = PackageRef {
            id: PackageId::from(format!("test.{id}")),
            version: VersionString::from("1.0.0"),
        };
        let manifest = CapabilityManifest {
            id: CapabilityId::from(id),
            contribution_id: ContributionId::from(format!("capability:{id}")),
            version: VersionString::from("1.0.0"),
            kind: CapabilityKind::Tool,
            package: package.clone(),
            display: LocalizedMetadata {
                name: id.to_owned(),
                description: format!("test {id}"),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces: capability_surface_declarations(["desktop"], consumers),
            requires_runtime_features: Vec::new(),
            supported_platforms: vec![PlatformConstraint::Any],
            config_schema: StrictJsonValue(json!({
                "type": "object",
                "additionalProperties": false
            })),
            contributions: CapabilityContributions::default(),
        };
        let contract_digest = digest_payload(&manifest).unwrap();
        let artifact_digest = DigestHex::from("a".repeat(64));
        let source = PluginSourceMetadata {
            source_kind: PluginSourceKind::Bundled,
            source_identity: package.id.as_ref().to_owned(),
            source_digest: Some(artifact_digest.clone()),
        };
        let contribution_lock = ContributionLock {
            source_kind: ContributionSourceKind::PlatformBuiltin,
            source_identity: StableSourceIdentity::from(
                source.source_identity.clone(),
            ),
            mount_id: None,
            miniapp_id: None,
            mcp_binding_id: None,
            contribution_id: manifest.contribution_id.clone(),
            contract_digest: contract_digest.clone(),
        };
        let materialized = MaterializedCapability {
            manifest: manifest.clone(),
            schema_digest: contract_digest,
            contribution_id: manifest.contribution_id.clone(),
            contribution_lock: contribution_lock.clone(),
            target_artifact_digest: artifact_digest.clone(),
            mount_id: PluginMountId::from(format!("mount.{id}")),
            source,
        };
        let availability = manifest
            .supported_consumers()
            .unwrap()
            .into_iter()
            .map(|consumer| (consumer, CatalogAvailability::Active))
            .collect();
        let entry = CapabilityCatalogMaterializer::materialize(
            CapabilityCatalogMaterialization {
                manifest,
                provenance: CapabilityProvenance {
                    owner: CapabilityOwner::Package { package },
                    source_kind: contribution_lock.source_kind,
                    source_identity: contribution_lock.source_identity,
                    mount_id: None,
                    miniapp_id: None,
                    mcp_binding_id: None,
                    artifact_digest: Some(artifact_digest),
                },
                release_state: CapabilityReleaseState::PublishedActive,
                availability,
            },
        )
        .unwrap();
        (materialized, entry)
    }

    #[test]
    fn official_key_parser_is_exactly_the_frozen_seven() {
        assert_eq!(
            parse_official_key("coding.codex"),
            Some(OfficialPresetKey::CodingCodex)
        );
        assert!(parse_official_key("research").is_none());
        assert!(parse_official_key("autowork.executor").is_none());
    }

    #[test]
    fn shared_catalog_resolves_agent_and_gateway_and_filters_agent_only_view() {
        let (shared, shared_entry) = catalog_capability(
            "knowledge.search",
            [CapabilityConsumer::Agent, CapabilityConsumer::Gateway],
        );
        let (knowledge_only, knowledge_only_entry) = catalog_capability(
            "browser.render_content",
            [CapabilityConsumer::Knowledge],
        );
        let snapshot = CatalogSnapshot {
            capabilities: vec![shared, knowledge_only],
            formal_capability_entries: BTreeMap::from([
                (shared_entry.capability.clone(), shared_entry),
                (
                    knowledge_only_entry.capability.clone(),
                    knowledge_only_entry,
                ),
            ]),
            ..CatalogSnapshot::default()
        };
        let catalog = Arc::new(StaticCatalogProvider::new(snapshot));
        let templates = OfficialTemplateCatalog::load().expect("official templates");
        let control_plane = AgentControlPlane::new(
            Arc::new(InMemoryControlPlaneStore::new()),
            catalog,
            templates.clone(),
            test_compiler(&templates),
        );
        let shared_ref = CapabilityRef {
            id: CapabilityId::from("knowledge.search"),
            version: VersionString::from("1.0.0"),
        };
        let knowledge_only_ref = CapabilityRef {
            id: CapabilityId::from("browser.render_content"),
            version: VersionString::from("1.0.0"),
        };

        let agent_lock = control_plane
            .resolve_capability(&shared_ref, CapabilityConsumer::Agent)
            .expect("Agent must resolve shared capability");
        let gateway_lock = control_plane
            .resolve_capability(&shared_ref, CapabilityConsumer::Gateway)
            .expect("Gateway must resolve the same shared capability");
        assert_eq!(agent_lock.contribution, gateway_lock.contribution);
        assert!(
            control_plane
                .resolve_capability(&knowledge_only_ref, CapabilityConsumer::Agent)
                .is_err(),
            "Knowledge-only capability must not enter the Agent consumer view"
        );
        assert_eq!(
            control_plane
                .catalog()
                .expect("Agent catalog")
                .capabilities
                .iter()
                .map(|item| {
                    let id: &str = item.capability.id.as_ref();
                    id.to_owned()
                })
                .collect::<Vec<String>>(),
            vec!["knowledge.search"]
        );
    }

    #[tokio::test]
    async fn create_from_template_commits_initial_revision_and_does_not_persist_template_key() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let catalog = Arc::new(StaticCatalogProvider::new(Default::default()));
        let templates = OfficialTemplateCatalog::load().unwrap();
        let compiler = test_compiler(&templates);
        let control_plane = AgentControlPlane::new(
            store.clone(),
            catalog,
            templates,
            compiler,
        );
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");

        let created = control_plane
            .create_from_template(
                &owner,
                "chat.minimal",
                CreateAgentPresetFromTemplateRequest {
                    display_name: "Minimal".into(),
                    description: None,
                    model_route_refs: BTreeMap::new(),
                    chat_route_records: BTreeMap::new(),
                },
            )
            .await
            .unwrap();

        let revision = created.revision.as_ref().expect("initial Revision");
        assert_eq!(revision.reference.revision, 1);
        assert_eq!(
            created.preset.current_stable_revision.as_ref(),
            Some(&revision.reference)
        );
        assert_eq!(
            created.draft.source_template_key,
            Some(nomifun_api_types::OfficialPresetKeyDto::ChatMinimal)
        );
        let stored = store
            .get_preset(&AgentPresetId::from(created.preset.preset_id.clone()))
            .await
            .unwrap()
            .expect("stored preset");
        assert_eq!(
            stored.preset.current_stable_revision.as_ref().unwrap().revision,
            1
        );
        assert!(
            store
                .get_snapshot(
                    stored
                        .preset
                        .current_stable_revision
                        .as_ref()
                        .unwrap()
                )
                .await
                .unwrap()
                .is_some()
        );

        let reloaded = control_plane
            .editor(&owner, stored.preset.preset_id.as_ref(), None)
            .await
            .unwrap();
        assert!(reloaded.draft.source_template_key.is_none());
    }

    #[tokio::test]
    async fn retire_preset_closes_product_admission_but_preserves_immutable_artifacts() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control_plane = test_control_plane(store.clone());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let other_owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let created = control_plane
            .create_from_template(
                &owner,
                "chat.minimal",
                CreateAgentPresetFromTemplateRequest {
                    display_name: "Retire me".into(),
                    description: None,
                    model_route_refs: BTreeMap::new(),
                    chat_route_records: BTreeMap::new(),
                },
            )
            .await
            .unwrap();
        let revision = created.revision.clone().expect("initial Revision");
        let stable: PresetRevisionRef = wire_cast(&revision.reference).unwrap();
        let snapshot = store
            .get_snapshot(&stable)
            .await
            .unwrap()
            .expect("initial Snapshot");
        let binding_value = AgentBindingValue {
            preset_revision_ref: stable.clone(),
            resolved_snapshot_ref: snapshot.snapshot_ref.clone(),
            typed_resource_bindings: Vec::new(),
            binding_version: 1,
        };
        store
            .put_agent_binding(
                StoredAgentBinding {
                    target: AgentBindingTarget {
                        target_kind: "conversation".into(),
                        target_id: "conversation-1".into(),
                    },
                    owner_user_id: owner.clone(),
                    value: binding_value.clone(),
                },
                None,
            )
            .await
            .unwrap();
        store
            .insert_remote_binding(RemoteBinding {
                remote_binding_id: RemoteBindingId::from("remote-retirement"),
                owner_user_id: owner.clone(),
                name: "Remote retirement".into(),
                agent_binding: binding_value,
            })
            .await
            .unwrap();
        let preview = control_plane
            .preview(
                &owner,
                &created.preset.preset_id,
                ResolveAgentPresetPreviewRequest {
                    expected_current_revision: Some(revision.reference.clone()),
                    draft: created.draft.clone(),
                    scene: SETTINGS_SCENE.into(),
                    surface: SETTINGS_SURFACE.into(),
                    audience: SETTINGS_AUDIENCE.into(),
                },
            )
            .await
            .unwrap();

        let owner_error = control_plane
            .retire_preset(&other_owner, &created.preset.preset_id)
            .await
            .expect_err("another owner must see the same response as a missing Preset");
        assert_eq!(owner_error.status(), axum::http::StatusCode::NOT_FOUND);
        assert_eq!(owner_error.code().as_ref(), "AGENT_PRESET_NOT_FOUND");
        assert!(
            control_plane
                .library(&owner)
                .await
                .unwrap()
                .user_presets
                .iter()
                .any(|preset| preset.preset_id == created.preset.preset_id)
        );

        control_plane
            .retire_preset(&owner, &created.preset.preset_id)
            .await
            .unwrap();
        assert!(
            control_plane
                .library(&owner)
                .await
                .unwrap()
                .user_presets
                .is_empty()
        );
        assert!(store.list_agent_bindings(&owner).await.unwrap().is_empty());
        assert!(store.list_remote_bindings(&owner).await.unwrap().is_empty());
        for error in [
            control_plane
                .editor(&owner, &created.preset.preset_id, None)
                .await
                .expect_err("retired Preset must leave the editor"),
            control_plane
                .save_revision(
                    &owner,
                    &created.preset.preset_id,
                    SaveAgentPresetRevisionRequest {
                        expected_current_revision: Some(revision.reference.clone()),
                        preview_digest: preview.preview_digest,
                        draft: created.draft,
                        reason: Some("must not save after retirement".into()),
                    },
                )
                .await
                .expect_err("retired Preset must not save"),
            control_plane
                .resolve_agent_session_binding(&owner, &created.preset.preset_id)
                .await
                .expect_err("retired Preset must not admit a new Session"),
        ] {
            assert_eq!(error.status(), axum::http::StatusCode::NOT_FOUND);
            assert_eq!(error.code().as_ref(), "AGENT_PRESET_NOT_FOUND");
        }
        assert!(
            store
                .get_revision(&stable)
                .await
                .unwrap()
                .is_some(),
            "immutable Revision history must survive product retirement"
        );
        assert!(
            store
                .get_snapshot(&stable)
                .await
                .unwrap()
                .is_some(),
            "immutable Snapshot history must survive product retirement"
        );
        let repeated_error = control_plane
            .retire_preset(&owner, &created.preset.preset_id)
            .await
            .expect_err("a retired Preset is no longer a deletable product entry");
        assert_eq!(repeated_error.code().as_ref(), "AGENT_PRESET_NOT_FOUND");

        let official_error = control_plane
            .retire_preset(&owner, "chat.minimal")
            .await
            .expect_err("official template seeds must not be deleted");
        assert_eq!(official_error.status(), axum::http::StatusCode::NOT_FOUND);
        assert_eq!(official_error.code().as_ref(), "AGENT_PRESET_NOT_FOUND");
    }

    #[tokio::test]
    async fn session_binding_is_owner_scoped_and_freezes_current_stable_snapshot() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let catalog = Arc::new(StaticCatalogProvider::new(Default::default()));
        let templates = OfficialTemplateCatalog::load().unwrap();
        let compiler = test_compiler(&templates);
        let control_plane = AgentControlPlane::new(
            store.clone(),
            catalog,
            templates,
            compiler,
        );
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let other_owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let created = control_plane
            .create_from_template(
                &owner,
                "chat.minimal",
                CreateAgentPresetFromTemplateRequest {
                    display_name: "Minimal".into(),
                    description: None,
                    model_route_refs: BTreeMap::new(),
                    chat_route_records: BTreeMap::new(),
                },
            )
            .await
            .unwrap();
        let revision = created.revision.as_ref().expect("initial Revision");

        let binding = control_plane
            .resolve_agent_session_binding(&owner, &created.preset.preset_id)
            .await
            .unwrap();
        assert_eq!(binding.preset_revision_ref, revision.reference);
        assert!(binding.typed_resource_bindings.is_empty());
        assert_eq!(binding.binding_version, 1);

        let stable: PresetRevisionRef = wire_cast(&revision.reference).unwrap();
        let stored_revision = store
            .get_revision(&stable)
            .await
            .unwrap()
            .expect("persisted stable Revision");
        let snapshot_error = session_binding_from_stable_artifacts(
            stable,
            Some(stored_revision),
            None,
        )
        .expect_err("a missing persisted Snapshot must fail closed");
        assert_eq!(
            snapshot_error.status(),
            axum::http::StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            snapshot_error.code().as_ref(),
            "CAPABILITY_NOT_MATERIALIZED"
        );

        let mut next_draft = control_plane
            .editor(&owner, &created.preset.preset_id, None)
            .await
            .unwrap()
            .draft;
        next_draft.document.instructions = "Revision two".into();
        let next_preview_request = ResolveAgentPresetPreviewRequest {
            expected_current_revision: Some(revision.reference.clone()),
            draft: next_draft.clone(),
            scene: SETTINGS_SCENE.into(),
            surface: SETTINGS_SURFACE.into(),
            audience: SETTINGS_AUDIENCE.into(),
        };
        let next_preview = control_plane
            .preview(
                &owner,
                &created.preset.preset_id,
                next_preview_request,
            )
            .await
            .unwrap();
        let saved = control_plane
            .save_revision(
                &owner,
                &created.preset.preset_id,
                SaveAgentPresetRevisionRequest {
                    expected_current_revision: Some(revision.reference.clone()),
                    preview_digest: next_preview.preview_digest,
                    draft: next_draft,
                    reason: Some("freeze test".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(saved.revision.reference.revision, 2);
        let next_binding = control_plane
            .resolve_agent_session_binding(&owner, &created.preset.preset_id)
            .await
            .unwrap();
        assert_eq!(binding.preset_revision_ref.revision, 1);
        assert_eq!(next_binding.preset_revision_ref.revision, 2);

        let owner_error = control_plane
            .resolve_agent_session_binding(&other_owner, &created.preset.preset_id)
            .await
            .expect_err("another owner must not bind this Preset");
        assert_eq!(owner_error.status(), axum::http::StatusCode::FORBIDDEN);
        assert_eq!(owner_error.code().as_ref(), "RESOURCE_OWNER_MISMATCH");

        let no_stable = control_plane
            .create_preset(
                &owner,
                CreateAgentPresetRequest {
                    display_name: "No stable Revision".into(),
                    description: None,
                    fork_from_revision: None,
                    document: None,
                },
            )
            .await
            .unwrap();
        assert!(no_stable.preset.current_stable_revision.is_none());
        let stable_error = control_plane
            .resolve_agent_session_binding(&owner, &no_stable.preset.preset_id)
            .await
            .expect_err("a Preset without a stable Revision must not create a Session");
        assert_eq!(
            stable_error.status(),
            axum::http::StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            stable_error.code().as_ref(),
            "CAPABILITY_NOT_MATERIALIZED"
        );
    }

    #[tokio::test]
    async fn get_remote_binding_returns_only_the_authenticated_owners_binding() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let catalog = Arc::new(StaticCatalogProvider::new(Default::default()));
        let templates = OfficialTemplateCatalog::load().unwrap();
        let compiler = test_compiler(&templates);
        let control_plane = AgentControlPlane::new(
            store.clone(),
            catalog,
            templates,
            compiler,
        );
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let other_owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let binding_id = RemoteBindingId::from("remote-binding-1");
        store
            .insert_preset(StoredPreset {
                preset: AgentPreset {
                    preset_id: AgentPresetId::from("preset-1"),
                    owner_user_id: Some(owner.clone()),
                    source: AgentPresetSource::User,
                    display_name: "Preset".into(),
                    description: None,
                    current_stable_revision: None,
                },
            })
            .await
            .unwrap();
        store
            .insert_remote_binding(RemoteBinding {
                remote_binding_id: binding_id.clone(),
                owner_user_id: owner.clone(),
                name: "Remote".into(),
                agent_binding: AgentBindingValue {
                    preset_revision_ref: PresetRevisionRef {
                        preset_id: AgentPresetId::from("preset-1"),
                        revision: 1,
                        revision_digest: DigestHex::from("revision"),
                    },
                    resolved_snapshot_ref:
                        nomifun_agent_contracts::ResolvedSnapshotRef {
                            snapshot_id:
                                nomifun_agent_contracts::ResolvedSnapshotId::from("snapshot-1"),
                            snapshot_digest: DigestHex::from("snapshot"),
                        },
                    typed_resource_bindings: Vec::new(),
                    binding_version: 1,
                },
            })
            .await
            .unwrap();

        let found = control_plane
            .get_remote_binding(&owner, binding_id.as_ref())
            .await
            .unwrap()
            .expect("the owner must see its remote binding");
        assert_eq!(found.remote_binding_id, binding_id.as_ref());
        assert_eq!(found.owner_user_id, owner.as_ref());
        assert_eq!(found.name, "Remote");

        assert!(
            control_plane
                .get_remote_binding(&other_owner, binding_id.as_ref())
                .await
                .unwrap()
                .is_none(),
            "a binding owned by another user must be indistinguishable from a missing binding"
        );
        assert!(
            control_plane
                .get_remote_binding(&owner, "missing-remote-binding")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn revision_impact_is_owner_scoped_read_only_and_lists_bound_consumers() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let catalog = Arc::new(StaticCatalogProvider::new(Default::default()));
        let templates = OfficialTemplateCatalog::load().unwrap();
        let compiler = test_compiler(&templates);
        let control_plane = AgentControlPlane::new(
            store.clone(),
            catalog,
            templates,
            compiler,
        )
        .with_revision_impact_catalog_provider(Arc::new(
            StaticRevisionImpactCatalogProvider::new(Vec::new()),
        ));
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let other_owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let created = control_plane
            .create_from_template(
                &owner,
                "chat.minimal",
                CreateAgentPresetFromTemplateRequest {
                    display_name: "Minimal".into(),
                    description: None,
                    model_route_refs: BTreeMap::new(),
                    chat_route_records: BTreeMap::new(),
                },
            )
            .await
            .unwrap();
        let revision = created.revision.as_ref().expect("initial Revision");
        let revision_ref: PresetRevisionRef = wire_cast(&revision.reference).unwrap();
        let snapshot = store
            .get_snapshot(&revision_ref)
            .await
            .unwrap()
            .expect("persisted Snapshot");
        let binding_value = AgentBindingValue {
            preset_revision_ref: revision_ref.clone(),
            resolved_snapshot_ref: snapshot.snapshot_ref.clone(),
            typed_resource_bindings: Vec::new(),
            binding_version: 1,
        };
        store
            .put_agent_binding(
                StoredAgentBinding {
                    target: AgentBindingTarget {
                        target_kind: "automation".into(),
                        target_id: "job-1".into(),
                    },
                    owner_user_id: owner.clone(),
                    value: binding_value.clone(),
                },
                None,
            )
            .await
            .unwrap();
        store
            .insert_remote_binding(RemoteBinding {
                remote_binding_id: RemoteBindingId::from("remote-impact-1"),
                owner_user_id: owner.clone(),
                name: "Remote impact".into(),
                agent_binding: binding_value,
            })
            .await
            .unwrap();

        let impact = control_plane
            .revision_impact(
                &owner,
                &created.preset.preset_id,
                revision.reference.revision,
            )
            .await
            .unwrap();
        assert_eq!(impact.preset_revision_ref, revision.reference);
        assert_eq!(impact.affected_consumers.len(), 2);
        assert_eq!(
            impact.affected_consumers[0].kind,
            RevisionImpactConsumerKindDto::AgentBinding
        );
        assert_eq!(
            impact.affected_consumers[1].kind,
            RevisionImpactConsumerKindDto::RemoteBinding
        );

        assert!(
            control_plane
                .revision_impact(
                    &other_owner,
                    &created.preset.preset_id,
                    revision.reference.revision,
                )
                .await
                .is_err(),
            "another owner must not inspect Revision provenance or impact"
        );
        let unchanged = control_plane
            .get_revision(
                &owner,
                &created.preset.preset_id,
                revision.reference.revision,
            )
            .await
            .unwrap();
        assert_eq!(unchanged, *revision, "impact reads must not rewrite Revision");
    }
}
