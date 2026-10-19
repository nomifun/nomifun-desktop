use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nomifun_agent_contracts::{
    CapabilityConsumer, CapabilityOperationLock, CapabilityRef,
    AgentBindingValue, AgentPreset, AgentPresetId, AgentPresetRevision, AgentPresetSource,
    CapabilitySelection, ChatRouteFeature, ChatRouteProtocol, ExactVersionRef, OfficialPresetKey,
    PresetRevisionRef, RemoteBinding, RemoteBindingId, UserId,
    compare_revision_contribution_locks,
};
use nomifun_api_types::{
    AgentBindingRecordDto, AgentBindingSummaryDto, AgentBindingTargetDto, AgentBindingValueDto,
    AgentCatalogResponse, AgentPresetDraftDto, AgentPresetEditorResponse, AgentPresetLibraryResponse,
    AgentPresetRevisionImpactResponse, AgentPresetSummaryDto,
    CreateAgentPresetFromTemplateRequest, CreateAgentPresetRequest, CreateRemoteBindingRequest,
    ExactCatalogRefDto, FreshStartPresentationDto, PutAgentBindingRequest,
    RemoteBindingDto, RevisionImpactConsumerDto, RevisionImpactConsumerKindDto,
    SaveAgentPresetRevisionRequest, SaveAgentPresetRevisionResponse, UpdateRemoteBindingRequest,
};
use serde_json::json;
use uuid::Uuid;

use crate::catalog::{CatalogProvider, CatalogSnapshot, OfficialTemplateCatalog};
use crate::compiler::{PresetRevisionCompiler, revision_api};
use crate::error::ControlPlaneError;
use crate::impact::{
    ControlPlaneRevisionImpactCatalogProvider, RevisionImpactCatalogProvider,
};
use crate::store::{
    AgentBindingTarget, ControlPlaneStore, StoredAgentBinding, StoredPreset,
};
use crate::wire::{document_payload, payload_document, wire_cast};

const CHAT_MODEL_TASK: &str = nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT;

#[path = "model_detachment.rs"]
mod model_detachment;
pub use model_detachment::PreparedAgentModelDetachment;

#[path = "channel_guest_binding.rs"]
mod channel_guest_binding;
#[cfg(test)]
#[path = "session_selected_skills_tests.rs"]
mod session_selected_skills_tests;

// This small product workflow is authored into new immutable AppDeveloper
// Revisions. The detailed platform guide remains an on-demand tool resource.
const APP_DEVELOPMENT_INSTRUCTIONS: &str = "You are NomiFun's built-in application developer. Treat a request to create, import or change an application as completing the usable application in this workspace, including its application record and preview, not just writing source files. At the start of application work, discover app.hosting/current and app.hosting/guide with ToolSearch, then call current_app and read_app_guide. When current_app returns an application, keep its exact app_id and edit its existing managed workspace; do not import a second copy or create another application. Infer a useful application name, short description and appropriate tags from the user's request, and use update_app to keep the actual application metadata and ordinary runtime settings aligned with the files you create. Read existing project scripts and configuration; preserve user data and existing explicit settings unless the requested change requires modifying them. Use update_app with detect_settings:true to recognize ordinary supported project commands, then inspect the returned settings and fill any missing custom commands or real web_url yourself. Settings that do not apply may remain null; never invent install or build commands. Once the files and configuration are ready, call preview_app or rebuild_app on the same application through the hosting owner; do not start a separate unmanaged web server. Verify the returned state, actual HTTP response and relevant business behavior using already-authorized tools, and use the same owner to repair and restart after changes. Clearly distinguish verified behavior from checks unavailable with your granted tools. A generated file, a running process, or an empty web_url does not prove that an embedded preview works. Finish with the application's actual name, preview result and any real remaining failure; never claim preview success from prose alone. For ordinary questions or an explicit request to inspect only, do not modify or start an application.";

fn official_template_instructions(key: OfficialPresetKey) -> &'static str {
    match key {
        OfficialPresetKey::AppDeveloper => APP_DEVELOPMENT_INSTRUCTIONS,
        OfficialPresetKey::CompanionDefault => "You are one Companion Agent across desktop, approved channels and authenticated devices, with independent Sessions. Handle ordinary chat, current information, research, knowledge, files, office, computer and device requests directly with the tools authorized in this Session. A physical device entry does not require another Agent. Preserve the user's location, date and constraints. Use background or parallel execution only when it benefits durable, long or independent work; respect an explicit request to avoid multiple Agents. For a clear supported device request, call the already available authorized device tool immediately. Discover a device tool only when the needed operation is not visible. For a device-action-only request, finish silently after every requested action has a successful receipt under an actually advertised silent-finish control; never stop before all requested actions. Speak for a question, an explicitly requested verbal receipt, a real failure, blocker, uncertain result or necessary clarification. Report an action from the device tool's actual result, distinguishing acceptance, execution completion and physical completion. Do not repeat a device action after an uncertain result without resolving its status. Shared identity does not grant additional tools or device authority. Keep physical spoken replies concise and natural, with detail when requested.",
        _ => "",
    }
}

fn required_chat_features<'a>(
    capability_ids: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<ChatRouteFeature> {
    capability_ids
        .into_iter()
        // Web Research and media Creation resolve through their own owners.
        // Current-turn image input is Session input and is checked against the
        // selected Chat route when that turn is admitted, not as a persistent
        // Agent capability.
        .filter_map(|_| None)
        .collect()
}

fn chat_route_candidate_supports(
    candidate: &nomifun_agent_contracts::ChatRouteCandidate,
    required: &BTreeSet<ChatRouteFeature>,
) -> bool {
    required.is_subset(&candidate.features)
        && (!required.contains(&ChatRouteFeature::WebSearch)
            || candidate.protocol == ChatRouteProtocol::OpenaiResponses)
}

fn ensure_chat_route_supports(
    route: &nomifun_agent_contracts::ChatRouteRecord,
    required: &BTreeSet<ChatRouteFeature>,
) -> Result<(), ControlPlaneError> {
    if chat_route_candidate_supports(&route.primary, required) {
        return Ok(());
    }
    let missing_features = required
        .difference(&route.primary.features)
        .map(|feature| format!("{feature:?}"))
        .collect::<Vec<_>>();
    Err(ControlPlaneError::with_details(
        "MODEL_ROUTE_FEATURES_MISSING",
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "the selected Chat model cannot satisfy this Agent's required model features",
        serde_json::json!({
            "provider_id": route.primary.provider_id,
            "model": route.primary.model,
            "missing_features": missing_features,
            "required_protocol": required
                .contains(&ChatRouteFeature::WebSearch)
                .then_some("openai.responses"),
        }),
    ))
}

/// Host-owned source for selected and initial execution Chat routes.
///
/// The control-plane contract deliberately does not know how a host stores
/// provider credentials or model capabilities.  A host may inject this
/// resolver to materialize a complete, opaque [`ChatRouteRecord`] when a
/// model is selected. An authored default may stay empty; actual execution
/// still requires a usable Chat route.
#[async_trait::async_trait]
pub trait DefaultChatRouteResolver: Send + Sync {
    async fn resolve_default_chat_route(
        &self,
        owner: &UserId,
    ) -> Result<Option<nomifun_agent_contracts::ChatRouteRecord>, ControlPlaneError>;

    async fn resolve_selected_chat_route(
        &self,
        owner: &UserId,
        model: &nomifun_api_types::AgentChatModelSelectionDto,
    ) -> Result<Option<nomifun_agent_contracts::ChatRouteRecord>, ControlPlaneError> {
        let Some(mut route) = self.resolve_default_chat_route(owner).await? else { return Ok(None); };
        let selected = std::iter::once(route.primary.clone()).chain(route.failovers)
            .find(|candidate| candidate.provider_id == model.provider_id && candidate.model == model.model);
        let Some(selected) = selected else { return Ok(None); };
        route.primary = selected;
        route.failovers = Vec::new();
        Ok(Some(route))
    }
}

pub struct ResolvedSessionCapabilities {
    pub skills: Vec<nomifun_agent_contracts::FrozenLibrarySkill>,
    pub selected_skill_names: BTreeSet<String>,
    pub mcp_server_ids: BTreeSet<String>,
}

/// Captures current installation-owned extensions. Only this host resolver
/// supplies library bytes; API callers select names and server identities.
#[async_trait::async_trait]
pub trait SessionCapabilitiesResolver: Send + Sync {
    async fn resolve(
        &self,
        owner: &UserId,
        selection: Option<&nomifun_api_types::SessionCapabilitySelectionDto>,
    ) -> Result<ResolvedSessionCapabilities, ControlPlaneError>;
}

pub struct AgentControlPlane {
    store: Arc<dyn ControlPlaneStore>,
    catalog: Arc<dyn CatalogProvider>,
    impact_catalog: Arc<dyn RevisionImpactCatalogProvider>,
    templates: OfficialTemplateCatalog,
    compiler: PresetRevisionCompiler,
    role_bindings: Option<Arc<dyn crate::InstallationRoleBindingStore>>,
    default_chat_route_resolver: Option<Arc<dyn DefaultChatRouteResolver>>,
    session_capabilities_resolver: Option<Arc<dyn SessionCapabilitiesResolver>>,
    template_launch_lock: tokio::sync::Mutex<()>,
}

impl AgentControlPlane {
    pub fn with_installation_role_binding_store(mut self, store: Arc<dyn crate::InstallationRoleBindingStore>) -> Self {
        self.role_bindings = Some(store);
        self
    }

    async fn authoring_compiler(&self, owner: &UserId) -> Result<PresetRevisionCompiler, ControlPlaneError> {
        match &self.role_bindings {
            Some(store) => self.compiler.clone().with_current_role_bindings(store.load(owner).await?),
            None => Ok(self.compiler.clone()),
        }
    }

    fn role_binding_store(&self) -> Result<&dyn crate::InstallationRoleBindingStore, ControlPlaneError> {
        self.role_bindings.as_deref().ok_or_else(|| ControlPlaneError::canonical(
            "ROLE_DEFAULT_MANAGEMENT_UNAVAILABLE", axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "this host has not configured installation default management",
        ))
    }

    pub async fn role_defaults(&self, owner: &UserId) -> Result<Vec<nomifun_api_types::InstallationRoleBindingDto>, ControlPlaneError> {
        self.role_binding_store()?.load(owner).await?.values().map(wire_cast).collect()
    }

    pub async fn put_role_default(&self, owner: &UserId, role_id: &str, request: nomifun_api_types::PutAgentRoleDefaultRequest)
        -> Result<nomifun_api_types::InstallationRoleBindingDto, ControlPlaneError> {
        if request.selection.role.key.role_id != role_id {
            return Err(ControlPlaneError::canonical("ROLE_DEFAULT_IDENTITY_MISMATCH",
                axum::http::StatusCode::BAD_REQUEST, "selection must match the requested Role"));
        }
        let store = self.role_binding_store()?;
        store.load(owner).await?;
        let selection = wire_cast(&request.selection)?;
        self.compiler.validate_role_default(&selection)?;
        wire_cast(&store.put(owner, selection, request.expected_binding_version).await?)
    }

    pub fn new(
        store: Arc<dyn ControlPlaneStore>,
        catalog: Arc<dyn CatalogProvider>,
        templates: OfficialTemplateCatalog,
        compiler: PresetRevisionCompiler,
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
            role_bindings: None,
            default_chat_route_resolver: None,
            session_capabilities_resolver: None,
            template_launch_lock: tokio::sync::Mutex::new(()),
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

    pub fn with_session_capabilities_resolver(mut self, resolver: Arc<dyn SessionCapabilitiesResolver>) -> Self {
        self.session_capabilities_resolver = Some(resolver);
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
        let mut official_rows = Vec::new();
        for key in OfficialPresetKey::ALL {
            let preset = self.ensure_official_preset(owner, key).await?;
            official_rows.push((key, preset));
        }
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

        let official_presets = official_rows.iter().map(|(key, preset)| {
            Ok((wire_cast(key)?, preset_summary(preset,
                bound_counts.get(&preset.preset.preset_id).copied().unwrap_or_default())?))
        }).collect::<Result<BTreeMap<_, _>, ControlPlaneError>>()?;

        let user_presets = presets
            .iter()
            .filter(|preset| !preset.session_only && preset.preset.source == AgentPresetSource::User)
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
                data_generation: nomifun_agent_contracts::AGENT_STORE_DATA_GENERATION,
                legacy_data_imported: false,
                official_template_count: official_templates.len() as u32,
                user_preset_count: user_presets.len() as u32,
            },
            official_templates,
            official_presets,
            user_presets,
            active_bindings,
        })
    }

    /// Retire only the replaced application-plugin grants in current owned
    /// configurations. New artifacts go through the canonical compiler/store;
    /// immutable revisions, snapshots, bindings and Session history stay intact.
    pub async fn retire_replaced_app_plugin_configuration(&self, owner: &UserId) -> Result<usize, ControlPlaneError> {
        let mut changed = 0;
        let retired = |id: &str| id == "plugin.development" || id.starts_with("plugin:");
        for stored in self.store.list_presets(owner).await? {
            let preset_id = stored.preset.preset_id.clone();
            let retired_configuration: Result<bool, ControlPlaneError> = async {
            let Some(current) = self.current_revision(&stored).await? else { return Ok(false); };
            if !current.payload.enabled_capabilities.iter().any(|item| retired(item.capability.id.as_ref())) { return Ok(false); }
            let mut payload = current.payload.clone();
            payload.enabled_capabilities.retain(|item| !retired(item.capability.id.as_ref()));
            payload.context_order.retain(|id| !retired(id.as_ref()));
            payload.middleware_order.retain(|id| !retired(id.as_ref()));
            let snapshot = self.current_snapshot(Some(&current)).await?;
            let draft = draft_api(&stored, Some(&current), payload_document(&payload)?, None)?;
            let compilation = self.authoring_compiler(owner).await?.compile_payload(
                owner, &draft, payload, Some(&current), snapshot.as_ref(), self.catalog.snapshot()?.as_ref(),
            )?;
            let snapshot = compilation.snapshot.ok_or_else(|| ControlPlaneError::with_details(
                "APP_PLUGIN_RETIREMENT_REQUIRES_CONFIGURATION", axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                format!("Agent {} has other unavailable references; its old configuration was preserved", stored.preset.display_name),
                json!({"preset_id":stored.preset.preset_id,"diagnostics":compilation.diagnostics}),
            ))?;
            let revision = AgentPresetRevision {
                reference: compilation.candidate_revision_ref, payload: compilation.payload,
                contribution_locks: compilation.contribution_locks, created_by: owner.clone(),
                created_at_ms: snapshot.created_at_ms, reason: Some("Retire replaced application-plugin grants".into()),
            };
            revision.validate().map_err(|violation| ControlPlaneError::Wire(violation.message))?;
            self.store.append_revision(stored.preset.current_stable_revision.as_ref(), revision, snapshot,
                stored.preset.display_name, stored.preset.description).await?;
            Ok(true)
            }.await;
            match retired_configuration {
                Ok(true) => changed += 1,
                Ok(false) => {},
                Err(error) => tracing::warn!(preset_id = preset_id.as_ref(), code = error.code().as_ref(),
                    "Application-plugin retirement requires this Agent's configuration to be corrected; its original revision and immutable history were preserved"),
            }
        }
        Ok(changed)
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
                        "capability {} is not materialized",
                        reference.id.as_ref()
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
                    payload_document(&revision.payload)?,
                    None,
                )
                .await;
        }
        let has_configuration = request.document.is_some();
        let document = request.document.unwrap_or_else(empty_document);
        if !has_configuration {
            let stored = StoredPreset {
                session_only: false,
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
        // An authored configuration may leave its default model empty. The
        // execution entry point owns the eventual explicit model selection.
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
        if request.reuse_existing {
            return self.prepare_official_preset(owner, template_key, request).await;
        }
        let seed = self
            .templates
            .seed(template_key)
            .ok_or_else(|| not_found("OfficialPresetTemplate"))?;
        let display_name = nonempty_name(request.display_name)?;
        let required_chat_features = required_chat_features(
            seed.enabled_capabilities
                .iter()
                .map(|selection| selection.capability.id.as_ref()),
        );
        let mut model_route_refs = request.model_route_refs;
        let mut chat_route_records = request.chat_route_records;
        let uses_default_route = !model_route_refs.contains_key(CHAT_MODEL_TASK)
            && !chat_route_records.contains_key(CHAT_MODEL_TASK);
        if request.model.is_some() && !uses_default_route {
            return Err(ControlPlaneError::canonical("MODEL_ROUTE_RECORD_INVALID", axum::http::StatusCode::BAD_REQUEST,
                "select a model or provide an explicit route, not both"));
        }
        // Creating an authored Agent does not authorize adopting the current
        // installation model. Only an explicit model/route enters its defaults.
        if uses_default_route && let Some(model) = request.model.as_ref()
        {
            let record = self.resolve_selected_chat_route(owner, model, &required_chat_features).await?;
            model_route_refs.insert(
                CHAT_MODEL_TASK.to_owned(),
                record.primary.model_route_id.as_ref().to_owned(),
            );
            chat_route_records.insert(
                CHAT_MODEL_TASK.to_owned(),
                serde_json::to_value(record)?,
            );
        }
        let catalog = self.catalog.snapshot()?;
        let document = nomifun_api_types::AgentPresetDocumentDto {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: "1.0.0".into(),
            model_route_refs,
            chat_route_records,
            enabled_capabilities: seed
                .enabled_capabilities
                .iter()
                .map(|reference| template_selection_api(reference, &catalog))
                .collect::<Result<Vec<_>, _>>()?,
            skill_bindings: seed.skill_bindings.iter().map(exact_ref_api).collect(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: String::new(),
            instructions: official_template_instructions(template_key).to_owned(),
            starter_prompts: Vec::new(),
            runtime_policy: Default::default(),
        };
        self.create_configuration_with_initial_revision(
            owner,
            AgentPresetId::from(Uuid::now_v7().to_string()),
            display_name,
            request.description,
            document,
            Some(template_key),
            false,
            None,
        )
        .await
    }

    /// Each owner edits one stable official identity. Seed content is used only
    /// before its first Revision; later launches preserve the authored payload.
    async fn ensure_official_preset(
        &self, owner: &UserId, key: OfficialPresetKey,
    ) -> Result<StoredPreset, ControlPlaneError> {
        let id = official_preset_id(owner, key);
        if let Some(preset) = self.store.get_preset(&id).await? {
            if preset.preset.owner_user_id.as_ref() != Some(owner)
                || preset.preset.source != AgentPresetSource::Official || preset.session_only
            {
                return Err(ControlPlaneError::Wire("official Agent identity does not match its owner".into()));
            }
            return Ok(preset);
        }
        let preset = StoredPreset {
            session_only: false,
            preset: AgentPreset {
                preset_id: id.clone(), owner_user_id: Some(owner.clone()),
                source: AgentPresetSource::Official, display_name: key.as_str().to_owned(),
                description: None, current_stable_revision: None,
            },
        };
        if let Err(error) = self.store.insert_preset(preset.clone()).await {
            // Another request/process may have inserted the same stable row.
            let Some(existing) = self.store.get_preset(&id).await? else { return Err(error); };
            if existing.preset.owner_user_id.as_ref() != Some(owner)
                || existing.preset.source != AgentPresetSource::Official || existing.session_only
            { return Err(error); }
            return Ok(existing);
        }
        Ok(preset)
    }

    fn official_seed_document(
        &self, key: OfficialPresetKey, catalog: &CatalogSnapshot,
    ) -> Result<nomifun_api_types::AgentPresetDocumentDto, ControlPlaneError> {
        let seed = self.templates.seed(key).ok_or_else(|| not_found("OfficialPresetTemplate"))?;
        let mut document = empty_document();
        document.enabled_capabilities = seed.enabled_capabilities.iter()
            .map(|selection| template_selection_api(selection, catalog))
            .collect::<Result<Vec<_>, _>>()?;
        document.skill_bindings = seed.skill_bindings.iter().map(exact_ref_api).collect();
        document.instructions = official_template_instructions(key).to_owned();
        Ok(document)
    }

    async fn prepare_official_preset(
        &self, owner: &UserId, key: OfficialPresetKey,
        request: CreateAgentPresetFromTemplateRequest,
    ) -> Result<AgentPresetEditorResponse, ControlPlaneError> {
        let has_explicit_route = request.model_route_refs.contains_key(CHAT_MODEL_TASK)
            || request.chat_route_records.contains_key(CHAT_MODEL_TASK);
        if request.model.is_some() && has_explicit_route {
            return Err(ControlPlaneError::canonical("MODEL_ROUTE_RECORD_INVALID", axum::http::StatusCode::BAD_REQUEST,
                "select a model or provide an explicit route, not both"));
        }
        let id = {
            let _guard = self.template_launch_lock.lock().await;
            let mut preset = self.ensure_official_preset(owner, key).await?;
            let editor = self.editor(owner, preset.preset.preset_id.as_ref(), None).await?;
            if let Some(reference) = preset.preset.current_stable_revision.as_ref() {
                let revision = self.store.get_revision(reference).await?.ok_or_else(|| not_found("AgentPresetRevision"))?;
                if nomifun_agent_contracts::bounded_process_launch_payload(&revision.payload) != revision.payload {
                    let bounded = self.resolve_agent_session_binding_locked(owner, preset.preset.preset_id.as_ref()).await?;
                    preset = self.owned_preset(owner, &bounded.preset_revision_ref.preset_id).await?;
                } else {
                    self.refresh_session_configuration(owner, &preset, &revision, Some(key)).await?;
                }
            } else {
                let mut draft = editor.draft;
                if has_explicit_route {
                    draft.document.model_route_refs = request.model_route_refs.clone();
                    draft.document.chat_route_records = request.chat_route_records.clone();
                } else if let Some(model) = request.model.as_ref() {
                    let required = required_chat_features(draft.document.enabled_capabilities.iter().map(|selection| selection.capability.id.as_str()));
                    let route = self.resolve_selected_chat_route(owner, model, &required).await?;
                    draft.document.model_route_refs.insert(CHAT_MODEL_TASK.to_owned(), route.primary.model_route_id.as_ref().to_owned());
                    draft.document.chat_route_records.insert(CHAT_MODEL_TASK.to_owned(), serde_json::to_value(route)?);
                } else {
                    draft.document = self.materialize_default_chat_route(owner, draft.document).await?;
                }
                // A first launch is not an authoring save. Its selected or
                // inherited execution model belongs only to the existing
                // hidden Session configuration, never the official default.
                let payload = document_payload(&draft.document)?;
                for existing in self.store.list_presets(owner).await? {
                    if !existing.session_only || owned_official_key(&existing) != Some(key)
                        || existing.preset.description != preset.preset.description { continue; }
                    let Some((revision, _)) = self.reusable_session_configuration(
                        &existing, &payload, !has_explicit_route,
                    ).await else { continue; };
                    self.refresh_session_configuration(owner, &existing, &revision, Some(key)).await?;
                    return self.editor(owner, existing.preset.preset_id.as_ref(), None).await;
                }
                return self.create_configuration_with_payload(owner, AgentPresetId::from(Uuid::now_v7().to_string()),
                    key.as_str().to_owned(), preset.preset.description, payload, Some(key), true, None).await;
            }
            // Explicit route objects are launch-local overrides, like model
            // selections. They never overwrite a saved official configuration.
            if has_explicit_route {
                let source = self.owned_preset(owner, preset.preset.preset_id.as_ref()).await?;
                let revision = self.current_revision(&source).await?.ok_or_else(|| not_found("AgentPresetRevision"))?;
                let mut document = payload_document(&revision.payload)?;
                document.model_route_refs = request.model_route_refs;
                document.chat_route_records = request.chat_route_records;
                let payload = document_payload(&document)?;
                if payload != revision.payload {
                    let snapshot = self.current_snapshot(Some(&revision)).await?;
                    let variant = self.create_configuration_with_payload(owner, AgentPresetId::from(Uuid::now_v7().to_string()),
                        key.as_str().to_owned(), source.preset.description, payload, Some(key), true, snapshot.as_ref()).await?;
                    return Ok(variant);
                }
            }
            preset.preset.preset_id
        };
        if let Some(model) = request.model.as_ref() {
            let binding = self.resolve_base_agent_session_binding_with_model(owner, id.as_ref(), Some(model)).await?;
            return self.editor(owner, &binding.preset_revision_ref.preset_id, None).await;
        }
        self.editor(owner, id.as_ref(), None).await
    }

    async fn resolve_selected_chat_route(
        &self,
        owner: &UserId,
        model: &nomifun_api_types::AgentChatModelSelectionDto,
        required: &BTreeSet<ChatRouteFeature>,
    ) -> Result<nomifun_agent_contracts::ChatRouteRecord, ControlPlaneError> {
        let unavailable = || ControlPlaneError::canonical("MODEL_ROUTE_NOT_FOUND",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY, "the selected provider/model has no enabled Chat capability");
        let resolver = self.default_chat_route_resolver.as_ref().ok_or_else(unavailable)?;
        let route = resolver.resolve_selected_chat_route(owner, model).await?.ok_or_else(unavailable)?;
        route.validate().map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        if route.primary.provider_id != model.provider_id || route.primary.model != model.model {
            return Err(unavailable());
        }
        ensure_chat_route_supports(&route, required)?;
        Ok(route)
    }

    /// Resolve one friendly model selection into the host-owned route record
    /// for editing. Inspecting a choice never saves a Revision or changes a
    /// Session; the caller submits the selected route through normal authoring.
    pub async fn preset_chat_model_route(
        &self,
        owner: &UserId,
        preset_id: &str,
        model: &nomifun_api_types::AgentChatModelSelectionDto,
    ) -> Result<nomifun_agent_contracts::ChatRouteRecord, ControlPlaneError> {
        self.owned_preset(owner, preset_id).await?;
        self.resolve_selected_chat_route(owner, model, &BTreeSet::new()).await
    }

    /// Freeze the chosen model for this new session without updating the user's
    /// stable Agent configuration. Internal variants are hidden from the library.
    pub async fn resolve_agent_session_binding_with_model(
        &self,
        owner: &UserId,
        preset_id: &str,
        model: Option<&nomifun_api_types::AgentChatModelSelectionDto>,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        self.resolve_agent_session_binding_with_capabilities(owner, preset_id, model, None).await
    }

    /// Resolve the shared Companion definition while preserving its authored
    /// Skill choices before global defaults are applied. All entry surfaces
    /// consume this same definition; resources remain local to each Session.
    pub async fn resolve_companion_session_binding_with_model(
        &self,
        owner: &UserId,
        preset_id: &str,
        model: Option<&nomifun_api_types::AgentChatModelSelectionDto>,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let binding = self.resolve_base_agent_session_binding_with_model(owner, preset_id, model).await?;
        let contract: AgentBindingValue = wire_cast(&binding)?;
        let (_, snapshot) = self.load_binding_artifacts(owner, &contract).await?;
        let selection = nomifun_api_types::SessionCapabilitySelectionDto {
            skill_names: snapshot.content.skill_locks.iter().filter_map(|lock| match lock {
                nomifun_agent_contracts::ResolvedSkillLock::Package(lock) => Some(lock.skill.id.as_ref().to_owned()),
                nomifun_agent_contracts::ResolvedSkillLock::Library { skill, selected: true, .. } => Some(skill.name.clone()),
                _ => None,
            }).collect::<BTreeSet<_>>().into_iter().collect(),
            mcp_server_ids: snapshot.content.mcp_tool_locks.iter().map(|lock| lock.server_id.as_ref().to_owned())
                .collect::<BTreeSet<_>>().into_iter().collect(),
        };
        self.resolve_agent_session_capabilities_binding_selected_skills(owner, &binding, &selection).await
    }

    pub async fn resolve_agent_session_binding_with_capabilities(
        &self,
        owner: &UserId,
        preset_id: &str,
        model: Option<&nomifun_api_types::AgentChatModelSelectionDto>,
        selection: Option<&nomifun_api_types::SessionCapabilitySelectionDto>,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let binding = self.resolve_base_agent_session_binding_with_model(owner, preset_id, model).await?;
        self.resolve_agent_session_capabilities_binding(owner, &binding, selection).await
    }

    /// Global extensions belong to the Session, independent of the Agent's
    /// authored modules. Compile them through the same immutable artifacts.
    pub async fn resolve_agent_session_capabilities_binding(
        &self,
        owner: &UserId,
        current: &AgentBindingValueDto,
        selection: Option<&nomifun_api_types::SessionCapabilitySelectionDto>,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        self.resolve_session_capabilities_binding(owner, current, selection, false).await
    }

    /// Freeze only explicitly selected Skill data for a product configuration.
    /// All Companion entries share this policy and their author-selected data;
    /// model input and host extras cannot choose another authority ceiling. Unselected frozen library locks from a
    /// previous binding are retired through the same canonical compilation.
    pub async fn resolve_agent_session_capabilities_binding_selected_skills(
        &self,
        owner: &UserId,
        current: &AgentBindingValueDto,
        selection: &nomifun_api_types::SessionCapabilitySelectionDto,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        self.resolve_session_capabilities_binding(owner, current, Some(selection), true).await
    }

    async fn resolve_session_capabilities_binding(
        &self,
        owner: &UserId,
        current: &AgentBindingValueDto,
        selection: Option<&nomifun_api_types::SessionCapabilitySelectionDto>,
        selected_skills_only: bool,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let Some(resolver) = &self.session_capabilities_resolver else {
            if selection.is_some() {
                return Err(ControlPlaneError::canonical("SESSION_CAPABILITIES_UNAVAILABLE", axum::http::StatusCode::SERVICE_UNAVAILABLE, "global extension selection is unavailable on this host"));
            }
            return Ok(current.clone());
        };
        if selection.is_some_and(|selection| selection.skill_names.len() > 128
            || selection.skill_names.iter().collect::<BTreeSet<_>>().len() != selection.skill_names.len()) {
            return Err(ControlPlaneError::canonical("SESSION_CAPABILITIES_INVALID", axum::http::StatusCode::UNPROCESSABLE_ENTITY, "Skill selection must contain distinct names within the Session limit"));
        }
        let _guard = self.template_launch_lock.lock().await;
        let current_contract: AgentBindingValue = wire_cast(current)?;
        let (source_revision, source_snapshot) = self.load_binding_artifacts(owner, &current_contract).await?;
        let package_names = source_revision.payload.skill_bindings.iter().filter_map(|binding| binding.package_ref())
            .map(|skill| skill.id.as_ref().to_owned()).collect::<BTreeSet<_>>();
        let requested_library = selection.map(|selection| {
            let mut selection = selection.clone();
            selection.skill_names.retain(|name| !package_names.contains(name));
            selection
        });
        let mut resolved = resolver.resolve(owner, requested_library.as_ref()).await?;
        let mut authored_only_servers = BTreeSet::new();
        // An Agent's saved MCP grants are explicit choices and survive normal
        // Session compilation without selecting other installed sources.
        if selection.is_none() && !source_snapshot.content.mcp_tool_locks.is_empty() {
            let mut selected = resolved.mcp_server_ids.clone();
            for lock in &source_snapshot.content.mcp_tool_locks {
                let id = lock.server_id.as_ref().to_owned();
                if !selected.contains(&id) { authored_only_servers.insert(id.clone()); }
                selected.insert(id);
            }
            let explicit = nomifun_api_types::SessionCapabilitySelectionDto {
                skill_names: resolved.selected_skill_names.iter().cloned().collect(),
                mcp_server_ids: selected.into_iter().collect(),
            };
            resolved = resolver.resolve(owner, Some(&explicit)).await?;
        }
        if selected_skills_only {
            // Global discovery normally freezes every installed Skill, even
            // when none is selected. This selected-data compilation does not
            // freeze unrelated Skill bodies into the product configuration.
            let expected = requested_library.as_ref().ok_or_else(|| ControlPlaneError::canonical(
                "SESSION_CAPABILITIES_INVALID", axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "the product configuration requires an explicit Skill selection"))?
                .skill_names.iter().cloned().collect::<BTreeSet<_>>();
            if resolved.selected_skill_names != expected || expected.iter().any(|name|
                !resolved.skills.iter().any(|skill| &skill.name == name)) {
                return Err(ControlPlaneError::canonical("SESSION_CAPABILITIES_INVALID", axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "resolved product Skills differ from the explicit selection"));
            }
            resolved.skills.retain(|skill| resolved.selected_skill_names.contains(&skill.name));
        }
        let source = self.owned_preset(owner, source_revision.reference.preset_id.as_ref()).await?;
        let catalog = self.catalog.snapshot()?;
        let mut payload = source_revision.payload.clone();
        payload.enabled_capabilities.retain(|item| !is_global_extension_module(item.capability.id.as_ref()));
        payload.skill_bindings.retain(|binding| binding.package_ref().is_some_and(|skill|
            selection.is_none_or(|selection| selection.skill_names.iter().any(|name| name == skill.id.as_ref()))));
        payload.skill_bindings.extend(resolved.skills.into_iter().map(|skill| {
            let selected = resolved.selected_skill_names.contains(&skill.name);
            nomifun_agent_contracts::AgentSkillBinding::library_selected(skill, selected)
        }));
        let mut mapped_servers = BTreeSet::new();
        for tool in &catalog.mcp_tools {
            let server_id = tool.mapping.server_id.as_ref();
            if !resolved.mcp_server_ids.contains(server_id) { continue; }
            let authored = source_revision.payload.enabled_capabilities.iter()
                .find(|selection| selection.capability == tool.mapping.capability);
            if authored_only_servers.contains(server_id) && authored.is_none() { continue; }
            mapped_servers.insert(server_id.to_owned());
            let capability = catalog.capabilities.iter().find(|item| item.manifest.id == tool.mapping.capability.id)
                .ok_or_else(|| ControlPlaneError::canonical("MCP_CATALOG_UNAVAILABLE", axum::http::StatusCode::UNPROCESSABLE_ENTITY, "the selected MCP tool is unavailable"))?;
            payload.enabled_capabilities.push(CapabilitySelection {
                capability: tool.mapping.capability.clone(),
                action_allowlist: if authored_only_servers.contains(server_id) {
                    authored.expect("authored selection checked").action_allowlist.clone()
                } else { capability.manifest.contributions.actions.iter().map(|action| action.action_id.clone()).collect() },
            });
        }
        if mapped_servers != resolved.mcp_server_ids {
            return Err(ControlPlaneError::canonical("MCP_CATALOG_UNAVAILABLE", axum::http::StatusCode::UNPROCESSABLE_ENTITY, "test and enable the selected MCP server before using it"));
        }
        // Tool discovery is a built-in runtime facility, never a user module gate.
        if !payload.enabled_capabilities.is_empty() || !payload.skill_bindings.is_empty() {
            if let Some(capability) = catalog.capabilities.iter().find(|item| item.manifest.id.as_ref() == "agent.tool-discovery") {
                payload.enabled_capabilities.push(CapabilitySelection {
                    capability: CapabilityRef { id: capability.manifest.id.clone() },
                    action_allowlist: capability.manifest.contributions.actions.iter().map(|action| action.action_id.clone()).collect(),
                });
            }
        }
        payload.enabled_capabilities.sort_by(|left, right| left.capability.id.cmp(&right.capability.id));
        if payload.enabled_capabilities.len() > 128 {
            let selected = payload.enabled_capabilities.iter().map(|item|&item.capability.id).collect::<BTreeSet<_>>();
            let mut counts = BTreeMap::<&str,usize>::new();
            for tool in &catalog.mcp_tools {
                if selected.contains(&tool.mapping.capability.id) { *counts.entry(tool.mapping.server_id.as_ref()).or_default() += 1; }
            }
            let sources = counts.into_iter().map(|(id,count)|format!("MCP source {id}={count}")).collect::<Vec<_>>().join(", ");
            return Err(ControlPlaneError::canonical("SESSION_CAPABILITY_LIMIT", axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                format!("selected {} capabilities exceeds limit 128; source counts: {sources}; choose fewer tools or sources for this Session",payload.enabled_capabilities.len())));
        }
        let mutable_roots = source_revision.payload.enabled_capabilities.iter()
            .chain(payload.enabled_capabilities.iter())
            .filter(|selection| is_global_extension_module(selection.capability.id.as_ref()))
            .map(|selection| selection.capability.id.clone()).collect::<BTreeSet<_>>();
        // Discovery is host infrastructure. An extension selection can add or
        // retire it, but cannot authorize new grants on an existing reader.
        let protected_retained_roots = source_revision.payload.enabled_capabilities.iter()
            .filter(|selection| selection.capability.id.as_ref() == "agent.tool-discovery"
                && payload.enabled_capabilities.iter().any(|target| target.capability == selection.capability))
            .map(|selection| selection.capability.id.clone()).collect::<BTreeSet<_>>();
        let preset_id = AgentPresetId::from(Uuid::now_v7().to_string());
        let (revision, snapshot) = self.authoring_compiler(owner).await?.derive_session_extensions(
            owner, preset_id.clone(), payload, &source_revision, &source_snapshot, &catalog,
            &mutable_roots, &protected_retained_roots,
        )?;
        if revision.payload == source_revision.payload
            && same_non_model_revision_contract(&revision, &source_revision)
            && same_execution_snapshot_contract(&snapshot, &source_snapshot)
        {
            return Ok(current.clone());
        }
        // The fresh candidate has already passed canonical compilation on the
        // Session's saved mounts. Reuse only its exact executable contract.
        for candidate in self.store.list_presets(owner).await? {
            if !candidate.session_only
                || candidate.preset.source != source.preset.source
                || candidate.preset.display_name != variant_display_name(&source)
                || candidate.preset.description != source.preset.description
            { continue; }
            let Some(saved) = self.current_revision(&candidate).await? else { continue; };
            if saved.payload != revision.payload || saved.contribution_locks != revision.contribution_locks { continue; }
            let Some(saved_snapshot) = self.current_snapshot(Some(&saved)).await? else { continue; };
            if !same_execution_snapshot_contract(&saved_snapshot, &snapshot) { continue; }
            let mut binding = session_binding_from_stable_artifacts(saved.reference.clone(), Some(saved), Some(saved_snapshot))?;
            binding.binding_version = current.binding_version;
            self.validate_agent_binding(owner, &binding).await?;
            return wire_cast(&binding);
        }
        let reference = revision.reference.clone();
        let mut binding = session_binding_from_stable_artifacts(reference.clone(), Some(revision.clone()), Some(snapshot.clone()))?;
        self.store.insert_preset_with_revision(StoredPreset {
            session_only: true,
            preset: AgentPreset {
                preset_id, owner_user_id: Some(owner.clone()), source: source.preset.source.clone(),
                display_name: variant_display_name(&source), description: source.preset.description,
                current_stable_revision: Some(reference),
            },
        }, revision, snapshot).await?;
        binding.binding_version = current.binding_version;
        self.validate_agent_binding(owner, &binding).await?;
        wire_cast(&binding)
    }

    /// Prepare an explicit App workspace workflow update from the exact saved
    /// binding. The caller still owns the ordinary idle/teardown/Effect fence
    /// and must commit the returned target through the canonical transition.
    /// This does not replace a binding or upgrade it while reading/reopening.
    pub async fn resolve_app_development_workflow_binding(
        &self,
        owner: &UserId,
        current: &AgentBindingValueDto,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let _guard = self.template_launch_lock.lock().await;
        let current_contract: AgentBindingValue = wire_cast(current)?;
        let (source_revision, source_snapshot) = self.load_binding_artifacts(owner, &current_contract).await?;
        let source = self.owned_preset(owner, source_revision.reference.preset_id.as_ref()).await?;
        if self.official_template_for_revision(&source, &source_revision)? != Some(OfficialPresetKey::AppDeveloper) {
            return Err(ControlPlaneError::canonical("APP_DEVELOPMENT_SESSION_MISMATCH", axum::http::StatusCode::CONFLICT,
                "The saved Session is not an official built-in application configuration; its instructions were preserved"));
        }
        if source_revision.payload.instructions == APP_DEVELOPMENT_INSTRUCTIONS {
            return Ok(current.clone());
        }
        if !source_revision.payload.instructions.is_empty() {
            return Err(ControlPlaneError::canonical("APP_DEVELOPMENT_SESSION_MISMATCH", axum::http::StatusCode::CONFLICT,
                "The saved application instructions cannot be replaced by a product workflow update"));
        }

        let mut payload = source_revision.payload.clone();
        payload.instructions = APP_DEVELOPMENT_INSTRUCTIONS.to_owned();
        let preset_id = AgentPresetId::from(Uuid::now_v7().to_string());
        let draft = AgentPresetDraftDto {
            preset_id: preset_id.as_ref().to_owned(), display_name: OfficialPresetKey::AppDeveloper.as_str().to_owned(),
            description: source.preset.description.clone(), source_template_key: None, current_revision: None,
            document: payload_document(&payload)?,
        };
        let catalog = self.catalog.snapshot()?;
        let compilation = self.authoring_compiler(owner).await?.compile_payload(
            owner, &draft, payload.clone(), None, Some(&source_snapshot), &catalog,
        )?;
        let snapshot = compilation.snapshot.ok_or_else(|| ControlPlaneError::with_details(
            "APP_DEVELOPMENT_WORKFLOW_UPDATE_FAILED", axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "The built-in application workflow could not be compiled; the saved Session was preserved",
            json!({"diagnostics":compilation.diagnostics}),
        ))?;
        // Refresh only the selected hosting contract used by this product
        // command. All other frozen tools, roles, Skills, MCP and model remain
        // exact; catalog changes cannot become new authority during this update.
        if compilation.payload != payload || !same_app_workflow_snapshot_contract(&snapshot, &source_snapshot) {
            return Err(ControlPlaneError::canonical("APP_DEVELOPMENT_WORKFLOW_CONTRACT_CHANGED", axum::http::StatusCode::CONFLICT,
                "Updating application instructions would change another frozen contract; the saved Session was preserved"));
        }
        let revision = AgentPresetRevision {
            reference: compilation.candidate_revision_ref, payload: compilation.payload,
            contribution_locks: compilation.contribution_locks, created_by: owner.clone(),
            created_at_ms: snapshot.created_at_ms, reason: Some("Explicit application development workflow update".into()),
        };
        revision.validate().map_err(|violation| ControlPlaneError::canonical(violation.code,
            axum::http::StatusCode::UNPROCESSABLE_ENTITY, violation.message))?;
        let mut binding = session_binding_from_stable_artifacts(revision.reference.clone(), Some(revision.clone()), Some(snapshot.clone()))?;
        binding.binding_version = current_contract.binding_version;
        binding.typed_resource_bindings = current_contract.typed_resource_bindings;
        self.store.insert_preset_with_revision(StoredPreset { session_only: true, preset: AgentPreset {
            preset_id, owner_user_id: Some(owner.clone()), source: source.preset.source.clone(),
            display_name: draft.display_name, description: source.preset.description,
            current_stable_revision: Some(revision.reference.clone()),
        }}, revision, snapshot).await?;
        self.validate_agent_binding(owner, &binding).await?;
        wire_cast(&binding)
    }

    async fn resolve_base_agent_session_binding_with_model(
        &self,
        owner: &UserId,
        preset_id: &str,
        model: Option<&nomifun_api_types::AgentChatModelSelectionDto>,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let Some(model) = model else {
            let _guard = self.template_launch_lock.lock().await;
            return self.resolve_agent_session_binding_locked(owner, preset_id).await;
        };
        let _guard = self.template_launch_lock.lock().await;
        let mut source = self.owned_preset(owner, preset_id).await?;
        require_current_preset_admission(&source, owner)?;
        let mut revision = self.current_revision(&source).await?.ok_or_else(|| not_found("AgentPresetRevision"))?;
        if nomifun_agent_contracts::bounded_process_launch_payload(&revision.payload) != revision.payload {
            // Rebase the execution contract before deriving an explicitly
            // selected model. The old authorial Agent is never rewritten.
            let bounded = self.resolve_agent_session_binding_locked(owner, preset_id).await?;
            source = self.owned_preset(owner, &bounded.preset_revision_ref.preset_id).await?;
            revision = self.current_revision(&source).await?.ok_or_else(|| not_found("AgentPresetRevision"))?;
        }
        // A media-only configuration may omit Chat, but an explicit language
        // model selection still creates a validated Session-local Chat route.
        let source_snapshot = self.current_snapshot(Some(&revision)).await?;
        if source_snapshot.is_none() {
            return Err(ControlPlaneError::canonical("CAPABILITY_NOT_MATERIALIZED",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY, "the saved Agent has no compiled Snapshot"));
        }
        let required = required_chat_features(
            revision
                .payload
                .enabled_capabilities
                .iter()
                .map(|selection| selection.capability.id.as_ref()),
        );
        let route = self
            .resolve_selected_chat_route(owner, model, &required)
            .await?;
        let mut payload = revision.payload.clone();
        payload.model_route_refs.insert(CHAT_MODEL_TASK.into(), route.primary.model_route_id.clone());
        payload.chat_route_records.insert(CHAT_MODEL_TASK.into(), route);
        if template_launch_payload_matches(&revision.payload, &payload, true) {
            return self.resolve_agent_session_binding_locked(owner, preset_id).await;
        }
        for existing in self.store.list_presets(owner).await? {
            if existing.preset.source != source.preset.source
                || owned_official_key(&existing) != owned_official_key(&source)
                || existing.preset.display_name != variant_display_name(&source)
                || existing.preset.description != source.preset.description
            { continue; }
            let Some((_saved, snapshot)) = self.reusable_session_configuration(
                &existing, &payload, true,
            ).await else { continue; };
            if Some(snapshot.content.required_runtime_profile)
                == source_snapshot.as_ref().map(|value| value.content.required_runtime_profile)
            {
                return self.resolve_agent_session_binding_locked(owner, existing.preset.preset_id.as_ref()).await;
            }
        }
        let prepared = self.create_configuration_with_payload(
            owner, AgentPresetId::from(Uuid::now_v7().to_string()), variant_display_name(&source),
            source.preset.description.clone(), payload, owned_official_key(&source), true, source_snapshot.as_ref(),
        ).await?;
        self.resolve_agent_session_binding_locked(owner, &prepared.preset.preset_id).await
    }

    /// Resolve a complete target Agent identity for an existing Session while
    /// preserving its current Chat model unless the caller explicitly selects
    /// another one. Resources are intentionally left empty for the host-owned
    /// resource resolver to re-admit; the client never supplies a binding.
    pub async fn resolve_agent_session_agent_binding(
        &self,
        owner: &UserId,
        current: &AgentBindingValueDto,
        target_preset_id: &str,
        explicit_model: Option<&nomifun_api_types::AgentChatModelSelectionDto>,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let replacement = self.resolve_agent_session_agent_binding_candidate(owner, current,
            target_preset_id, explicit_model).await?;
        if replacement.preset_revision_ref == current.preset_revision_ref
            && replacement.resolved_snapshot_ref == current.resolved_snapshot_ref
        {
            return Err(ControlPlaneError::canonical("AGENT_SESSION_AGENT_UNCHANGED",
                axum::http::StatusCode::CONFLICT, "the selected Agent is already active in this Session"));
        }
        Ok(replacement)
    }

    /// Resolve the validated execution candidate before the product owner
    /// decides whether an exact match represents the same authored selection.
    pub async fn resolve_agent_session_agent_binding_candidate(
        &self,
        owner: &UserId,
        current: &AgentBindingValueDto,
        target_preset_id: &str,
        explicit_model: Option<&nomifun_api_types::AgentChatModelSelectionDto>,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let current_contract: AgentBindingValue = wire_cast(current)?;
        let (current_revision, _) = self
            .load_binding_artifacts(owner, &current_contract)
            .await?;
        let preserved_model = current_revision
            .payload
            .chat_route_records
            .get(CHAT_MODEL_TASK)
            .map(|route| nomifun_api_types::AgentChatModelSelectionDto {
                provider_id: route.primary.provider_id.clone(),
                model: route.primary.model.clone(),
            });
        let selected_model = explicit_model.or(preserved_model.as_ref());
        let mut replacement = self
            .resolve_agent_session_binding_with_model(owner, target_preset_id, selected_model)
            .await?;
        replacement.binding_version = current.binding_version.checked_add(1).ok_or_else(|| {
            ControlPlaneError::canonical(
                "AGENT_SESSION_BINDING_VERSION_EXHAUSTED",
                axum::http::StatusCode::CONFLICT,
                "AgentSession binding version cannot advance",
            )
        })?;
        replacement.typed_resource_bindings.clear();
        Ok(replacement)
    }

    /// Preserve admitted immutable execution facts and verified authoring
    /// provenance while giving an explicit binding change a distinct identity.
    /// The caller must prove a different authored selection or resolved resource
    /// set before using the ordinary Session binding transition.
    pub async fn distinguish_agent_session_binding_identity(
        &self,
        owner: &UserId,
        binding: &AgentBindingValueDto,
        target_preset_id: &str,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let _guard = self.template_launch_lock.lock().await;
        let target = self.owned_preset(owner, target_preset_id).await?;
        let contract: AgentBindingValue = wire_cast(binding)?;
        let (revision, snapshot) = self.load_binding_artifacts(owner, &contract).await?;
        let preset_id = AgentPresetId::from(Uuid::now_v7().to_string());
        let (revision, snapshot) = self.compiler.derive_session_identity(owner, preset_id.clone(), &revision, &snapshot)?;
        let reference = revision.reference.clone();
        let mut replacement = session_binding_from_stable_artifacts(reference.clone(), Some(revision.clone()), Some(snapshot.clone()))?;
        self.store.insert_preset_with_revision(StoredPreset {
            session_only: true,
            preset: AgentPreset { preset_id, owner_user_id: Some(owner.clone()), source: target.preset.source.clone(),
                display_name: variant_display_name(&target), description: target.preset.description,
                current_stable_revision: Some(reference) },
        }, revision, snapshot).await?;
        replacement.binding_version = binding.binding_version;
        self.validate_agent_binding(owner, &replacement).await?;
        wire_cast(&replacement)
    }

    /// Compare already admitted immutable execution contracts while ignoring
    /// their cache identities. Product owners use this only after proving the
    /// selected authored Agent itself has not changed.
    pub async fn same_agent_session_execution_contract(
        &self,
        owner: &UserId,
        left: &AgentBindingValueDto,
        right: &AgentBindingValueDto,
    ) -> Result<bool, ControlPlaneError> {
        let (left_revision, left_snapshot) = self.load_binding_artifacts(owner, &wire_cast(left)?).await?;
        let (right_revision, right_snapshot) = self.load_binding_artifacts(owner, &wire_cast(right)?).await?;
        Ok(left_revision.payload == right_revision.payload
            && left_revision.contribution_locks == right_revision.contribution_locks
            && same_non_model_revision_contract(&left_revision, &right_revision)
            && same_execution_snapshot_contract(&left_snapshot, &right_snapshot))
    }

    /// Replace only the exact Chat route of an existing Session binding.
    ///
    /// Unlike new-Session resolution, this starts from the Session's saved
    /// immutable Revision/Snapshot rather than the Preset's current stable
    /// revision. That distinction is what keeps later Agent edits, capability
    /// catalog changes, Skills and resource authority out of a model switch.
    /// The returned binding carries the existing typed resources and advances
    /// its optimistic version exactly once.
    pub async fn resolve_agent_session_model_binding(
        &self,
        owner: &UserId,
        current: &AgentBindingValueDto,
        model: &nomifun_api_types::AgentChatModelSelectionDto,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let _guard = self.template_launch_lock.lock().await;
        let current: AgentBindingValue = wire_cast(current)?;
        let (source_revision, source_snapshot) = self
            .load_binding_artifacts(owner, &current)
            .await?;
        if !source_revision
            .payload
            .chat_route_records
            .contains_key(CHAT_MODEL_TASK)
        {
            return Err(ControlPlaneError::canonical(
                "AGENT_SESSION_MODEL_NOT_SWITCHABLE",
                axum::http::StatusCode::CONFLICT,
                "this AgentSession does not own a switchable Chat model route",
            ));
        }

        let required = required_chat_features(
            source_revision
                .payload
                .enabled_capabilities
                .iter()
                .map(|selection| selection.capability.id.as_ref()),
        );
        let route = self
            .resolve_selected_chat_route(owner, model, &required)
            .await?;
        if source_revision
            .payload
            .chat_route_records
            .get(CHAT_MODEL_TASK)
            .is_some_and(|current| same_chat_route_configuration(current, &route))
        {
            return wire_cast(&current);
        }
        let mut payload = source_revision.payload.clone();
        payload.model_route_refs.insert(
            CHAT_MODEL_TASK.into(),
            route.primary.model_route_id.clone(),
        );
        payload
            .chat_route_records
            .insert(CHAT_MODEL_TASK.into(), route.clone());
        if payload == source_revision.payload {
            return wire_cast(&current);
        }

        let next_binding_version = current.binding_version.checked_add(1).ok_or_else(|| {
            ControlPlaneError::canonical(
                "AGENT_SESSION_BINDING_VERSION_EXHAUSTED",
                axum::http::StatusCode::CONFLICT,
                "AgentSession binding version cannot advance",
            )
        })?;

        let source = self
            .owned_preset(owner, source_revision.reference.preset_id.as_ref())
            .await?;
        // Hidden variants are an implementation cache. Reuse one only when
        // both its authoring payload and every non-model materialized contract
        // are byte-for-byte equivalent to the currently frozen Session and
        // its user-selected presentation is preserved.
        for existing in self.store.list_presets(owner).await? {
            if !existing.session_only || existing.preset.source != source.preset.source
                || existing.preset.display_name != variant_display_name(&source)
                || existing.preset.description != source.preset.description
            {
                continue;
            }
            let candidate = async {
                let Some(revision) = self.current_revision(&existing).await? else {
                    return Ok(None);
                };
                if revision.payload != payload
                    || revision.contribution_locks != source_revision.contribution_locks
                {
                    return Ok(None);
                }
                let Some(snapshot) = self.current_snapshot(Some(&revision)).await? else {
                    return Ok(None);
                };
                if !same_non_model_snapshot_contract(&snapshot, &source_snapshot) {
                    return Ok(None);
                }
                let binding = session_binding_from_stable_artifacts(
                    revision.reference.clone(),
                    Some(revision),
                    Some(snapshot),
                )?;
                Ok::<_, ControlPlaneError>(Some(binding))
            }
            .await;
            match candidate {
                Ok(Some(mut binding)) => {
                    binding.typed_resource_bindings = current.typed_resource_bindings.clone();
                    binding.binding_version = next_binding_version;
                    self.validate_agent_binding(owner, &binding).await?;
                    return wire_cast(&binding);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        preset_id = existing.preset.preset_id.as_ref(),
                        code = error.code().as_ref(),
                        "skipping unreadable exact Session model variant"
                    );
                }
            }
        }

        let preset_id = AgentPresetId::from(Uuid::now_v7().to_string());
        let (revision, snapshot) = self.compiler.derive_session_model(
            owner, preset_id.clone(), route, &source_revision, &source_snapshot,
        )?;
        let reference = revision.reference.clone();

        let base_binding = session_binding_from_stable_artifacts(
            reference.clone(),
            Some(revision.clone()),
            Some(snapshot.clone()),
        )?;
        self.store
            .insert_preset_with_revision(
                StoredPreset {
                    session_only: true,
                    preset: AgentPreset {
                        preset_id,
                        owner_user_id: Some(owner.clone()),
                        source: source.preset.source.clone(),
                        display_name: variant_display_name(&source),
                        description: source.preset.description,
                        current_stable_revision: Some(reference),
                    },
                },
                revision,
                snapshot,
            )
            .await?;

        let mut binding = base_binding;
        binding.typed_resource_bindings = current.typed_resource_bindings;
        binding.binding_version = next_binding_version;
        self.validate_agent_binding(owner, &binding).await?;
        wire_cast(&binding)
    }

    /// Prepare an explicit forward binding transition for current-generation
    /// bundled contracts. This only stores immutable candidate artifacts;
    /// the canonical Session owner must commit the returned binding under its
    /// idle/cleanup/effect/checkpoint and optimistic-version fences.
    pub async fn resolve_agent_session_contract_evolution(
        &self,
        owner: &UserId,
        current: &AgentBindingValueDto,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let _guard = self.template_launch_lock.lock().await;
        self.resolve_agent_session_contract_evolution_locked(owner, current).await
    }

    /// The caller owns template_launch_lock. New Session creation uses this
    /// same proof before it has any live Session or resources to transition.
    async fn resolve_agent_session_contract_evolution_locked(
        &self,
        owner: &UserId,
        current: &AgentBindingValueDto,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let current: AgentBindingValue = wire_cast(current)?;
        let (source_revision, source_snapshot) = self.load_binding_artifacts(owner, &current).await?;
        let preset_id = AgentPresetId::from(Uuid::now_v7().to_string());
        let catalog = self.catalog.snapshot()?;
        let source = self.owned_preset(owner, source_revision.reference.preset_id.as_ref()).await?;
        // Only the host-owned default personal Agent's retired exact-empty
        // contract receives the new ordinary local OS tools. A custom Agent or
        // a deliberately trimmed task does not gain inferred authority.
        let add_default_local_os = owned_official_key(&source) == Some(OfficialPresetKey::ChatMinimal)
            && source_revision.payload.enabled_capabilities.is_empty()
            && source_revision.payload.skill_bindings.is_empty();
        let (revision, snapshot) = self.compiler.evolve_session_contract(
            owner, preset_id.clone(), &source_revision, &source_snapshot, &catalog, add_default_local_os,
        )?;
        if revision.payload == source_revision.payload
            && same_non_model_revision_contract(&revision, &source_revision)
            && same_execution_snapshot_contract(&snapshot, &source_snapshot)
        {
            return wire_cast(&current);
        }
        let next_binding_version = current.binding_version.checked_add(1).ok_or_else(|| {
            ControlPlaneError::canonical(
                "AGENT_SESSION_BINDING_VERSION_EXHAUSTED", axum::http::StatusCode::CONFLICT,
                "AgentSession binding version cannot advance",
            )
        })?;

        // Repeated preparation and concurrent consumers share immutable
        // artifacts, but each Session still commits its own canonical event.
        for existing in self.store.list_presets(owner).await? {
            if !existing.session_only || existing.preset.source != source.preset.source
                || existing.preset.display_name != variant_display_name(&source)
                || existing.preset.description != source.preset.description
            { continue; }
            let Some(saved) = self.current_revision(&existing).await? else { continue; };
            if saved.payload != revision.payload || saved.contribution_locks != revision.contribution_locks { continue; }
            let Some(saved_snapshot) = self.current_snapshot(Some(&saved)).await? else { continue; };
            if !same_execution_snapshot_contract(&saved_snapshot, &snapshot) { continue; }
            let mut binding = session_binding_from_stable_artifacts(saved.reference.clone(), Some(saved), Some(saved_snapshot))?;
            binding.typed_resource_bindings = current.typed_resource_bindings.clone();
            binding.binding_version = next_binding_version;
            self.validate_agent_binding(owner, &binding).await?;
            return wire_cast(&binding);
        }
        let reference = revision.reference.clone();
        let mut binding = session_binding_from_stable_artifacts(reference.clone(), Some(revision.clone()), Some(snapshot.clone()))?;
        self.store.insert_preset_with_revision(StoredPreset {
            session_only: true,
            preset: AgentPreset {
                preset_id, owner_user_id: Some(owner.clone()), source: source.preset.source.clone(),
                display_name: variant_display_name(&source), description: source.preset.description,
                current_stable_revision: Some(reference),
            },
        }, revision, snapshot).await?;
        binding.typed_resource_bindings = current.typed_resource_bindings;
        binding.binding_version = next_binding_version;
        self.validate_agent_binding(owner, &binding).await?;
        wire_cast(&binding)
    }

    /// Prove that a historical turn Snapshot belongs to the same frozen Agent
    /// contract and differs from the current binding only by its Chat route.
    /// Runtime history uses this to retain prior-model turns after a model
    /// switch without accepting history from another Agent or resource scope.
    pub async fn agent_session_model_history_compatible(
        &self,
        owner: &UserId,
        current: &AgentBindingValue,
        historical: &AgentBindingValue,
    ) -> Result<bool, ControlPlaneError> {
        if current.typed_resource_bindings != historical.typed_resource_bindings {
            return Ok(false);
        }
        let (current_revision, current_snapshot) =
            self.load_binding_artifacts(owner, current).await?;
        let (historical_revision, historical_snapshot) =
            self.load_binding_artifacts(owner, historical).await?;
        Ok(same_non_model_revision_contract(
            &current_revision,
            &historical_revision,
        ) && same_non_model_snapshot_contract(
            &current_snapshot,
            &historical_snapshot,
        ))
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

    async fn resolve_default_chat_route_for_features(
        &self,
        owner: &UserId,
        required: &BTreeSet<ChatRouteFeature>,
    ) -> Result<Option<nomifun_agent_contracts::ChatRouteRecord>, ControlPlaneError> {
        let Some(mut route) = self.resolve_default_chat_route(owner).await? else {
            return Ok(None);
        };
        if required.is_empty() {
            return Ok(Some(route));
        }
        let candidates = std::iter::once(route.primary.clone())
            .chain(route.failovers.clone())
            .collect::<Vec<_>>();
        let Some(primary) = candidates
            .iter()
            .find(|candidate| chat_route_candidate_supports(candidate, required))
            .cloned()
        else {
            return Ok(None);
        };
        route.primary = primary.clone();
        route.failovers = candidates
            .into_iter()
            .filter(|candidate| {
                candidate.model_route_id != primary.model_route_id
                    && chat_route_candidate_supports(candidate, required)
            })
            .collect();
        Ok(Some(route))
    }

    async fn materialize_default_chat_route(
        &self,
        owner: &UserId,
        mut document: nomifun_api_types::AgentPresetDocumentDto,
    ) -> Result<nomifun_api_types::AgentPresetDocumentDto, ControlPlaneError> {
        if nomifun_agent_contracts::is_direct_creation_agent(document.enabled_capabilities.iter().map(|selection| selection.capability.id.as_str()))
            || document.model_route_refs.contains_key(CHAT_MODEL_TASK)
            || document.chat_route_records.contains_key(CHAT_MODEL_TASK)
        {
            return Ok(document);
        }
        let required = required_chat_features(
            document
                .enabled_capabilities
                .iter()
                .map(|selection| selection.capability.id.as_str()),
        );
        if let Some(record) = self
            .resolve_default_chat_route_for_features(owner, &required)
            .await?
        {
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

    /// Prepare today's editable contract without saving a Revision or opening
    /// a Session. Reuse the execution evolution proof so a retired launch grant
    /// cannot silently gain controls, resources or a different publisher.
    async fn current_authoring_payload(
        &self,
        owner: &UserId,
        revision: &AgentPresetRevision,
        catalog: &CatalogSnapshot,
    ) -> Result<nomifun_agent_contracts::AgentPresetRevisionPayload, ControlPlaneError> {
        if nomifun_agent_contracts::bounded_process_launch_payload(&revision.payload) == revision.payload {
            return Ok(revision.payload.clone());
        }
        let snapshot = self.current_snapshot(Some(revision)).await?
            .ok_or_else(|| not_found("ResolvedSnapshot"))?;
        let (candidate, _) = self.compiler.evolve_session_contract(
            owner, Uuid::now_v7().to_string().into(), revision, &snapshot, catalog, false,
        )?;
        Ok(candidate.payload)
    }

    pub async fn editor(
        &self,
        owner: &UserId,
        preset_id: &str,
        revision_number: Option<u64>,
    ) -> Result<AgentPresetEditorResponse, ControlPlaneError> {
        let stored = self.owned_preset(owner, preset_id).await?;
        let revision = match revision_number {
            Some(number) => Some(self.store
                .get_revision_number(&stored.preset.preset_id, number)
                .await?.ok_or_else(|| not_found("AgentPresetRevision"))?),
            None => match stored.preset.current_stable_revision.as_ref() {
                Some(reference) => self.store.get_revision(reference).await?,
                None => None,
            },
        };
        let template_key = owned_official_key(&stored);
        let document = match revision.as_ref() {
            Some(revision) if revision_number.is_none()
                && nomifun_agent_contracts::bounded_process_launch_payload(&revision.payload) != revision.payload => {
                let prepared = match self.catalog.snapshot() {
                    Ok(catalog) => self.current_authoring_payload(owner, revision, &catalog).await,
                    Err(error) => Err(error),
                };
                match prepared {
                    Ok(payload) => payload_document(&payload)?,
                    Err(error) => {
                        // A broken contract must remain editable. Preserve its
                        // exact selections and let the workbench highlight them;
                        // selection admission still rejects the failed proof.
                        tracing::warn!(preset_id, error = %error, "saved Agent contract could not be prepared for editing");
                        payload_document(&revision.payload)?
                    }
                }
            }
            Some(revision) => payload_document(&revision.payload)?,
            None => match template_key {
                Some(key) => self.official_seed_document(key, self.catalog.snapshot()?.as_ref())?,
                None => empty_document(),
            },
        };
        editor_response(stored, revision, document, template_key)
    }

    /// Propose only missing defaults for this owner's stable official Agent.
    /// There is no trustworthy persisted seed baseline for an authored Revision;
    /// absence may be an intentional user removal and must remain reviewable.
    /// Saving the proposal uses the existing immutable Revision and CAS path.
    pub async fn recommended_capabilities(
        &self, owner: &UserId, preset_id: &str,
    ) -> Result<nomifun_api_types::AgentPresetRecommendedCapabilitiesResponse, ControlPlaneError> {
        let stored = self.owned_preset(owner, preset_id).await?;
        let key = owned_official_key(&stored)
            .filter(|_| !stored.session_only && stored.preset.source == AgentPresetSource::Official)
            .ok_or_else(|| not_found("AgentPreset"))?;
        let mut draft = self.editor(owner, preset_id, None).await?.draft;
        let recommended = self.official_seed_document(key, self.catalog.snapshot()?.as_ref())?;
        let (added_capabilities, added_skills) = add_recommended_capabilities(&mut draft.document, &recommended);
        Ok(nomifun_api_types::AgentPresetRecommendedCapabilitiesResponse {
            draft, added_capabilities, added_skills,
        })
    }

    /// Official identities and their host-derived variants retain their key.
    /// Older internal user configurations require the complete current seed;
    /// neither a name nor a personal model variant proves official identity.
    /// Presentation repair changes only owned metadata;
    /// immutable Revisions, Snapshots and Session bindings stay byte-for-byte exact.
    pub async fn internal_official_template(
        &self, owner: &UserId, id: &str,
    ) -> Result<Option<OfficialPresetKey>, ControlPlaneError> {
        let mut stored = self.owned_preset(owner, id).await?;
        let Some(revision) = self.current_revision(&stored).await? else { return Ok(None); };
        let key = self.official_template_for_revision(&stored, &revision)?;
        if stored.session_only && let Some(key) = key.as_ref() {
            if stored.preset.display_name != key.as_str() {
                stored.preset.display_name = key.as_str().to_owned();
                self.store.update_preset_metadata(&stored).await?;
            }
        }
        Ok(key)
    }

    /// Presentation only, after exact owner/Revision/Snapshot validation. Retired
    /// authoring metadata cannot grant a new Session, capability or execution.
    pub async fn saved_binding_presentation(
        &self, owner: &UserId, binding: &AgentBindingValueDto,
    ) -> Result<(String, Option<OfficialPresetKey>), ControlPlaneError> {
        let binding: AgentBindingValue = wire_cast(binding)?;
        let (revision, _) = self.load_binding_artifacts(owner, &binding).await?;
        let stored = self.store.get_bound_preset(&binding.preset_revision_ref.preset_id).await?
            .ok_or_else(|| not_found("AgentPreset"))?;
        if stored.preset.owner_user_id.as_ref() != Some(owner) {
            return Err(not_found("AgentPreset"));
        }
        let key = self.official_template_for_revision(&stored, &revision)?;
        let name = key.as_ref().map_or_else(|| stored.preset.display_name.clone(), |key| key.as_str().to_owned());
        Ok((name, key))
    }

    /// Read-only recognition of the retired current-generation device Agent.
    /// Only the roster's formal idle transition may consume this result. It
    /// never changes a Revision, Snapshot, binding or author-selected grant.
    pub async fn is_retired_companion_device_binding(
        &self, owner: &UserId, binding: &AgentBindingValueDto,
    ) -> Result<bool, ControlPlaneError> {
        let binding: AgentBindingValue = wire_cast(binding)?;
        let _ = self.load_binding_artifacts(owner, &binding).await?;
        let stored = self.store.get_bound_preset(&binding.preset_revision_ref.preset_id).await?
            .ok_or_else(|| not_found("AgentPreset"))?;
        if stored.preset.owner_user_id.as_ref() != Some(owner) {
            return Err(not_found("AgentPreset"));
        }
        Ok(retired_companion_device_configuration(&stored, owner))
    }

    fn official_template_for_revision(
        &self, stored: &StoredPreset, _revision: &AgentPresetRevision,
    ) -> Result<Option<OfficialPresetKey>, ControlPlaneError> {
        // Only the host-owned source and owner-scoped official identity confer
        // a product template. Old User session caches contain random IDs and no
        // persisted template provenance; a display name or matching seed must
        // not promote their immutable history into an official product binding.
        Ok(owned_official_key(stored))
    }

    /// Read-only admission check for product selectors. This uses the same
    /// route resolver and compiler as creation without creating hidden presets.
    pub async fn validate_product_selection(
        &self,
        owner: &UserId,
        template_id: Option<&str>,
        preset_id: Option<&str>,
        model: Option<&nomifun_api_types::AgentChatModelSelectionDto>,
    ) -> Result<(), ControlPlaneError> {
        let catalog = self.catalog.snapshot()?;
        let (mut document, template_key, source_snapshot) = match (template_id, preset_id) {
            (Some(id), None) => {
                let key = parse_official_key(id).ok_or_else(|| not_found("OfficialPresetTemplate"))?;
                let preset = self.store.get_preset(&official_preset_id(owner, key)).await?;
                let revision = match preset.as_ref() {
                    Some(preset) if owned_official_key(preset) == Some(key)
                        && preset.preset.owner_user_id.as_ref() == Some(owner) => self.current_revision(preset).await?,
                    Some(_) => return Err(not_found("AgentPreset")),
                    None => None,
                };
                match revision {
                    Some(revision) => {
                        let snapshot = self.current_snapshot(Some(&revision)).await?.ok_or_else(|| not_found("ResolvedSnapshot"))?;
                        (payload_document(&self.current_authoring_payload(owner, &revision, &catalog).await?)?, Some(key), Some(snapshot))
                    }
                    None => (self.official_seed_document(key, &catalog)?, Some(key), None),
                }
            }
            (None, Some(id)) => {
                let preset = self.owned_preset(owner, id).await?;
                require_current_preset_admission(&preset, owner)?;
                let revision = self.current_revision(&preset).await?
                    .ok_or_else(|| not_found("AgentPresetRevision"))?;
                let snapshot = self.current_snapshot(Some(&revision)).await?.ok_or_else(|| {
                    ControlPlaneError::canonical("CAPABILITY_NOT_MATERIALIZED",
                        axum::http::StatusCode::UNPROCESSABLE_ENTITY, "Agent has no compiled snapshot")
                })?;
                (payload_document(&self.current_authoring_payload(owner, &revision, &catalog).await?)?, None, Some(snapshot))
            }
            _ => return Err(ControlPlaneError::canonical("AGENT_PRESET_NOT_FOUND",
                axum::http::StatusCode::BAD_REQUEST, "select exactly one Agent")),
        };
        let mut diagnostics = Vec::new();
        self.compiler.validate_catalog_availability(&document_payload(&document)?, &catalog, &mut diagnostics)?;
        if !diagnostics.is_empty() {
            return Err(ControlPlaneError::with_details("CAPABILITY_NOT_MATERIALIZED",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY, "Agent capability is unavailable",
                json!({ "diagnostics": diagnostics })));
        }
        // Selecting authoring intent does not require or silently choose a model.
        let Some(model) = model else { return Ok(()); };
        let required = required_chat_features(document.enabled_capabilities.iter().map(|item| item.capability.id.as_str()));
        let route = self.resolve_selected_chat_route(owner, model, &required).await?;
        document.model_route_refs.insert(CHAT_MODEL_TASK.to_owned(), route.primary.model_route_id.as_ref().to_owned());
        document.chat_route_records.insert(CHAT_MODEL_TASK.to_owned(), serde_json::to_value(route)?);
        let draft = AgentPresetDraftDto {
            preset_id: Uuid::now_v7().to_string(),
            display_name: "Agent availability check".to_owned(),
            description: None,
            source_template_key: template_key.map(|key| wire_cast(&key)).transpose()?,
            current_revision: None,
            document,
        };
        let compilation = self.authoring_compiler(owner).await?.compile(owner, &draft, None, source_snapshot.as_ref(), &catalog)?;
        if compilation.snapshot.is_none() {
            return Err(ControlPlaneError::with_details("PRESET_REVISION_SAVE_FAILED",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY, "Agent is unavailable",
                json!({ "diagnostics": compilation.diagnostics })));
        }
        Ok(())
    }

    pub async fn save_revision(
        &self,
        owner: &UserId,
        preset_id: &str,
        mut request: SaveAgentPresetRevisionRequest,
    ) -> Result<SaveAgentPresetRevisionResponse, ControlPlaneError> {
        if request.draft.preset_id != preset_id {
            return Err(ControlPlaneError::canonical(
                "PRESET_REVISION_DIGEST_MISMATCH",
                axum::http::StatusCode::BAD_REQUEST,
                "draft preset_id must match the route preset_id",
            ));
        }
        let stored = self.owned_preset(owner, preset_id).await?;
        if stored.session_only && stored.preset.source == AgentPresetSource::Official {
            // Internal variants retain their host-assigned official lineage.
            request.draft.display_name = stored.preset.display_name.clone();
        }
        if matches!(owned_official_key(&stored), Some(OfficialPresetKey::CompanionDefault)) {
            let missing = ["companion", "companion.memory"].into_iter().filter(|required|
                !request.draft.document.enabled_capabilities.iter().any(|selection| selection.capability.id == *required))
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(ControlPlaneError::with_details("PRESET_REVISION_SAVE_FAILED",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "The companion Agent must retain its companion and memory modules",
                    json!({ "diagnostics": missing.into_iter().map(|subject| json!({
                        "code": "AGENT_CORE_MODULE_REQUIRED", "subject": subject,
                        "message": "This module preserves the companion's product identity", "details": null,
                    })).collect::<Vec<_>>() })));
            }
        }
        ensure_expected_current(
            stored.preset.current_stable_revision.as_ref(),
            request.expected_current_revision.as_ref(),
        )?;
        let current = self.current_revision(&stored).await?;
        let current_snapshot = self.current_snapshot(current.as_ref()).await?;
        let catalog = self.catalog.snapshot()?;
        let compilation = self.authoring_compiler(owner).await?.compile(
            owner,
            &request.draft,
            current.as_ref(),
            current_snapshot.as_ref(),
            &catalog,
        )?;
        let snapshot = compilation.snapshot.ok_or_else(|| {
            tracing::warn!(
                diagnostics = ?compilation.diagnostics,
                "official template expansion failed compiler validation"
            );
            ControlPlaneError::with_details(
                "PRESET_REVISION_SAVE_FAILED",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "The Agent configuration is invalid; no immutable Revision was created",
                json!({ "diagnostics": compilation.diagnostics }),
            )
        })?;
        if current
            .as_ref()
            .is_some_and(|revision| revision.reference == compilation.candidate_revision_ref)
        {
            let current = current.expect("clean compilation has a current Revision");
            let mut stored = stored;
            if stored.preset.display_name != request.draft.display_name
                || stored.preset.description != request.draft.description
            {
                stored.preset.display_name = request.draft.display_name;
                stored.preset.description = request.draft.description;
                self.store.update_preset_metadata(&stored).await?;
            }
            return Ok(SaveAgentPresetRevisionResponse {
                preset: preset_summary(
                    &stored,
                    self.bound_count(owner, &stored.preset.preset_id).await?,
                )?,
                revision: revision_api(&current)?,
                resolved_snapshot_ref: wire_cast(&snapshot.snapshot_ref)?,
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
    /// Session binding. Hidden compilation caches are refreshed before a new
    /// binding; personal presets and historical bindings remain immutable.
    pub async fn resolve_agent_session_binding(
        &self,
        owner: &UserId,
        preset_id: &str,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        self.resolve_agent_session_binding_with_capabilities(owner, preset_id, None, None).await
    }

    /// The caller already owns template_launch_lock (including model variants).
    async fn resolve_agent_session_binding_locked(
        &self,
        owner: &UserId,
        preset_id: &str,
    ) -> Result<AgentBindingValueDto, ControlPlaneError> {
        let mut stored = self
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
        require_current_preset_admission(&stored, owner)?;
        if let Some(revision) = self.current_revision(&stored).await?
            && nomifun_agent_contracts::bounded_process_launch_payload(&revision.payload) != revision.payload {
            let snapshot = self.current_snapshot(Some(&revision)).await?;
            let source = session_binding_from_stable_artifacts(revision.reference.clone(), Some(revision), snapshot)?;
            let mut bounded = self.resolve_agent_session_contract_evolution_locked(owner, &wire_cast(&source)?).await?;
            // This is a new Session opening, not an in-place transition or a
            // continuation of any historical native checkpoint.
            bounded.binding_version = 1;
            return Ok(bounded);
        }
        if stored.session_only || owned_official_key(&stored).is_some() {
            if let Some(revision) = self.current_revision(&stored).await? {
                self.refresh_session_configuration(owner, &stored, &revision, None).await?;
                stored = self.owned_preset(owner, preset_id).await?;
            }
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

    pub async fn delete_remote_binding_if_current(&self, owner: &UserId, expected: &RemoteBindingDto) -> Result<(), ControlPlaneError> {
        let expected: RemoteBinding = wire_cast(expected)?;
        if &expected.owner_user_id != owner { return Err(not_found("RemoteBinding")); }
        self.store.delete_remote_binding_if_current(&expected).await
    }

    /// Compensate a failed local configuration save using an exact captured
    /// current definition. This is not a historical import or Session restore.
    pub async fn restore_remote_binding_after_failed_local_change(&self, owner: &UserId, previous: &RemoteBindingDto) -> Result<(), ControlPlaneError> {
        let previous: RemoteBinding = wire_cast(previous)?;
        if &previous.owner_user_id != owner { return Err(not_found("RemoteBinding")); }
        self.validate_agent_binding(owner,&previous.agent_binding).await?;
        if self.store.get_remote_binding(&previous.remote_binding_id).await?.is_some() {
            return Err(ControlPlaneError::canonical("REMOTE_BINDING_VERSION_CONFLICT", axum::http::StatusCode::CONFLICT,
                "RemoteBinding was recreated while the local change was being compensated"));
        }
        self.store.insert_remote_binding(previous).await?;
        Ok(())
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
        self.create_configuration_with_initial_revision(owner, preset_id, display_name, description,
            document, transient_template_key, false, None).await
    }

    async fn create_configuration_with_initial_revision(
        &self,
        owner: &UserId,
        preset_id: AgentPresetId,
        display_name: String,
        description: Option<String>,
        document: nomifun_api_types::AgentPresetDocumentDto,
        transient_template_key: Option<OfficialPresetKey>,
        session_only: bool,
        source_snapshot: Option<&nomifun_agent_contracts::ResolvedSnapshotEnvelope>,
    ) -> Result<AgentPresetEditorResponse, ControlPlaneError> {
        self.create_configuration_with_payload(owner, preset_id, display_name, description, document_payload(&document)?, transient_template_key, session_only, source_snapshot).await
    }

    async fn create_configuration_with_payload(
        &self,
        owner: &UserId,
        preset_id: AgentPresetId,
        display_name: String,
        description: Option<String>,
        payload: nomifun_agent_contracts::AgentPresetRevisionPayload,
        transient_template_key: Option<OfficialPresetKey>,
        session_only: bool,
        source_snapshot: Option<&nomifun_agent_contracts::ResolvedSnapshotEnvelope>,
    ) -> Result<AgentPresetEditorResponse, ControlPlaneError> {
        let draft = AgentPresetDraftDto {
            preset_id: preset_id.as_ref().to_owned(),
            display_name: display_name.clone(),
            description: description.clone(),
            source_template_key: transient_template_key
                .map(|key| wire_cast(&key))
                .transpose()?,
            current_revision: None,
            document: payload_document(&payload)?,
        };
        let catalog = self.catalog.snapshot()?;
        let compilation = self.authoring_compiler(owner).await?.compile_payload(
            owner,
            &draft,
            payload,
            None,
            source_snapshot,
            &catalog,
        )?;
        let snapshot = compilation.snapshot.ok_or_else(|| {
            tracing::warn!(
                diagnostics = ?compilation.diagnostics,
                "official template expansion failed compiler validation"
            );
            ControlPlaneError::with_details(
                "PRESET_REVISION_SAVE_FAILED",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "template expansion did not pass compiler validation",
                json!({ "diagnostics": compilation.diagnostics }),
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
            payload_document(&revision.payload)?;
        let stored = StoredPreset {
            session_only,
            preset: AgentPreset {
                preset_id,
                owner_user_id: Some(owner.clone()),
                source: if session_only && transient_template_key.is_some() {
                    AgentPresetSource::Official
                } else { AgentPresetSource::User },
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

    /// Internal session configurations are an optional reuse cache. A retained
    /// development dataset can contain revisions/snapshots from an older
    /// contract; one unreadable candidate must not prevent compiling a fresh
    /// configuration. Never use this recovery path for an explicitly selected
    /// preset or an existing session binding: those remain strictly validated.
    async fn reusable_session_configuration(
        &self,
        preset: &StoredPreset,
        requested: &nomifun_agent_contracts::AgentPresetRevisionPayload,
        uses_default_route: bool,
    ) -> Option<(AgentPresetRevision, nomifun_agent_contracts::ResolvedSnapshotEnvelope)> {
        if !preset.session_only {
            return None;
        }
        let candidate = async {
            let Some(revision) = self.current_revision(preset).await? else {
                return Ok(None);
            };
            if !template_launch_payload_matches(&revision.payload, requested, uses_default_route) {
                return Ok(None);
            }
            let snapshot = self.current_snapshot(Some(&revision)).await?;
            Ok::<_, ControlPlaneError>(snapshot.map(|snapshot| (revision, snapshot)))
        }.await;
        match candidate {
            Ok(value) => value,
            Err(error) => {
                // Log only identifiers/codes, never persisted instructions or
                // provider credentials embedded in an obsolete document.
                tracing::warn!(
                    preset_id = preset.preset.preset_id.as_ref(),
                    code = error.code().as_ref(),
                    "skipping unreadable internal Agent configuration"
                );
                None
            }
        }
    }

    /// Revalidate an internal compilation cache without changing its payload.
    /// The formal save path retains old immutable artifacts and reuses the
    /// current revision when its materialized contracts are still exact.
    async fn refresh_session_configuration(
        &self,
        owner: &UserId,
        preset: &StoredPreset,
        revision: &AgentPresetRevision,
        template_key: Option<OfficialPresetKey>,
    ) -> Result<(), ControlPlaneError> {
        if !preset.session_only
            && owned_official_key(preset).is_none()
        {
            return Err(ControlPlaneError::Wire("only official or internal session configurations may refresh during binding".into()));
        }
        if revision.payload.skill_bindings.iter().any(|binding| binding.package_ref().is_none()) {
            // A model or extension variant retains captured current-generation
            // Skill bytes; a public authoring projection must not overwrite it.
            return Ok(());
        }
        self.save_revision(owner, preset.preset.preset_id.as_ref(), SaveAgentPresetRevisionRequest {
            expected_current_revision: Some(wire_cast(&revision.reference)?),
            draft: AgentPresetDraftDto {
                preset_id: preset.preset.preset_id.as_ref().to_owned(),
                display_name: if preset.preset.source == AgentPresetSource::Official {
                    preset.preset.display_name.clone()
                } else {
                    template_key.map(|key| key.as_str().to_owned())
                        .unwrap_or_else(|| preset.preset.display_name.clone())
                },
                description: preset.preset.description.clone(),
                source_template_key: template_key.map(|key| wire_cast(&key)).transpose()?,
                current_revision: Some(wire_cast(&revision.reference)?),
                document: payload_document(&revision.payload)?,
            },
            reason: Some("Refresh internal session runtime contracts".into()),
        }).await?;
        Ok(())
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
        if preset.preset.owner_user_id.as_ref() != Some(owner) {
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

pub fn is_global_extension_module(id: &str) -> bool {
    id == "agent.tool-discovery" || id.starts_with("nomi.mcp.v1.")
}

fn empty_document() -> nomifun_api_types::AgentPresetDocumentDto {
    nomifun_api_types::AgentPresetDocumentDto {
        context_order: Vec::new(),
        middleware_order: Vec::new(),
        schema_version: "1.0.0".into(),
        model_route_refs: BTreeMap::new(),
        chat_route_records: BTreeMap::new(),
        enabled_capabilities: Vec::new(),
        skill_bindings: Vec::new(),
        system_role_provider_overrides: BTreeMap::new(),
        persona: String::new(),
        instructions: String::new(),
        starter_prompts: Vec::new(),
        runtime_policy: Default::default(),
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

fn add_recommended_capabilities(
    document: &mut nomifun_api_types::AgentPresetDocumentDto,
    recommended: &nomifun_api_types::AgentPresetDocumentDto,
) -> (Vec<nomifun_api_types::CapabilitySelectionDto>, Vec<ExactCatalogRefDto>) {
    let mut added_capabilities = Vec::new();
    for selection in &recommended.enabled_capabilities {
        if let Some(existing) = document.enabled_capabilities.iter_mut()
            .find(|existing| existing.capability.id == selection.capability.id)
        {
            let missing = selection.action_allowlist.difference(&existing.action_allowlist)
                .cloned().collect::<BTreeSet<_>>();
            if !missing.is_empty() {
                existing.action_allowlist.extend(missing.iter().cloned());
                added_capabilities.push(nomifun_api_types::CapabilitySelectionDto {
                    capability: selection.capability.clone(), action_allowlist: missing,
                });
            }
        } else {
            document.enabled_capabilities.push(selection.clone());
            added_capabilities.push(selection.clone());
        }
    }
    let mut added_skills = Vec::new();
    for skill in &recommended.skill_bindings {
        // A user-pinned version of a bundled Skill is an authored choice.
        // Add absent identities, never overwrite the selected version.
        if !document.skill_bindings.iter().any(|existing| existing.id == skill.id) {
            document.skill_bindings.push(skill.clone());
            added_skills.push(skill.clone());
        }
    }
    (added_capabilities, added_skills)
}

fn template_selection(
    selection: &CapabilitySelection,
    catalog: &CatalogSnapshot,
) -> Result<CapabilitySelection, ControlPlaneError> {
    let manifest = catalog.find_capability(&selection.capability).ok_or_else(|| {
        ControlPlaneError::canonical(
            "CAPABILITY_NOT_MATERIALIZED",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "official template capability {} is unavailable",
                selection.capability.id.as_ref()
            ),
        )
    })?;
    let declared = manifest
        .contributions
        .actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect::<BTreeSet<_>>();
    if selection.action_allowlist.is_empty()
        || !selection.action_allowlist.is_subset(&declared)
    {
        return Err(ControlPlaneError::canonical(
            "CAPABILITY_ACTION_NOT_MATERIALIZED",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "official template capability {} contains an empty or undeclared Action grant",
                selection.capability.id.as_ref()
            ),
        ));
    }
    Ok(selection.clone())
}

fn template_selection_api(
    selection: &CapabilitySelection,
    catalog: &CatalogSnapshot,
) -> Result<nomifun_api_types::CapabilitySelectionDto, ControlPlaneError> {
    wire_cast(&template_selection(selection, catalog)?)
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

fn same_non_model_revision_contract(
    left: &AgentPresetRevision,
    right: &AgentPresetRevision,
) -> bool {
    let mut left_payload = left.payload.clone();
    left_payload.model_route_refs.clear();
    left_payload.chat_route_records.clear();
    let mut right_payload = right.payload.clone();
    right_payload.model_route_refs.clear();
    right_payload.chat_route_records.clear();
    let mut left_locks = left.contribution_locks.clone();
    left_locks.sort();
    let mut right_locks = right.contribution_locks.clone();
    right_locks.sort();
    left_payload == right_payload
        && left_locks == right_locks
        && left.created_by == right.created_by
}

fn same_chat_route_configuration(
    left: &nomifun_agent_contracts::ChatRouteRecord,
    right: &nomifun_agent_contracts::ChatRouteRecord,
) -> bool {
    fn candidate_configuration(
        candidate: &nomifun_agent_contracts::ChatRouteCandidate,
    ) -> (
        &str,
        &str,
        u64,
        nomifun_agent_contracts::ChatRouteProtocol,
        &nomifun_agent_contracts::ConnectionConfigRef,
        &nomifun_agent_contracts::DigestHex,
        &BTreeSet<ChatRouteFeature>,
        &BTreeSet<ChatRouteFeature>,
    ) {
        (
            &candidate.provider_id,
            &candidate.model,
            candidate.model_route_revision,
            candidate.protocol,
            &candidate.connection_config_ref,
            &candidate.config_revision_digest,
            &candidate.features,
            &candidate.activation_features,
        )
    }

    left.schema == right.schema
        && left.task == right.task
        && candidate_configuration(&left.primary) == candidate_configuration(&right.primary)
        && left.failovers.len() == right.failovers.len()
        && left
            .failovers
            .iter()
            .zip(&right.failovers)
            .all(|(left, right)| candidate_configuration(left) == candidate_configuration(right))
}

fn same_non_model_snapshot_contract(
    candidate: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    source: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
) -> bool {
    let mut normalized = candidate.content.clone();
    normalized.preset_revision_ref = source.content.preset_revision_ref.clone();
    normalized.model_route_refs = source.content.model_route_refs.clone();
    normalized.chat_route_identity = source.content.chat_route_identity.clone();
    normalized.compiled_runtime_profile_digest =
        source.content.compiled_runtime_profile_digest.clone();
    normalized == source.content
        && candidate.actor == source.actor
        && candidate.scene == source.scene
        && candidate.surface == source.surface
        && candidate.audience == source.audience
        && candidate.availability_evidence_revision == source.availability_evidence_revision
}

fn same_app_workflow_snapshot_contract(
    candidate: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    source: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
) -> bool {
    let Some(previous_hosting) = source.content.enabled_capabilities.iter().find(|value| value.capability.id.as_ref() == "app.hosting") else { return false; };
    let mut normalized = candidate.content.clone();
    let Some(hosting) = normalized.enabled_capabilities.iter_mut().find(|value| value.capability.id.as_ref() == "app.hosting") else { return false; };
    if hosting.capability != previous_hosting.capability || hosting.action_allowlist != previous_hosting.action_allowlist
        || hosting.source_package != previous_hosting.source_package || hosting.resolved_source != previous_hosting.resolved_source
        || hosting.contribution_id != previous_hosting.contribution_id || hosting.resolved_mount_id != previous_hosting.resolved_mount_id
        || hosting.consumption != previous_hosting.consumption || hosting.dependency_refs != previous_hosting.dependency_refs
        || hosting.required_resource_kinds != previous_hosting.required_resource_kinds
        || hosting.required_runtime_features != previous_hosting.required_runtime_features {
        return false;
    }
    let current_actions = hosting.actions.iter().map(|value| (&value.action_id, value.effect_class, value.presentation)).collect::<Vec<_>>();
    let prior_actions = previous_hosting.actions.iter().map(|value| (&value.action_id, value.effect_class, value.presentation)).collect::<Vec<_>>();
    if current_actions != prior_actions { return false; }
    if hosting.actions.iter().zip(&previous_hosting.actions).any(|(current, previous)|
        current.output_schema != previous.output_schema
            || current.action_id.as_ref() != "app.hosting/update" && current.input_schema != previous.input_schema) {
        return false;
    }
    *hosting = previous_hosting.clone();
    normalized.preset_revision_ref = source.content.preset_revision_ref.clone();
    if let (Some(candidate_route), Some(previous_route)) = (&mut normalized.chat_route_identity, &source.content.chat_route_identity) {
        // The same real model route is addressed by the new immutable Revision.
        candidate_route.preset_revision_id = previous_route.preset_revision_id.clone();
    }
    normalized.compiled_runtime_profile_digest = source.content.compiled_runtime_profile_digest.clone();
    normalized.canonical_schema_manifest_digest = source.content.canonical_schema_manifest_digest.clone();
    normalized.target_contribution_manifest_digest = source.content.target_contribution_manifest_digest.clone();
    normalized == source.content && candidate.actor == source.actor && candidate.scene == source.scene
        && candidate.surface == source.surface && candidate.audience == source.audience
        && candidate.availability_evidence_revision == source.availability_evidence_revision
}

fn same_execution_snapshot_contract(
    candidate: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    source: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
) -> bool {
    let mut content = candidate.content.clone();
    content.preset_revision_ref = source.content.preset_revision_ref.clone();
    if let (Some(candidate), Some(original)) = (&mut content.chat_route_identity, &source.content.chat_route_identity) {
        candidate.preset_revision_id = original.preset_revision_id.clone();
    }
    content == source.content
        && candidate.actor == source.actor
        && candidate.scene == source.scene
        && candidate.surface == source.surface
        && candidate.audience == source.audience
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

fn template_launch_payload_matches(
    saved: &nomifun_agent_contracts::AgentPresetRevisionPayload,
    requested: &nomifun_agent_contracts::AgentPresetRevisionPayload,
    uses_default_route: bool,
) -> bool {
    let mut requested = requested.clone();
    if uses_default_route
        && let Some(saved_route) = saved.chat_route_records.get(CHAT_MODEL_TASK)
        && let Some(requested_route) = requested.chat_route_records.get_mut(CHAT_MODEL_TASK)
    {
        // The host allocates fresh opaque aliases whenever it resolves the
        // default model. Compare all provider/model/config/feature facts while
        // retaining the saved aliases. Explicit caller routes remain exact.
        requested_route.primary.model_route_id = saved_route.primary.model_route_id.clone();
        requested_route.primary.credential_ref = saved_route.primary.credential_ref.clone();
        for (candidate, saved_candidate) in requested_route.failovers.iter_mut().zip(&saved_route.failovers) {
            candidate.model_route_id = saved_candidate.model_route_id.clone();
            candidate.credential_ref = saved_candidate.credential_ref.clone();
        }
        requested.model_route_refs.insert(CHAT_MODEL_TASK.into(), saved_route.primary.model_route_id.clone());
    }
    saved == &requested
}

fn parse_official_key(value: &str) -> Option<OfficialPresetKey> {
    OfficialPresetKey::ALL
        .into_iter()
        .find(|key| key.as_str() == value)
}

/// Stable, owner-scoped UUIDv7 derived from a namespaced canonical identity.
/// Authored fields and the bundled seed cannot change this identity.
fn official_preset_id(owner: &UserId, key: OfficialPresetKey) -> AgentPresetId {
    official_preset_id_for_key(owner, key.as_str())
}

fn official_preset_id_for_key(owner: &UserId, key: &str) -> AgentPresetId {
    let digest = nomifun_agent_contracts::digest_payload(&(
        "nomifun.official-agent-preset.v1", owner.as_ref(), key,
    )).expect("official Agent identity is serializable");
    let mut bytes = *Uuid::parse_str(&digest.as_ref()[..32])
        .expect("canonical digest is hexadecimal").as_bytes();
    // Authenticated owners carry the UUIDv7 timestamp. Opaque internal
    // fixtures use a fixed epoch while retaining their owner-scoped digest.
    let timestamp = Uuid::parse_str(owner.as_ref()).ok()
        .filter(|id| id.get_version_num() == 7)
        .map_or([0; 6], |id| {
            let mut timestamp = [0; 6];
            timestamp.copy_from_slice(&id.as_bytes()[..6]);
            timestamp
        });
    bytes[..6].copy_from_slice(&timestamp);
    bytes[6] = (bytes[6] & 0x0f) | 0x70;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    AgentPresetId::from(Uuid::from_bytes(bytes).to_string())
}

fn retired_companion_device_configuration(preset: &StoredPreset, owner: &UserId) -> bool {
    preset.preset.source == AgentPresetSource::Official
        && preset.preset.owner_user_id.as_ref() == Some(owner)
        && (preset.preset.preset_id == official_preset_id_for_key(owner, "companion.robot")
            // Internal variants were authored only by this Control Plane with
            // the exact official source key; user labels cannot create them.
            || preset.session_only && preset.preset.display_name == "companion.robot")
}

fn require_current_preset_admission(preset: &StoredPreset, owner: &UserId) -> Result<(), ControlPlaneError> {
    if retired_companion_device_configuration(preset, owner) {
        return Err(ControlPlaneError::canonical("AGENT_PRESET_NOT_FOUND", axum::http::StatusCode::NOT_FOUND,
            "The independent device Agent is retired; select this Companion's shared Agent"));
    }
    Ok(())
}

fn owned_official_key(preset: &StoredPreset) -> Option<OfficialPresetKey> {
    if preset.preset.source != AgentPresetSource::Official { return None; }
    if preset.session_only { return parse_official_key(&preset.preset.display_name); }
    let owner = preset.preset.owner_user_id.as_ref()?;
    OfficialPresetKey::ALL.into_iter().find(|key| official_preset_id(owner, *key) == preset.preset.preset_id)
}

fn variant_display_name(preset: &StoredPreset) -> String {
    owned_official_key(preset).map(|key| key.as_str().to_owned())
        .unwrap_or_else(|| preset.preset.display_name.clone())
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
pub(crate) mod tests {
    use super::*;
    include!("authoring_contract_evolution_tests.rs");
    use crate::{
        CatalogSnapshot, InMemoryControlPlaneStore, OfficialTemplateCatalog,
        PresetRevisionCompiler, StaticCatalogProvider,
        StaticRevisionImpactCatalogProvider,
    };
    use nomifun_agent_contracts::{
        ActionId, CapabilityActionDescriptor, CapabilityCatalogEntry, CapabilityCatalogMaterialization,
        CapabilityCatalogMaterializer, CapabilityContributions,
        CapabilityConsumer, CapabilityId, CapabilityKind,
        CapabilityManifest, CapabilityOwner, CapabilityProvenance,
        CapabilityPublicationState, CapabilityRef, CatalogAvailability,
        ContributionId, ContributionLock, ContributionSourceKind, DigestHex, EffectClass,
        LocalizedMetadata, PackageId, PackageRef, PlatformConstraint,
        AgentModuleId, PluginSourceKind, PluginSourceMetadata,
        RuntimeProfileKind, RuntimeTarget, StableSourceIdentity,
        StrictJsonValue, ToolPresentationKind, VersionString, capability_surface_declarations,
        SkillDefinition, LogicalArtifactRef,
        digest_payload,
    };
    use nomifun_agent_kernel::{
        CompilerEnvironment, MaterializedCapability, MaterializedRegistry, MaterializedSkill,
    };
    use serde_json::json;

    fn test_compiler() -> PresetRevisionCompiler {
        PresetRevisionCompiler::new().with_materialized_registry(
            Arc::new(MaterializedRegistry::empty()),
            CompilerEnvironment {
                resolver_version: VersionString::from("1.0.0"),
                required_runtime_protocol_version: VersionString::from("1.0.0"),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: DigestHex::from("runtime-features"),
                available_runtime_features: BTreeSet::new(),
                installation_role_bindings: BTreeMap::new(),
                canonical_schema_manifest_digest: DigestHex::from("schema"),
                target_contribution_manifest_digest: DigestHex::from("contributions"),
                host_target: RuntimeTarget::from("test"),
                host_surface: "desktop".into(),
                availability_evidence_revision: "fixture".into(),
            },
        )
    }

    pub(crate) fn test_control_plane(
        store: Arc<InMemoryControlPlaneStore>,
    ) -> AgentControlPlane {
        template_control_plane(store, OfficialPresetKey::ChatMinimal, false)
    }

    fn chat_route_with(
        protocol: ChatRouteProtocol,
        features: impl IntoIterator<Item = ChatRouteFeature>,
    ) -> nomifun_agent_contracts::ChatRouteRecord {
        nomifun_agent_contracts::ChatRouteRecord {
            schema: nomifun_agent_contracts::ChatRouteRecordSchema::V1,
            task: nomifun_agent_contracts::ChatRouteTask::AgentChat,
            primary: nomifun_agent_contracts::ChatRouteCandidate {
                model_route_id: nomifun_agent_contracts::ModelRouteId::from(
                    "0190f5fe-7c00-7a00-8000-000000000010",
                ),
                model_route_revision: 1,
                provider_id: "provider-a".into(),
                model: "model-a".into(),
                protocol,
                connection_config_ref: nomifun_agent_contracts::ConnectionConfigRef::from(
                    "default",
                ),
                config_revision_digest: DigestHex::from("a".repeat(64)),
                credential_ref: "credential-a".into(),
                features: features.into_iter().collect(),
                activation_features: BTreeSet::new(),
            },
            failovers: Vec::new(),
        }
    }

    #[tokio::test]
    async fn default_chat_route_only_fills_an_unspecified_selection() {
        struct DefaultRoute(nomifun_agent_contracts::ChatRouteRecord);
        #[async_trait::async_trait]
        impl DefaultChatRouteResolver for DefaultRoute {
            async fn resolve_default_chat_route(&self, _: &UserId)
                -> Result<Option<nomifun_agent_contracts::ChatRouteRecord>, ControlPlaneError>
            {
                Ok(Some(self.0.clone()))
            }
        }
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let mut default = chat_route_with(ChatRouteProtocol::OpenaiChat,
            [ChatRouteFeature::TextInput, ChatRouteFeature::TextOutput]);
        default.primary.provider_id = "0190f5fe-7c00-7a00-8000-000000000001".into();
        let mut selected = default.clone();
        selected.primary.model = "explicit-model".into();
        let control = test_control_plane(store.clone())
            .with_default_chat_route_resolver(Arc::new(DefaultRoute(default.clone())));
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let empty = control.materialize_default_chat_route(&owner, empty_document()).await.unwrap();
        assert_eq!(empty.chat_route_records[CHAT_MODEL_TASK], serde_json::to_value(default).unwrap());
        for (has_ref, has_record) in [(true, true), (true, false), (false, true)] {
            let mut document = empty_document();
            if has_ref {
                document.model_route_refs.insert(CHAT_MODEL_TASK.into(), selected.primary.model_route_id.as_ref().into());
            }
            if has_record {
                document.chat_route_records.insert(CHAT_MODEL_TASK.into(), serde_json::to_value(&selected).unwrap());
            }
            let resolved = control.materialize_default_chat_route(&owner, document.clone()).await.unwrap();
            assert_eq!(serde_json::to_value(&resolved).unwrap(), serde_json::to_value(&document).unwrap());
            if has_ref && has_record {
                let created = control.create_preset(&owner, CreateAgentPresetRequest {
                    display_name: "Explicit".into(), description: None,
                    document: Some(document), fork_from_revision: None,
                }).await.unwrap();
                let revision = control.get_revision(&owner, &created.preset.preset_id, 1).await.unwrap();
                assert_eq!(revision.document.chat_route_records[CHAT_MODEL_TASK]["primary"]["model"], "explicit-model");
                let mut draft = created.draft;
                draft.document.chat_route_records.get_mut(CHAT_MODEL_TASK).unwrap()["primary"]["model"] = json!("edited-model");
                let saved = control.save_revision(&owner, &created.preset.preset_id, SaveAgentPresetRevisionRequest {
                    expected_current_revision: Some(revision.reference), draft, reason: None,
                }).await.unwrap();
                assert_eq!(saved.revision.document.chat_route_records[CHAT_MODEL_TASK]["primary"]["model"], "edited-model");
            }
        }
    }

    #[test]
    fn web_search_does_not_constrain_the_conversation_chat_route() {
        let required = required_chat_features(["web.research", "creation.media"]);
        assert!(required.is_empty());
        ensure_chat_route_supports(&chat_route_with(ChatRouteProtocol::OpenaiChat, []), &required).unwrap();
    }

    #[tokio::test]
    async fn catalog_detachment_plans_a_new_empty_default_without_mutating_frozen_artifacts() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control = test_control_plane(store.clone());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let route = media_chat_route("delete-model");
        let mut document = empty_document();
        document.instructions = "Keep these authorial instructions".into();
        document.model_route_refs.insert(CHAT_MODEL_TASK.into(),route.primary.model_route_id.as_ref().into());
        document.chat_route_records.insert(CHAT_MODEL_TASK.into(),serde_json::to_value(&route).unwrap());
        let created = control.create_preset(&owner, CreateAgentPresetRequest {
            display_name:"My explicit Agent".into(), description:Some("Keep this description".into()),
            document:Some(document), fork_from_revision:None,
        }).await.unwrap();
        let reference: PresetRevisionRef = wire_cast(created.preset.current_stable_revision.as_ref().unwrap()).unwrap();
        let original = store.get_revision(&reference).await.unwrap().unwrap();
        let snapshot = store.get_snapshot(&reference).await.unwrap().unwrap();
        assert!(control.plan_catalog_model_detachment(&owner,&route.primary.provider_id,Some("other-model")).await.unwrap().is_empty());
        assert!(control.plan_catalog_model_detachment(&UserId::from("other-owner"),&route.primary.provider_id,None).await.unwrap().is_empty());
        let planned = control.plan_catalog_model_detachment(&owner,&route.primary.provider_id,Some("delete-model")).await.unwrap();
        assert_eq!(planned.len(),1);
        let next = &planned[0];
        assert_eq!(next.expected_current, reference);
        assert!(next.revision.payload.chat_route_records.is_empty());
        assert!(next.revision.payload.model_route_refs.is_empty());
        assert_eq!(next.revision.payload.instructions, original.payload.instructions);
        assert!(next.snapshot.content.chat_route_identity.is_none());
        assert_eq!(store.get_revision(&reference).await.unwrap().unwrap(), original);
        assert_eq!(store.get_snapshot(&reference).await.unwrap().unwrap(), snapshot);
        assert_eq!(control.editor(&owner,&created.preset.preset_id,None).await.unwrap().draft.document.chat_route_records.len(),1,"planning is read-only");
        let mut tampered = next.revision.clone();
        tampered.payload.instructions = "Unrelated overwrite".into();
        tampered.reference.revision_digest = tampered.revision_digest().unwrap();
        assert!(nomifun_agent_contracts::validate_catalog_model_detachment(&original,&snapshot,&tampered,&route.primary.provider_id,None).is_err());
        store.append_revision(Some(&reference),next.revision.clone(),next.snapshot.clone(),created.preset.display_name.clone(),created.preset.description.clone()).await.unwrap();
        assert!(control.editor(&owner,&created.preset.preset_id,None).await.unwrap().draft.document.chat_route_records.is_empty());
        assert_eq!(store.get_revision(&reference).await.unwrap().unwrap(),original);
        assert_eq!(store.get_snapshot(&reference).await.unwrap().unwrap(),snapshot);
        assert!(control.plan_catalog_model_detachment(&owner,&route.primary.provider_id,None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn catalog_detachment_prunes_only_matching_failovers_and_never_promotes_a_replacement() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control = test_control_plane(store);
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let mut route = media_chat_route("keep-primary");
        let mut fallback = route.primary.clone();
        fallback.model = "delete-fallback".into();
        fallback.model_route_id = "fallback-route".into();
        route.failovers.push(fallback);
        let mut document = empty_document();
        document.model_route_refs.insert(CHAT_MODEL_TASK.into(),route.primary.model_route_id.as_ref().into());
        document.chat_route_records.insert(CHAT_MODEL_TASK.into(),serde_json::to_value(&route).unwrap());
        control.create_preset(&owner, CreateAgentPresetRequest {display_name:"Keep primary".into(),description:None,document:Some(document),fork_from_revision:None}).await.unwrap();
        let planned = control.plan_catalog_model_detachment(&owner,&route.primary.provider_id,Some("delete-fallback")).await.unwrap();
        assert_eq!(planned.len(),1);
        let retained = &planned[0].revision.payload.chat_route_records[CHAT_MODEL_TASK];
        assert_eq!(retained.primary,route.primary);
        assert!(retained.failovers.is_empty());
        let removed = control.plan_catalog_model_detachment(&owner,&route.primary.provider_id,Some("keep-primary")).await.unwrap();
        assert!(removed[0].revision.payload.chat_route_records.is_empty(),"a remaining failover never becomes an implicit default");
    }

    #[tokio::test]
    async fn configured_creation_without_a_model_saves_an_immutable_preset() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control_plane = test_control_plane(store.clone());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let created = control_plane.create_preset(&owner, CreateAgentPresetRequest {
            display_name: "Edited official preset".into(), description: None,
            fork_from_revision: None, document: Some(empty_document()),
        }).await.unwrap();
        assert!(created.preset.current_stable_revision.is_some());
        assert!(created.draft.document.model_route_refs.is_empty());
        assert!(created.draft.document.chat_route_records.is_empty());
        assert_eq!(store.list_presets(&owner).await.unwrap().len(), 1);
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
        catalog_capability_with_schema(id, consumers, json!({"type":"object", "additionalProperties":false}))
    }

    fn catalog_capability_with_schema(
        id: &str,
        consumers: impl IntoIterator<Item = CapabilityConsumer>,
        schema: serde_json::Value,
    ) -> (MaterializedCapability, CapabilityCatalogEntry) {
        catalog_capability_with_actions_schema(
            id,
            consumers,
            std::iter::empty::<ActionId>(),
            schema,
        )
    }

    fn catalog_capability_with_actions_schema(
        id: &str,
        consumers: impl IntoIterator<Item = CapabilityConsumer>,
        actions: impl IntoIterator<Item = ActionId>,
        schema: serde_json::Value,
    ) -> (MaterializedCapability, CapabilityCatalogEntry) {
        let package = PackageRef {
            id: PackageId::from(format!("test.{id}")),
            version: VersionString::from("1.0.0"),
        };
        let manifest = CapabilityManifest {
            id: CapabilityId::from(id),
            contribution_id: ContributionId::from(format!("capability:{id}")),
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
            config_schema: StrictJsonValue(schema),
            contributions: CapabilityContributions {
                actions: actions
                    .into_iter()
                    .map(|action_id| CapabilityActionDescriptor {
                        input_schema: format!("schema://{}/input", action_id.as_ref()).into(),
                        output_schema: format!("schema://{}/output", action_id.as_ref()).into(),
                        action_id,
                        effect_class: EffectClass::ReadLocal,
                        presentation: ToolPresentationKind::FunctionTool,
                        required_resource_operations: Default::default(),
                    })
                    .collect(),
                ..CapabilityContributions::default()
            },
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
            mount_id: AgentModuleId::from(format!("mount.{id}")),
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
                    mcp_binding_id: None,
                    artifact_digest: Some(artifact_digest),
                },
                publication_state: CapabilityPublicationState::Active,
                availability,
            },
        )
        .unwrap();
        (materialized, entry)
    }

    #[test]
    fn official_key_parser_rejects_retired_robot_and_workflow_templates() {
        assert_eq!(
            parse_official_key("coding.codex"),
            Some(OfficialPresetKey::CodingCodex)
        );
        assert!(parse_official_key("research").is_none());
        assert!(parse_official_key("autowork.executor").is_none());
        assert!(parse_official_key("robot.default").is_none());
    }

    #[tokio::test]
    async fn retired_device_source_recognition_requires_the_exact_official_owner_identity() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control = template_control_plane(store.clone(), OfficialPresetKey::CompanionDefault, false);
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let created = publish_official_fixture(&control, &owner, OfficialPresetKey::CompanionDefault, None).await;
        let binding = control.resolve_agent_session_binding(&owner, &created.preset.preset_id).await.unwrap();
        assert!(!control.is_retired_companion_device_binding(&owner, &binding).await.unwrap());
        let id = AgentPresetId::from(created.preset.preset_id);
        let mut stored = store.get_preset(&id).await.unwrap().unwrap();
        // Renaming a current main configuration is not a retired-source proof.
        stored.preset.display_name = "companion.robot".into();
        assert!(!retired_companion_device_configuration(&stored, &owner));
        stored.preset.preset_id = official_preset_id_for_key(&owner, "companion.robot");
        assert!(retired_companion_device_configuration(&stored, &owner));
        assert_eq!(require_current_preset_admission(&stored, &owner).unwrap_err().code().as_ref(), "AGENT_PRESET_NOT_FOUND");
        assert!(!retired_companion_device_configuration(&stored, &UserId::from("other-owner")));
        stored.preset.preset_id = AgentPresetId::from(Uuid::now_v7().to_string());
        stored.session_only = true;
        assert!(retired_companion_device_configuration(&stored, &owner), "host-authored frozen variants retain the exact source key");
        stored.preset.source = AgentPresetSource::User;
        assert!(!retired_companion_device_configuration(&stored, &owner), "user labels and caches cannot impersonate an official source");
        require_current_preset_admission(&stored, &owner).unwrap();
    }

    #[test]
    fn official_template_selection_preserves_the_seed_exact_action_subset() {
        let (capability, entry) = catalog_capability_with_actions_schema(
            "workspace.files",
            [CapabilityConsumer::Agent],
            [
                ActionId::from("workspace.files/read"),
                ActionId::from("workspace.files/write"),
            ],
            json!({"type":"object", "additionalProperties":false}),
        );
        let reference = CapabilityRef {
            id: capability.manifest.id.clone(),
        };
        let catalog = CatalogSnapshot {
            capabilities: vec![capability],
            formal_capability_entries: BTreeMap::from([(entry.capability.clone(), entry)]),
            ..Default::default()
        };
        let selection = template_selection(
            &CapabilitySelection {
                capability: reference,
                action_allowlist: BTreeSet::from([ActionId::from("workspace.files/read")]),
            },
            &catalog,
        )
        .unwrap();
        assert_eq!(
            selection.action_allowlist,
            BTreeSet::from([ActionId::from("workspace.files/read")])
        );
    }

    #[test]
    fn shared_catalog_resolves_agent_and_gateway_and_filters_agent_only_view() {
        let (shared, shared_entry) = catalog_capability(
            "knowledge",
            [CapabilityConsumer::Agent, CapabilityConsumer::Gateway],
        );
        let (knowledge_only, knowledge_only_entry) = catalog_capability(
            "knowledge.render.internal",
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
            test_compiler(),
        );
        let shared_ref = CapabilityRef {
            id: CapabilityId::from("knowledge"),
        };
        let knowledge_only_ref = CapabilityRef {
            id: CapabilityId::from("knowledge.render.internal"),
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
            vec!["knowledge"]
        );
    }

    #[tokio::test]
    async fn creation_template_without_a_saved_chat_route_rejects_an_invalid_session_override() {
        // Generation providers and Chat routes may be absent; the professional
        // Agent's bundled capabilities must still exist in the host registry.
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control_plane = template_control_plane(store.clone(), OfficialPresetKey::CreativeStudioDefault, false);
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let created = control_plane.create_from_template(&owner, "creative-studio.default", CreateAgentPresetFromTemplateRequest {
            model: None, reuse_existing: true, display_name: "Creative".into(), description: None,
            model_route_refs: BTreeMap::new(), chat_route_records: BTreeMap::new(),
        }).await.unwrap();
        assert!(created.draft.document.chat_route_records.is_empty());
        assert!(created.draft.document.model_route_refs.is_empty());
        let ignored = nomifun_api_types::AgentChatModelSelectionDto { provider_id: "unconfigured".into(), model: "unconfigured".into() };
        let invalid_override = tokio::time::timeout(std::time::Duration::from_secs(5),
            control_plane.resolve_agent_session_binding_with_model(&owner, &created.preset.preset_id, Some(&ignored))).await.unwrap().unwrap_err();
        assert_eq!(invalid_override.code().as_ref(), "MODEL_ROUTE_NOT_FOUND");
        let binding = tokio::time::timeout(std::time::Duration::from_secs(5),
            control_plane.resolve_agent_session_binding_with_model(&owner, &created.preset.preset_id, None)).await.unwrap().unwrap();
        assert_eq!(binding.preset_revision_ref.preset_id, created.preset.preset_id);
        assert_eq!(Some(binding.preset_revision_ref.clone()), created.preset.current_stable_revision);
        let repeated = control_plane.create_from_template(&owner, "creative-studio.default", CreateAgentPresetFromTemplateRequest {
            model: None, reuse_existing: true, display_name: "Creative".into(), description: None,
            model_route_refs: BTreeMap::new(), chat_route_records: BTreeMap::new(),
        }).await.unwrap();
        assert_eq!(repeated.preset.current_stable_revision, created.preset.current_stable_revision);
        assert!(repeated.draft.document.chat_route_records.is_empty());
        assert!(repeated.draft.document.model_route_refs.is_empty());
        let reference: PresetRevisionRef = wire_cast(&binding.preset_revision_ref).unwrap();
        let snapshot = store.get_snapshot(&reference).await.unwrap().unwrap();
        let expected_capability_count = OfficialTemplateCatalog::load()
            .unwrap()
            .seed(OfficialPresetKey::CreativeStudioDefault)
            .unwrap()
            .enabled_capabilities
            .len();
        assert_eq!(snapshot.content.enabled_capabilities.len(), expected_capability_count);
        let creation = snapshot
            .content
            .enabled_capabilities
            .iter()
            .find(|capability| capability.capability.id.as_ref() == "creation.media")
            .unwrap();
        assert_eq!(
            creation.action_allowlist,
            BTreeSet::from([
                ActionId::from("creation.media/audio"),
                ActionId::from("creation.media/image"),
                ActionId::from("creation.media/image_edit"),
                ActionId::from("creation.media/music"),
                ActionId::from("creation.media/text"),
                ActionId::from("creation.media/video"),
            ]),
            "official template Action grants must survive creation and compilation exactly"
        );
    }

    struct MediaChatRoutes(Vec<nomifun_agent_contracts::ChatRouteRecord>);

    #[async_trait::async_trait]
    impl DefaultChatRouteResolver for MediaChatRoutes {
        async fn resolve_default_chat_route(
            &self,
            _: &UserId,
        ) -> Result<Option<nomifun_agent_contracts::ChatRouteRecord>, ControlPlaneError> {
            Ok(self.0.first().cloned())
        }

        async fn resolve_selected_chat_route(
            &self,
            _: &UserId,
            model: &nomifun_api_types::AgentChatModelSelectionDto,
        ) -> Result<Option<nomifun_agent_contracts::ChatRouteRecord>, ControlPlaneError> {
            Ok(self.0.iter().find(|route| {
                route.primary.provider_id == model.provider_id
                    && route.primary.model == model.model
            }).cloned())
        }
    }

    fn media_chat_route(model: &str) -> nomifun_agent_contracts::ChatRouteRecord {
        let mut route = chat_route_with(
            ChatRouteProtocol::OpenaiChat,
            [ChatRouteFeature::TextInput, ChatRouteFeature::TextOutput, ChatRouteFeature::ToolCalls],
        );
        route.primary.provider_id = "0190f5fe-7c00-7a00-8000-000000000011".into();
        route.primary.model = model.into();
        route
    }

    /// Authoring scenarios explicitly publish the editable official identity;
    /// preparing an execution configuration is no longer an authoring save.
    async fn publish_official_fixture(
        control: &AgentControlPlane, owner: &UserId, key: OfficialPresetKey,
        model: Option<&nomifun_api_types::AgentChatModelSelectionDto>,
    ) -> AgentPresetEditorResponse {
        let preset = control.ensure_official_preset(owner, key).await.unwrap();
        let id = preset.preset.preset_id.as_ref();
        let mut draft = control.editor(owner, id, None).await.unwrap().draft;
        if let Some(model) = model {
            let route = control.preset_chat_model_route(owner, id, model).await.unwrap();
            draft.document.model_route_refs.insert(CHAT_MODEL_TASK.into(), route.primary.model_route_id.as_ref().into());
            draft.document.chat_route_records.insert(CHAT_MODEL_TASK.into(), serde_json::to_value(route).unwrap());
        }
        control.save_revision(owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(), draft,
            reason: Some("Explicit authoring fixture".into()),
        }).await.unwrap();
        control.editor(owner, id, None).await.unwrap()
    }

    #[tokio::test]
    async fn template_authoring_keeps_an_empty_model_default_until_explicit_selection() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let route = media_chat_route("installation-model");
        let control = test_control_plane(store.clone())
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route.clone()])));
        let created = control.create_from_template(&owner, "chat.minimal", official_launch_request(false)).await.unwrap();
        assert!(created.draft.document.model_route_refs.is_empty());
        assert!(created.draft.document.chat_route_records.is_empty(), "authoring cannot adopt the global or first available model");
        let refreshed = test_control_plane(store)
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![media_chat_route("replacement-model")])));
        let reopened = refreshed.editor(&owner, &created.preset.preset_id, None).await.unwrap();
        assert!(reopened.draft.document.chat_route_records.is_empty());
        assert!(reopened.draft.document.model_route_refs.is_empty());
        let explicit = control.create_from_template(&owner, "chat.minimal", CreateAgentPresetFromTemplateRequest {
            model: Some(nomifun_api_types::AgentChatModelSelectionDto {
                provider_id: route.primary.provider_id.clone(), model: route.primary.model.clone(),
            }), ..official_launch_request(false)
        }).await.unwrap();
        assert_eq!(explicit.draft.document.chat_route_records[CHAT_MODEL_TASK], serde_json::to_value(route).unwrap());
    }

    #[tokio::test]
    async fn initial_official_launch_uses_the_global_model_without_authoring_an_agent_default() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let route = media_chat_route("installation-model");
        let control = test_control_plane(store.clone())
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route.clone()])));
        let library = control.library(&owner).await.unwrap();
        let id = &library.official_presets[&nomifun_api_types::OfficialPresetKeyDto::ChatMinimal].preset_id;
        let before = control.editor(&owner, id, None).await.unwrap();
        let launch = control.create_from_template(&owner, "chat.minimal", official_launch_request(true)).await.unwrap();
        assert_ne!(launch.preset.preset_id, *id, "execution model belongs to the existing hidden configuration path");
        assert!(store.get_preset(&AgentPresetId::from(launch.preset.preset_id.clone())).await.unwrap().unwrap().session_only);
        let binding = control.resolve_agent_session_binding(&owner, &launch.preset.preset_id).await.unwrap();
        let (_, revision, _) = control.saved_binding_artifacts(&owner, &binding).await.unwrap();
        assert_eq!(revision.payload.chat_route_records[CHAT_MODEL_TASK], route);
        assert_eq!(control.editor(&owner, id, None).await.unwrap(), before, "launching cannot mutate the editable official Agent");
        let repeated = control.create_from_template(&owner, "chat.minimal", official_launch_request(true)).await.unwrap();
        assert_eq!(repeated.preset.preset_id, launch.preset.preset_id, "matching execution configurations remain reusable");
        assert_eq!(control.library(&owner).await.unwrap().official_presets[&nomifun_api_types::OfficialPresetKeyDto::ChatMinimal].current_stable_revision, None);
        let without_models = test_control_plane(store).with_default_chat_route_resolver(Arc::new(MediaChatRoutes(Vec::new())));
        assert!(without_models.editor(&owner, id, None).await.unwrap().draft.document.chat_route_records.is_empty());
    }

    #[tokio::test]
    async fn first_explicit_session_model_does_not_become_the_official_agent_default() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let route = media_chat_route("selected-for-session");
        let control = test_control_plane(store.clone())
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route.clone()])));
        let library = control.library(&owner).await.unwrap();
        let id = &library.official_presets[&nomifun_api_types::OfficialPresetKeyDto::ChatMinimal].preset_id;
        let launch = control.create_from_template(&owner, "chat.minimal", CreateAgentPresetFromTemplateRequest {
            model: Some(nomifun_api_types::AgentChatModelSelectionDto {
                provider_id: route.primary.provider_id.clone(), model: route.primary.model.clone(),
            }), ..official_launch_request(true)
        }).await.unwrap();
        assert_eq!(launch.draft.document.chat_route_records[CHAT_MODEL_TASK], serde_json::to_value(&route).unwrap());
        let editor = control.editor(&owner, id, None).await.unwrap();
        assert!(editor.draft.document.chat_route_records.is_empty());
        assert!(editor.draft.document.model_route_refs.is_empty());
        let mut draft = editor.draft;
        draft.document.chat_route_records.insert(CHAT_MODEL_TASK.into(), serde_json::to_value(route.clone()).unwrap());
        draft.document.model_route_refs.insert(CHAT_MODEL_TASK.into(), route.primary.model_route_id.as_ref().into());
        control.save_revision(&owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: None, draft, reason: None,
        }).await.unwrap();
        assert_eq!(control.editor(&owner, id, None).await.unwrap().draft.document.chat_route_records[CHAT_MODEL_TASK], serde_json::to_value(route).unwrap());
    }

    #[tokio::test]
    async fn authored_default_model_can_be_selected_cleared_and_overridden_at_session_creation() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let route = media_chat_route("selected-model");
        let control = test_control_plane(store.clone())
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route.clone()])));
        let created = control.create_preset(&owner, CreateAgentPresetRequest {
            display_name: "Optional default".into(), description: None,
            document: Some(empty_document()), fork_from_revision: None,
        }).await.unwrap();
        assert!(created.draft.document.chat_route_records.is_empty(), "authoring must not save the installation default implicitly");
        let selection = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route.primary.provider_id.clone(), model: route.primary.model.clone(),
        };
        let resolved = control.preset_chat_model_route(&owner, &created.preset.preset_id, &selection).await.unwrap();
        assert_eq!(resolved, route);
        assert_eq!(control.editor(&owner, &created.preset.preset_id, None).await.unwrap(), created,
            "resolving a draft choice must be read-only");
        let mut draft = created.draft;
        draft.document.model_route_refs.insert(CHAT_MODEL_TASK.into(), route.primary.model_route_id.as_ref().into());
        draft.document.chat_route_records.insert(CHAT_MODEL_TASK.into(), serde_json::to_value(&route).unwrap());
        let selected = control.save_revision(&owner, &draft.preset_id.clone(), SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(), draft, reason: None,
        }).await.unwrap();
        let mut draft = control.editor(&owner, &created.preset.preset_id, None).await.unwrap().draft;
        draft.document.model_route_refs.remove(CHAT_MODEL_TASK);
        draft.document.chat_route_records.remove(CHAT_MODEL_TASK);
        let cleared = control.save_revision(&owner, &draft.preset_id.clone(), SaveAgentPresetRevisionRequest {
            expected_current_revision: Some(selected.revision.reference), draft, reason: None,
        }).await.unwrap();
        let reopened = control.editor(&owner, &created.preset.preset_id, None).await.unwrap();
        assert!(reopened.draft.document.chat_route_records.is_empty());
        assert!(reopened.draft.document.model_route_refs.is_empty());
        let binding = control.resolve_agent_session_binding_with_model(&owner, &created.preset.preset_id, Some(&selection)).await.unwrap();
        let (_, revision, _) = control.saved_binding_artifacts(&owner, &binding).await.unwrap();
        assert_eq!(revision.payload.chat_route_records[CHAT_MODEL_TASK], route);
        let unchanged = control.editor(&owner, &created.preset.preset_id, None).await.unwrap();
        assert_eq!(unchanged.preset.current_stable_revision, Some(cleared.revision.reference));
        assert!(unchanged.draft.document.chat_route_records.is_empty(), "session selection must preserve the cleared default");
    }

    #[tokio::test]
    async fn saved_empty_official_default_is_preserved_when_preparing_a_session() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let route = media_chat_route("installation-model");
        let control = test_control_plane(store)
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route.clone()])));
        let library = control.library(&owner).await.unwrap();
        let id = &library.official_presets[&nomifun_api_types::OfficialPresetKeyDto::ChatMinimal].preset_id;
        let draft = control.editor(&owner, id, None).await.unwrap().draft;
        let saved = control.save_revision(&owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: None, draft, reason: None,
        }).await.unwrap();
        let prepared = control.create_from_template(&owner, "chat.minimal", official_launch_request(true)).await.unwrap();
        assert_eq!(prepared.preset.current_stable_revision, Some(saved.revision.reference.clone()));
        assert!(prepared.draft.document.chat_route_records.is_empty(), "an installation default must not overwrite an authored empty default");
        let selection = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route.primary.provider_id.clone(), model: route.primary.model.clone(),
        };
        let binding = control.resolve_agent_session_binding_with_model(&owner, id, Some(&selection)).await.unwrap();
        let (_, revision, _) = control.saved_binding_artifacts(&owner, &binding).await.unwrap();
        assert_eq!(revision.payload.chat_route_records[CHAT_MODEL_TASK], route);
        let preserved = control.editor(&owner, id, None).await.unwrap();
        assert_eq!(preserved.preset.current_stable_revision, Some(saved.revision.reference));
        assert!(preserved.draft.document.chat_route_records.is_empty());
    }

    fn image_only_document(control: &AgentControlPlane) -> nomifun_api_types::AgentPresetDocumentDto {
        let catalog = control.catalog.snapshot().unwrap();
        let mut media = control.templates.seed(OfficialPresetKey::CreativeStudioDefault).unwrap()
            .enabled_capabilities.iter()
            .find(|selection| selection.capability.id.as_ref() == "creation.media")
            .unwrap().clone();
        media.action_allowlist = BTreeSet::from([ActionId::from("creation.media/image")]);
        let mut document = empty_document();
        document.enabled_capabilities = vec![template_selection_api(&media, &catalog).unwrap()];
        document
    }

    fn assert_image_only_snapshot(snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope) {
        assert_eq!(snapshot.content.enabled_capabilities.len(), 1);
        let media = &snapshot.content.enabled_capabilities[0];
        assert_eq!(media.capability.id.as_ref(), "creation.media");
        assert_eq!(media.action_allowlist, BTreeSet::from([ActionId::from("creation.media/image")]));
        assert!(snapshot.content.required_resource_kinds.is_empty(),
            "selecting a text model must not add Workshop or asset-library authority");
    }

    #[tokio::test]
    async fn media_only_with_explicit_chat_route_supports_session_model_selection_and_switch() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let route_a = media_chat_route("text-a");
        let mut route_b = media_chat_route("text-b");
        route_b.primary.model_route_id = "0190f5fe-7c00-7a00-8000-000000000012".into();
        let control = template_control_plane(store.clone(), OfficialPresetKey::CreativeStudioDefault, false)
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route_a.clone(), route_b.clone()])));
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let mut document = image_only_document(&control);
        document.model_route_refs.insert(CHAT_MODEL_TASK.into(), route_a.primary.model_route_id.as_ref().into());
        document.chat_route_records.insert(CHAT_MODEL_TASK.into(), serde_json::to_value(&route_a).unwrap());
        let created = control.create_with_initial_revision(
            &owner, AgentPresetId::from(Uuid::now_v7().to_string()), "Image tool Agent".into(), None, document, None,
        ).await.unwrap();
        let source_revision = created.preset.current_stable_revision.clone();
        let model_b = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route_b.primary.provider_id.clone(), model: route_b.primary.model.clone(),
        };
        let selected = control.resolve_agent_session_binding_with_model(
            &owner, &created.preset.preset_id, Some(&model_b),
        ).await.unwrap();
        let (_, selected_revision, selected_snapshot) = control.saved_binding_artifacts(&owner, &selected).await.unwrap();
        assert_eq!(selected_revision.payload.chat_route_records[CHAT_MODEL_TASK], route_b);
        assert_image_only_snapshot(&selected_snapshot);
        assert!(selected.typed_resource_bindings.is_empty());
        assert_eq!(control.resolve_agent_session_model_binding(&owner, &selected, &model_b).await.unwrap(), selected,
            "selecting the same model is an exact no-op even for a media-only Agent");

        let model_a = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route_a.primary.provider_id.clone(), model: route_a.primary.model.clone(),
        };
        let switched = control.resolve_agent_session_model_binding(&owner, &selected, &model_a).await.unwrap();
        assert_eq!(switched.binding_version, selected.binding_version + 1);
        let (_, switched_revision, switched_snapshot) = control.saved_binding_artifacts(&owner, &switched).await.unwrap();
        assert_eq!(switched_revision.payload.chat_route_records[CHAT_MODEL_TASK], route_a);
        assert_image_only_snapshot(&switched_snapshot);
        assert!(switched.typed_resource_bindings.is_empty());
        let unchanged = control.editor(&owner, &created.preset.preset_id, None).await.unwrap();
        assert_eq!(unchanged.preset.current_stable_revision, source_revision);
        assert_eq!(unchanged.draft.document.chat_route_records[CHAT_MODEL_TASK], serde_json::to_value(route_a).unwrap());
    }

    #[tokio::test]
    async fn route_optional_media_preset_gets_chat_only_in_the_explicit_new_session_binding() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let route = media_chat_route("text-model");
        let control = template_control_plane(store.clone(), OfficialPresetKey::CreativeStudioDefault, false)
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route.clone()])));
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let created = control.create_with_initial_revision(
            &owner, AgentPresetId::from(Uuid::now_v7().to_string()), "Image tool Agent".into(), None,
            image_only_document(&control), None,
        ).await.unwrap();
        assert!(created.draft.document.chat_route_records.is_empty());
        let model = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route.primary.provider_id.clone(), model: route.primary.model.clone(),
        };
        let selected = control.resolve_agent_session_binding_with_model(
            &owner, &created.preset.preset_id, Some(&model),
        ).await.unwrap();
        assert_ne!(selected.preset_revision_ref.preset_id, created.preset.preset_id);
        let (_, revision, snapshot) = control.saved_binding_artifacts(&owner, &selected).await.unwrap();
        assert_eq!(revision.payload.chat_route_records[CHAT_MODEL_TASK], route);
        assert_image_only_snapshot(&snapshot);
        assert!(selected.typed_resource_bindings.is_empty());
        let unchanged = control.editor(&owner, &created.preset.preset_id, None).await.unwrap();
        assert_eq!(unchanged.preset.current_stable_revision, created.preset.current_stable_revision);
        assert!(unchanged.draft.document.chat_route_records.is_empty());
        assert!(unchanged.draft.document.model_route_refs.is_empty());
    }

    #[tokio::test]
    async fn historical_media_session_without_chat_still_rejects_a_model_switch() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let route = media_chat_route("text-model");
        let control = template_control_plane(store.clone(), OfficialPresetKey::CreativeStudioDefault, false)
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route.clone()])));
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let created = control.create_with_initial_revision(
            &owner, AgentPresetId::from(Uuid::now_v7().to_string()), "Image tool Agent".into(), None,
            image_only_document(&control), None,
        ).await.unwrap();
        let historical = control.resolve_agent_session_binding_with_model(&owner, &created.preset.preset_id, None).await.unwrap();
        let (_, revision, snapshot) = control.saved_binding_artifacts(&owner, &historical).await.unwrap();
        assert!(revision.payload.chat_route_records.is_empty());
        assert_image_only_snapshot(&snapshot);
        let model = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route.primary.provider_id, model: route.primary.model,
        };
        let error = control.resolve_agent_session_model_binding(&owner, &historical, &model).await.unwrap_err();
        assert_eq!(error.code().as_ref(), "AGENT_SESSION_MODEL_NOT_SWITCHABLE");
        assert_eq!(store.list_presets(&owner).await.unwrap().len(), 1,
            "rejecting an existing route-less Session must not create a model variant");
        let (_, still_historical, _) = control.saved_binding_artifacts(&owner, &historical).await.unwrap();
        assert_eq!(still_historical, revision);
    }

    #[tokio::test]
    async fn create_from_template_commits_initial_revision_and_does_not_persist_template_key() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control_plane = test_control_plane(store.clone());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");

        let created = control_plane
            .create_from_template(
                &owner,
                "chat.minimal",
                CreateAgentPresetFromTemplateRequest {
                    model: None,
                    reuse_existing: false,
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

    pub(super) fn official_launch_request(reuse_existing: bool) -> CreateAgentPresetFromTemplateRequest {
        CreateAgentPresetFromTemplateRequest {
            model: None,
            reuse_existing,
            display_name: "Minimal".into(),
            description: None,
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn app_developer_authors_product_workflow_without_changing_other_template_roles() {
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        for key in [OfficialPresetKey::AppDeveloper, OfficialPresetKey::CodingCodex, OfficialPresetKey::AssistantGeneral] {
            let store = Arc::new(InMemoryControlPlaneStore::new());
            let control = template_control_plane(store, key, false);
            let created = control.create_from_template(&owner, key.as_str(), official_launch_request(true)).await.unwrap();
            let binding = control.resolve_agent_session_binding(&owner, &created.preset.preset_id).await.unwrap();
            let (_, revision, _) = control.saved_binding_artifacts(&owner, &binding).await.unwrap();
            assert!(revision.payload.persona.is_empty());
            assert_eq!(revision.payload.instructions, official_template_instructions(key));
            assert_eq!(control.saved_binding_presentation(&owner, &binding).await.unwrap().1, Some(key));
            if key == OfficialPresetKey::AppDeveloper {
                assert!(!revision.payload.instructions.is_empty());
                assert_eq!(control.resolve_app_development_workflow_binding(&owner, &binding).await.unwrap(), binding,
                    "a current workflow is an exact no-op, without a new hidden configuration or binding version");
            }
        }
    }

    #[test]
    fn companion_seed_directly_handles_work_and_device_requests() {
        let instructions = official_template_instructions(OfficialPresetKey::CompanionDefault);
        assert!(instructions.contains("Handle ordinary chat, current information"));
        assert!(instructions.contains("finish silently after every requested action"));
        assert!(instructions.contains("never stop before all requested actions"));
        assert!(!instructions.contains("target_role"));
        assert!(parse_official_key("companion.robot").is_none());
    }

    #[tokio::test]
    async fn companion_official_presets_preserve_explicit_capabilities_and_immutable_history() {
        let templates = OfficialTemplateCatalog::load().unwrap();
        for (key, extra_key) in [(OfficialPresetKey::CompanionDefault, OfficialPresetKey::AssistantGeneral)] {
            let store = Arc::new(InMemoryControlPlaneStore::new());
            let extras = &templates.seed(extra_key).unwrap().enabled_capabilities;
            let control = template_control_plane_with_extra_capabilities(store.clone(), key, false, false, extras);
            let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
            let created = publish_official_fixture(&control, &owner, key, None).await;
            let frozen = control.resolve_agent_session_binding(&owner, &created.preset.preset_id).await.unwrap();
            let (_, original, original_snapshot) = control.saved_binding_artifacts(&owner, &frozen).await.unwrap();
            let mut draft = created.draft;
            draft.document.persona = "My explicit persona".into();
            draft.document.instructions = "Use every capability I explicitly authorize".into();
            for extra in extras {
                if let Some(selected) = draft.document.enabled_capabilities.iter_mut().find(|selection| selection.capability.id == extra.capability.id.as_ref()) {
                    selected.action_allowlist.extend(extra.action_allowlist.iter().map(|action| action.as_ref().to_owned()));
                } else { draft.document.enabled_capabilities.push(wire_cast(extra).unwrap()); }
            }
            let expected = draft.document.clone();
            let stale = draft.clone();
            let saved = control.save_revision(&owner, &created.preset.preset_id, SaveAgentPresetRevisionRequest {
                expected_current_revision: draft.current_revision.clone(), draft, reason: Some("Explicit authoring choice".into()),
            }).await.unwrap();
            assert_eq!(saved.revision.document, expected, "defaults must not limit explicit authoring");
            let latest = control.resolve_agent_session_binding(&owner, &created.preset.preset_id).await.unwrap();
            let (_, revision, snapshot) = control.saved_binding_artifacts(&owner, &latest).await.unwrap();
            assert_eq!(revision.payload, document_payload(&expected).unwrap());
            for extra in extras {
                assert!(snapshot.content.enabled_capabilities.iter().any(|selection| selection.capability == extra.capability
                    && extra.action_allowlist.is_subset(&selection.action_allowlist)));
            }
            assert_eq!(store.get_revision(&original.reference).await.unwrap().unwrap(), original);
            assert_eq!(store.get_snapshot(&original.reference).await.unwrap().unwrap(), original_snapshot);
            let reopened = control.create_from_template(&owner, key.as_str(), official_launch_request(true)).await.unwrap();
            assert_eq!(reopened.draft.document, expected, "reopening must not apply role defaults over saved choices");
            assert_eq!(control.save_revision(&owner, &created.preset.preset_id, SaveAgentPresetRevisionRequest {
                expected_current_revision: stale.current_revision.clone(), draft: stale, reason: None,
            }).await.unwrap_err().code().as_ref(), "PRESET_REVISION_DIGEST_MISMATCH");
        }
    }

    #[tokio::test]
    async fn app_developer_workflow_target_preserves_exact_frozen_configuration_and_history() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let route = media_chat_route("user-selected-app-model");
        let control = template_control_plane(store.clone(), OfficialPresetKey::AppDeveloper, false)
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route.clone()])));
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let created = control.create_from_template(&owner, "app.developer", official_launch_request(true)).await.unwrap();
        // Materialize an exact current-generation pre-workflow Revision, rather
        // than changing stored facts after a binding has been frozen.
        let mut draft = created.draft;
        draft.document.instructions.clear();
        let old = control.save_revision(&owner, &created.preset.preset_id, SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(), draft, reason: None,
        }).await.unwrap();
        let reference: PresetRevisionRef = wire_cast(&old.revision.reference).unwrap();
        let old_revision = store.get_revision(&reference).await.unwrap().unwrap();
        let old_snapshot = store.get_snapshot(&reference).await.unwrap().unwrap();
        let mut saved = session_binding_from_stable_artifacts(reference.clone(), Some(old_revision.clone()), Some(old_snapshot.clone())).unwrap();
        let mut resource: nomifun_agent_contracts::TypedResourceBinding = serde_json::from_value(json!({
            "binding_id":"pending", "resource_kind":"workspace", "resource_id":"managed-workspace",
            "owner_id":owner.as_ref(), "operations":["read","write"], "typed_parameters":{"workspace_root":"C:/owned/application","read_only":"false"},
        })).unwrap();
        resource.binding_id = nomifun_agent_contracts::resource_definition_id(&resource).unwrap();
        saved.typed_resource_bindings = vec![resource];
        saved.binding_version = 7;
        let saved: AgentBindingValueDto = wire_cast(&saved).unwrap();
        assert_eq!(control.saved_binding_presentation(&owner, &saved).await.unwrap().1, Some(OfficialPresetKey::AppDeveloper),
            "an old empty role keeps its legal identity without acquiring new instructions on read");
        assert!(control.saved_binding_artifacts(&owner, &saved).await.unwrap().1.payload.instructions.is_empty());

        let target = control.resolve_app_development_workflow_binding(&owner, &saved).await.unwrap();
        assert_ne!(target.preset_revision_ref, saved.preset_revision_ref);
        assert_eq!(target.binding_version, saved.binding_version, "only the canonical owner commits the version advance");
        assert_eq!(target.typed_resource_bindings, saved.typed_resource_bindings);
        let (_, target_revision, target_snapshot) = control.saved_binding_artifacts(&owner, &target).await.unwrap();
        let mut normalized = target_revision.payload.clone();
        assert_eq!(normalized.instructions, APP_DEVELOPMENT_INSTRUCTIONS);
        normalized.instructions.clear();
        assert_eq!(normalized, old_revision.payload, "model, grants, extensions, policy and all other authored fields remain exact");
        assert_eq!(target_revision.payload.chat_route_records[CHAT_MODEL_TASK], route);
        assert_eq!(control.editor(&owner, &target.preset_revision_ref.preset_id, None).await.unwrap().preset.source,
            nomifun_api_types::AgentPresetSourceDto::Official,
            "a workflow derivation preserves the host-verified official Agent provenance");
        assert_eq!(control.saved_binding_presentation(&owner, &target).await.unwrap().1,
            Some(OfficialPresetKey::AppDeveloper));
        assert!(same_app_workflow_snapshot_contract(&target_snapshot, &old_snapshot));
        assert_eq!(store.get_revision(&reference).await.unwrap().unwrap(), old_revision);
        assert_eq!(store.get_snapshot(&reference).await.unwrap().unwrap(), old_snapshot);
        assert_eq!(control.resolve_app_development_workflow_binding(&owner, &target).await.unwrap(), target);
    }

    #[tokio::test]
    async fn app_developer_workflow_rejects_custom_instructions_and_unrelated_agents() {
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let other = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        for key in [OfficialPresetKey::AppDeveloper, OfficialPresetKey::CodingCodex] {
            let store = Arc::new(InMemoryControlPlaneStore::new());
            let control = template_control_plane(store.clone(), key, false);
            let created = control.create_from_template(&owner, key.as_str(), official_launch_request(true)).await.unwrap();
            let mut draft = created.draft;
            draft.document.instructions = "Preserve this user's explicit application-independent behavior".into();
            let saved = control.save_revision(&owner, &created.preset.preset_id, SaveAgentPresetRevisionRequest {
                expected_current_revision: draft.current_revision.clone(), draft, reason: None,
            }).await.unwrap();
            let reference: PresetRevisionRef = wire_cast(&saved.revision.reference).unwrap();
            let revision = store.get_revision(&reference).await.unwrap().unwrap();
            let snapshot = store.get_snapshot(&reference).await.unwrap().unwrap();
            let binding = session_binding_from_stable_artifacts(reference.clone(), Some(revision.clone()), Some(snapshot.clone())).unwrap();
            let binding: AgentBindingValueDto = wire_cast(&binding).unwrap();
            let configurations_before = store.list_presets(&owner).await.unwrap().len();
            assert_eq!(control.resolve_app_development_workflow_binding(&owner, &binding).await.unwrap_err().code().as_ref(), "APP_DEVELOPMENT_SESSION_MISMATCH");
            assert!(control.resolve_app_development_workflow_binding(&other, &binding).await.is_err());
            assert_eq!(store.get_revision(&reference).await.unwrap().unwrap(), revision);
            assert_eq!(store.get_snapshot(&reference).await.unwrap().unwrap(), snapshot);
            assert_eq!(store.list_presets(&owner).await.unwrap().len(), configurations_before, "rejection does not create a replacement configuration");
        }
    }

    #[tokio::test]
    async fn app_developer_workflow_snapshot_guard_rejects_authority_or_model_changes() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control = template_control_plane(store.clone(), OfficialPresetKey::AppDeveloper, false);
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let created = control.create_from_template(&owner, "app.developer", official_launch_request(true)).await.unwrap();
        let reference: PresetRevisionRef = wire_cast(created.preset.current_stable_revision.as_ref().unwrap()).unwrap();
        let snapshot = store.get_snapshot(&reference).await.unwrap().unwrap();
        let mut changed = snapshot.clone();
        changed.content.enabled_capabilities.iter_mut().find(|value| value.capability.id.as_ref() == "app.hosting").unwrap()
            .action_allowlist.insert("app.hosting/new-authority".into());
        assert!(!same_app_workflow_snapshot_contract(&changed, &snapshot));
        let mut changed = snapshot.clone();
        changed.content.enabled_capabilities.iter_mut().find(|value| value.capability.id.as_ref() == "workspace.files").unwrap()
            .schema_digest = "changed-workspace-contract".into();
        assert!(!same_app_workflow_snapshot_contract(&changed, &snapshot));
        let mut changed = snapshot.clone();
        changed.content.model_route_refs.insert(CHAT_MODEL_TASK.into(), "another-model-route".into());
        assert!(!same_app_workflow_snapshot_contract(&changed, &snapshot));
    }

    #[test]
    fn official_launch_matches_default_route_facts_without_ignoring_configuration_changes() {
        let mut document = empty_document();
        document.model_route_refs.insert(CHAT_MODEL_TASK.into(), "0190f5fe-7c00-7a00-8000-000000000010".into());
        document.chat_route_records.insert(CHAT_MODEL_TASK.into(), json!({
            "schema": "nomifun.chat-route-record.v1", "task": "agent_chat",
            "primary": {
                "model_route_id": "0190f5fe-7c00-7a00-8000-000000000010", "model_route_revision": 1,
                "provider_id": "0190f5fe-7c00-7a00-8000-000000000011", "model": "test-model",
                "protocol": "openai_chat", "connection_config_ref": "default",
                "config_revision_digest": "a".repeat(64), "credential_ref": "opaque-alias-1",
                "features": ["text_input", "text_output"]
            }, "failovers": []
        }));
        let saved: nomifun_agent_contracts::AgentPresetRevisionPayload = document_payload(&document).unwrap();
        let mut current = saved.clone();
        current.model_route_refs.insert(CHAT_MODEL_TASK.into(), "0190f5fe-7c00-7a00-8000-000000000012".into());
        let primary = &mut current.chat_route_records.get_mut(CHAT_MODEL_TASK).unwrap().primary;
        primary.model_route_id = "0190f5fe-7c00-7a00-8000-000000000012".into();
        primary.credential_ref = "opaque-alias-2".into();
        assert!(template_launch_payload_matches(&saved, &current, true));
        assert!(!template_launch_payload_matches(&saved, &current, false));
        for field in ["provider_id", "model", "config_revision_digest", "connection_config_ref"] {
            let mut value = serde_json::to_value(&current).unwrap();
            value["chat_route_records"][CHAT_MODEL_TASK]["primary"][field] = if field == "config_revision_digest" {
                json!("b".repeat(64))
            } else {
                json!("changed")
            };
            let changed = serde_json::from_value(value).unwrap();
            assert!(!template_launch_payload_matches(&saved, &changed, true), "must compare {field}");
        }
    }

    #[tokio::test]
    async fn official_launch_reuses_one_owned_configuration_for_concurrent_requests() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control_plane = test_control_plane(store.clone());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let other = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let (first, second) = tokio::join!(
            control_plane.create_from_template(&owner, "chat.minimal", official_launch_request(true)),
            control_plane.create_from_template(&owner, "chat.minimal", official_launch_request(true)),
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(first.preset.preset_id, second.preset.preset_id);
        assert_eq!(first.preset.current_stable_revision, second.preset.current_stable_revision);
        let session_request = serde_json::from_value::<nomifun_api_types::CreateAgentSessionRequestDto>(
            json!({"preset_id": first.preset.preset_id}),
        ).expect("the official Agent must be accepted by the canonical Session request contract");
        assert_eq!(session_request.preset_id, first.preset.preset_id);
        let configurations = store.list_presets(&owner).await.unwrap();
        assert_eq!(configurations.iter().filter(|preset| preset.session_only).count(), 1);
        assert_eq!(configurations.iter().filter(|preset| !preset.session_only).count(), 1);
        assert!(control_plane.library(&owner).await.unwrap().user_presets.is_empty());
        let other_copy = control_plane.create_from_template(&other, "chat.minimal", official_launch_request(true)).await.unwrap();
        assert_ne!(first.preset.preset_id, other_copy.preset.preset_id);
        let explicit_copy = control_plane.create_from_template(&owner, "chat.minimal", official_launch_request(false)).await.unwrap();
        assert_ne!(first.preset.preset_id, explicit_copy.preset.preset_id);
        let library = control_plane.library(&owner).await.unwrap();
        assert_eq!(library.user_presets.len(), 1);
        assert_eq!(library.user_presets[0].preset_id, explicit_copy.preset.preset_id);
    }

    #[tokio::test]
    async fn official_launch_does_not_reuse_an_adjusted_personal_configuration() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control_plane = test_control_plane(store.clone());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let original = control_plane.create_from_template(&owner, "chat.minimal", official_launch_request(false)).await.unwrap();
        let mut draft = original.draft.clone();
        draft.document.instructions = "User-specific behavior".into();
        control_plane.save_revision(&owner, &original.preset.preset_id, SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(),
            draft, reason: None,
        }).await.unwrap();
        let fresh = control_plane.create_from_template(&owner, "chat.minimal", official_launch_request(true)).await.unwrap();
        assert_ne!(fresh.preset.preset_id, original.preset.preset_id);
        assert!(fresh.draft.document.instructions.is_empty());
        let preserved = control_plane.editor(&owner, &original.preset.preset_id, None).await.unwrap();
        assert_eq!(preserved.draft.document.instructions, "User-specific behavior");
    }

    #[tokio::test]
    async fn official_identity_preserves_edited_metadata_and_immutable_artifacts() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control = template_control_plane(store.clone(), OfficialPresetKey::CreativeStudioDefault, false);
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let mut request = official_launch_request(true);
        request.display_name = "Caller supplied name".into();
        let created = publish_official_fixture(&control, &owner, OfficialPresetKey::CreativeStudioDefault, None).await;
        assert_eq!(created.preset.display_name, "creative-studio.default");
        let id = AgentPresetId::from(created.preset.preset_id.clone());
        let reference: PresetRevisionRef = wire_cast(created.preset.current_stable_revision.as_ref().unwrap()).unwrap();
        let original_revision = serde_json::to_value(store.get_revision(&reference).await.unwrap().unwrap()).unwrap();
        let original_snapshot = serde_json::to_value(store.get_snapshot(&reference).await.unwrap().unwrap()).unwrap();

        // The stable owner-scoped identity is independent of editable metadata.
        for old_name in ["\u{591a}\u{6a21}", "\u{521b}\u{610f}\u{5de5}\u{574a}", "Multimodal", "Creative Studio", "创作", "Creation"] {
            let mut cached = store.get_preset(&id).await.unwrap().unwrap();
            cached.preset.display_name = old_name.into();
            store.update_preset_metadata(&cached).await.unwrap();
            assert_eq!(control.internal_official_template(&owner, id.as_ref()).await.unwrap(),
                Some(OfficialPresetKey::CreativeStudioDefault));
            assert_eq!(store.get_preset(&id).await.unwrap().unwrap().preset.display_name,
                old_name);
        }
        let mut cached = store.get_preset(&id).await.unwrap().unwrap();
        cached.preset.display_name = "Old launch label".into();
        store.update_preset_metadata(&cached).await.unwrap();
        let reused = control.create_from_template(&owner, "creative-studio.default", request).await.unwrap();
        assert_eq!(reused.preset.preset_id, created.preset.preset_id);
        assert_eq!(reused.preset.display_name, "Old launch label");
        assert_eq!(reused.preset.current_stable_revision, created.preset.current_stable_revision);
        assert_eq!(serde_json::to_value(store.get_revision(&reference).await.unwrap().unwrap()).unwrap(), original_revision);
        assert_eq!(serde_json::to_value(store.get_snapshot(&reference).await.unwrap().unwrap()).unwrap(), original_snapshot);
    }

    #[tokio::test]
    async fn personal_creation_preset_keeps_its_owned_name_and_is_not_an_official_configuration() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control = template_control_plane(store.clone(), OfficialPresetKey::CreativeStudioDefault, false);
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let mut request = official_launch_request(false);
        request.display_name = "My personal creator".into();
        let personal = control.create_from_template(&owner, "creative-studio.default", request).await.unwrap();
        assert_eq!(control.internal_official_template(&owner, &personal.preset.preset_id).await.unwrap(), None);
        assert_eq!(control.editor(&owner, &personal.preset.preset_id, None).await.unwrap().preset.display_name,
            "My personal creator");
        // Personal model variants are also session-only caches. Complete seed
        // equality must not discard their custom identity.
        let id = AgentPresetId::from(personal.preset.preset_id.clone());
        let mut variant = store.get_preset(&id).await.unwrap().unwrap();
        variant.session_only = true;
        store.update_preset_metadata(&variant).await.unwrap();
        assert_eq!(control.internal_official_template(&owner, id.as_ref()).await.unwrap(), None);
        assert_eq!(store.get_preset(&id).await.unwrap().unwrap().preset.display_name,
            "My personal creator");
    }

    #[tokio::test]
    async fn user_session_cache_named_app_developer_keeps_history_without_official_product_authority() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control = template_control_plane(store.clone(), OfficialPresetKey::AppDeveloper, false);
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let mut request = official_launch_request(false);
        request.display_name = "app.developer".into();
        let personal = control.create_from_template(&owner, "app.developer", request).await.unwrap();
        let id = AgentPresetId::from(personal.preset.preset_id.clone());
        let mut stored = store.get_preset(&id).await.unwrap().unwrap();
        assert_eq!(stored.preset.source, AgentPresetSource::User);
        stored.session_only = true;
        store.update_preset_metadata(&stored).await.unwrap();
        let binding = control.resolve_agent_session_binding(&owner, id.as_ref()).await.unwrap();
        let (_, revision, snapshot) = control.saved_binding_artifacts(&owner, &binding).await.unwrap();
        assert_eq!(control.internal_official_template(&owner, id.as_ref()).await.unwrap(), None);
        assert_eq!(control.saved_binding_presentation(&owner, &binding).await.unwrap(),
            ("app.developer".into(), None));
        assert_eq!(control.resolve_app_development_workflow_binding(&owner, &binding).await.unwrap_err().code().as_ref(),
            "APP_DEVELOPMENT_SESSION_MISMATCH");
        let (_, retained_revision, retained_snapshot) = control.saved_binding_artifacts(&owner, &binding).await.unwrap();
        assert_eq!(retained_revision, revision);
        assert_eq!(retained_snapshot, snapshot);
        assert_eq!(store.list_presets(&owner).await.unwrap().len(), 1,
            "a legacy User cache is neither promoted nor replaced during a history read");
    }

    #[tokio::test]
    async fn official_agent_edits_keep_identity_and_survive_launch_and_contract_refresh() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control = template_control_plane_with_order_contributions(store.clone(), OfficialPresetKey::CompanionDefault, false, true);
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let created = publish_official_fixture(&control, &owner, OfficialPresetKey::CompanionDefault, None).await;
        let id = &created.preset.preset_id;
        assert_eq!(control.internal_official_template(&owner, id).await.unwrap(), Some(OfficialPresetKey::CompanionDefault));
        let reference: PresetRevisionRef = wire_cast(created.preset.current_stable_revision.as_ref().unwrap()).unwrap();
        let former = store.get_revision(&reference).await.unwrap().unwrap();
        let snapshot = store.get_snapshot(&reference).await.unwrap().unwrap();
        let mut draft = created.draft;
        draft.document.instructions = "User-specific behavior".into();
        draft.document.persona = "My companion".into();
        draft.document.enabled_capabilities.push(nomifun_api_types::CapabilitySelectionDto {
            capability: nomifun_api_types::CapabilityRefDto { id: ORDER_TEST_MIDDLEWARE_ID.into() },
            action_allowlist: BTreeSet::from([nomifun_agent_contracts::tool_middleware::before_action().action_id.as_ref().to_owned()]),
        });
        let mut unsupported = draft.clone();
        unsupported.document.enabled_capabilities.retain(|selection| selection.capability.id != "companion.memory");
        assert_eq!(control.save_revision(&owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: unsupported.current_revision.clone(), draft: unsupported, reason: None,
        }).await.unwrap_err().code().as_ref(), "PRESET_REVISION_SAVE_FAILED");
        let saved = control.save_revision(&owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(), draft: draft.clone(), reason: None,
        }).await.unwrap();
        assert_eq!(saved.revision.reference.revision, reference.revision + 1);
        assert_eq!(saved.preset.source, nomifun_api_types::AgentPresetSourceDto::Official);
        assert_eq!(control.save_revision(&owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(), draft, reason: None,
        }).await.unwrap_err().code().as_ref(), "PRESET_REVISION_DIGEST_MISMATCH");
        let refreshed = template_control_plane_with_order_contributions(store.clone(), OfficialPresetKey::CompanionDefault, true, true);
        let launched = refreshed.create_from_template(&owner, "companion.default", official_launch_request(true)).await.unwrap();
        assert_eq!(launched.preset.preset_id, *id);
        assert_eq!(launched.draft.document, saved.revision.document);
        assert_eq!(refreshed.internal_official_template(&owner, id).await.unwrap(), Some(OfficialPresetKey::CompanionDefault));
        let binding = refreshed.resolve_agent_session_binding(&owner, id).await.unwrap();
        assert_eq!(refreshed.saved_binding_presentation(&owner, &binding).await.unwrap().1, Some(OfficialPresetKey::CompanionDefault));
        assert_eq!(store.get_revision(&reference).await.unwrap().unwrap(), former);
        assert_eq!(store.get_snapshot(&reference).await.unwrap().unwrap(), snapshot);
        assert_eq!(refreshed.retire_preset(&owner, id).await.unwrap_err().code().as_ref(), "AGENT_PRESET_NOT_FOUND");
        let copy = refreshed.create_from_template(&owner, "companion.default", official_launch_request(false)).await.unwrap();
        assert_ne!(copy.preset.preset_id, *id);
        assert_eq!(copy.draft.document.instructions, official_template_instructions(OfficialPresetKey::CompanionDefault),
            "a new copy starts from the shared shipped guidance without inheriting another Agent's authored override");
        assert!(!copy.draft.document.enabled_capabilities.iter().any(|selection| selection.capability.id == ORDER_TEST_MIDDLEWARE_ID));
    }

    #[test]
    fn recommended_capabilities_add_only_missing_authority_and_preserve_authored_fields() {
        let selection = |id: &str, actions: &[&str]| nomifun_api_types::CapabilitySelectionDto {
            capability: nomifun_api_types::CapabilityRefDto { id: id.into() },
            action_allowlist: actions.iter().map(|action| (*action).to_owned()).collect(),
        };
        let mut document = empty_document();
        document.enabled_capabilities = vec![
            selection("workspace.files", &["workspace.files/read", "workspace.files/custom"]),
            selection("user.module", &["user.module/run"]),
        ];
        document.skill_bindings = vec![ExactCatalogRefDto { id: "chosen.skill".into(), version: "1.0.0".into() }];
        document.persona = "My companion".into();
        document.instructions = "Keep this personal instruction".into();
        document.starter_prompts = vec!["Personal starter".into()];
        document.context_order = vec!["user.context".into()];
        document.middleware_order = vec!["user.middleware".into()];
        document.model_route_refs.insert(CHAT_MODEL_TASK.into(), "selected-route".into());
        document.chat_route_records.insert(CHAT_MODEL_TASK.into(), json!({"primary":{"model":"selected-model"}}));
        let original = document.clone();
        let mut recommended = empty_document();
        recommended.enabled_capabilities = vec![
            selection("workspace.files", &["workspace.files/read", "workspace.files/write"]),
            selection("workspace.process", &["workspace.process/start"]),
        ];
        recommended.skill_bindings = vec![
            ExactCatalogRefDto { id: "chosen.skill".into(), version: "2.0.0".into() },
            ExactCatalogRefDto { id: "new.skill".into(), version: "1.0.0".into() },
        ];
        let (added, added_skills) = add_recommended_capabilities(&mut document, &recommended);
        assert_eq!(added, vec![
            selection("workspace.files", &["workspace.files/write"]),
            selection("workspace.process", &["workspace.process/start"]),
        ]);
        assert_eq!(added_skills, vec![recommended.skill_bindings[1].clone()]);
        assert_eq!(document.skill_bindings[0], original.skill_bindings[0], "pinned Skill versions stay authored");
        assert!(document.enabled_capabilities[0].action_allowlist.contains("workspace.files/custom"));
        assert!(document.enabled_capabilities.contains(&original.enabled_capabilities[1]));
        let mut without_additions = document.clone();
        without_additions.enabled_capabilities = original.enabled_capabilities.clone();
        without_additions.skill_bindings = original.skill_bindings.clone();
        assert_eq!(without_additions, original, "only capabilities and absent Skills may change");
        assert_eq!(add_recommended_capabilities(&mut document, &recommended), (vec![], vec![]));
    }

    #[tokio::test]
    async fn companion_device_recommendations_do_not_widen_saved_or_frozen_bindings() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let control = template_control_plane(store.clone(), OfficialPresetKey::CompanionDefault, false);
        let created = publish_official_fixture(&control, &owner, OfficialPresetKey::CompanionDefault, None).await;
        let id = &created.preset.preset_id;
        // An existing authored configuration can intentionally retain only
        // vision. Publishing broader recommendations must not overwrite it.
        let mut draft = created.draft;
        draft.document.persona = "Keep my companion persona".into();
        draft.document.instructions = "Keep my chosen robot behavior".into();
        draft.document.enabled_capabilities.iter_mut().find(|selection| selection.capability.id == "robot")
            .unwrap().action_allowlist = BTreeSet::from(["robot/vision".into()]);
        control.save_revision(&owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(), draft, reason: Some("Explicit vision-only choice".into()),
        }).await.unwrap();
        let frozen = control.resolve_agent_session_binding(&owner, id).await.unwrap();
        let (_, old_revision, old_snapshot) = control.saved_binding_artifacts(&owner, &frozen).await.unwrap();
        let before = control.editor(&owner, id, None).await.unwrap();

        let restarted = template_control_plane(store.clone(), OfficialPresetKey::CompanionDefault, false);
        let reopened = restarted.create_from_template(&owner, "companion.default", official_launch_request(true)).await.unwrap();
        assert_eq!(reopened.draft.document, before.draft.document, "ordinary launch preserves the saved action subset");
        assert_eq!(reopened.preset.current_stable_revision, before.preset.current_stable_revision);
        let proposal = restarted.recommended_capabilities(&owner, id).await.unwrap();
        assert_eq!(proposal.added_capabilities, vec![nomifun_api_types::CapabilitySelectionDto {
            capability: nomifun_api_types::CapabilityRefDto { id: "robot".into() },
            action_allowlist: BTreeSet::from(["robot/display".into(), "robot/motion".into(), "robot/device".into()]),
        }]);
        assert_eq!(proposal.draft.document.persona, before.draft.document.persona);
        assert_eq!(proposal.draft.document.instructions, before.draft.document.instructions);
        assert_eq!(proposal.draft.document.chat_route_records, before.draft.document.chat_route_records);
        assert_eq!(restarted.editor(&owner, id, None).await.unwrap(), before, "recommendation inspection is read-only");
        let (_, still_frozen_revision, still_frozen_snapshot) = restarted.saved_binding_artifacts(&owner, &frozen).await.unwrap();
        assert_eq!(still_frozen_revision, old_revision);
        assert_eq!(still_frozen_snapshot, old_snapshot);

        let stale_draft = proposal.draft.clone();
        let saved = restarted.save_revision(&owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: proposal.draft.current_revision.clone(), draft: proposal.draft,
            reason: Some("Explicitly accept Robot recommendations".into()),
        }).await.unwrap();
        let actions = &saved.revision.document.enabled_capabilities.iter()
            .find(|selection| selection.capability.id == "robot").unwrap().action_allowlist;
        assert_eq!(actions, &BTreeSet::from([
            "robot/vision".into(), "robot/display".into(), "robot/motion".into(), "robot/device".into(),
        ]));
        assert_eq!(restarted.save_revision(&owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: stale_draft.current_revision.clone(), draft: stale_draft, reason: None,
        }).await.unwrap_err().code().as_ref(), "PRESET_REVISION_DIGEST_MISMATCH");
        let latest = restarted.resolve_agent_session_binding(&owner, id).await.unwrap();
        assert_ne!(latest.preset_revision_ref, frozen.preset_revision_ref, "only a new explicit binding can consume the saved additions");
        let (_, unchanged_revision, unchanged_snapshot) = restarted.saved_binding_artifacts(&owner, &frozen).await.unwrap();
        assert_eq!(unchanged_revision, old_revision, "saving recommendations cannot rewrite a frozen Revision");
        assert_eq!(unchanged_snapshot, old_snapshot, "saving recommendations cannot rewrite a frozen Snapshot");
    }

    #[tokio::test]
    async fn recommended_capabilities_are_owner_scoped_reviewable_and_save_with_existing_revision_fence() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control = template_control_plane(store.clone(), OfficialPresetKey::AssistantGeneral, false);
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let other = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let created = publish_official_fixture(&control, &owner, OfficialPresetKey::AssistantGeneral, None).await;
        let id = &created.preset.preset_id;
        let initial_ref: PresetRevisionRef = wire_cast(created.preset.current_stable_revision.as_ref().unwrap()).unwrap();
        let initial = store.get_revision(&initial_ref).await.unwrap().unwrap();
        let initial_snapshot = store.get_snapshot(&initial_ref).await.unwrap().unwrap();
        let mut draft = created.draft;
        draft.display_name = "My edited General".into();
        draft.document.persona = "My chosen persona".into();
        draft.document.instructions = "User-specific behavior".into();
        draft.document.enabled_capabilities.retain(|selection| selection.capability.id != "workspace.process");
        draft.document.enabled_capabilities.iter_mut().find(|selection| selection.capability.id == "workspace.files")
            .unwrap().action_allowlist.remove("workspace.files/write");
        control.save_revision(&owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(), draft, reason: Some("Explicit user choices".into()),
        }).await.unwrap();
        let before = control.editor(&owner, id, None).await.unwrap();
        let preview = control.recommended_capabilities(&owner, id).await.unwrap();
        assert_eq!(preview.draft.display_name, before.draft.display_name);
        assert_eq!(preview.draft.document.persona, before.draft.document.persona);
        assert_eq!(preview.draft.document.instructions, before.draft.document.instructions);
        assert_eq!(preview.draft.document.chat_route_records, before.draft.document.chat_route_records);
        assert_eq!(preview.draft.current_revision, before.draft.current_revision);
        assert_eq!(preview.added_capabilities.len(), 2);
        assert!(preview.added_capabilities.iter().any(|selection| selection.capability.id == "workspace.files"
            && selection.action_allowlist == BTreeSet::from(["workspace.files/write".into()])));
        assert_eq!(control.editor(&owner, id, None).await.unwrap(), before, "inspection must never save the proposal");
        assert_eq!(control.recommended_capabilities(&other, id).await.unwrap_err().code().as_ref(), "AGENT_PRESET_NOT_FOUND");
        let saved = control.save_revision(&owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: preview.draft.current_revision.clone(),
            draft: preview.draft.clone(), reason: Some("Complete recommended capabilities".into()),
        }).await.unwrap();
        assert_eq!(saved.revision.reference.revision, before.revision.unwrap().reference.revision + 1);
        assert_eq!(control.save_revision(&owner, id, SaveAgentPresetRevisionRequest {
            expected_current_revision: preview.draft.current_revision.clone(), draft: preview.draft, reason: None,
        }).await.unwrap_err().code().as_ref(), "PRESET_REVISION_DIGEST_MISMATCH");
        let complete = control.recommended_capabilities(&owner, id).await.unwrap();
        assert!(complete.added_capabilities.is_empty());
        assert!(complete.added_skills.is_empty());
        assert_eq!(store.get_revision(&initial_ref).await.unwrap().unwrap(), initial);
        assert_eq!(store.get_snapshot(&initial_ref).await.unwrap().unwrap(), initial_snapshot);
        let personal = control.create_from_template(&owner, "assistant.general", official_launch_request(false)).await.unwrap();
        assert_eq!(control.recommended_capabilities(&owner, &personal.preset.preset_id).await.unwrap_err().status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn official_library_and_editor_are_owner_scoped_without_a_model() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control = template_control_plane(store.clone(), OfficialPresetKey::CompanionDefault, false);
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let other = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let library = control.library(&owner).await.unwrap();
        assert_eq!(library.official_presets.len(), OfficialPresetKey::ALL.len());
        assert!(library.user_presets.is_empty());
        let official = &library.official_presets[&nomifun_api_types::OfficialPresetKeyDto::CompanionDefault];
        assert!(official.current_stable_revision.is_none());
        let editor = control.editor(&owner, &official.preset_id, None).await.unwrap();
        assert_eq!(editor.draft.source_template_key, Some(nomifun_api_types::OfficialPresetKeyDto::CompanionDefault));
        assert_eq!(editor.draft.document.enabled_capabilities.len(), control.templates.seed(OfficialPresetKey::CompanionDefault).unwrap().enabled_capabilities.len());
        assert!(editor.draft.document.chat_route_records.is_empty());
        assert_eq!(control.editor(&other, &official.preset_id, None).await.unwrap_err().code().as_ref(), "AGENT_PRESET_NOT_FOUND");
        assert_eq!(control.library(&owner).await.unwrap().official_presets, library.official_presets);
        assert_ne!(control.library(&other).await.unwrap().official_presets[&nomifun_api_types::OfficialPresetKeyDto::CompanionDefault].preset_id, official.preset_id);
    }

    #[tokio::test]
    async fn historical_official_model_variants_keep_template_identity_after_edits() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let route_a = media_chat_route("model-a");
        let mut route_b = media_chat_route("model-b");
        route_b.primary.model_route_id = "0190f5fe-7c00-7a00-8000-000000000012".into();
        let control = template_control_plane(store.clone(), OfficialPresetKey::CompanionDefault, false)
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route_a.clone(), route_b.clone()])));
        let selection_a = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route_a.primary.provider_id.clone(), model: route_a.primary.model.clone(),
        };
        let created = publish_official_fixture(&control, &owner, OfficialPresetKey::CompanionDefault, Some(&selection_a)).await;
        let current = control.resolve_agent_session_binding(&owner, &created.preset.preset_id).await.unwrap();
        let selection = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route_b.primary.provider_id.clone(), model: route_b.primary.model.clone(),
        };
        let variant = control.resolve_agent_session_model_binding(&owner, &current, &selection).await.unwrap();
        let (_, old_revision, old_snapshot) = control.saved_binding_artifacts(&owner, &variant).await.unwrap();
        let mut draft = created.draft;
        draft.display_name = "My extended companion".into();
        draft.document.instructions = "Additional user behavior".into();
        control.save_revision(&owner, &created.preset.preset_id, SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(), draft, reason: None,
        }).await.unwrap();
        assert_eq!(control.saved_binding_presentation(&owner, &variant).await.unwrap().1, Some(OfficialPresetKey::CompanionDefault));
        assert_eq!(control.internal_official_template(&owner, &variant.preset_revision_ref.preset_id).await.unwrap(), Some(OfficialPresetKey::CompanionDefault));
        let (_, unchanged_revision, unchanged_snapshot) = control.saved_binding_artifacts(&owner, &variant).await.unwrap();
        assert_eq!(unchanged_revision, old_revision);
        assert_eq!(unchanged_snapshot, old_snapshot);
        let launched = control.create_from_template(&owner, "companion.default", CreateAgentPresetFromTemplateRequest {
            model: Some(selection), ..official_launch_request(true)
        }).await.unwrap();
        assert_eq!(launched.draft.document.instructions, "Additional user behavior");
        assert_eq!(launched.preset.source, nomifun_api_types::AgentPresetSourceDto::Official);
    }

    #[tokio::test]
    async fn equal_official_payloads_do_not_share_a_different_template_model_variant() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let route_a = media_chat_route("model-a");
        let mut route_b = media_chat_route("model-b");
        route_b.primary.model_route_id = "0190f5fe-7c00-7a00-8000-000000000012".into();
        let control = test_control_plane(store).with_default_chat_route_resolver(
            Arc::new(MediaChatRoutes(vec![route_a.clone(), route_b.clone()])));
        let mut document = empty_document();
        document.model_route_refs.insert(CHAT_MODEL_TASK.into(), route_a.primary.model_route_id.as_ref().into());
        document.chat_route_records.insert(CHAT_MODEL_TASK.into(), serde_json::to_value(route_a).unwrap());
        let mut roots = Vec::new();
        for key in [OfficialPresetKey::AssistantGeneral, OfficialPresetKey::ChatMinimal] {
            let root = control.ensure_official_preset(&owner, key).await.unwrap();
            control.save_revision(&owner, root.preset.preset_id.as_ref(), SaveAgentPresetRevisionRequest {
                expected_current_revision: None,
                draft: AgentPresetDraftDto {
                    preset_id: root.preset.preset_id.as_ref().into(), display_name: "Identical edited name".into(),
                    description: None, source_template_key: Some(wire_cast(&key).unwrap()),
                    current_revision: None, document: document.clone(),
                }, reason: None,
            }).await.unwrap();
            roots.push((key, root.preset.preset_id));
        }
        let selected = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route_b.primary.provider_id, model: route_b.primary.model,
        };
        let first = control.resolve_agent_session_binding_with_model(&owner, roots[0].1.as_ref(), Some(&selected)).await.unwrap();
        let second = control.resolve_agent_session_binding_with_model(&owner, roots[1].1.as_ref(), Some(&selected)).await.unwrap();
        assert_ne!(first.preset_revision_ref.preset_id, second.preset_revision_ref.preset_id);
        assert_eq!(control.saved_binding_presentation(&owner, &first).await.unwrap().1, Some(roots[0].0));
        assert_eq!(control.saved_binding_presentation(&owner, &second).await.unwrap().1, Some(roots[1].0));
        assert_eq!(control.resolve_agent_session_binding_with_model(&owner, roots[1].1.as_ref(), Some(&selected)).await.unwrap(), second);
    }

    fn general_template_control_plane(store: Arc<InMemoryControlPlaneStore>, updated_schema: bool) -> AgentControlPlane {
        template_control_plane(store, OfficialPresetKey::AssistantGeneral, updated_schema)
    }

    #[tokio::test]
    async fn session_model_derivation_preserves_frozen_contract_without_authoring_inputs() {
        struct UnavailableCatalog;
        impl CatalogProvider for UnavailableCatalog {
            fn snapshot(&self) -> Result<Arc<CatalogSnapshot>, ControlPlaneError> {
                panic!("model derivation must not consult today's catalog")
            }
        }
        struct UnavailableRoleDefaults;
        #[async_trait::async_trait]
        impl crate::InstallationRoleBindingStore for UnavailableRoleDefaults {
            async fn load(&self, _: &UserId) -> Result<BTreeMap<nomifun_agent_contracts::ExecutionRoleId, nomifun_agent_contracts::InstallationRoleBinding>, ControlPlaneError> {
                panic!("model derivation must not consult today's provider defaults")
            }
            async fn put(&self, _: &UserId, _: nomifun_agent_contracts::RoleProviderSelection, _: u64) -> Result<nomifun_agent_contracts::InstallationRoleBinding, ControlPlaneError> {
                panic!("model derivation must not mutate provider defaults")
            }
        }
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let route_a = media_chat_route("model-a");
        let mut route_b = media_chat_route("model-b");
        route_b.primary.model_route_id = "0190f5fe-7c00-7a00-8000-000000000012".into();
        let old_control = general_template_control_plane(store.clone(), false)
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route_a.clone(), route_b.clone()])));
        let created = old_control.create_from_template(&owner, "assistant.general", CreateAgentPresetFromTemplateRequest {
            model: Some(nomifun_api_types::AgentChatModelSelectionDto {
                provider_id: route_a.primary.provider_id.clone(), model: route_a.primary.model.clone(),
            }), ..official_launch_request(false)
        }).await.unwrap();
        let mut payload = document_payload(&created.draft.document).unwrap();
        payload.instructions = "Preserve the Session's authored instruction".into();
        payload.skill_bindings.push(nomifun_agent_contracts::AgentSkillBinding::library_selected(
            nomifun_agent_contracts::FrozenLibrarySkill::new("saved-guide".into(), "Saved guide".into(),
                nomifun_agent_contracts::LibrarySkillSource::Custom, "Keep my saved context".into(), BTreeMap::new()).unwrap(), true,
        ));
        let configured = old_control.create_configuration_with_payload(&owner, Uuid::now_v7().to_string().into(),
            "Frozen user Agent".into(), None, payload, None, true, None).await.unwrap();
        let mut current = old_control.resolve_agent_session_binding(&owner, &configured.preset.preset_id).await.unwrap();
        current.typed_resource_bindings.push(wire_cast(&nomifun_agent_contracts::TypedResourceBinding {
            binding_id: "saved-browser-resource".into(), resource_kind: "browser".into(), resource_id: "session-browser".into(),
            owner_id: owner.as_ref().into(), operations: BTreeSet::from(["observe".into()]),
            connection_config_ref: None, typed_parameters: BTreeMap::from([("profile".into(), "saved".into())]),
        }).unwrap());
        let (_, source_revision, source_snapshot) = old_control.saved_binding_artifacts(&owner, &current).await.unwrap();
        let other_configuration = old_control.create_configuration_with_payload(&owner, Uuid::now_v7().to_string().into(),
            "Another named Agent".into(), Some("Another Agent description".into()), source_revision.payload.clone(), None, true, None).await.unwrap();
        let other_binding = old_control.resolve_agent_session_binding(&owner, &other_configuration.preset.preset_id).await.unwrap();
        let mut control = general_template_control_plane(store.clone(), true)
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![route_a.clone(), route_b.clone()])))
            .with_installation_role_binding_store(Arc::new(UnavailableRoleDefaults));
        control.catalog = Arc::new(UnavailableCatalog);
        control.compiler = PresetRevisionCompiler::new();
        let selection = nomifun_api_types::AgentChatModelSelectionDto { provider_id: route_b.primary.provider_id.clone(), model: route_b.primary.model.clone() };
        let other_switched = control.resolve_agent_session_model_binding(&owner, &other_binding, &selection).await.unwrap();
        let switched = control.resolve_agent_session_model_binding(&owner, &current, &selection).await.unwrap();
        assert_ne!(switched.preset_revision_ref, other_switched.preset_revision_ref,
            "hidden variant reuse must preserve the user's selected Agent presentation");
        let presentation = control.editor(&owner, &switched.preset_revision_ref.preset_id, None).await.unwrap().preset;
        assert_eq!(presentation.display_name, "Frozen user Agent");
        assert_eq!(presentation.description, None);
        let (_, revision, snapshot) = control.saved_binding_artifacts(&owner, &switched).await.unwrap();
        assert_eq!(switched.binding_version, current.binding_version + 1);
        assert_eq!(switched.typed_resource_bindings, current.typed_resource_bindings);
        assert!(same_non_model_revision_contract(&revision, &source_revision));
        assert!(same_non_model_snapshot_contract(&snapshot, &source_snapshot));
        assert_eq!(revision.payload.chat_route_records[CHAT_MODEL_TASK], route_b);
        assert_eq!(control.resolve_agent_session_model_binding(&owner, &switched, &selection).await.unwrap(), switched);
        let back = control.resolve_agent_session_model_binding(&owner, &switched, &nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: route_a.primary.provider_id, model: route_a.primary.model,
        }).await.unwrap();
        assert!(control.agent_session_model_history_compatible(&owner, &wire_cast(&back).unwrap(), &wire_cast(&current).unwrap()).await.unwrap());
        assert_eq!(store.get_revision(&source_revision.reference).await.unwrap().unwrap(), source_revision);
        assert_eq!(store.get_snapshot(&source_revision.reference).await.unwrap().unwrap(), source_snapshot);
    }

    #[tokio::test]
    async fn retired_empty_default_gains_local_os_tools_through_immutable_evolution() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let control = test_control_plane(store.clone());
        // Author the old current-generation empty contract through the normal
        // store, then assign the host-owned default's existing exact identity.
        let preset_id = official_preset_id(&owner, OfficialPresetKey::ChatMinimal);
        let created = control.create_configuration_with_initial_revision(
            &owner, preset_id.clone(), "chat.minimal".into(), None,
            empty_document(), Some(OfficialPresetKey::ChatMinimal), true, None,
        ).await.unwrap();
        let current = control.resolve_agent_session_binding(&owner, &created.preset.preset_id).await.unwrap();
        let (_, source_revision, source_snapshot) = control.saved_binding_artifacts(&owner, &current).await.unwrap();
        assert!(source_revision.payload.enabled_capabilities.is_empty());
        let next = control.resolve_agent_session_contract_evolution(&owner, &current).await.unwrap();
        let (_, revision, snapshot) = control.saved_binding_artifacts(&owner, &next).await.unwrap();
        assert_eq!(next.binding_version, current.binding_version + 1);
        assert_eq!(revision.payload.enabled_capabilities, nomifun_agent_contracts::default_local_os_capability_selections());
        assert!(snapshot.content.capability_allowlist.contains(&CapabilityId::from("workspace.files")));
        assert!(snapshot.content.capability_allowlist.contains(&CapabilityId::from("workspace.process")));
        assert_eq!(control.resolve_agent_session_contract_evolution(&owner, &next).await.unwrap(), next);
        assert_eq!(store.get_revision(&source_revision.reference).await.unwrap().unwrap(), source_revision);
        assert_eq!(store.get_snapshot(&source_revision.reference).await.unwrap().unwrap(), source_snapshot);

        let custom = control.create_preset(&owner, CreateAgentPresetRequest {
            display_name: "chat.minimal".into(), description: None,
            document: Some(empty_document()), fork_from_revision: None,
        }).await.unwrap();
        let custom = control.resolve_agent_session_binding(&owner, &custom.preset.preset_id).await.unwrap();
        assert_eq!(control.resolve_agent_session_contract_evolution(&owner, &custom).await.unwrap(), custom,
            "a matching name does not grant the official default's product transition");
    }

    #[tokio::test]
    async fn session_contract_evolution_preserves_configuration_and_requires_an_explicit_history_boundary() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let old_control = general_template_control_plane(store.clone(), false)
            .with_default_chat_route_resolver(Arc::new(MediaChatRoutes(vec![media_chat_route("saved-model")])));
        let created = old_control.create_from_template(&owner, "assistant.general", official_launch_request(false)).await.unwrap();
        let current = old_control.resolve_agent_session_binding(&owner, &created.preset.preset_id).await.unwrap();
        let (_, source_revision, source_snapshot) = old_control.saved_binding_artifacts(&owner, &current).await.unwrap();
        let control = general_template_control_plane(store.clone(), true);
        let evolved = control.resolve_agent_session_contract_evolution(&owner, &current).await.unwrap();
        let repeated = control.resolve_agent_session_contract_evolution(&owner, &current).await.unwrap();
        assert_eq!(evolved, repeated, "preparation reuses exact immutable artifacts");
        assert_eq!(evolved.binding_version, current.binding_version + 1);
        assert_eq!(evolved.typed_resource_bindings, current.typed_resource_bindings);
        let (_, revision, snapshot) = control.saved_binding_artifacts(&owner, &evolved).await.unwrap();
        assert_eq!(revision.payload, source_revision.payload);
        assert_ne!(revision.contribution_locks, source_revision.contribution_locks);
        assert_ne!(snapshot.snapshot_ref, source_snapshot.snapshot_ref);
        assert_eq!(snapshot.content.skill_locks, source_snapshot.content.skill_locks);
        assert_eq!(snapshot.content.mcp_tool_locks, source_snapshot.content.mcp_tool_locks);
        assert_eq!(control.resolve_agent_session_contract_evolution(&owner, &evolved).await.unwrap(), evolved);
        assert!(!control.agent_session_model_history_compatible(&owner, &wire_cast(&evolved).unwrap(), &wire_cast(&current).unwrap()).await.unwrap(),
            "a capability evolution is not permission to accept prior Tool protocols as a model-only history");
        assert_eq!(store.get_revision(&source_revision.reference).await.unwrap().unwrap(), source_revision);
        assert_eq!(store.get_snapshot(&source_revision.reference).await.unwrap().unwrap(), source_snapshot);
        assert_eq!(control.editor(&owner, &created.preset.preset_id, None).await.unwrap().preset.current_stable_revision, created.preset.current_stable_revision);
    }

    #[tokio::test]
    async fn session_contract_evolution_rejects_changed_effect_or_resource_authority_before_persistence() {
        for changed_effect in [true, false] {
            let store = Arc::new(InMemoryControlPlaneStore::new());
            let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
            let old_control = general_template_control_plane(store.clone(), false);
            let created = old_control.create_from_template(&owner, "assistant.general", official_launch_request(false)).await.unwrap();
            let current = old_control.resolve_agent_session_binding(&owner, &created.preset.preset_id).await.unwrap();
            let mut control = general_template_control_plane(store.clone(), false);
            let mut catalog = (*control.catalog.snapshot().unwrap()).clone();
            let browser = catalog.capabilities.iter_mut().find(|value| value.manifest.id.as_ref() == "browser").unwrap();
            if changed_effect {
                browser.manifest.contributions.actions[0].effect_class = EffectClass::WriteReversible;
            } else {
                browser.manifest.contributions.resource_kinds.insert("new-resource".into());
            }
            let entry = republish_catalog_capability(browser);
            catalog.formal_capability_entries.insert(entry.capability.clone(), entry);
            let mut registry = MaterializedRegistry::empty();
            for capability in &catalog.capabilities { registry.capabilities.insert(capability.manifest.id.clone(), capability.clone()); }
            for skill in &catalog.skills { registry.skills.insert(skill.definition.id.clone(), skill.clone()); }
            control.catalog = Arc::new(StaticCatalogProvider::new(catalog));
            control.compiler = test_compiler().with_materialized_registry(Arc::new(registry), CompilerEnvironment {
                resolver_version: "1.0.0".into(), required_runtime_protocol_version: "1.0.0".into(),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: "runtime-features".into(), available_runtime_features: BTreeSet::new(),
                installation_role_bindings: BTreeMap::new(), canonical_schema_manifest_digest: "schema".into(),
                target_contribution_manifest_digest: "contributions".into(), host_target: "test".into(),
                host_surface: "desktop".into(), availability_evidence_revision: "fixture".into(),
            });
            let error = control.resolve_agent_session_contract_evolution(&owner, &current).await.unwrap_err();
            assert_eq!(error.code().as_ref(), "AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED");
            assert_eq!(store.list_presets(&owner).await.unwrap().len(), 1, "rejected evolution must not persist candidate artifacts");
        }
    }

    fn install_bounded_launch_catalog(control: &mut AgentControlPlane, catalog: CatalogSnapshot) {
        let mut registry = MaterializedRegistry::empty();
        for capability in &catalog.capabilities {
            registry.capabilities.insert(capability.manifest.id.clone(), capability.clone());
        }
        control.catalog = Arc::new(StaticCatalogProvider::new(catalog));
        control.compiler = test_compiler().with_materialized_registry(Arc::new(registry), CompilerEnvironment {
            resolver_version: "1.0.0".into(), required_runtime_protocol_version: "1.0.0".into(),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: "runtime-features".into(), available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(), canonical_schema_manifest_digest: "schema".into(),
            target_contribution_manifest_digest: "contributions".into(), host_target: "test".into(),
            host_surface: "desktop".into(), availability_evidence_revision: "fixture".into(),
        });
    }

    async fn retired_official_launch_fixture(store: Arc<InMemoryControlPlaneStore>)
        -> (AgentControlPlane, UserId, AgentPresetRevision, nomifun_agent_contracts::ResolvedSnapshotEnvelope) {
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let mut route_b = media_chat_route("model-b");
        route_b.primary.model_route_id = "0190f5fe-7c00-7a00-8000-000000000012".into();
        let mut control = test_control_plane(store.clone()).with_default_chat_route_resolver(
            Arc::new(MediaChatRoutes(vec![media_chat_route("model-a"), route_b])));
        let mut catalog = (*control.catalog.snapshot().unwrap()).clone();
        let process = catalog.capabilities.iter_mut().find(|value| value.manifest.id.as_ref() == "workspace.process").unwrap();
        process.manifest.package.id = "nomifun.workspace-execution".into();
        process.source.source_identity = "nomifun.workspace-execution".into();
        process.contribution_lock.source_identity = "nomifun.workspace-execution".into();
        process.manifest.contributions.resource_kinds = BTreeSet::from(["process_session".into()]);
        process.manifest.contributions.actions.iter_mut().find(|action| action.action_id.as_ref() == "workspace.process/start")
            .unwrap().effect_class = EffectClass::ExecuteLocal;
        let entry = republish_catalog_capability(process);
        catalog.formal_capability_entries.insert(entry.capability.clone(), entry);
        install_bounded_launch_catalog(&mut control, catalog);
        let root = control.ensure_official_preset(&owner, OfficialPresetKey::ChatMinimal).await.unwrap();
        let mut editor = control.editor(&owner, root.preset.preset_id.as_ref(), None).await.unwrap();
        editor.draft.document.instructions = "Preserve the saved official instructions".into();
        let route = media_chat_route("model-a");
        editor.draft.document.model_route_refs.insert(CHAT_MODEL_TASK.into(), route.primary.model_route_id.as_ref().into());
        editor.draft.document.chat_route_records.insert(CHAT_MODEL_TASK.into(), serde_json::to_value(route).unwrap());
        control.save_revision(&owner, root.preset.preset_id.as_ref(), SaveAgentPresetRevisionRequest {
            expected_current_revision: None, draft: editor.draft, reason: None,
        }).await.unwrap();
        let binding = control.resolve_agent_session_binding(&owner, root.preset.preset_id.as_ref()).await.unwrap();
        let (_, mut revision, mut snapshot) = control.saved_binding_artifacts(&owner, &binding).await.unwrap();
        let previous_ref = revision.reference.clone();
        let process = snapshot.content.enabled_capabilities.iter_mut()
            .find(|value| value.capability.id.as_ref() == "workspace.process").unwrap();
        let mut retired = process.actions.iter().find(|action| action.action_id.as_ref() == "workspace.process/start").unwrap().clone();
        retired.action_id = "workspace.process/exec".into();
        process.actions.insert(0, retired);
        process.action_allowlist = BTreeSet::from(["workspace.process/exec".into()]);
        revision.payload.enabled_capabilities.iter_mut().find(|value| value.capability.id.as_ref() == "workspace.process")
            .unwrap().action_allowlist = process.action_allowlist.clone();
        revision.reference.revision += 1;
        revision.reference.revision_digest = revision.revision_digest().unwrap();
        snapshot.content.preset_revision_ref = revision.reference.clone();
        snapshot.content.chat_route_identity = revision.chat_route_identity().unwrap();
        snapshot.snapshot_ref.snapshot_digest = digest_payload(&snapshot.content).unwrap();
        snapshot = nomifun_agent_kernel::AgentPresetCompiler::derive_model_snapshot(&revision, &snapshot,
            nomifun_agent_kernel::CompileRequest {
                revision: revision.clone(), principal: snapshot.actor.clone(), scene: snapshot.scene.clone(),
                surface: snapshot.surface.clone(), audience: snapshot.audience.clone(),
                created_at_ms: snapshot.created_at_ms, resolver_run_id: snapshot.resolver_run_id.clone(),
            }).unwrap();
        store.append_revision(Some(&previous_ref), revision.clone(), snapshot.clone(),
            "Saved official Agent".into(), Some("Preserve my presentation".into())).await.unwrap();
        (control, owner, revision, snapshot)
    }

    #[tokio::test]
    async fn retired_official_launch_opens_new_sessions_without_rewriting_authored_history() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let (control, owner, source_revision, source_snapshot) = retired_official_launch_fixture(store.clone()).await;
        let preset_id = source_revision.reference.preset_id.as_ref();
        let opened = control.resolve_agent_session_binding(&owner, preset_id).await.unwrap();
        assert_eq!(opened.binding_version, 1);
        assert_ne!(opened.preset_revision_ref.preset_id, preset_id);
        assert_eq!(control.resolve_agent_session_binding(&owner, preset_id).await.unwrap(), opened);
        let (_, revision, snapshot) = control.saved_binding_artifacts(&owner, &opened).await.unwrap();
        assert_eq!(revision.payload.instructions, source_revision.payload.instructions);
        let process = snapshot.content.enabled_capabilities.iter().find(|value| value.capability.id.as_ref() == "workspace.process").unwrap();
        assert_eq!(process.action_allowlist, BTreeSet::from(["workspace.process/start".into()]));
        assert!(!process.actions.iter().any(|action| action.action_id.as_ref() == "workspace.process/exec"));

        let selected = nomifun_api_types::AgentChatModelSelectionDto {
            provider_id: media_chat_route("model-b").primary.provider_id, model: "model-b".into(),
        };
        let modeled = control.resolve_agent_session_binding_with_model(&owner, preset_id, Some(&selected)).await.unwrap();
        let (_, modeled_revision, modeled_snapshot) = control.saved_binding_artifacts(&owner, &modeled).await.unwrap();
        assert_eq!(modeled_revision.payload.chat_route_records[CHAT_MODEL_TASK].primary.model, "model-b");
        assert_eq!(modeled_snapshot.content.enabled_capabilities, snapshot.content.enabled_capabilities);
        assert_eq!(control.resolve_agent_session_binding_with_model(&owner, preset_id, Some(&selected)).await.unwrap(), modeled);
        let prepared = control.create_from_template(&owner, "chat.minimal", CreateAgentPresetFromTemplateRequest {
            model: Some(selected), ..official_launch_request(true)
        }).await.unwrap();
        assert_eq!(prepared.preset.preset_id, modeled.preset_revision_ref.preset_id);
        let source = store.get_preset(&source_revision.reference.preset_id).await.unwrap().unwrap();
        assert_eq!(source.preset.current_stable_revision, Some(source_revision.reference.clone()));
        assert_eq!(source.preset.display_name, "Saved official Agent");
        assert_eq!(store.get_revision(&source_revision.reference).await.unwrap().unwrap(), source_revision);
        assert_eq!(store.get_snapshot(&source_revision.reference).await.unwrap().unwrap(), source_snapshot);
    }

    #[tokio::test]
    async fn retired_official_launch_rejects_publisher_effect_and_resource_drift_before_persistence() {
        for drift in ["publisher", "effect", "resource"] {
            let store = Arc::new(InMemoryControlPlaneStore::new());
            let (mut control, owner, source_revision, source_snapshot) = retired_official_launch_fixture(store.clone()).await;
            let mut catalog = (*control.catalog.snapshot().unwrap()).clone();
            let process = catalog.capabilities.iter_mut().find(|value| value.manifest.id.as_ref() == "workspace.process").unwrap();
            match drift {
                "publisher" => { process.manifest.package.id = "other.publisher".into(); process.source.source_identity = "other.publisher".into();
                    process.contribution_lock.source_identity = "other.publisher".into(); }
                "effect" => process.manifest.contributions.actions.iter_mut().find(|value| value.action_id.as_ref() == "workspace.process/start")
                    .unwrap().effect_class = EffectClass::Destructive,
                _ => { process.manifest.contributions.resource_kinds.insert("new-resource".into()); }
            }
            let entry = republish_catalog_capability(process);
            catalog.formal_capability_entries.insert(entry.capability.clone(), entry);
            install_bounded_launch_catalog(&mut control, catalog);
            let count = store.list_presets(&owner).await.unwrap().len();
            let error = control.resolve_agent_session_binding(&owner, source_revision.reference.preset_id.as_ref()).await.unwrap_err();
            assert_eq!(error.code().as_ref(), "AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED", "{drift}");
            assert_eq!(store.list_presets(&owner).await.unwrap().len(), count);
            assert_eq!(store.get_revision(&source_revision.reference).await.unwrap().unwrap(), source_revision);
            assert_eq!(store.get_snapshot(&source_revision.reference).await.unwrap().unwrap(), source_snapshot);
        }
    }

    const ORDER_TEST_MIDDLEWARE_ID: &str = "fixture.order.guard";

    fn republish_catalog_capability(capability: &mut MaterializedCapability) -> CapabilityCatalogEntry {
        let contract_digest = digest_payload(&capability.manifest).unwrap();
        capability.schema_digest = contract_digest.clone();
        capability.contribution_lock.contract_digest = contract_digest;
        CapabilityCatalogMaterializer::materialize(CapabilityCatalogMaterialization {
            manifest: capability.manifest.clone(),
            provenance: CapabilityProvenance {
                owner: CapabilityOwner::Package { package: capability.manifest.package.clone() },
                source_kind: capability.contribution_lock.source_kind,
                source_identity: capability.contribution_lock.source_identity.clone(),
                mount_id: capability.contribution_lock.mount_id.clone(),
                mcp_binding_id: capability.contribution_lock.mcp_binding_id.clone(),
                artifact_digest: Some(capability.target_artifact_digest.clone()),
            },
            publication_state: CapabilityPublicationState::Active,
            availability: capability.manifest.supported_consumers().unwrap().into_iter()
                .map(|consumer| (consumer, CatalogAvailability::Active)).collect(),
        }).unwrap()
    }

    fn global_extensions_control_plane(store: Arc<InMemoryControlPlaneStore>) -> AgentControlPlane {
        struct Routes;
        #[async_trait::async_trait]
        impl DefaultChatRouteResolver for Routes {
            async fn resolve_default_chat_route(&self, _: &UserId)
                -> Result<Option<nomifun_agent_contracts::ChatRouteRecord>, ControlPlaneError>
            {
                Ok(Some(chat_route_with(ChatRouteProtocol::OpenaiChat,
                    [ChatRouteFeature::TextInput, ChatRouteFeature::TextOutput])))
            }

            async fn resolve_selected_chat_route(&self, _: &UserId, model: &nomifun_api_types::AgentChatModelSelectionDto)
                -> Result<Option<nomifun_agent_contracts::ChatRouteRecord>, ControlPlaneError>
            {
                let mut route = chat_route_with(ChatRouteProtocol::OpenaiChat,
                    [ChatRouteFeature::TextInput, ChatRouteFeature::TextOutput]);
                // The host can allocate a fresh route alias on every resolve.
                route.primary.model_route_id = Uuid::now_v7().to_string().into();
                route.primary.provider_id = model.provider_id.clone();
                route.primary.model = model.model.clone();
                Ok(Some(route))
            }
        }
        struct GlobalExtensions;
        #[async_trait::async_trait]
        impl SessionCapabilitiesResolver for GlobalExtensions {
            async fn resolve(&self, _: &UserId, selection: Option<&nomifun_api_types::SessionCapabilitySelectionDto>)
                -> Result<ResolvedSessionCapabilities, ControlPlaneError>
            {
                assert!(selection.is_none(), "authoring captures the user's normal global defaults");
                Ok(ResolvedSessionCapabilities {
                    skills: vec![nomifun_agent_contracts::FrozenLibrarySkill::new(
                        "authoring-guide".into(), "My reusable creation guide".into(),
                        nomifun_agent_contracts::LibrarySkillSource::Custom,
                        "Keep the user's preferred app design and file organization.".into(), BTreeMap::new(),
                    ).unwrap()],
                    selected_skill_names: BTreeSet::from(["authoring-guide".into()]),
                    mcp_server_ids: BTreeSet::from(["fixture.authoring-server".into()]),
                })
            }
        }
        let mut control = template_control_plane_with_order_contributions(store, OfficialPresetKey::AssistantGeneral, false, true)
            .with_default_chat_route_resolver(Arc::new(Routes))
            .with_session_capabilities_resolver(Arc::new(GlobalExtensions));
        let mut catalog = (*control.catalog.snapshot().unwrap()).clone();
        let (mut capability, _) = catalog_capability_with_actions_schema(
            "nomi.mcp.v1.authoring-helper", [CapabilityConsumer::Agent],
            [ActionId::from("nomi.mcp.v1.authoring-helper/read")], json!({"type":"object", "additionalProperties":false}),
        );
        let binding_id = nomifun_agent_contracts::McpBindingId::from("fixture.authoring-server:read");
        capability.contribution_lock.source_kind = ContributionSourceKind::McpBinding;
        capability.contribution_lock.source_identity = "mcp:fixture.authoring-server".into();
        capability.contribution_lock.mount_id = Some(capability.mount_id.clone());
        capability.contribution_lock.mcp_binding_id = Some(binding_id.clone());
        capability.source.source_kind = PluginSourceKind::ManagedLocal;
        capability.source.source_identity = capability.mount_id.as_ref().to_owned();
        let entry = republish_catalog_capability(&mut capability);
        let mapping = nomifun_agent_contracts::McpToolCapabilityMapping {
            package: capability.manifest.package.clone(), server_id: "fixture.authoring-server".into(),
            canonical_tool_key: "read".into(), schema_digest: "b".repeat(64).into(),
            capability: CapabilityRef { id: capability.manifest.id.clone() }, materialization_version: "1.0.0".into(),
        };
        catalog.mcp_tools.push(nomifun_agent_kernel::MaterializedMcpTool {
            mapping, binding_id, contribution_lock: capability.contribution_lock.clone(),
            target_artifact_digest: capability.target_artifact_digest.clone(), mount_id: capability.mount_id.clone(),
            source: capability.source.clone(),
        });
        catalog.formal_capability_entries.insert(entry.capability.clone(), entry);
        catalog.capabilities.push(capability);
        let mut registry = MaterializedRegistry::empty();
        for item in &catalog.capabilities { registry.capabilities.insert(item.manifest.id.clone(), item.clone()); }
        for item in &catalog.skills { registry.skills.insert(item.definition.id.clone(), item.clone()); }
        for item in &catalog.mcp_tools {
            let key = (item.mapping.server_id.clone(), item.mapping.canonical_tool_key.clone());
            registry.mcp_by_capability.insert(item.mapping.capability.id.clone(), key.clone());
            registry.mcp_tools.insert(key, item.clone());
        }
        control.catalog = Arc::new(StaticCatalogProvider::new(catalog));
        control.compiler = test_compiler().with_materialized_registry(Arc::new(registry), CompilerEnvironment {
            resolver_version: VersionString::from("1.0.0"), required_runtime_protocol_version: VersionString::from("1.0.0"),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: DigestHex::from("runtime-features"), available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(), canonical_schema_manifest_digest: DigestHex::from("schema"),
            target_contribution_manifest_digest: DigestHex::from("contributions"), host_target: RuntimeTarget::from("test"),
            host_surface: "desktop".into(), availability_evidence_revision: "fixture".into(),
        });
        control
    }

    #[tokio::test]
    async fn explicit_extension_selection_removes_unavailable_mcp_without_accepting_core_authority_drift() {
        struct EmptyExtensions;
        #[async_trait::async_trait]
        impl SessionCapabilitiesResolver for EmptyExtensions {
            async fn resolve(&self, _: &UserId, selection: Option<&nomifun_api_types::SessionCapabilitySelectionDto>)
                -> Result<ResolvedSessionCapabilities, ControlPlaneError>
            {
                let selection = selection.expect("the owner explicitly removed extensions");
                assert!(selection.skill_names.is_empty() && selection.mcp_server_ids.is_empty());
                Ok(ResolvedSessionCapabilities {
                    skills: Vec::new(), selected_skill_names: BTreeSet::new(), mcp_server_ids: BTreeSet::new(),
                })
            }
        }
        for changed_authority in [None, Some(true), Some(false)] {
            let store = Arc::new(InMemoryControlPlaneStore::new());
            let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
            let mut control = global_extensions_control_plane(store.clone());
            let prepared = control.create_from_template(&owner, "assistant.general", official_launch_request(true)).await.unwrap();
            let current = control.resolve_agent_session_binding_with_model(&owner, &prepared.preset.preset_id, None).await.unwrap();
            let (_, source_revision, source_snapshot) = control.saved_binding_artifacts(&owner, &current).await.unwrap();
            assert!(!source_snapshot.content.mcp_tool_locks.is_empty());
            assert!(!source_snapshot.content.skill_locks.is_empty());

            // The old MCP is no longer materialized. The final authorized
            // request removes it and the old Skill; it never compiles that
            // obsolete selection before compiling the user's explicit intent.
            let mut catalog = (*control.catalog.snapshot().unwrap()).clone();
            catalog.capabilities.retain(|capability| !capability.manifest.id.as_ref().starts_with("nomi.mcp.v1."));
            catalog.formal_capability_entries.retain(|reference, _| !reference.id.as_ref().starts_with("nomi.mcp.v1."));
            catalog.mcp_tools.clear();
            if let Some(changed_effect) = changed_authority {
                let browser = catalog.capabilities.iter_mut().find(|capability| capability.manifest.id.as_ref() == "browser").unwrap();
                if changed_effect {
                    browser.manifest.contributions.actions[0].effect_class = EffectClass::WriteReversible;
                } else {
                    browser.manifest.contributions.resource_kinds.insert("unrequested-resource".into());
                }
                let entry = republish_catalog_capability(browser);
                catalog.formal_capability_entries.insert(entry.capability.clone(), entry);
            }
            let mut registry = MaterializedRegistry::empty();
            for capability in &catalog.capabilities { registry.capabilities.insert(capability.manifest.id.clone(), capability.clone()); }
            for skill in &catalog.skills { registry.skills.insert(skill.definition.id.clone(), skill.clone()); }
            control.catalog = Arc::new(StaticCatalogProvider::new(catalog));
            control.compiler = test_compiler().with_materialized_registry(Arc::new(registry), CompilerEnvironment {
                resolver_version: "1.0.0".into(), required_runtime_protocol_version: "1.0.0".into(),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: "runtime-features".into(), available_runtime_features: BTreeSet::new(),
                installation_role_bindings: BTreeMap::new(), canonical_schema_manifest_digest: "schema".into(),
                target_contribution_manifest_digest: "contributions".into(), host_target: "test".into(),
                host_surface: "desktop".into(), availability_evidence_revision: "fixture".into(),
            });
            control.session_capabilities_resolver = Some(Arc::new(EmptyExtensions));
            let original_count = store.list_presets(&owner).await.unwrap().len();
            let result = control.resolve_agent_session_capabilities_binding(&owner, &current,
                Some(&nomifun_api_types::SessionCapabilitySelectionDto { skill_names: Vec::new(), mcp_server_ids: Vec::new() })).await;
            if changed_authority.is_some() {
                assert_eq!(result.unwrap_err().code().as_ref(), "AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED");
                assert_eq!(store.list_presets(&owner).await.unwrap().len(), original_count,
                    "extension intent must not persist an unproven core contract");
            } else {
                let selected = result.unwrap();
                let (_, revision, snapshot) = control.saved_binding_artifacts(&owner, &selected).await.unwrap();
                assert!(snapshot.content.mcp_tool_locks.is_empty());
                assert!(snapshot.content.skill_locks.is_empty());
                assert_eq!(revision.payload.instructions, source_revision.payload.instructions);
                assert_eq!(revision.payload.model_route_refs, source_revision.payload.model_route_refs);
                assert_eq!(snapshot.content.enabled_capabilities,
                    source_snapshot.content.enabled_capabilities.iter()
                        .filter(|capability| !capability.capability.id.as_ref().starts_with("nomi.mcp.v1."))
                        .cloned().collect::<Vec<_>>());
            }
            assert_eq!(store.get_revision(&source_revision.reference).await.unwrap().unwrap(), source_revision);
            assert_eq!(store.get_snapshot(&source_revision.reference).await.unwrap().unwrap(), source_snapshot);
        }
    }

    pub(super) fn template_control_plane(store: Arc<InMemoryControlPlaneStore>, key: OfficialPresetKey, updated_schema: bool) -> AgentControlPlane {
        template_control_plane_with_order_contributions(store, key, updated_schema, false)
    }

    fn template_control_plane_with_order_contributions(
        store: Arc<InMemoryControlPlaneStore>, key: OfficialPresetKey,
        updated_schema: bool, order_contributions: bool,
    ) -> AgentControlPlane {
        template_control_plane_with_extra_capabilities(store, key, updated_schema, order_contributions, &[])
    }

    fn template_control_plane_with_extra_capabilities(
        store: Arc<InMemoryControlPlaneStore>, key: OfficialPresetKey,
        updated_schema: bool, order_contributions: bool, extras: &[CapabilitySelection],
    ) -> AgentControlPlane {
        let templates = OfficialTemplateCatalog::load().unwrap();
        let mut registry = MaterializedRegistry::empty();
        let mut catalog = CatalogSnapshot::default();
        let mut available = templates.seed(key).unwrap().enabled_capabilities.iter()
            .map(|selection| (selection.capability.id.clone(), selection.clone())).collect::<BTreeMap<_, _>>();
        for extra in extras {
            available.entry(extra.capability.id.clone()).and_modify(|selection| selection.action_allowlist.extend(extra.action_allowlist.iter().cloned()))
                .or_insert_with(|| extra.clone());
        }
        for selected in available.values() {
            let schema = if matches!(selected.capability.id.as_ref(), "creation.media" | "knowledge" | "companion.memory") && updated_schema {
                json!({"type":"object", "additionalProperties":false, "properties":{"model_selection":{"type":"object"}}})
            } else { json!({"type":"object", "additionalProperties":false}) };
            let (mut capability, mut entry) = catalog_capability_with_actions_schema(
                selected.capability.id.as_ref(),
                [CapabilityConsumer::Agent],
                selected.action_allowlist.iter().cloned(),
                schema,
            );
            if order_contributions && selected.capability.id.as_ref() == "project.memory" {
                capability.manifest.contributions.context_schema_refs = vec!["schema://fixture/project-memory/context@1".into()];
                entry = republish_catalog_capability(&mut capability);
            }
            registry.capabilities.insert(selected.capability.id.clone(), capability.clone());
            catalog.capabilities.push(capability);
            catalog.formal_capability_entries.insert(entry.capability.clone(), entry);
        }
        if order_contributions {
            let (mut middleware, _) = catalog_capability(ORDER_TEST_MIDDLEWARE_ID,
                [CapabilityConsumer::Agent, CapabilityConsumer::PluginService]);
            middleware.manifest.kind = CapabilityKind::TurnMiddleware;
            middleware.manifest.supported_surfaces = nomifun_agent_contracts::capability_module_surface_declarations(
                ["desktop"], [CapabilityConsumer::Agent, CapabilityConsumer::PluginService],
                nomifun_agent_contracts::CapabilityAuthoringPolicy::Direct,
            );
            middleware.manifest.contributions.actions = vec![nomifun_agent_contracts::tool_middleware::before_action()];
            let entry = republish_catalog_capability(&mut middleware);
            registry.capabilities.insert(middleware.manifest.id.clone(), middleware.clone());
            catalog.capabilities.push(middleware);
            catalog.formal_capability_entries.insert(entry.capability.clone(), entry);
        }
        // Keep the fixture complete when an official seed binds bundled Skills.
        for reference in &templates.seed(key).unwrap().skill_bindings {
            let parent = registry.capabilities.values().next().unwrap();
            let definition = SkillDefinition {
                id: reference.id.clone(), version: reference.version.clone(),
                package: parent.manifest.package.clone(),
                display: LocalizedMetadata {
                    name: reference.id.as_ref().to_owned(), description: "Official Skill fixture".into(),
                    localized_names: BTreeMap::new(), localized_descriptions: BTreeMap::new(),
                },
                body_ref: LogicalArtifactRef {
                    artifact_id: format!("{}.body", reference.id.as_ref()).into(),
                    normalized_relative_path: format!("skills/{}/SKILL.md", reference.id.as_ref()),
                    digest: DigestHex::from("b".repeat(64)),
                },
                resources: Vec::new(), requires_capabilities: Vec::new(),
                supported_surfaces: BTreeSet::from(["desktop".to_owned()]),
            };
            let contract_digest = digest_payload(&definition).unwrap();
            let contribution_id = ContributionId::from(format!("skill:{}", reference.id.as_ref()));
            let skill = MaterializedSkill {
                definition, contribution_id: contribution_id.clone(), contract_digest: contract_digest.clone(),
                contribution_lock: ContributionLock {
                    contribution_id, contract_digest, ..parent.contribution_lock.clone()
                },
                target_artifact_digest: parent.target_artifact_digest.clone(),
                mount_id: parent.mount_id.clone(), source: parent.source.clone(),
            };
            registry.skills.insert(reference.id.clone(), skill.clone());
            catalog.skills.push(skill);
        }
        let compiler = test_compiler().with_materialized_registry(Arc::new(registry), CompilerEnvironment {
            resolver_version: VersionString::from("1.0.0"), required_runtime_protocol_version: VersionString::from("1.0.0"),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: DigestHex::from("runtime-features"), available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(), canonical_schema_manifest_digest: DigestHex::from("schema"),
            target_contribution_manifest_digest: DigestHex::from("contributions"), host_target: RuntimeTarget::from("test"),
            host_surface: "desktop".into(), availability_evidence_revision: "fixture".into(),
        });
        AgentControlPlane::new(store, Arc::new(StaticCatalogProvider::new(catalog)), templates, compiler)
    }

    #[tokio::test]
    async fn official_reuse_recompiles_changed_contracts_and_preserves_historical_artifacts() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let old_control = general_template_control_plane(store.clone(), false);
        let old = old_control.create_from_template(&owner, "assistant.general", official_launch_request(true)).await.unwrap();
        let old_ref: PresetRevisionRef = wire_cast(old.preset.current_stable_revision.as_ref().unwrap()).unwrap();
        let old_revision = store.get_revision(&old_ref).await.unwrap().unwrap();
        let old_snapshot = store.get_snapshot(&old_ref).await.unwrap().unwrap();
        let explicit = old_control.create_from_template(&owner, "assistant.general", official_launch_request(false)).await.unwrap();
        let control = general_template_control_plane(store.clone(), true);
        let refreshed = control.create_from_template(&owner, "assistant.general", official_launch_request(true)).await.unwrap();
        let new_ref: PresetRevisionRef = wire_cast(refreshed.preset.current_stable_revision.as_ref().unwrap()).unwrap();
        assert_eq!(new_ref.preset_id, old_ref.preset_id);
        assert_eq!(new_ref.revision, old_ref.revision + 1);
        let new_revision = store.get_revision(&new_ref).await.unwrap().unwrap();
        let new_snapshot = store.get_snapshot(&new_ref).await.unwrap().unwrap();
        assert_eq!(new_revision.payload, old_revision.payload, "only runtime contracts changed");
        assert_ne!(new_revision.contribution_locks, old_revision.contribution_locks);
        assert_ne!(new_snapshot.snapshot_ref, old_snapshot.snapshot_ref);
        assert_eq!(store.get_revision(&old_ref).await.unwrap().unwrap(), old_revision);
        assert_eq!(store.get_snapshot(&old_ref).await.unwrap().unwrap(), old_snapshot);
        let stable = control.create_from_template(&owner, "assistant.general", official_launch_request(true)).await.unwrap();
        assert_eq!(stable.preset.current_stable_revision, refreshed.preset.current_stable_revision, "current contracts must not create redundant revisions");
        let personal = control.editor(&owner, &explicit.preset.preset_id, None).await.unwrap();
        assert_eq!(personal.preset.current_stable_revision, explicit.preset.current_stable_revision, "explicit personal copies retain their chosen immutable revision");
        let configurations = store.list_presets(&owner).await.unwrap();
        assert_eq!(configurations.iter().filter(|preset| preset.session_only).count(), 1);
        assert_eq!(configurations.iter().filter(|preset| !preset.session_only).count(), 2);
    }

    #[tokio::test]
    async fn session_binding_refreshes_internal_cache_without_template_identity_or_payload_changes() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let old_control = general_template_control_plane(store.clone(), false);
        let internal = old_control.create_from_template(&owner, "assistant.general", official_launch_request(true)).await.unwrap();
        // An internal model variant may have user-specific content. Refresh
        // exactly that payload; never infer or apply an official template.
        let mut draft = internal.draft;
        draft.document.instructions = "Keep this exact user-selected behavior".into();
        let saved = old_control.save_revision(&owner, &internal.preset.preset_id, SaveAgentPresetRevisionRequest {
            expected_current_revision: draft.current_revision.clone(), draft, reason: None,
        }).await.unwrap();
        let old_ref: PresetRevisionRef = wire_cast(&saved.revision.reference).unwrap();
        let old_revision = store.get_revision(&old_ref).await.unwrap().unwrap();
        let old_snapshot = store.get_snapshot(&old_ref).await.unwrap().unwrap();
        let personal = old_control.create_from_template(&owner, "assistant.general", official_launch_request(false)).await.unwrap();
        let control = general_template_control_plane(store.clone(), true);
        let binding = tokio::time::timeout(std::time::Duration::from_secs(5),
            control.resolve_agent_session_binding_with_model(&owner, &internal.preset.preset_id, None)).await.unwrap().unwrap();
        let new_ref: PresetRevisionRef = wire_cast(&binding.preset_revision_ref).unwrap();
        assert_eq!(new_ref.preset_id, old_ref.preset_id);
        assert_eq!(new_ref.revision, old_ref.revision + 1);
        let refreshed = store.get_revision(&new_ref).await.unwrap().unwrap();
        let snapshot = store.get_snapshot(&new_ref).await.unwrap().unwrap();
        assert_eq!(refreshed.payload, old_revision.payload);
        assert_eq!(snapshot.content.required_runtime_profile, old_snapshot.content.required_runtime_profile);
        assert_ne!(refreshed.contribution_locks, old_revision.contribution_locks);
        assert_eq!(store.get_revision(&old_ref).await.unwrap().unwrap(), old_revision);
        assert_eq!(store.get_snapshot(&old_ref).await.unwrap().unwrap(), old_snapshot);
        let again = control.resolve_agent_session_binding(&owner, &internal.preset.preset_id).await.unwrap();
        assert_eq!(again.preset_revision_ref, binding.preset_revision_ref);
        let untouched = control.resolve_agent_session_binding(&owner, &personal.preset.preset_id).await.unwrap();
        assert_eq!(Some(untouched.preset_revision_ref), personal.preset.current_stable_revision);
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
                    model: None,
                    reuse_existing: false,
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
        let saved_binding_dto: AgentBindingValueDto = wire_cast(&binding_value).unwrap();
        store
            .insert_remote_binding(RemoteBinding {
                remote_binding_id: RemoteBindingId::from("remote-retirement"),
                owner_user_id: owner.clone(),
                name: "Remote retirement".into(),
                agent_binding: binding_value,
            })
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
        assert_eq!(control_plane.saved_binding_presentation(&owner, &saved_binding_dto).await.unwrap(),
            ("Retire me".to_owned(), None));
        assert!(control_plane.saved_binding_presentation(&other_owner, &saved_binding_dto).await.is_err(),
            "retired metadata must not escape the exact bound owner");
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
    async fn replaced_app_grants_retire_through_new_revisions_preserving_frozen_skills_history_and_unrelated_configuration() {
        fn control(store: Arc<InMemoryControlPlaneStore>, old: bool) -> AgentControlPlane {
            let mut registry = MaterializedRegistry::empty();
            let mut catalog = CatalogSnapshot::default();
            let modules = if old { vec!["plugin.development", "kept.module", "missing.other-module"] } else { vec!["kept.module"] };
            for id in modules {
                let (capability, entry) = catalog_capability_with_actions_schema(id, [CapabilityConsumer::Agent],
                    [ActionId::from(format!("{id}/read"))], json!({"type":"object","additionalProperties":false}));
                registry.capabilities.insert(id.into(), capability.clone());
                catalog.capabilities.push(capability);
                catalog.formal_capability_entries.insert(entry.capability.clone(), entry);
            }
            let compiler = test_compiler().with_materialized_registry(Arc::new(registry), CompilerEnvironment {
                resolver_version:"1.0.0".into(), required_runtime_protocol_version:"1.0.0".into(),
                required_runtime_profile:RuntimeProfileKind::ManagedMinimal, runtime_feature_inventory_digest:"runtime-features".into(),
                available_runtime_features:BTreeSet::new(), installation_role_bindings:BTreeMap::new(), canonical_schema_manifest_digest:"schema".into(),
                target_contribution_manifest_digest:"contributions".into(),host_target:"test".into(),host_surface:"desktop".into(),availability_evidence_revision:"fixture".into(),
            });
            AgentControlPlane::new(store,Arc::new(StaticCatalogProvider::new(catalog)),OfficialTemplateCatalog::load().unwrap(),compiler)
        }
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let owner: UserId = "0190f5fe-7c00-7a00-8000-000000000001".into();
        let old = control(store.clone(), true);
        let mut payload = document_payload(&empty_document()).unwrap();
        payload.instructions = "Preserve my ordinary application-independent instructions".into();
        payload.enabled_capabilities = ["plugin.development", "kept.module"].into_iter().map(|id| CapabilitySelection {
            capability:CapabilityRef {id:id.into()},action_allowlist:BTreeSet::from([format!("{id}/read").into()]),
        }).collect();
        let skill = nomifun_agent_contracts::FrozenLibrarySkill::new("frozen-skill".into(),"Frozen fixture".into(),
            nomifun_agent_contracts::LibrarySkillSource::Custom,"Do not lose these immutable captured bytes".into(),BTreeMap::new()).unwrap();
        payload.skill_bindings.push(nomifun_agent_contracts::AgentSkillBinding::library_selected(skill,true));
        let good = old.create_configuration_with_payload(&owner,Uuid::now_v7().to_string().into(),"Personal Agent".into(),None,payload.clone(),None,true,None).await.unwrap();
        payload.enabled_capabilities.push(CapabilitySelection {capability:CapabilityRef {id:"missing.other-module".into()},action_allowlist:BTreeSet::from(["missing.other-module/read".into()])});
        let unavailable = old.create_configuration_with_payload(&owner,Uuid::now_v7().to_string().into(),"Needs correction".into(),None,payload,None,true,None).await.unwrap();
        let good_ref: PresetRevisionRef = wire_cast(good.preset.current_stable_revision.as_ref().unwrap()).unwrap();
        let bad_ref: PresetRevisionRef = wire_cast(unavailable.preset.current_stable_revision.as_ref().unwrap()).unwrap();
        let previous_revision = store.get_revision(&good_ref).await.unwrap().unwrap();
        let previous_snapshot = store.get_snapshot(&good_ref).await.unwrap().unwrap();
        let current = control(store.clone(), false);
        assert_eq!(current.retire_replaced_app_plugin_configuration(&owner).await.unwrap(),1,
            "one unavailable Agent must not stop valid Agents from retiring replaced grants");
        let good_stored = store.get_preset(&good_ref.preset_id).await.unwrap().unwrap();
        let new_ref = good_stored.preset.current_stable_revision.unwrap();
        assert_eq!(new_ref.revision,good_ref.revision+1);
        let revision = store.get_revision(&new_ref).await.unwrap().unwrap();
        assert_eq!(revision.payload.instructions, previous_revision.payload.instructions);
        assert_eq!(revision.payload.skill_bindings, previous_revision.payload.skill_bindings);
        assert_eq!(revision.payload.enabled_capabilities.iter().map(|item|item.capability.id.as_ref()).collect::<Vec<_>>(),vec!["kept.module"]);
        assert_eq!(store.get_revision(&good_ref).await.unwrap().unwrap(),previous_revision);
        assert_eq!(store.get_snapshot(&good_ref).await.unwrap().unwrap(),previous_snapshot);
        assert_eq!(store.get_preset(&bad_ref.preset_id).await.unwrap().unwrap().preset.current_stable_revision,Some(bad_ref));
        assert_eq!(current.retire_replaced_app_plugin_configuration(&owner).await.unwrap(),0);
    }

    #[tokio::test]
    async fn session_binding_is_owner_scoped_and_freezes_current_stable_snapshot() {
        let store = Arc::new(InMemoryControlPlaneStore::new());
        let control_plane = test_control_plane(store.clone());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let other_owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let created = control_plane
            .create_from_template(
                &owner,
                "chat.minimal",
                CreateAgentPresetFromTemplateRequest {
                    model: None,
                    reuse_existing: false,
                    display_name: "Minimal".into(),
                    description: None,
                    model_route_refs: BTreeMap::new(),
                    chat_route_records: BTreeMap::new(),
                },
            )
            .await
            .unwrap();
        let revision = created.revision.as_ref().expect("initial Revision");
        let missing = control_plane.editor(&owner, &created.preset.preset_id, Some(2))
            .await.expect_err("an explicit missing Revision must not open a blank draft");
        assert_eq!(missing.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(missing.code().as_ref(), "PRESET_REVISION_DIGEST_MISMATCH");

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
        next_draft.display_name = "Renamed without a new Revision".into();
        next_draft.description = Some("Updated metadata".into());
        let renamed = control_plane.save_revision(&owner, &created.preset.preset_id, SaveAgentPresetRevisionRequest {
            expected_current_revision: Some(revision.reference.clone()),
            draft: next_draft.clone(), reason: None,
        }).await.unwrap();
        assert_eq!(renamed.revision.reference, revision.reference);
        assert_eq!(renamed.preset.display_name, next_draft.display_name);
        let reloaded = control_plane.editor(&owner, &created.preset.preset_id, None).await.unwrap();
        assert_eq!(reloaded.draft.display_name, next_draft.display_name);
        assert_eq!(reloaded.draft.description, next_draft.description);
        next_draft.document.instructions = "Revision two".into();
        let saved = control_plane
            .save_revision(
                &owner,
                &created.preset.preset_id,
                SaveAgentPresetRevisionRequest {
                    expected_current_revision: Some(revision.reference.clone()),
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
        let control_plane = test_control_plane(store.clone());
        let owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000001");
        let other_owner = UserId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let binding_id = RemoteBindingId::from("remote-binding-1");
        store
            .insert_preset(StoredPreset {
                session_only: false,
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
        let control_plane = test_control_plane(store.clone())
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
                    model: None,
                    reuse_existing: false,
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
