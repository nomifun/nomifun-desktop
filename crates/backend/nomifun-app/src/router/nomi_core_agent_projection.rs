//! Pure projection from the canonical Agent Capability document to Nomi-core.
//!
//! This module deliberately has no HTTP, database, runtime, or factory
//! dependency.  It validates the immutable binding/revision/snapshot chain and
//! produces only the fields that the current Nomi conversation creator already
//! understands.  It must not be used as an authority to persist a binding or
//! to start a Remote lifecycle.

use std::collections::{BTreeSet, HashMap};

use nomifun_agent_contracts::{
    AgentBindingValue, AgentPresetRevision, AgentPresetRevisionPayload, ChatRouteIdentity,
    ChatRouteRecord, PresetRevisionRef, ResolvedSnapshotEnvelope, TypedResourceBinding,
};
use nomifun_api_types::{
    AgentBindingValueDto, AgentPresetEditorResponse, CreateConversationRequest, KnowledgeMountInfo,
    ModelPreference, PresetKnowledgePolicy, PresetTarget, PreviewStatusDto,
    ResolveAgentPresetPreviewResponse, ResolvedPresetSnapshot,
};
use nomifun_common::{AgentType, AppError, ProviderWithModel, UserId, validate_uuidv7};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Value, json};

const CHAT_TASK: &str = nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT;
const WORKSPACE_KIND: &str = "workspace";
const WORKSPACE_ROOT: &str = "workspace_root";
const KNOWLEDGE_KIND: &str = "knowledge_base";
const MCP_KIND: &str = "mcp_server";

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
    pub snapshot: ResolvedPresetSnapshot,
    pub request: CreateConversationRequest,
}

/// Exact control-plane inputs required by the app-local Nomi-core adapter.
///
/// The control-plane repository remains behind `AgentControlPlane`; callers
/// must first load the editor revision and its saved-revision preview under the
/// authenticated owner.  This prevents the adapter from resolving a newer
/// revision or silently selecting a default resource.
#[derive(Debug, Clone, Copy)]
pub struct SavedBindingProjectionInput<'a> {
    pub owner: &'a UserId,
    pub binding: &'a AgentBindingValueDto,
    pub editor: &'a AgentPresetEditorResponse,
    pub preview: &'a ResolveAgentPresetPreviewResponse,
    pub title: Option<&'a str>,
}

#[derive(Debug)]
pub struct NomiCoreSavedBindingProjection {
    pub binding: AgentBindingValue,
    pub projection: NomiCoreAgentProjection,
}

/// Project the exact saved Agent Settings binding into the existing Nomi
/// Conversation creator.
///
/// Unlike [`project`], this is the DTO-facing bridge used by the app router.
/// It accepts a control-plane `ready` preview as the source of the immutable
/// snapshot reference, validates all identities that are available through the
/// public DTO API, and fails closed when the preview is blocked or incomplete.
pub fn project_saved_binding(
    input: SavedBindingProjectionInput<'_>,
) -> Result<NomiCoreSavedBindingProjection, AppError> {
    let binding: AgentBindingValue = wire_cast(input.binding)?;
    if binding
        .typed_resource_bindings
        .iter()
        .any(|resource| resource.owner_id.as_str() != input.owner.as_ref())
    {
        return Err(AppError::Forbidden(
            "typed resource binding owner does not match the authenticated owner".into(),
        ));
    }

    let revision_dto = input.editor.revision.as_ref().ok_or_else(|| {
        AppError::UnprocessableEntity(
            "the saved Agent Preset has no immutable revision to project".into(),
        )
    })?;
    let revision = revision_from_dto(revision_dto)?;
    if revision.reference != binding.preset_revision_ref {
        return Err(AppError::Conflict(
            "AgentBinding revision does not match the exact editor revision".into(),
        ));
    }
    if input.preview.status != PreviewStatusDto::Ready || !input.preview.can_create_session {
        return Err(AppError::UnprocessableEntity(format!(
            "saved Agent Preset preview is not ready for a Nomi-core Session: {}",
            serde_json::to_string(&input.preview.diagnostics)
                .unwrap_or_else(|_| "preview diagnostics unavailable".to_owned())
        )));
    }

    let preview_revision: PresetRevisionRef = wire_cast(&input.preview.candidate_revision_ref)?;
    if preview_revision != revision.reference {
        return Err(AppError::Conflict(
            "saved Agent Preset preview revision does not match the binding".into(),
        ));
    }
    let snapshot_ref: nomifun_agent_contracts::ResolvedSnapshotRef = input
        .preview
        .resolved_snapshot_ref
        .as_ref()
        .ok_or_else(|| {
            AppError::UnprocessableEntity(
                "ready Agent Preset preview did not return a resolved Snapshot reference".into(),
            )
        })
        .and_then(wire_cast)?;
    if snapshot_ref != binding.resolved_snapshot_ref {
        return Err(AppError::Conflict(
            "AgentBinding Snapshot reference does not match the saved preview".into(),
        ));
    }

    let projection = project_revision(
        input.owner,
        &binding,
        &revision,
        input.title.or(Some(input.editor.preset.display_name.as_str())),
    )?;
    Ok(NomiCoreSavedBindingProjection {
        binding,
        projection,
    })
}

/// Validate the canonical identity chain and project a saved Agent Preset to
/// the current Nomi conversation request.
///
/// The returned `extra` contains only request/build inputs recognized by the
/// existing Nomi path (`system_prompt`, `allowed_tools`, `workspace`,
/// `knowledge_mounts`, and `selected_mcp_server_ids`).  It never contains
/// Remote provenance or lifecycle state.
#[allow(dead_code)] // The typed seam is retained for a future native Nomi adapter.
pub fn project(input: ProjectionInput<'_>) -> Result<NomiCoreAgentProjection, AppError> {
    validate_identity_chain(&input)?;
    let route = exact_chat_route(input.revision, input.snapshot)?;
    let resources = project_resources(input.owner, input.binding, input.revision)?;
    let allowed_tools = project_capabilities(input.revision)?;
    validate_unsupported_document_fields(input.revision)?;

    project_revision_parts(
        input.revision,
        input.title,
        route,
        resources,
        allowed_tools,
        input
            .snapshot
            .content
            .skill_locks
            .iter()
            .map(|lock| lock.skill.id.as_ref().to_owned())
            .collect(),
    )
}

fn project_revision(
    owner: &UserId,
    binding: &AgentBindingValue,
    revision: &AgentPresetRevision,
    title: Option<&str>,
) -> Result<NomiCoreAgentProjection, AppError> {
    let route = exact_chat_route_from_revision(revision)?;
    let resources = project_resources(owner, binding, revision)?;
    validate_on_demand_projection(revision)?;
    let allowed_tools = project_capabilities(revision)?;
    validate_unsupported_document_fields(revision)?;
    project_revision_parts(
        revision,
        title,
        route,
        resources,
        allowed_tools,
        revision
            .payload
            .skill_bindings
            .iter()
            .map(|skill| skill.id.as_ref().to_owned())
            .collect(),
    )
}

fn validate_on_demand_projection(revision: &AgentPresetRevision) -> Result<(), AppError> {
    if revision.payload.on_demand_capabilities.is_empty() {
        return Ok(());
    }
    let ids = revision
        .payload
        .on_demand_capabilities
        .iter()
        .map(|selection| selection.capability.id.as_ref())
        .collect::<Vec<_>>();
    Err(unsupported(
        "on-demand capability activation",
        format!(
            "the current Nomi-core runtime has no activation port for [{}]; \
             Preview/Session creation must remain unavailable instead of \
             treating deferred capabilities as always active",
            ids.join(", ")
        ),
    ))
}

fn project_revision_parts(
    revision_document: &AgentPresetRevision,
    title_input: Option<&str>,
    route: ChatRouteRecord,
    resources: ProjectedResources,
    allowed_tools: Vec<String>,
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

    let knowledge_base_ids = resources
        .knowledge_mounts
        .iter()
        .map(|mount| mount.knowledge_base_id.clone())
        .collect::<Vec<_>>();
    let knowledge_policy = PresetKnowledgePolicy {
        enabled: !knowledge_base_ids.is_empty(),
        writeback: allowed_tools.iter().any(|tool| tool == "knowledge_write"),
        eagerness: None,
        grounded: !knowledge_base_ids.is_empty(),
    };
    let resolved_model = ModelPreference {
        provider_id: Some(route.primary.provider_id.clone()),
        model: route.primary.model.clone(),
        required: true,
    };
    let projected_snapshot = ResolvedPresetSnapshot {
        preset_id: revision_document.reference.preset_id.as_ref().to_owned(),
        preset_revision: revision,
        preset_name: title.clone(),
        target: PresetTarget::Conversation,
        routing_description: None,
        instructions: instructions.clone(),
        resolved_agent_id: None,
        resolved_agent_type: Some(AgentType::Nomi.serde_name().to_owned()),
        resolved_agent_backend: Some("nomi".to_owned()),
        resolved_model: Some(resolved_model.clone()),
        included_skills,
        excluded_auto_skills: Vec::new(),
        knowledge_policy,
        knowledge_base_ids,
        warnings: Vec::new(),
    };

    let mut extra = json!({
        "system_prompt": instructions,
        "allowed_tools": allowed_tools,
    });
    let object = extra
        .as_object_mut()
        .expect("projection extra literal is an object");
    if let Some(workspace) = resources.workspace {
        object.insert("workspace".into(), Value::String(workspace));
    }
    if !resources.knowledge_mounts.is_empty() {
        object.insert(
            "knowledge_mounts".into(),
            serde_json::to_value(&resources.knowledge_mounts)
                .map_err(|error| AppError::Internal(error.to_string()))?,
        );
        object.insert(
            "knowledge_writeback".into(),
            Value::Bool(
                projected_snapshot
                    .knowledge_policy
                    .writeback,
            ),
        );
    }
    if !resources.mcp_server_ids.is_empty() {
        object.insert(
            "selected_mcp_server_ids".into(),
            serde_json::to_value(&resources.mcp_server_ids)
                .map_err(|error| AppError::Internal(error.to_string()))?,
        );
    }

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
            preset_overrides: None,
            delegation_policy: Default::default(),
            execution_model_pool: None,
            decision_policy: Default::default(),
            execution_template_id: None,
            extra,
        },
    })
}

fn exact_chat_route_from_revision(
    revision: &AgentPresetRevision,
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
        .ok_or_else(|| {
            unsupported(
                "model route",
                "agent_chat canonical route record is required",
            )
        })?;
    let identity = ChatRouteIdentity::new(
        revision.reference.revision_id(),
        CHAT_TASK,
        route_id.clone(),
        record.primary.model_route_revision,
    );
    record
        .validate_for(&identity)
        .map_err(|error| unsupported("model route", error.to_string()))?;
    Ok(record.clone())
}

fn revision_from_dto(
    revision: &nomifun_api_types::AgentPresetRevisionDto,
) -> Result<AgentPresetRevision, AppError> {
    let payload: AgentPresetRevisionPayload = wire_cast(&revision.document)?;
    let reference: PresetRevisionRef = wire_cast(&revision.reference)?;
    let created_by = UserId::parse(revision.created_by.clone())
        .map_err(|error| AppError::UnprocessableEntity(format!("invalid revision owner: {error}")))?;
    let revision = AgentPresetRevision {
        reference,
        payload,
        created_by: nomifun_agent_contracts::UserId::from(created_by.as_ref().to_owned()),
        created_at_ms: revision.created_at_ms,
        reason: revision.reason.clone(),
    };
    revision.validate().map_err(contract_error)?;
    Ok(revision)
}

fn wire_cast<T: Serialize, U: DeserializeOwned>(value: &T) -> Result<U, AppError> {
    serde_json::from_value(serde_json::to_value(value).map_err(|error| {
        AppError::Internal(format!("Nomi-core DTO serialization failed: {error}"))
    })?)
    .map_err(|error| AppError::UnprocessableEntity(format!("Nomi-core DTO conversion failed: {error}")))
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
    if input.binding.typed_resource_bindings != input.revision.payload.resource_bindings
        || input.binding.typed_resource_bindings
            != input.snapshot.content.typed_resource_bindings
    {
        return Err(AppError::Conflict(
            "AgentBindingValue resources do not exactly match the saved revision and snapshot"
                .into(),
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
struct ProjectedResources {
    workspace: Option<String>,
    knowledge_mounts: Vec<KnowledgeMountInfo>,
    mcp_server_ids: Vec<String>,
}

fn project_resources(
    owner: &UserId,
    binding: &AgentBindingValue,
    revision: &AgentPresetRevision,
) -> Result<ProjectedResources, AppError> {
    let selected = binding
        .typed_resource_bindings
        .iter()
        .map(|resource| (resource.binding_id.as_ref(), resource))
        .collect::<HashMap<_, _>>();
    let mut projected = ProjectedResources::default();
    for resource in &revision.payload.resource_bindings {
        if resource.owner_id != owner.as_ref() {
            return Err(AppError::Forbidden(format!(
                "typed resource binding {} is owned by a different authenticated owner",
                resource.binding_id.as_ref()
            )));
        }
        let Some(bound) = selected.get(resource.binding_id.as_ref()) else {
            return Err(AppError::Conflict(format!(
                "typed resource binding {} is not present in the exact binding",
                resource.binding_id.as_ref()
            )));
        };
        if *bound != resource {
            return Err(AppError::Conflict(format!(
                "typed resource binding {} differs from the saved revision",
                resource.binding_id.as_ref()
            )));
        }
        match resource.resource_kind.as_ref() {
            WORKSPACE_KIND => {
                let root = required_parameter(resource, WORKSPACE_ROOT)?;
                if projected.workspace.replace(root).is_some() {
                    return Err(unsupported(
                        "workspace resource",
                        "multiple workspace bindings cannot be projected to one Nomi conversation",
                    ));
                }
            }
            KNOWLEDGE_KIND => {
                let knowledge_base_id = nomifun_common::KnowledgeBaseId::parse(
                    resource.resource_id.as_ref().to_owned(),
                )
                .map_err(|error| unsupported("knowledge resource", error.to_string()))?;
                let name = resource
                    .typed_parameters
                    .get("knowledge_name")
                    .cloned()
                    .unwrap_or_else(|| resource.resource_id.as_ref().to_owned());
                let rel_path = resource
                    .typed_parameters
                    .get("knowledge_rel_path")
                    .cloned()
                    .unwrap_or_else(|| format!(".nomi/knowledge/{name}"));
                let root = required_parameter(resource, "knowledge_root")?;
                projected.knowledge_mounts.push(KnowledgeMountInfo {
                    knowledge_base_id,
                    name,
                    description: resource
                        .typed_parameters
                        .get("knowledge_description")
                        .cloned()
                        .unwrap_or_default(),
                    rel_path,
                    toc: Vec::new(),
                    summary: None,
                    live_sources: Vec::new(),
                });
                if projected.workspace.is_none() {
                    projected.workspace = Some(root);
                }
            }
            MCP_KIND => {
                validate_uuidv7(resource.resource_id.as_ref())
                    .map_err(|error| unsupported("MCP resource", error.to_string()))?;
                projected
                    .mcp_server_ids
                    .push(resource.resource_id.as_ref().to_owned());
            }
            other => {
                return Err(unsupported(
                    "typed resource",
                    format!("resource kind {other:?} has no Nomi-core projection"),
                ));
            }
        }
    }
    projected.mcp_server_ids.sort();
    projected.mcp_server_ids.dedup();
    Ok(projected)
}

fn required_parameter(
    resource: &TypedResourceBinding,
    key: &str,
) -> Result<String, AppError> {
    resource
        .typed_parameters
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            unsupported(
                "typed resource",
                format!(
                    "{} resource {} lacks required parameter {key:?}",
                    resource.resource_kind.as_ref(),
                    resource.binding_id.as_ref()
                ),
            )
        })
}

fn project_capabilities(revision: &AgentPresetRevision) -> Result<Vec<String>, AppError> {
    let mut tools = BTreeSet::new();
    // The current Nomi engine has no canonical deferred-capability activation
    // port.  Only the immutable initial set may therefore shape its native
    // tool allowlist.  Silently merging on-demand selections here would turn a
    // discoverable capability into an always-authorized tool.
    for selection in &revision.payload.initial_capabilities {
        let id = selection.capability.id.as_ref();
        let mapped = match id {
            "fs.read" => &["Read"][..],
            "fs.search" => &["Grep", "Glob"][..],
            "fs.write" | "fs.patch" | "fs.delete" => &["Write", "Edit", "ApplyPatch"][..],
            "process.exec" | "terminal.pty" => &["Bash", "exec_command", "write_stdin"][..],
            "process.session" => &["exec_command", "write_stdin"][..],
            "agent.execution.plan" => &["update_plan"][..],
            // VCS has a distinct native Nomi owner. Do not widen this family to
            // Bash: a VCS grant must not become a general shell grant.
            "vcs.status" => &["vcs.status"][..],
            "vcs.diff" => &["vcs.diff"][..],
            "vcs.stage" => &["vcs.stage"][..],
            "vcs.commit" => &["vcs.commit"][..],
            "vcs.push" => {
                return Err(unsupported(
                    "capability",
                    format!(
                        "{id:?} has no typed Nomi owner in the current Nomi runtime; \
                         external push remains unavailable until a credential-aware \
                         owner is commissioned"
                    ),
                ));
            }
            "knowledge.search" => &["knowledge_search"][..],
            "knowledge.read" => &["knowledge_read"][..],
            "knowledge.write" => &["knowledge_write"][..],
            "skill.invoke" => &["Skill"][..],
            "chat.basic" | "chat.minimal" => &[][..],
            other => {
                return Err(unsupported(
                    "capability",
                    format!(
                        "capability {other:?} cannot be represented by the current Nomi runtime"
                    ),
                ));
            }
        };
        tools.extend(mapped.iter().map(|tool| (*tool).to_owned()));
    }
    Ok(tools.into_iter().collect())
}

fn validate_unsupported_document_fields(
    revision: &AgentPresetRevision,
) -> Result<(), AppError> {
    if !revision.payload.system_role_provider_overrides.is_empty() {
        return Err(unsupported(
            "role provider override",
            "system role provider overrides are runtime-only and not projected",
        ));
    }
    let defaults = [
        (
            "context_policy",
            json!({
                "max_system_tokens": 12000,
                "max_dynamic_context_tokens": 16000,
                "max_catalog_tokens": 3000,
            }),
        ),
        (
            "execution_constraints",
            json!({
                "max_active_capabilities": 64,
                "max_advertised_tools": 48,
                "max_runtime_rebuilds": 4,
            }),
        ),
        (
            "runtime_budget",
            json!({
                "max_context_tokens": 32000,
                "max_tool_calls_per_turn": 64,
            }),
        ),
    ];
    for (name, value, default_value) in [
        (
            "context_policy",
            &revision.payload.context_policy.0,
            &defaults[0].1,
        ),
        (
            "execution_constraints",
            &revision.payload.execution_constraints.0,
            &defaults[1].1,
        ),
        ("runtime_budget", &revision.payload.runtime_budget.0, &defaults[2].1),
    ] {
        if value != default_value && !value.is_null() && !value.as_object().is_some_and(|object| object.is_empty()) {
            return Err(unsupported(
                "runtime-only document field",
                format!(
                    "{name} differs from the Nomi-core default and is not representable by the current Nomi request"
                ),
            ));
        }
    }
    Ok(())
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
        AgentPresetId, AgentPresetRevisionPayload, CapabilitySelection, CapabilityExposure,
        CapabilityRef, ChatRouteCandidate, ChatRouteFeature, ChatRouteProtocol,
        ChatRouteRecordSchema, ChatRouteTask, ConnectionConfigRef, DigestHex, ModelRouteId,
        OperationId, PresetRevisionRef, PrincipalRef, ResolvedCapability, ResolvedSnapshotContent,
        ResolvedSnapshotId, ResolvedSnapshotRef, RuntimeFeatureId, RuntimeProfileKind,
        SkillRef, StrictJsonValue,
    };
    use std::collections::{BTreeMap, BTreeSet};

    const DIGEST: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000003";
    const MCP: &str = "0190f5fe-7c00-7a00-8000-000000000004";
    const KB: &str = "0190f5fe-7c00-7a00-8000-000000000005";

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

    fn resource(
        binding_id: &str,
        kind: &str,
        resource_id: &str,
        parameters: BTreeMap<String, String>,
    ) -> TypedResourceBinding {
        TypedResourceBinding {
            binding_id: binding_id.into(),
            resource_kind: kind.into(),
            resource_id: resource_id.into(),
            owner_id: OWNER.into(),
            operations: BTreeSet::new(),
            connection_config_ref: None,
            typed_parameters: parameters,
        }
    }

    fn fixture() -> (
        UserId,
        AgentBindingValue,
        AgentPresetRevision,
        ResolvedSnapshotEnvelope,
    ) {
        let workspace = resource(
            "workspace-binding",
            WORKSPACE_KIND,
            "workspace-1",
            BTreeMap::from([(WORKSPACE_ROOT.into(), "C:\\work".into())]),
        );
        let knowledge = resource(
            "knowledge-binding",
            KNOWLEDGE_KIND,
            KB,
            BTreeMap::from([
                ("knowledge_root".into(), "C:\\work\\.nomi\\knowledge\\docs".into()),
                ("knowledge_name".into(), "docs".into()),
            ]),
        );
        let mcp = resource("mcp-binding", MCP_KIND, MCP, BTreeMap::new());
        let resources = vec![workspace, knowledge, mcp];
        let payload = AgentPresetRevisionPayload {
            schema_version: "1.0.0".into(),
            surfaces: BTreeSet::from(["desktop".into()]),
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
            resource_bindings: resources.clone(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "You are Nomi.".into(),
            instructions: "Be precise.".into(),
            context_policy: StrictJsonValue(json!({})),
            execution_constraints: StrictJsonValue(json!({})),
            runtime_budget: StrictJsonValue(json!({})),
        };
        let reference = PresetRevisionRef {
            preset_id: AgentPresetId::from("assistant.general"),
            revision: 1,
            revision_digest: nomifun_agent_contracts::digest_payload(&payload).unwrap(),
        };
        let revision = AgentPresetRevision {
            reference: reference.clone(),
            payload,
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
            typed_resource_bindings: resources.clone(),
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
            typed_resource_bindings: resources,
            binding_version: 1,
        };
        (
            UserId::parse(OWNER).expect("canonical owner UUIDv7"),
            binding,
            revision,
            snapshot,
        )
    }

    fn capability(id: &str, required: bool) -> CapabilitySelection {
        CapabilitySelection {
            capability: CapabilityRef {
                id: id.into(),
                version: "1.0.0".into(),
            },
            required,
            exposure: CapabilityExposure::Advertised,
            action_allowlist: BTreeSet::new(),
            resource_binding_refs: Vec::new(),
            destination_constraints: BTreeSet::new(),
            context_budget_override: None,
            tool_budget_override: None,
            config: StrictJsonValue(json!({})),
        }
    }

    fn resolved_capability(id: &str) -> ResolvedCapability {
        ResolvedCapability {
            capability: CapabilityRef {
                id: id.into(),
                version: "1.0.0".into(),
            },
            source_package: nomifun_agent_contracts::PackageRef {
                id: "test.package".into(),
                version: "1.0.0".into(),
            },
            schema_digest: DIGEST.into(),
            dependency_path: Vec::new(),
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

    #[test]
    fn exact_binding_projects_chat_route_and_resources() {
        let fixture = fixture();
        let result = project(input(&fixture)).unwrap();
        assert_eq!(result.snapshot.target, PresetTarget::Conversation);
        assert_eq!(
            result.snapshot.resolved_model.as_ref().unwrap().model,
            "step-3.7-flash"
        );
        assert_eq!(result.request.model.as_ref().unwrap().provider_id, OWNER);
        assert_eq!(result.request.extra["workspace"], "C:\\work");
        assert_eq!(result.request.extra["selected_mcp_server_ids"], json!([MCP]));
        assert_eq!(result.snapshot.knowledge_base_ids.len(), 1);
        assert!(result.request.extra["allowed_tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "Bash"));
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
    fn unsupported_required_capability_fails_closed() {
        let mut fixture = fixture();
        fixture
            .2
            .payload
            .initial_capabilities
            .push(capability("browser.navigate", true));
        fixture.2.reference.revision_digest =
            nomifun_agent_contracts::digest_payload(&fixture.2.payload).unwrap();
        fixture.3.content.preset_revision_ref = fixture.2.reference.clone();
        fixture.3.snapshot_ref.snapshot_digest =
            nomifun_agent_contracts::digest_payload(&fixture.3.content).unwrap();
        fixture.1.preset_revision_ref = fixture.2.reference.clone();
        fixture.1.resolved_snapshot_ref = fixture.3.snapshot_ref.clone();
        assert!(matches!(
            project(input(&fixture)),
            Err(AppError::UnprocessableEntity(message)) if message.contains("browser.navigate")
        ));
    }

    #[test]
    fn on_demand_capability_without_activation_port_fails_closed() {
        let mut fixture = fixture();
        fixture
            .2
            .payload
            .on_demand_capabilities
            .push(capability("fs.delete", false));
        let error = validate_on_demand_projection(&fixture.2)
            .expect_err("deferred capability activation must not be silently widened");
        assert!(error.to_string().contains("activation port"));
        assert!(error.to_string().contains("fs.delete"));
    }

    #[test]
    fn vcs_capabilities_use_dedicated_native_tool_names() {
        let mut fixture = fixture();
        fixture
            .2
            .payload
            .initial_capabilities
            .push(capability("vcs.status", true));
        fixture.2.reference.revision_digest =
            nomifun_agent_contracts::digest_payload(&fixture.2.payload).unwrap();
        fixture.3.content.preset_revision_ref = fixture.2.reference.clone();
        fixture.3.snapshot_ref.snapshot_digest =
            nomifun_agent_contracts::digest_payload(&fixture.3.content).unwrap();
        fixture.1.preset_revision_ref = fixture.2.reference.clone();
        fixture.1.resolved_snapshot_ref = fixture.3.snapshot_ref.clone();

        let result = project(input(&fixture)).expect("typed VCS projection");
        let tools = result.request.extra["allowed_tools"]
            .as_array()
            .expect("native tool allowlist");
        assert_eq!(
            tools.iter().filter(|tool| *tool == "vcs.status").count(),
            1
        );
    }
}
