//! Pure projection from the canonical Agent Capability document to Nomi-core.
//!
//! This module deliberately has no HTTP, database, runtime, or factory
//! dependency.  It validates the immutable binding/revision/snapshot chain and
//! produces only the fields that the current Nomi conversation creator already
//! understands.  It must not be used as an authority to persist a binding or
//! to start a Remote lifecycle.

use std::collections::BTreeSet;

use nomifun_agent_contracts::{
    AgentBindingValue, AgentPresetRevision, ChatRouteIdentity, ChatRouteRecord,
    ResolvedSnapshotEnvelope,
};
use nomifun_api_types::{
    AgentKnowledgePolicy, AgentResolvedSnapshot, CreateConversationRequest, ExecutionModelRef,
};
use nomifun_ai_agent::NomiPluginToolSession;
use nomifun_common::{AgentType, AppError, ProviderWithModel, UserId};
use serde_json::json;

const CHAT_TASK: &str = nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT;
/// Inputs to [`project`].  All four values must come from the same
/// authenticated, persisted control-plane read.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // Reserved for the future fully typed Nomi projection path.
pub struct ProjectionInput<'a> {
    pub owner: &'a UserId,
    pub binding: &'a AgentBindingValue,
    pub revision: &'a AgentPresetRevision,
    pub snapshot: &'a ResolvedSnapshotEnvelope,
    pub title: Option<&'a str>,
}

/// The two Nomi-core values needed by a later adapter.
#[derive(Debug)]
#[allow(dead_code)] // Kept as the source-neutral projection seam, not a second authority.
pub struct NomiCoreAgentProjection {
    pub snapshot: AgentResolvedSnapshot,
    pub request: CreateConversationRequest,
}

#[derive(Debug)]
pub struct NomiCoreSavedBindingProjection {
    pub binding: AgentBindingValue,
    pub revision: AgentPresetRevision,
    pub snapshot: ResolvedSnapshotEnvelope,
    pub projection: NomiCoreAgentProjection,
}

/// Project one exact persisted Binding/Revision/Snapshot chain.
///
/// Product retirement may hide the mutable Agent entry, but it cannot rewrite
/// or invalidate immutable artifacts already frozen into a Session.
pub fn project_saved_artifacts(
    owner: &UserId,
    binding: AgentBindingValue,
    revision: AgentPresetRevision,
    snapshot: ResolvedSnapshotEnvelope,
    title: Option<&str>,
) -> Result<NomiCoreSavedBindingProjection, AppError> {
    let projection = project(ProjectionInput {
        owner,
        binding: &binding,
        revision: &revision,
        snapshot: &snapshot,
        title,
    })?;
    Ok(NomiCoreSavedBindingProjection {
        binding,
        revision,
        snapshot,
        projection,
    })
}

/// Project one exact saved binding with a host-materialized Plugin Tool set.
///
/// The dynamic set is already bound to the same immutable Snapshot and shared
/// Kernel. This function only merges its stable provider names into Nomi's
/// presentation policy; it never resolves a Catalog or accepts action schema
/// from a request.
#[allow(dead_code)] // Activated when the app composition installs the session provider.
pub fn project_saved_artifacts_with_plugin_tools(
    owner: &UserId,
    binding: AgentBindingValue,
    revision: AgentPresetRevision,
    snapshot: ResolvedSnapshotEnvelope,
    title: Option<&str>,
    plugin_tools: &NomiPluginToolSession,
) -> Result<NomiCoreSavedBindingProjection, AppError> {
    let projection = project_with_plugin_tools(
        ProjectionInput {
            owner,
            binding: &binding,
            revision: &revision,
            snapshot: &snapshot,
            title,
        },
        plugin_tools,
    )?;
    Ok(NomiCoreSavedBindingProjection {
        binding,
        revision,
        snapshot,
        projection,
    })
}

/// Validate the canonical identity chain and project a saved Agent Preset to
/// the current Nomi conversation request.
///
/// The returned `extra` contains only request/build inputs recognized by the
/// existing Nomi path (`system_prompt`, `allowed_tools`, and
/// `deferred_tools`). Concrete resources remain owned by the target Session,
/// companion, workpath, or automation binding.
#[allow(dead_code)] // The typed seam is retained for a future native Nomi adapter.
pub fn project(input: ProjectionInput<'_>) -> Result<NomiCoreAgentProjection, AppError> {
    project_internal(input, None)
}

#[allow(dead_code)] // Activated when the app composition installs the session provider.
pub fn project_with_plugin_tools(
    input: ProjectionInput<'_>,
    plugin_tools: &NomiPluginToolSession,
) -> Result<NomiCoreAgentProjection, AppError> {
    if plugin_tools.resolved_snapshot_ref() != &input.snapshot.snapshot_ref {
        return Err(AppError::Conflict(
            "Nomi Plugin Tool set is bound to a different resolved Snapshot"
                .to_owned(),
        ));
    }
    project_internal(input, Some(plugin_tools))
}

fn project_internal(
    input: ProjectionInput<'_>,
    plugin_tools: Option<&NomiPluginToolSession>,
) -> Result<NomiCoreAgentProjection, AppError> {
    validate_identity_chain(&input)?;
    let route = exact_chat_route(input.revision, input.snapshot)?;
    let capability_tools = project_capabilities_with_dynamic(
        input.revision,
        |capability_id, deferred| {
            plugin_tools
                .map(|tools| {
                    tools.provider_names_for(capability_id, deferred)
                })
                .unwrap_or_default()
        },
    )?;
    project_revision_parts(
        input.revision,
        input.title,
        route,
        capability_tools,
        input
            .snapshot
            .content
            .required_resource_kinds
            .iter()
            .map(|kind| kind.as_ref().to_owned())
            .collect(),
        input
            .snapshot
            .content
            .skill_locks
            .iter()
            .map(|lock| lock.skill.id.as_ref().to_owned())
            .collect(),
    )
}

fn project_revision_parts(
    revision_document: &AgentPresetRevision,
    title_input: Option<&str>,
    route: ChatRouteRecord,
    capability_tools: ProjectedCapabilityTools,
    required_resource_kinds: BTreeSet<String>,
    included_skills: Vec<String>,
) -> Result<NomiCoreAgentProjection, AppError> {
    let instructions = merge_instructions(
        &revision_document.payload.persona,
        &revision_document.payload.instructions,
    );
    let title = title_input
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(revision_document.reference.preset_id.as_ref())
        .to_owned();
    let revision = i64::try_from(revision_document.reference.revision).map_err(|_| {
        AppError::UnprocessableEntity("preset revision does not fit Nomi's integer field".into())
    })?;

    let knowledge_enabled = capability_tools
        .all_capability_ids()
        .any(|id| matches!(id, "knowledge.search" | "knowledge.read" | "knowledge.write"));
    let knowledge_policy = AgentKnowledgePolicy {
        enabled: knowledge_enabled,
        writeback: capability_tools
            .all_capability_ids()
            .any(|id| id == "knowledge.write"),
        eagerness: None,
        grounded: knowledge_enabled,
    };
    let resolved_model = ExecutionModelRef {
        provider_id: route.primary.provider_id.clone(),
        model: route.primary.model.clone(),
    };
    let projected_snapshot = AgentResolvedSnapshot {
        preset_id: revision_document.reference.preset_id.as_ref().to_owned(),
        preset_revision: revision,
        preset_name: title.clone(),
        routing_description: None,
        instructions: instructions.clone(),
        resolved_agent_id: None,
        resolved_agent_type: Some(AgentType::Nomi.serde_name().to_owned()),
        resolved_agent_backend: Some("nomi".to_owned()),
        resolved_model: Some(resolved_model.clone()),
        included_skills,
        excluded_auto_skills: Vec::new(),
        initial_capabilities: capability_tools.initial_capability_ids.clone(),
        on_demand_capabilities: capability_tools.on_demand_capability_ids.clone(),
        required_resource_kinds,
        knowledge_policy,
        warnings: Vec::new(),
    };

    let extra = json!({
        "system_prompt": instructions,
        "allowed_tools": capability_tools.allowed_tools,
        "enforce_tool_allowlist": true,
        "deferred_tools": capability_tools.deferred_tools,
        "browser_use": capability_tools.browser_use,
        "computer_use": capability_tools.computer_use,
    });

    Ok(NomiCoreAgentProjection {
        snapshot: projected_snapshot,
        request: CreateConversationRequest {
            r#type: AgentType::Nomi,
            name: Some(title),
            model: Some(ProviderWithModel {
                provider_id: route.primary.provider_id,
                model: route.primary.model,
                use_model: None,
            }),
            source: None,
            channel_chat_id: None,
            preset_id: None,
            delegation_policy: Default::default(),
            execution_model_pool: None,
            decision_policy: Default::default(),
            execution_template_id: None,
            extra,
        },
    })
}

#[allow(dead_code)] // Used by the source-neutral projection seam above.
fn validate_identity_chain(input: &ProjectionInput<'_>) -> Result<(), AppError> {
    if input.revision.created_by.as_ref() != input.owner.as_ref() {
        return Err(AppError::Forbidden(
            "saved preset revision is owned by a different authenticated owner".into(),
        ));
    }
    if input.snapshot.actor.principal_kind != "user"
        || input.snapshot.actor.principal_id != input.owner.as_ref()
    {
        return Err(AppError::Forbidden(
            "resolved snapshot actor does not match the authenticated owner".into(),
        ));
    }
    input.revision.validate().map_err(contract_error)?;
    input.snapshot.validate().map_err(contract_error)?;
    if input.binding.preset_revision_ref != input.revision.reference {
        return Err(AppError::Conflict(
            "AgentBindingValue preset revision reference does not exactly match the saved revision"
                .into(),
        ));
    }
    if input.snapshot.content.preset_revision_ref != input.revision.reference {
        return Err(AppError::Conflict(
            "resolved snapshot preset revision reference does not exactly match the saved revision"
                .into(),
        ));
    }
    if input.binding.resolved_snapshot_ref != input.snapshot.snapshot_ref {
        return Err(AppError::Conflict(
            "AgentBindingValue snapshot reference does not exactly match the saved snapshot".into(),
        ));
    }
    Ok(())
}

fn contract_error(error: nomifun_agent_contracts::PresetContractViolation) -> AppError {
    AppError::UnprocessableEntity(format!("{}: {}", error.code.as_ref(), error.message))
}

#[allow(dead_code)] // Used only by the source-neutral projection seam.
fn exact_chat_route(
    revision: &AgentPresetRevision,
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<ChatRouteRecord, AppError> {
    let route_id = revision
        .payload
        .model_route_refs
        .get(CHAT_TASK)
        .ok_or_else(|| unsupported("model route", "agent_chat route is required"))?;
    let record = revision
        .payload
        .chat_route_records
        .get(CHAT_TASK)
        .ok_or_else(|| unsupported("model route", "agent_chat canonical route record is required"))?;
    let identity = ChatRouteIdentity::new(
        revision.reference.revision_id(),
        CHAT_TASK,
        route_id.clone(),
        record.primary.model_route_revision,
    );
    record
        .validate_for(&identity)
        .map_err(|error| unsupported("model route", error.to_string()))?;
    if snapshot.content.chat_route_identity.as_ref() != Some(&identity)
        || snapshot.content.model_route_refs.get(CHAT_TASK) != Some(route_id)
    {
        return Err(AppError::Conflict(
            "snapshot chat route identity does not exactly match the revision chat route".into(),
        ));
    }
    Ok(record.clone())
}

#[derive(Debug, Default)]
struct ProjectedCapabilityTools {
    initial_capability_ids: Vec<String>,
    on_demand_capability_ids: Vec<String>,
    allowed_tools: Vec<String>,
    deferred_tools: Vec<String>,
    browser_use: bool,
    computer_use: bool,
}

impl ProjectedCapabilityTools {
    fn all_capability_ids(&self) -> impl Iterator<Item = &str> {
        self.initial_capability_ids
            .iter()
            .chain(&self.on_demand_capability_ids)
            .map(String::as_str)
    }
}

/// Built-in Nomi adapter projection.
///
/// This table owns only native/host projections. Ordinary Plugin Tool actions
/// are supplied from an exact Snapshot-bound dynamic set and must not be added
/// here as capability-specific compatibility entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NomiCapabilityProjection {
    Tools(&'static [&'static str]),
    BrowserTools,
    ComputerTools,
    HostOnly {
        browser: bool,
        computer: bool,
    },
}

/// Return the Nomi-core projection for one canonical capability identity.
///
/// The app-local control-plane currently uses this as its native fallback
/// check. It is not an availability authority for dynamically materialized
/// Plugin actions.
pub(crate) fn nomi_capability_projection(
    capability_id: &str,
) -> Result<NomiCapabilityProjection, AppError> {
    let projection = match capability_id {
        // Native filesystem family.
        "fs.read" => NomiCapabilityProjection::Tools(&["Read"]),
        "fs.search" => NomiCapabilityProjection::Tools(&["Grep", "Glob"]),
        "fs.write" => NomiCapabilityProjection::Tools(&["Write"]),
        "fs.patch" => NomiCapabilityProjection::Tools(&["Edit", "ApplyPatch"]),

        // Native process and VCS families.
        "process.exec" => {
            NomiCapabilityProjection::Tools(&["Bash", "exec_command", "write_stdin"])
        }
        "vcs.status" => NomiCapabilityProjection::Tools(&["vcs.status"]),
        "vcs.diff" => NomiCapabilityProjection::Tools(&["vcs.diff"]),
        "vcs.stage" => NomiCapabilityProjection::Tools(&["vcs.stage"]),
        "vcs.commit" => NomiCapabilityProjection::Tools(&["vcs.commit"]),

        // The plan checklist is always registered by the Nomi bootstrap. Its
        // deferred placement is applied by the host registry when requested.
        "agent.execution.plan" => NomiCapabilityProjection::Tools(&["update_plan"]),
        "agent.delegate" => NomiCapabilityProjection::Tools(&["nomi_delegate"]),

        // Bundled native adapters reuse the application's existing HTTP and
        // owner-scoped Cron services. No dynamic Plugin runtime is required.
        "web.fetch" => NomiCapabilityProjection::Tools(&[
            nomifun_ai_agent::web_fetch::WEB_FETCH_TOOL_NAME,
        ]),
        "schedule.store" => NomiCapabilityProjection::Tools(&[
            "cron_create", "cron_list", "cron_delete",
        ]),

        // Knowledge and Skill tools are registered by the manager after the
        // target-scoped resource/sink wiring has been resolved.
        "knowledge.search" => NomiCapabilityProjection::Tools(&["knowledge_search"]),
        "knowledge.read" => NomiCapabilityProjection::Tools(&["knowledge_read"]),
        "knowledge.write" => NomiCapabilityProjection::Tools(&["knowledge_write"]),
        "skill.invoke" => NomiCapabilityProjection::Tools(&["Skill"]),

        // Project/session declarations are consumed outside the model tool
        // list. They do not freeze concrete resource identities.
        "chat.basic"
        | "chat.minimal"
        | "session.attachments.read"
        | "memory.project.read"
        | "memory.project.citation"
        | "memory.session.scratch"
        | "process.session"
        | "terminal.pty"
        | "workspace.bind"
        | "workspace.artifacts"
        | "skill.catalog"
        | "skill.describe"
        | "skill.hooks" => NomiCapabilityProjection::HostOnly {
            browser: false,
            computer: false,
        },

        // `remember` is the native project-memory write owner. Distillation
        // remains a host-side output projection, not a second model tool.
        "memory.project.write" => NomiCapabilityProjection::Tools(&["remember"]),
        "memory.project.distill" => NomiCapabilityProjection::HostOnly {
            browser: false,
            computer: false,
        },

        // Browser is one native tool with an action discriminator. The
        // compile-time feature is part of the host availability contract.
        "browser.identity" => {
            if cfg!(feature = "browser-use") {
                NomiCapabilityProjection::HostOnly {
                    browser: true,
                    computer: false,
                }
            } else {
                return Err(unsupported(
                    "capability",
                    "browser.identity has no Browser owner in this build",
                ));
            }
        }
        "browser.observe"
        | "browser.navigate"
        | "browser.act"
        | "browser.download"
        | "browser.upload"
        | "browser.evaluate"
        | "browser.takeover" => {
            if cfg!(feature = "browser-use") {
                NomiCapabilityProjection::BrowserTools
            } else {
                return Err(unsupported(
                    "capability",
                    format!("{capability_id:?} has no Browser owner in this build"),
                ));
            }
        }

        // Computer is one native tool with an action discriminator.
        "a11y.observe" | "computer.observe" | "computer.input" | "computer.launch" => {
            if cfg!(feature = "computer-use") {
                NomiCapabilityProjection::ComputerTools
            } else {
                return Err(unsupported(
                    "capability",
                    format!("{capability_id:?} has no Computer owner in this build"),
                ));
            }
        }

        other => {
            return Err(unsupported(
                "capability",
                format!("{other:?} has no Nomi projection on this host"),
            ));
        }
    };
    Ok(projection)
}

/// Validate all capability selections against the concrete Nomi projection.
///
/// The control-plane store calls this as a defense-in-depth check. Preview
/// should reject the same revision earlier through the host catalog, but a
/// persisted revision must never bypass the runtime projection boundary.
pub(crate) fn validate_nomi_capability_projection(
    revision: &AgentPresetRevision,
) -> Result<(), AppError> {
    revision
        .payload
        .initial_capabilities
        .iter()
        .try_for_each(|selection| {
            validate_native_capability_if_declared(
                selection.capability.id.as_ref(),
            )
        })?;
    revision
        .payload
        .on_demand_capabilities
        .iter()
        .try_for_each(|selection| {
            validate_native_capability_if_declared(
                selection.capability.id.as_ref(),
            )
        })?;
    Ok(())
}

fn validate_native_capability_if_declared(
    capability_id: &str,
) -> Result<(), AppError> {
    if is_native_nomi_capability(capability_id) {
        nomi_capability_projection(capability_id).map(|_| ())
    } else {
        // Dynamic Plugin capabilities are validated by the canonical Catalog,
        // Compiler and exact session provider. Absence from the native table
        // is no longer evidence that an Agent consumer is unavailable.
        Ok(())
    }
}

fn is_native_nomi_capability(capability_id: &str) -> bool {
    matches!(
        capability_id,
        "fs.read"
            | "fs.search"
            | "fs.write"
            | "fs.patch"
            | "process.exec"
            | "vcs.status"
            | "vcs.diff"
            | "vcs.stage"
            | "vcs.commit"
            | "agent.execution.plan"
            | "agent.delegate"
            | "web.fetch"
            | "schedule.store"
            | "knowledge.search"
            | "knowledge.read"
            | "knowledge.write"
            | "skill.invoke"
            | "chat.basic"
            | "chat.minimal"
            | "session.attachments.read"
            | "memory.project.read"
            | "memory.project.citation"
            | "memory.session.scratch"
            | "process.session"
            | "terminal.pty"
            | "workspace.bind"
            | "workspace.artifacts"
            | "skill.catalog"
            | "skill.describe"
            | "skill.hooks"
            | "memory.project.write"
            | "memory.project.distill"
            | "browser.identity"
            | "browser.observe"
            | "browser.navigate"
            | "browser.act"
            | "browser.download"
            | "browser.upload"
            | "browser.evaluate"
            | "browser.takeover"
            | "a11y.observe"
            | "computer.observe"
            | "computer.input"
            | "computer.launch"
    )
}

fn project_capabilities_with_dynamic(
    revision: &AgentPresetRevision,
    mut dynamic_provider_names: impl FnMut(&str, bool) -> Vec<String>,
) -> Result<ProjectedCapabilityTools, AppError> {
    let mut initial_tools = BTreeSet::new();
    let mut deferred_tools = BTreeSet::new();
    let mut browser_use = false;
    let mut computer_use = false;

    let mut project = |
        selection: &nomifun_agent_contracts::CapabilitySelection,
        target: &mut BTreeSet<String>,
        deferred: bool,
    | -> Result<(), AppError> {
        let capability_id = selection.capability.id.as_ref();
        let native = nomi_capability_projection(capability_id);
        let projection = match native {
            Ok(projection) => projection,
            Err(native_error) => {
                let dynamic =
                    dynamic_provider_names(capability_id, deferred);
                if dynamic.is_empty() {
                    if is_native_nomi_capability(capability_id) {
                        return Err(native_error);
                    }
                    // Ordinary Plugin tools are named and registered after the
                    // Conversation ID exists. Context/Resource contributions
                    // intentionally add no model tool route.
                    return Ok(());
                }
                target.extend(dynamic);
                return Ok(());
            }
        };
        match projection {
            NomiCapabilityProjection::Tools(tools) => {
                target.extend(tools.iter().map(|tool| (*tool).to_owned()));
            }
            NomiCapabilityProjection::BrowserTools => {
                browser_use = true;
                target.insert("Browser".to_owned());
            }
            NomiCapabilityProjection::ComputerTools => {
                computer_use = true;
                target.insert("Computer".to_owned());
            }
            NomiCapabilityProjection::HostOnly { browser, computer } => {
                browser_use |= browser;
                computer_use |= computer;
            }
        }
        Ok(())
    };

    for selection in &revision.payload.initial_capabilities {
        project(selection, &mut initial_tools, false)?;
    }
    for selection in &revision.payload.on_demand_capabilities {
        project(selection, &mut deferred_tools, true)?;
    }

    let mut allowed_tools = initial_tools.clone();
    allowed_tools.extend(deferred_tools.iter().cloned());
    if !deferred_tools.is_empty() {
        allowed_tools.insert("ToolSearch".to_owned());
    }

    Ok(ProjectedCapabilityTools {
        initial_capability_ids: revision
            .payload
            .initial_capabilities
            .iter()
            .map(|selection| selection.capability.id.as_ref().to_owned())
            .collect(),
        on_demand_capability_ids: revision
            .payload
            .on_demand_capabilities
            .iter()
            .map(|selection| selection.capability.id.as_ref().to_owned())
            .collect(),
        allowed_tools: allowed_tools.into_iter().collect(),
        deferred_tools: deferred_tools.into_iter().collect(),
        browser_use,
        computer_use,
    })
}

fn merge_instructions(persona: &str, instructions: &str) -> String {
    match (persona.trim(), instructions.trim()) {
        ("", right) => right.to_owned(),
        (left, "") => left.to_owned(),
        (left, right) => format!("{left}\n\n{right}"),
    }
}

fn unsupported(subject: &str, detail: impl Into<String>) -> AppError {
    AppError::UnprocessableEntity(format!("unsupported {subject}: {}", detail.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        AgentPresetId, AgentPresetRevisionPayload, CapabilitySelection,
        CapabilityRef, ChatRouteCandidate, ChatRouteFeature, ChatRouteProtocol,
        ChatRouteRecordSchema, ChatRouteTask, ConnectionConfigRef, ContributionLock,
        ContributionSourceKind, DigestHex, ModelRouteId, OperationId, PluginMountId,
        PluginSourceKind, PluginSourceMetadata, PresetRevisionRef, PrincipalRef,
        ResolvedCapability, ResolvedSnapshotContent, ResolvedSnapshotId, ResolvedSnapshotRef,
        RuntimeFeatureId, RuntimeProfileKind, SkillRef, StableSourceIdentity,
        TypedResourceBinding,
    };
    use std::collections::{BTreeMap, BTreeSet};

    const DIGEST: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000003";

    fn route() -> nomifun_agent_contracts::ChatRouteRecord {
        ChatRouteRecord {
            schema: ChatRouteRecordSchema::V1,
            task: ChatRouteTask::AgentChat,
            primary: ChatRouteCandidate {
                model_route_id: ModelRouteId::from("route-1"),
                model_route_revision: 1,
                provider_id: OWNER.to_owned(),
                model: "step-3.7-flash".to_owned(),
                protocol: ChatRouteProtocol::OpenaiChat,
                connection_config_ref: ConnectionConfigRef::from("config-1"),
                config_revision_digest: DigestHex::from(DIGEST),
                credential_ref: "credential-1".to_owned(),
                features: BTreeSet::from([ChatRouteFeature::TextOutput]),
            },
            failovers: Vec::new(),
        }
    }

    fn fixture() -> (
        UserId,
        AgentBindingValue,
        AgentPresetRevision,
        ResolvedSnapshotEnvelope,
    ) {
        let payload = AgentPresetRevisionPayload {
            schema_version: "1.0.0".into(),
            model_route_refs: BTreeMap::from([(CHAT_TASK.into(), "route-1".into())]),
            chat_route_records: BTreeMap::from([(CHAT_TASK.into(), route())]),
            initial_capabilities: vec![
                capability("fs.read", true),
                capability("fs.write", true),
                capability("process.exec", true),
                capability("knowledge.read", false),
            ],
            on_demand_capabilities: Vec::new(),
            skill_bindings: vec![SkillRef {
                id: "skill.review".into(),
                version: "1.0.0".into(),
            }],
            system_role_provider_overrides: BTreeMap::new(),
            persona: "You are Nomi.".into(),
            instructions: "Be precise.".into(),
            starter_prompts: Vec::new(),
        };
        let contribution_locks = Vec::new();
        let reference = PresetRevisionRef {
            preset_id: AgentPresetId::from("assistant.general"),
            revision: 1,
            revision_digest: nomifun_agent_contracts::digest_payload(
                &nomifun_agent_contracts::AgentPresetRevisionDigestInput {
                    payload: payload.clone(),
                    contribution_locks: contribution_locks.clone(),
                },
            )
            .unwrap(),
        };
        let revision = AgentPresetRevision {
            reference: reference.clone(),
            payload,
            contribution_locks,
            created_by: OWNER.into(),
            created_at_ms: 1,
            reason: None,
        };
        let content = ResolvedSnapshotContent {
            schema_version: "1.0.0".into(),
            resolver_version: "1.0.0".into(),
            preset_revision_ref: reference.clone(),
            required_runtime_protocol_version: "1.0.0".into(),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: DIGEST.into(),
            required_runtime_features: BTreeSet::<RuntimeFeatureId>::new(),
            compiled_runtime_profile_digest: DIGEST.into(),
            model_route_refs: revision.payload.model_route_refs.clone(),
            chat_route_identity: Some(
                revision
                    .payload
                    .chat_route_records
                    .get(CHAT_TASK)
                    .unwrap()
                    .identity_for(reference.revision_id(), CHAT_TASK)
                    .unwrap(),
            ),
            initial_capabilities: vec![resolved_capability("fs.read")],
            on_demand_capabilities: Vec::new(),
            initial_miniapp_capabilities: Vec::new(),
            on_demand_miniapp_capabilities: Vec::new(),
            required_resource_kinds: BTreeSet::from(["workspace".into()]),
            on_demand_activation_plans: BTreeMap::new(),
            compact_on_demand_index: Vec::new(),
            capability_allowlist: BTreeSet::from(["fs.read".into()]),
            skill_locks: vec![nomifun_agent_contracts::ResolvedSkillLock {
                skill: SkillRef {
                    id: "skill.review".into(),
                    version: "1.0.0".into(),
                },
                body_digest: DIGEST.into(),
                required_capabilities: BTreeSet::new(),
            }],
            mcp_tool_locks: Vec::new(),
            resolved_role_providers: BTreeMap::new(),
            canonical_schema_manifest_digest: DIGEST.into(),
            target_contribution_manifest_digest: DIGEST.into(),
        };
        let snapshot_ref = ResolvedSnapshotRef {
            snapshot_id: ResolvedSnapshotId::from("snapshot-1"),
            snapshot_digest: nomifun_agent_contracts::digest_payload(&content).unwrap(),
        };
        let snapshot = ResolvedSnapshotEnvelope {
            snapshot_ref: snapshot_ref.clone(),
            content,
            actor: PrincipalRef {
                principal_kind: "user".into(),
                principal_id: OWNER.into(),
            },
            scene: "test".into(),
            surface: "desktop".into(),
            audience: "user".into(),
            created_at_ms: 2,
            resolver_run_id: OperationId::from("run-1"),
            availability_evidence_revision: "evidence-1".into(),
        };
        let binding = AgentBindingValue {
            preset_revision_ref: reference,
            resolved_snapshot_ref: snapshot_ref,
            typed_resource_bindings: Vec::new(),
            binding_version: 1,
        };
        (
            UserId::parse(OWNER).expect("canonical owner UUIDv7"),
            binding,
            revision,
            snapshot,
        )
    }

    fn capability(id: &str, _required: bool) -> CapabilitySelection {
        CapabilitySelection {
            capability: CapabilityRef {
                id: id.into(),
                version: "1.0.0".into(),
            },
            action_allowlist: BTreeSet::new(),
        }
    }

    fn resolved_capability(id: &str) -> ResolvedCapability {
        let capability_id = nomifun_agent_contracts::CapabilityId::from(id);
        let contribution_id = nomifun_agent_contracts::ContributionId::from(format!(
            "capability:{id}"
        ));
        ResolvedCapability {
            capability: CapabilityRef {
                id: capability_id.clone(),
                version: "1.0.0".into(),
            },
            source_package: nomifun_agent_contracts::PackageRef {
                id: "test.package".into(),
                version: "1.0.0".into(),
            },
            contribution_id: contribution_id.clone(),
            contribution_lock: ContributionLock {
                source_kind: ContributionSourceKind::PlatformBuiltin,
                source_identity: StableSourceIdentity::from("test.package"),
                mount_id: None,
                miniapp_id: None,
                mcp_binding_id: None,
                contribution_id,
                contract_digest: DIGEST.into(),
            },
            resolved_mount_id: PluginMountId::from("test.mount"),
            resolved_source: PluginSourceMetadata {
                source_kind: PluginSourceKind::Bundled,
                source_identity: "test.package".into(),
                source_digest: Some(DIGEST.into()),
            },
            target_artifact_digest: DIGEST.into(),
            schema_digest: DIGEST.into(),
            dependency_path: vec![capability_id],
            required_runtime_features: BTreeSet::new(),
        }
    }

    fn input<'a>(
        fixture: &'a (
            UserId,
            AgentBindingValue,
            AgentPresetRevision,
            ResolvedSnapshotEnvelope,
        ),
    ) -> ProjectionInput<'a> {
        ProjectionInput {
            owner: &fixture.0,
            binding: &fixture.1,
            revision: &fixture.2,
            snapshot: &fixture.3,
            title: Some("General"),
        }
    }

    fn refresh_fixture_identity(
        fixture: &mut (
            UserId,
            AgentBindingValue,
            AgentPresetRevision,
            ResolvedSnapshotEnvelope,
        ),
    ) {
        fixture.2.reference.revision_digest = fixture.2.revision_digest().unwrap();
        fixture.3.content.preset_revision_ref = fixture.2.reference.clone();
        fixture.3.snapshot_ref.snapshot_digest =
            nomifun_agent_contracts::digest_payload(&fixture.3.content).unwrap();
        fixture.1.preset_revision_ref = fixture.2.reference.clone();
        fixture.1.resolved_snapshot_ref = fixture.3.snapshot_ref.clone();
    }

    #[test]
    fn exact_binding_projects_chat_route_without_freezing_target_resources() {
        let fixture = fixture();
        let result = project(input(&fixture)).unwrap();
        assert_eq!(
            result.snapshot.resolved_model.as_ref().unwrap().model,
            "step-3.7-flash"
        );
        assert_eq!(result.request.model.as_ref().unwrap().provider_id, OWNER);
        assert!(
            result.request.extra.get("workspace").is_none(),
            "workspace selection belongs to the consuming target"
        );
        assert!(
            result.request.extra.get("knowledge_mounts").is_none(),
            "knowledge-base selection belongs to the consuming target"
        );
        assert!(
            result.request.extra.get("selected_mcp_server_ids").is_none(),
            "MCP selection belongs to the consuming target"
        );
        assert_eq!(result.snapshot.knowledge_policy.enabled, true);
        assert!(result.snapshot.knowledge_policy.writeback == false);
        assert_eq!(
            result.snapshot.required_resource_kinds,
            BTreeSet::from(["workspace".to_owned()])
        );
        let tools = result.request.extra["allowed_tools"]
            .as_array()
            .expect("native tool allowlist");
        for expected in ["Read", "Write", "Bash", "knowledge_read"] {
            assert!(
                tools.iter().any(|tool| tool == expected),
                "{expected} must be projected from the declared capability"
            );
        }
    }

    #[test]
    fn target_resource_bindings_are_accepted_but_not_frozen_into_the_projection() {
        let mut fixture = fixture();
        fixture.1.typed_resource_bindings.push(TypedResourceBinding {
            binding_id: "target-workspace".into(),
            resource_kind: "workspace".into(),
            resource_id: "workspace-opaque-id".into(),
            owner_id: OWNER.into(),
            operations: BTreeSet::from(["read".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([(
                "workspace_root".to_owned(),
                "C:\\target".to_owned(),
            )]),
        });

        let result = project(input(&fixture)).expect("target resource binding is valid");
        assert!(
            result.request.extra.get("workspace").is_none(),
            "target resource identity must remain outside the Preset projection"
        );
        assert!(
            result.request.extra.get("knowledge_mounts").is_none(),
            "target knowledge resources must be selected by the consuming Session"
        );
    }

    #[test]
    fn binding_revision_mismatch_is_rejected() {
        let mut fixture = fixture();
        fixture.1.preset_revision_ref.revision = 2;
        assert!(matches!(project(input(&fixture)), Err(AppError::Conflict(_))));
    }

    #[test]
    fn owner_mismatch_is_rejected() {
        let mut fixture = fixture();
        fixture.0 = UserId::parse("0190f5fe-7c00-7a00-8000-000000000099")
            .expect("canonical mismatched owner UUIDv7");
        assert!(matches!(project(input(&fixture)), Err(AppError::Forbidden(_))));
    }

    #[test]
    fn snapshot_mismatch_is_rejected() {
        let mut fixture = fixture();
        fixture.1.resolved_snapshot_ref.snapshot_id = "snapshot-2".into();
        assert!(matches!(project(input(&fixture)), Err(AppError::Conflict(_))));
    }

    #[test]
    fn dynamic_capability_does_not_require_a_native_projection() {
        let mut fixture = fixture();
        fixture
            .2
            .payload
            .initial_capabilities
            .push(capability("vcs.push", true));
        refresh_fixture_identity(&mut fixture);
        let projected = project(input(&fixture))
            .expect("dynamic capability is registered by the session provider");
        assert!(
            !projected.request.extra["allowed_tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool == "vcs.push")
        );
        assert!(projected
            .snapshot
            .initial_capabilities
            .iter()
            .any(|capability| capability == "vcs.push"));
    }

    #[test]
    fn dynamic_capability_is_not_rejected_by_the_native_adapter_table() {
        let mut fixture = fixture();
        fixture
            .2
            .payload
            .initial_capabilities
            .push(capability("plugin.example.tool", true));
        refresh_fixture_identity(&mut fixture);

        validate_nomi_capability_projection(&fixture.2)
            .expect("canonical Catalog/provider owns dynamic availability");
        project(input(&fixture))
            .expect("dynamic capability does not require a native adapter row");
    }

    #[test]
    fn on_demand_capability_is_deferred_and_gets_tool_search() {
        let mut fixture = fixture();
        fixture
            .2
            .payload
            .on_demand_capabilities
            .push(capability("vcs.stage", false));
        refresh_fixture_identity(&mut fixture);

        let result = project(input(&fixture)).expect("on-demand capability projection");
        let allowed = result.request.extra["allowed_tools"]
            .as_array()
            .expect("allowlist");
        let deferred = result.request.extra["deferred_tools"]
            .as_array()
            .expect("deferred tool list");
        assert!(allowed.iter().any(|tool| tool == "vcs.stage"));
        assert!(deferred.iter().any(|tool| tool == "vcs.stage"));
        assert!(allowed.iter().any(|tool| tool == "ToolSearch"));
        assert_eq!(
            result.snapshot.on_demand_capabilities,
            vec!["vcs.stage".to_owned()]
        );
    }

    #[test]
    fn zero_capability_preset_projects_to_deny_all() {
        let mut fixture = fixture();
        fixture.2.payload.initial_capabilities.clear();
        fixture.2.payload.on_demand_capabilities.clear();
        refresh_fixture_identity(&mut fixture);

        let result = project(input(&fixture)).expect("zero-capability projection");
        assert_eq!(result.request.extra["allowed_tools"], json!([]));
        assert_eq!(result.request.extra["deferred_tools"], json!([]));
        assert_eq!(result.request.extra["enforce_tool_allowlist"], true);
        assert!(result.snapshot.initial_capabilities.is_empty());
        assert!(result.snapshot.on_demand_capabilities.is_empty());
    }

    #[test]
    fn initial_capability_is_full_schema_candidate_and_not_deferred() {
        let mut fixture = fixture();
        fixture.2.payload.initial_capabilities = vec![capability("vcs.stage", true)];
        refresh_fixture_identity(&mut fixture);

        let result = project(input(&fixture)).expect("initial capability projection");
        assert_eq!(result.request.extra["allowed_tools"], json!(["vcs.stage"]));
        assert_eq!(result.request.extra["deferred_tools"], json!([]));
        assert_eq!(
            result.snapshot.initial_capabilities,
            vec!["vcs.stage".to_owned()]
        );
    }

    #[test]
    fn vcs_capabilities_use_dedicated_native_tool_names() {
        let mut fixture = fixture();
        fixture
            .2
            .payload
            .initial_capabilities
            .push(capability("vcs.status", true));
        refresh_fixture_identity(&mut fixture);

        let result = project(input(&fixture)).expect("typed VCS projection");
        let tools = result.request.extra["allowed_tools"]
            .as_array()
            .expect("native tool allowlist");
        assert_eq!(
            tools.iter().filter(|tool| *tool == "vcs.status").count(),
            1
        );
    }

    #[test]
    fn repaired_builtins_project_only_their_native_tools_in_both_placements() {
        for (id, names) in [
            ("web.fetch", vec!["web_fetch"]),
            ("agent.delegate", vec!["nomi_delegate"]),
            ("schedule.store", vec!["cron_create", "cron_delete", "cron_list"]),
        ] {
            for deferred in [false, true] {
                let mut fixture = fixture();
                fixture.2.payload.initial_capabilities.clear();
                fixture.2.payload.on_demand_capabilities.clear();
                if deferred {
                    fixture.2.payload.on_demand_capabilities.push(capability(id, false));
                } else {
                    fixture.2.payload.initial_capabilities.push(capability(id, true));
                }
                refresh_fixture_identity(&mut fixture);
                let result = project(input(&fixture)).expect("repaired builtin projection");
                let mut allowed = names.clone();
                if deferred { allowed.push("ToolSearch"); }
                allowed.sort();
                assert_eq!(result.request.extra["allowed_tools"], json!(allowed), "{id}");
                assert_eq!(result.request.extra["deferred_tools"], if deferred { json!(names) } else { json!([]) }, "{id}");
                assert_eq!(result.request.extra["enforce_tool_allowlist"], true);
            }
        }
    }

    #[test]
    fn projection_table_rejects_capabilities_without_a_nomi_owner() {
        for capability_id in [
            "fs.delete",
            "fs.watch",
            "fs.snapshot",
            "vcs.push",
            "web.search",
            "agent.execution.observe",
            "agent.execution.steer",
            "llm.vision",
        ] {
            let error = nomi_capability_projection(capability_id)
                .expect_err("unsupported capability must fail closed");
            assert!(
                error.to_string().contains(capability_id),
                "error should identify {capability_id}: {error}"
            );
        }
    }

    #[test]
    fn browser_capability_availability_matches_the_host_build() {
        let result = nomi_capability_projection("browser.navigate");
        if cfg!(feature = "browser-use") {
            assert_eq!(result.unwrap(), NomiCapabilityProjection::BrowserTools);
        } else {
            assert!(result.is_err());
        }
    }

    #[test]
    fn computer_capability_availability_matches_the_host_build() {
        let result = nomi_capability_projection("computer.input");
        if cfg!(feature = "computer-use") {
            assert_eq!(result.unwrap(), NomiCapabilityProjection::ComputerTools);
        } else {
            assert!(result.is_err());
        }
    }
}
