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
        &route,
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
            .required_runtime_features
            .contains(&nomifun_agent_contracts::RuntimeFeatureId::from("code_mode")),
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
    coding_profile: bool,
    required_resource_kinds: BTreeSet<String>,
    included_skills: Vec<String>,
) -> Result<NomiCoreAgentProjection, AppError> {
    let runtime_profile = coding_profile.then_some("coding");
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
        "chat_config_revision_digest": route.primary.config_revision_digest,
        "allowed_tools": capability_tools.allowed_tools,
        "enforce_tool_allowlist": true,
        "deferred_tools": capability_tools.deferred_tools,
        "browser_use": capability_tools.browser_use,
        "computer_use": capability_tools.computer_use,
        // This is a subtractive runtime profile, not a capability grant. The
        // Nomi factory accepts it only when the complete server-projected
        // coding tool allowlist is already present.
        "runtime_profile": runtime_profile,
        // Vision is a message-context capability, not a model tool. The Nomi
        // factory intersects this flag with the exact configured Chat model's
        // `vision_input` trait before its attachment loader may emit an image
        // content block.
        "vision_input": capability_tools.vision_input,
        "vision_on_demand": capability_tools.vision_on_demand,
        "mcp_capabilities": {
            "connect": capability_tools.mcp_connect,
            "tool_proxy": capability_tools.mcp_tool_proxy,
            "resource": capability_tools.mcp_resource,
            "oauth": capability_tools.mcp_oauth,
        },
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
    vision_input: bool,
    vision_on_demand: bool,
    mcp_connect: bool,
    mcp_tool_proxy: bool,
    mcp_resource: bool,
    mcp_oauth: bool,
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
    /// Exact provider names are materialized from the current Session's
    /// server-owned Platform Builtin action set.
    HostedTools,
    BrowserTools,
    ComputerTools,
    /// Enables the bounded image-attachment -> multimodal-message path. This
    /// is deliberately distinct from `HostOnly`: both the selected Chat route
    /// and the runtime provider capability are checked before model delivery.
    VisionContext,
    /// Uses the exact OpenAI Responses route's provider-native web_search
    /// owner. The route feature is checked before the tool is projected.
    WebSearchTool,
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
        "fs.delete" | "fs.snapshot" => NomiCapabilityProjection::HostedTools,
        "fs.watch" => NomiCapabilityProjection::HostOnly {
            browser: false,
            computer: false,
        },

        // Native process and VCS families.
        "process.exec" => {
            NomiCapabilityProjection::Tools(&["Bash", "exec_command", "write_stdin"])
        }
        "vcs.status" => NomiCapabilityProjection::Tools(&["vcs.status"]),
        "vcs.diff" => NomiCapabilityProjection::Tools(&["vcs.diff", "review.status"]),
        "vcs.stage" => NomiCapabilityProjection::Tools(&["vcs.stage"]),
        "vcs.commit" => NomiCapabilityProjection::Tools(&["vcs.commit"]),
        "vcs.push" => NomiCapabilityProjection::HostedTools,

        // The plan checklist is always registered by the Nomi bootstrap. Its
        // deferred placement is applied by the host registry when requested.
        "agent.execution.plan" => NomiCapabilityProjection::Tools(&["update_plan"]),
        "agent.execution.observe" => NomiCapabilityProjection::Tools(&[
            nomifun_ai_agent::AGENT_EXECUTION_OBSERVE_TOOL_NAME,
        ]),
        "agent.execution.steer" => NomiCapabilityProjection::Tools(&[
            nomifun_ai_agent::AGENT_EXECUTION_STEER_TOOL_NAME,
        ]),
        "agent.fork" => NomiCapabilityProjection::Tools(&[
            nomifun_ai_agent::AGENT_FORK_TOOL_NAME,
        ]),
        "agent.delegate" => NomiCapabilityProjection::Tools(&[
            "nomi_delegate",
            nomifun_ai_agent::SUBAGENT_SEND_TOOL_NAME,
            nomifun_ai_agent::SUBAGENT_WAIT_TOOL_NAME,
        ]),

        // Bundled native adapters reuse the application's existing HTTP and
        // owner-scoped Cron services. No dynamic Plugin runtime is required.
        "web.fetch" => NomiCapabilityProjection::Tools(&[
            nomifun_ai_agent::web_fetch::WEB_FETCH_TOOL_NAME,
        ]),
        "web.search" => NomiCapabilityProjection::WebSearchTool,
        "citation.render" => NomiCapabilityProjection::Tools(&[
            nomifun_ai_agent::web_search::CITATION_RENDER_TOOL_NAME,
        ]),
        // MCP connection is an explicit Session-bound activation tool. It
        // resolves the exact server-owned binding/config/credential only when
        // called after ToolSearch; no server is contacted during bootstrap.
        "mcp.connect" => NomiCapabilityProjection::Tools(&[
            nomifun_ai_agent::MCP_CONNECT_TOOL_NAME,
        ]),
        "mcp.oauth" => NomiCapabilityProjection::HostOnly {
            browser: false,
            computer: false,
        },
        "mcp.tool_proxy" => NomiCapabilityProjection::Tools(&[
            nomifun_ai_agent::MCP_GENERIC_PROXY_TOOL_NAME,
        ]),
        "mcp.resource" => NomiCapabilityProjection::Tools(&[
            nomifun_ai_agent::MCP_RESOURCE_LIST_TOOL_NAME,
            nomifun_ai_agent::MCP_RESOURCE_READ_TOOL_NAME,
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

        // Image understanding is not a callable tool. It authorizes Nomi to
        // turn supported local image attachments into multimodal user content.
        "llm.vision" => NomiCapabilityProjection::VisionContext,

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
    validate_vision_route(revision)?;
    validate_web_search_route(revision)?;
    Ok(())
}

fn validate_web_search_route(revision: &AgentPresetRevision) -> Result<(), AppError> {
    let selected = revision
        .payload
        .initial_capabilities
        .iter()
        .chain(&revision.payload.on_demand_capabilities)
        .any(|selection| selection.capability.id.as_ref() == "web.search");
    if !selected {
        return Ok(());
    }
    let route = revision
        .payload
        .chat_route_records
        .get(CHAT_TASK)
        .ok_or_else(|| unsupported("web.search", "agent_chat route record is required"))?;
    if route.primary.protocol != nomifun_agent_contracts::ChatRouteProtocol::OpenaiResponses
        || !route
            .primary
            .features
            .contains(&nomifun_agent_contracts::ChatRouteFeature::WebSearch)
    {
        return Err(unsupported(
            "web.search",
            "the exact primary Chat route must use openai.responses and declare web_search",
        ));
    }
    Ok(())
}

fn validate_vision_route(revision: &AgentPresetRevision) -> Result<(), AppError> {
    let vision_selected = revision
        .payload
        .initial_capabilities
        .iter()
        .chain(&revision.payload.on_demand_capabilities)
        .any(|selection| selection.capability.id.as_ref() == "llm.vision");
    if !vision_selected {
        return Ok(());
    }

    let route = revision
        .payload
        .chat_route_records
        .get(CHAT_TASK)
        .ok_or_else(|| unsupported("llm.vision", "agent_chat route record is required"))?;
    if !route
        .primary
        .features
        .contains(&nomifun_agent_contracts::ChatRouteFeature::ImageInput)
    {
        return Err(unsupported(
            "llm.vision",
            format!(
                "the exact primary Chat model {}/{} does not declare image_input",
                route.primary.provider_id, route.primary.model
            ),
        ));
    }
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
            | "fs.delete"
            | "fs.watch"
            | "fs.snapshot"
            | "process.exec"
            | "vcs.status"
            | "vcs.diff"
            | "vcs.stage"
            | "vcs.commit"
            | "vcs.push"
            | "agent.execution.plan"
            | "agent.execution.observe"
            | "agent.execution.steer"
            | "agent.fork"
            | "agent.delegate"
            | "web.search"
            | "web.fetch"
            | "citation.render"
            | "mcp.connect"
            | "mcp.tool_proxy"
            | "mcp.resource"
            | "mcp.oauth"
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
            | "llm.vision"
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
    route: &ChatRouteRecord,
    mut dynamic_provider_names: impl FnMut(&str, bool) -> Vec<String>,
) -> Result<ProjectedCapabilityTools, AppError> {
    let mut initial_tools = BTreeSet::new();
    let mut deferred_tools = BTreeSet::new();
    let mut browser_use = false;
    let mut computer_use = false;
    let mut vision_input = false;
    let mut vision_on_demand = false;
    let mut mcp_connect = false;
    let mut mcp_tool_proxy = false;
    let mut mcp_resource = false;
    let mut mcp_oauth = false;

    let mut project = |
        selection: &nomifun_agent_contracts::CapabilitySelection,
        target: &mut BTreeSet<String>,
        deferred: bool,
    | -> Result<(), AppError> {
        let capability_id = selection.capability.id.as_ref();
        match capability_id {
            "mcp.connect" => mcp_connect = true,
            "mcp.tool_proxy" => mcp_tool_proxy = true,
            "mcp.resource" => mcp_resource = true,
            "mcp.oauth" => mcp_oauth = true,
            _ => {}
        }
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
            NomiCapabilityProjection::HostedTools => {
                let hosted = dynamic_provider_names(capability_id, deferred);
                if hosted.is_empty() {
                    // Conversation creation happens before a concrete
                    // AgentSession ID exists. The app-owned runtime provider
                    // materializes the exact hosted routes from the persisted
                    // Binding/Snapshot once that ID is known and extends the
                    // same allowlist before Nomi builds its registry.
                    return Ok(());
                }
                target.extend(hosted);
            }
            NomiCapabilityProjection::BrowserTools => {
                browser_use = true;
                target.insert("Browser".to_owned());
            }
            NomiCapabilityProjection::ComputerTools => {
                computer_use = true;
                target.insert("Computer".to_owned());
            }
            NomiCapabilityProjection::VisionContext => {
                if !route
                    .primary
                    .features
                    .contains(&nomifun_agent_contracts::ChatRouteFeature::ImageInput)
                {
                    return Err(unsupported(
                        "llm.vision",
                        format!(
                            "the exact primary Chat model {}/{} does not declare image_input",
                            route.primary.provider_id, route.primary.model
                        ),
                    ));
                }
                if deferred {
                    vision_on_demand = true;
                    target.insert(
                        nomifun_ai_agent::vision_activation::VISION_ACTIVATE_TOOL_NAME.to_owned(),
                    );
                } else {
                    vision_input = true;
                }
            }
            NomiCapabilityProjection::WebSearchTool => {
                if route.primary.protocol
                    != nomifun_agent_contracts::ChatRouteProtocol::OpenaiResponses
                    || !route
                        .primary
                        .features
                        .contains(&nomifun_agent_contracts::ChatRouteFeature::WebSearch)
                {
                    return Err(unsupported(
                        "web.search",
                        "the exact primary Chat route must use openai.responses and declare web_search",
                    ));
                }
                target.insert(nomifun_ai_agent::web_search::WEB_SEARCH_TOOL_NAME.to_owned());
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
        vision_input,
        vision_on_demand,
        mcp_connect,
        mcp_tool_proxy,
        mcp_resource,
        mcp_oauth,
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
    fn hosted_capability_defers_tool_name_until_session_materialization() {
        let mut fixture = fixture();
        let baseline = project(input(&fixture))
            .expect("baseline projection")
            .request
            .extra["allowed_tools"]
            .clone();
        fixture
            .2
            .payload
            .initial_capabilities
            .push(capability("vcs.push", true));
        refresh_fixture_identity(&mut fixture);
        let projection = project(input(&fixture))
            .expect("pre-Session projection keeps the hosted capability ceiling");
        assert!(projection
            .snapshot
            .initial_capabilities
            .contains(&"vcs.push".to_owned()));
        assert_eq!(projection.request.extra["allowed_tools"], baseline);
        assert_eq!(
            nomi_capability_projection("vcs.push").unwrap(),
            NomiCapabilityProjection::HostedTools
        );
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
        assert_eq!(result.request.extra["vision_input"], false);
        assert!(result.snapshot.initial_capabilities.is_empty());
        assert!(result.snapshot.on_demand_capabilities.is_empty());
    }

    #[test]
    fn vision_capability_projects_into_the_real_message_context_path() {
        let mut fixture = fixture();
        fixture.2.payload.initial_capabilities = vec![
            capability("session.attachments.read", true),
            capability("llm.vision", true),
        ];
        fixture
            .2
            .payload
            .chat_route_records
            .get_mut(CHAT_TASK)
            .unwrap()
            .primary
            .features
            .insert(ChatRouteFeature::ImageInput);
        refresh_fixture_identity(&mut fixture);

        validate_nomi_capability_projection(&fixture.2)
            .expect("vision-capable exact Chat route owns llm.vision");
        let result = project(input(&fixture)).expect("vision context projection");
        assert_eq!(result.request.extra["vision_input"], true);
        assert_eq!(result.request.extra["vision_on_demand"], false);
        assert_eq!(result.request.extra["allowed_tools"], json!([]));
        assert_eq!(
            nomi_capability_projection("llm.vision").unwrap(),
            NomiCapabilityProjection::VisionContext
        );
    }

    #[test]
    fn on_demand_vision_stays_inactive_until_its_deferred_tool_runs() {
        let mut fixture = fixture();
        fixture.2.payload.initial_capabilities.clear();
        fixture.2.payload.on_demand_capabilities = vec![capability("llm.vision", false)];
        fixture
            .2
            .payload
            .chat_route_records
            .get_mut(CHAT_TASK)
            .unwrap()
            .primary
            .features
            .insert(ChatRouteFeature::ImageInput);
        refresh_fixture_identity(&mut fixture);

        let result = project(input(&fixture)).expect("deferred vision projection");
        assert_eq!(result.request.extra["vision_input"], false);
        assert_eq!(result.request.extra["vision_on_demand"], true);
        assert_eq!(
            result.request.extra["deferred_tools"],
            json!(["activate_vision_input"])
        );
        assert_eq!(
            result.request.extra["allowed_tools"],
            json!(["ToolSearch", "activate_vision_input"])
        );
    }

    #[test]
    fn vision_capability_rejects_a_text_only_exact_chat_route() {
        let mut fixture = fixture();
        fixture.2.payload.initial_capabilities = vec![capability("llm.vision", true)];
        refresh_fixture_identity(&mut fixture);

        let validation = validate_nomi_capability_projection(&fixture.2)
            .expect_err("text-only route must fail capability validation");
        assert!(validation.to_string().contains("image_input"));

        let projection = project(input(&fixture))
            .expect_err("text-only route must fail runtime projection");
        assert!(projection.to_string().contains("image_input"));
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
            .extend([
                capability("vcs.status", true),
                capability("vcs.diff", true),
            ]);
        fixture
            .3
            .content
            .required_runtime_features
            .insert(RuntimeFeatureId::from("code_mode"));
        refresh_fixture_identity(&mut fixture);

        let result = project(input(&fixture)).expect("typed VCS projection");
        let tools = result.request.extra["allowed_tools"]
            .as_array()
            .expect("native tool allowlist");
        assert_eq!(
            tools.iter().filter(|tool| *tool == "vcs.status").count(),
            1
        );
        assert_eq!(
            tools.iter().filter(|tool| *tool == "review.status").count(),
            1,
            "vcs.diff must carry the read-only review workflow tool"
        );
        assert_eq!(
            result.request.extra["runtime_profile"],
            "coding",
            "coding.codex must select the explicit Nomi coding profile"
        );
    }

    #[test]
    fn repaired_builtins_project_only_their_native_tools_in_both_placements() {
        for (id, names) in [
            ("web.fetch", vec!["web_fetch"]),
            (
                "agent.delegate",
                vec!["nomi_delegate", "subagent_send", "subagent_wait"],
            ),
            ("agent.execution.observe", vec!["agent_execution_observe"]),
            ("agent.execution.steer", vec!["agent_execution_steer"]),
            ("agent.fork", vec!["agent_fork"]),
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
    fn web_search_requires_and_uses_an_exact_responses_search_route() {
        let mut search_fixture = fixture();
        search_fixture.2.payload.initial_capabilities = vec![capability("web.search", true)];
        let route = search_fixture
            .2
            .payload
            .chat_route_records
            .get_mut(CHAT_TASK)
            .unwrap();
        route.primary.protocol = ChatRouteProtocol::OpenaiResponses;
        route.primary.features.insert(ChatRouteFeature::WebSearch);
        refresh_fixture_identity(&mut search_fixture);

        validate_nomi_capability_projection(&search_fixture.2)
            .expect("search-capable exact route");
        let result = project(input(&search_fixture)).expect("web search projection");
        assert_eq!(result.request.extra["allowed_tools"], json!(["web_search"]));

        let mut unsupported = fixture();
        unsupported.2.payload.initial_capabilities = vec![capability("web.search", true)];
        refresh_fixture_identity(&mut unsupported);
        assert!(validate_nomi_capability_projection(&unsupported.2).is_err());
        assert!(project(input(&unsupported)).is_err());
    }

    #[test]
    fn citation_render_is_deferred_and_resolves_only_session_search_results() {
        let mut fixture = fixture();
        fixture.2.payload.initial_capabilities.clear();
        fixture.2.payload.on_demand_capabilities = vec![capability("citation.render", false)];
        refresh_fixture_identity(&mut fixture);

        let result = project(input(&fixture)).expect("citation context projection");
        assert_eq!(
            result.request.extra["allowed_tools"],
            json!(["ToolSearch", "citation_render"])
        );
        assert_eq!(
            result.request.extra["deferred_tools"],
            json!(["citation_render"])
        );
        assert_eq!(
            nomi_capability_projection("citation.render").unwrap(),
            NomiCapabilityProjection::Tools(&["citation_render"])
        );
    }

    #[test]
    fn mcp_capabilities_project_the_exact_session_lifecycle_and_tools() {
        let mut fixture = fixture();
        fixture.2.payload.initial_capabilities.clear();
        fixture.2.payload.on_demand_capabilities = [
            "mcp.connect",
            "mcp.tool_proxy",
            "mcp.resource",
            "mcp.oauth",
        ]
        .into_iter()
        .map(|id| capability(id, false))
        .collect();
        refresh_fixture_identity(&mut fixture);

        let result = project(input(&fixture)).expect("MCP Session projection");
        assert_eq!(
            result.request.extra["mcp_capabilities"],
            json!({
                "connect": true,
                "tool_proxy": true,
                "resource": true,
                "oauth": true,
            })
        );
        assert_eq!(
            result.request.extra["allowed_tools"],
            json!([
                "ToolSearch",
                "mcp_connect",
                "mcp_resource_list",
                "mcp_resource_read",
                "mcp_tool_proxy",
            ])
        );
        assert_eq!(
            result.request.extra["deferred_tools"],
            json!([
                "mcp_connect",
                "mcp_resource_list",
                "mcp_resource_read",
                "mcp_tool_proxy",
            ])
        );
        assert_eq!(
            nomi_capability_projection("mcp.connect").unwrap(),
            NomiCapabilityProjection::Tools(&[nomifun_ai_agent::MCP_CONNECT_TOOL_NAME])
        );
        assert_eq!(
            nomi_capability_projection("mcp.oauth").unwrap(),
            NomiCapabilityProjection::HostOnly {
                browser: false,
                computer: false,
            }
        );
        assert_eq!(
            nomi_capability_projection("mcp.tool_proxy").unwrap(),
            NomiCapabilityProjection::Tools(&[
                nomifun_ai_agent::MCP_GENERIC_PROXY_TOOL_NAME,
            ])
        );
        assert_eq!(
            nomi_capability_projection("mcp.resource").unwrap(),
            NomiCapabilityProjection::Tools(&[
                nomifun_ai_agent::MCP_RESOURCE_LIST_TOOL_NAME,
                nomifun_ai_agent::MCP_RESOURCE_READ_TOOL_NAME,
            ])
        );
    }

    #[test]
    fn wave2_workspace_capabilities_use_hosted_tools_and_real_watch_context() {
        for capability_id in ["fs.delete", "fs.snapshot", "vcs.push"] {
            assert_eq!(
                nomi_capability_projection(capability_id).unwrap(),
                NomiCapabilityProjection::HostedTools,
                "{capability_id}"
            );
        }
        assert_eq!(
            nomi_capability_projection("fs.watch").unwrap(),
            NomiCapabilityProjection::HostOnly {
                browser: false,
                computer: false,
            }
        );
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
