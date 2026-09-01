use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nomifun_agent_contracts::{
    AgentBindingValue, AgentPreset, AgentPresetId, AgentPresetRevision, AgentPresetSource,
    CapabilityExposure, CapabilitySelection, ExactVersionRef, OfficialPresetKey, PresetRevisionRef,
    RemoteBinding, RemoteBindingId, StrictJsonValue, UserId,
};
use nomifun_api_types::{
    AgentBindingRecordDto, AgentBindingSummaryDto, AgentBindingTargetDto, AgentCatalogResponse,
    AgentPresetDraftDto, AgentPresetEditorResponse, AgentPresetLibraryResponse,
    AgentPresetSummaryDto, CreateAgentPresetFromTemplateRequest, CreateAgentPresetRequest,
    CreateRemoteBindingRequest, EditorDraftStateDto, ExactCatalogRefDto,
    FreshStartPresentationDto, PutAgentBindingRequest, RemoteBindingDto,
    ResolveAgentPresetPreviewRequest, ResolveAgentPresetPreviewResponse,
    ResolveSavedRevisionPreviewRequest, SaveAgentPresetRevisionRequest,
    SaveAgentPresetRevisionResponse, TemplateResourceSelectionDto, TypedResourceBindingDto,
    UpdateRemoteBindingRequest,
};
use serde_json::json;
use uuid::Uuid;

use crate::catalog::{CatalogProvider, OfficialTemplateCatalog};
use crate::compiler::{PresetPreviewCompiler, revision_api};
use crate::continuation::editor_test_plan;
use crate::error::ControlPlaneError;
use crate::store::{
    AgentBindingTarget, ControlPlaneStore, StoredAgentBinding, StoredPreset,
};
use crate::wire::wire_cast;

const SETTINGS_SCENE: &str = "agent_settings";
const SETTINGS_SURFACE: &str = "desktop";
const SETTINGS_AUDIENCE: &str = "owner";

pub struct AgentControlPlane {
    store: Arc<dyn ControlPlaneStore>,
    catalog: Arc<dyn CatalogProvider>,
    templates: OfficialTemplateCatalog,
    compiler: PresetPreviewCompiler,
}

impl AgentControlPlane {
    pub fn new(
        store: Arc<dyn ControlPlaneStore>,
        catalog: Arc<dyn CatalogProvider>,
        templates: OfficialTemplateCatalog,
        compiler: PresetPreviewCompiler,
    ) -> Self {
        Self {
            store,
            catalog,
            templates,
            compiler,
        }
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

    pub async fn create_preset(
        &self,
        owner: &UserId,
        request: CreateAgentPresetRequest,
    ) -> Result<AgentPresetEditorResponse, ControlPlaneError> {
        let display_name = nonempty_name(request.display_name)?;
        let preset_id = AgentPresetId::from(Uuid::now_v7().to_string());
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
        let stored = StoredPreset {
            preset: AgentPreset {
                preset_id: preset_id.clone(),
                owner_user_id: Some(owner.clone()),
                source: AgentPresetSource::User,
                display_name: display_name.clone(),
                description: request.description.clone(),
                current_stable_revision: None,
            },
        };
        self.store.insert_preset(stored.clone()).await?;
        editor_response(stored, None, empty_document(), None)
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
        let resource_bindings =
            template_resource_bindings(owner, seed, request.resource_bindings)?;
        let document = nomifun_api_types::AgentPresetDocumentDto {
            schema_version: "1.0.0".into(),
            surfaces: BTreeSet::from([
                "desktop".into(),
                "remote".into(),
                "web".into(),
            ]),
            model_route_refs: request.model_route_refs,
            chat_route_records: request.chat_route_records,
            initial_capabilities: seed
                .initial_capabilities
                .iter()
                .map(|capability| selection_api(capability, CapabilityExposure::Advertised))
                .collect::<Result<Vec<_>, _>>()?,
            on_demand_capabilities: seed
                .on_demand_capabilities
                .iter()
                .map(|capability| selection_api(capability, CapabilityExposure::Discoverable))
                .collect::<Result<Vec<_>, _>>()?,
            skill_bindings: seed
                .skill_bindings
                .iter()
                .map(exact_ref_api)
                .collect(),
            resource_bindings,
            persona: String::new(),
            instructions: String::new(),
            context_policy: json!({
                "max_system_tokens": 12000,
                "max_dynamic_context_tokens": 16000,
                "max_catalog_tokens": 3000,
            }),
            execution_constraints: json!({
                "max_active_capabilities": 64,
                "max_advertised_tools": 48,
                "max_runtime_rebuilds": 4,
            }),
            runtime_budget: json!({
                "max_context_tokens": 32000,
                "max_tool_calls_per_turn": 64,
            }),
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
        if snapshot.snapshot_ref != binding.resolved_snapshot_ref {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::CONFLICT,
                "ResolvedSnapshotRef does not match the saved exact revision",
            ));
        }
        Ok(())
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
        surfaces: BTreeSet::from(["desktop".into(), "remote".into(), "web".into()]),
        model_route_refs: BTreeMap::new(),
        chat_route_records: BTreeMap::new(),
        initial_capabilities: Vec::new(),
        on_demand_capabilities: Vec::new(),
        skill_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        persona: String::new(),
        instructions: String::new(),
        context_policy: json!({
            "max_system_tokens": 12000,
            "max_dynamic_context_tokens": 16000,
            "max_catalog_tokens": 3000,
        }),
        execution_constraints: json!({
            "max_active_capabilities": 64,
            "max_advertised_tools": 48,
            "max_runtime_rebuilds": 4,
        }),
        runtime_budget: json!({
            "max_context_tokens": 32000,
            "max_tool_calls_per_turn": 64,
        }),
    }
}

fn template_resource_bindings(
    owner: &UserId,
    seed: &nomifun_agent_contracts::OfficialPresetSeed,
    selections: Vec<TemplateResourceSelectionDto>,
) -> Result<Vec<TypedResourceBindingDto>, ControlPlaneError> {
    let defaults = seed
        .typed_resource_defaults
        .iter()
        .map(|resource| (resource.slot_key.as_str(), resource))
        .collect::<BTreeMap<_, _>>();
    let mut selected_slots = BTreeSet::new();
    let mut bindings = Vec::new();
    for selection in selections {
        let Some(resource_default) = defaults.get(selection.slot_key.as_str()) else {
            return Err(ControlPlaneError::canonical(
                "PRESET_RESOURCE_NOT_BOUND",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                format!("template has no resource slot {}", selection.slot_key),
            ));
        };
        if !selected_slots.insert(selection.slot_key.clone())
            || selection.resource_kind != resource_default.resource_kind.as_ref()
            || selection.resource_id.trim().is_empty()
        {
            return Err(ControlPlaneError::canonical(
                "PRESET_RESOURCE_NOT_BOUND",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "resource selection for slot {} is duplicate, empty, or has the wrong kind",
                    selection.slot_key
                ),
            ));
        }
        bindings.push(TypedResourceBindingDto {
            binding_id: selection.slot_key,
            resource_kind: selection.resource_kind,
            resource_id: selection.resource_id.trim().to_owned(),
            owner_id: owner.as_ref().to_owned(),
            operations: resource_default.operations.clone(),
            connection_config_ref: selection.connection_config_ref,
            typed_parameters: selection.typed_parameters,
        });
    }
    for resource_default in &seed.typed_resource_defaults {
        if resource_default.required && !selected_slots.contains(&resource_default.slot_key) {
            return Err(ControlPlaneError::canonical(
                "PRESET_RESOURCE_NOT_BOUND",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "required template resource slot {} is not bound",
                    resource_default.slot_key
                ),
            ));
        }
    }
    Ok(bindings)
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
    exposure: CapabilityExposure,
) -> Result<nomifun_api_types::CapabilitySelectionDto, ControlPlaneError> {
    wire_cast(&CapabilitySelection {
        capability: reference.clone(),
        required: true,
        exposure,
        action_allowlist: BTreeSet::new(),
        resource_binding_refs: Vec::new(),
        destination_constraints: BTreeSet::new(),
        context_budget_override: None,
        tool_budget_override: None,
        config: StrictJsonValue(json!({})),
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
        CompilerReleaseInputs, InMemoryControlPlaneStore, OfficialTemplateCatalog,
        PresetPreviewCompiler, StaticCatalogProvider,
    };
    use nomifun_agent_contracts::{DigestHex, VersionString};

    #[test]
    fn official_key_parser_is_exactly_the_frozen_seven() {
        assert_eq!(
            parse_official_key("coding.codex"),
            Some(OfficialPresetKey::CodingCodex)
        );
        assert!(parse_official_key("research").is_none());
        assert!(parse_official_key("autowork.executor").is_none());
    }

    #[tokio::test]
    async fn create_from_template_commits_initial_revision_and_does_not_persist_template_key() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let catalog = Arc::new(StaticCatalogProvider::new(Default::default()));
        let templates = OfficialTemplateCatalog::load().unwrap();
        let compiler = PresetPreviewCompiler::new(
            CompilerReleaseInputs {
                resolver_version: VersionString::from("1.0.0"),
                runtime_protocol_version: VersionString::from("1.0.0"),
                runtime_feature_inventory_digest: DigestHex::from("runtime-features"),
                canonical_schema_manifest_digest: DigestHex::from("schema"),
                target_contribution_manifest_digest: DigestHex::from("contributions"),
                availability_evidence_revision: "fixture".into(),
            },
            templates.clone(),
        );
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
                    resource_bindings: Vec::new(),
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
    async fn get_remote_binding_returns_only_the_authenticated_owners_binding() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let catalog = Arc::new(StaticCatalogProvider::new(Default::default()));
        let templates = OfficialTemplateCatalog::load().unwrap();
        let compiler = PresetPreviewCompiler::new(
            CompilerReleaseInputs {
                resolver_version: VersionString::from("1.0.0"),
                runtime_protocol_version: VersionString::from("1.0.0"),
                runtime_feature_inventory_digest: DigestHex::from("runtime-features"),
                canonical_schema_manifest_digest: DigestHex::from("schema"),
                target_contribution_manifest_digest: DigestHex::from("contributions"),
                availability_evidence_revision: "fixture".into(),
            },
            templates.clone(),
        );
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
}
