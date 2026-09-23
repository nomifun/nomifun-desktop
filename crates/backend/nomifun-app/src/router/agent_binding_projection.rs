//! Source-neutral presentation projection for a canonical Agent binding.
//!
//! Capability execution is not projected here. The unified Driver compiles its
//! Tool/Context/Event plan from the exact ResolvedSnapshot and Kernel registry;
//! this module only produces legacy Conversation presentation fields while the
//! UI cutover is pending.

use std::collections::BTreeSet;

use nomifun_agent_contracts::{
    AgentBindingValue, AgentPresetRevision, ChatRouteIdentity, ChatRouteRecord,
    ResolvedSnapshotEnvelope,
};
use nomifun_api_types::{
    AgentKnowledgePolicy, AgentResolvedSnapshot, CreateConversationRequest, ExecutionModelRef,
};
use nomifun_common::{AgentType, AppError, ProviderWithModel, UserId};
use serde_json::json;

const CHAT_TASK: &str = nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT;

#[derive(Debug, Clone, Copy)]
pub struct ProjectionInput<'a> {
    pub owner: &'a UserId,
    pub binding: &'a AgentBindingValue,
    pub revision: &'a AgentPresetRevision,
    pub snapshot: &'a ResolvedSnapshotEnvelope,
    pub title: Option<&'a str>,
}

#[derive(Debug)]
pub struct AgentBindingProjection {
    pub snapshot: AgentResolvedSnapshot,
    pub request: CreateConversationRequest,
}

#[derive(Debug)]
pub struct SavedAgentBindingProjection {
    pub binding: AgentBindingValue,
    pub snapshot: ResolvedSnapshotEnvelope,
    pub runtime_policy: nomifun_agent_contracts::AgentRuntimePolicy,
    pub projection: AgentBindingProjection,
}

pub fn project_saved_artifacts(
    owner: &UserId,
    binding: AgentBindingValue,
    revision: AgentPresetRevision,
    snapshot: ResolvedSnapshotEnvelope,
    title: Option<&str>,
) -> Result<SavedAgentBindingProjection, AppError> {
    let runtime_policy = revision.payload.runtime_policy.clone();
    let projection = project(ProjectionInput {
        owner,
        binding: &binding,
        revision: &revision,
        snapshot: &snapshot,
        title,
    })?;
    Ok(SavedAgentBindingProjection {
        binding,
        snapshot,
        runtime_policy,
        projection,
    })
}

pub fn project(input: ProjectionInput<'_>) -> Result<AgentBindingProjection, AppError> {
    validate_identity_chain(&input)?;
    let route = exact_chat_route(input.revision, input.snapshot)?;
    let instructions = merge_instructions(
        &input.revision.payload.persona,
        &input.revision.payload.instructions,
    );
    let title = input
        .title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(input.revision.reference.preset_id.as_ref())
        .to_owned();
    let revision = i64::try_from(input.revision.reference.revision).map_err(|_| {
        AppError::UnprocessableEntity("preset revision does not fit the UI integer field".into())
    })?;
    let enabled_capabilities = input
        .snapshot
        .content
        .contributions()
        .map(|capability| capability.capability.id.as_ref().to_owned())
        .collect::<Vec<_>>();
    let enabled_capability_actions = input
        .snapshot
        .content
        .contributions()
        .map(|capability| {
            (
                capability.capability.id.as_ref().to_owned(),
                capability
                    .action_allowlist
                    .iter()
                    .map(|action| action.as_ref().to_owned())
                    .collect(),
            )
        })
        .collect();
    let required_resource_kinds = input
        .snapshot
        .content
        .required_resource_kinds
        .iter()
        .map(|kind| kind.as_ref().to_owned())
        .collect::<BTreeSet<_>>();
    let mut knowledge_binding_policy = None;
    let mut knowledge_enabled = false;
    for resource in input.binding.typed_resource_bindings.iter().filter(|binding| {
        binding.resource_kind.as_ref()
            == nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
    }) {
        let enabled = nomifun_agent_domain_wave1::agent_knowledge_enabled(resource)
            .map_err(|reason| AppError::Conflict(reason))?;
        if knowledge_binding_policy.is_some() && knowledge_enabled != enabled {
            return Err(AppError::Conflict(
                "Knowledge resources carry inconsistent enabled policies".into(),
            ));
        }
        knowledge_enabled = enabled;
        let policy = nomifun_agent_domain_wave1::agent_knowledge_writeback_policy(resource)
            .map_err(|reason| AppError::Conflict(reason))?;
        if knowledge_binding_policy.is_some_and(|expected| expected != policy) {
            return Err(AppError::Conflict(
                "Knowledge resources carry inconsistent write-back policies".into(),
            ));
        }
        knowledge_binding_policy = Some(policy);
    }
    let (knowledge_writeback, knowledge_eagerness) =
        knowledge_binding_policy.unwrap_or((false, "manual"));
    let included_skills = input
        .snapshot
        .content
        .skill_locks
        .iter()
        .map(|lock| lock.skill.id.as_ref().to_owned())
        .collect();
    let resolved_model = route.as_ref().map(|route| ExecutionModelRef {
        provider_id: route.primary.provider_id.clone(),
        model: route.primary.model.clone(),
    });
    let canonical_binding = Some(
        serde_json::to_value(input.binding)
            .and_then(serde_json::from_value)
            .map_err(|error| {
                AppError::Internal(format!("Agent binding projection failed: {error}"))
            })?,
    );
    let snapshot = AgentResolvedSnapshot {
        canonical_binding,
        preset_id: input.revision.reference.preset_id.as_ref().to_owned(),
        preset_revision: revision,
        preset_name: title.clone(),
        routing_description: None,
        instructions: instructions.clone(),
        resolved_agent_id: None,
        resolved_agent_type: Some(AgentType::Nomi.serde_name().to_owned()),
        resolved_agent_backend: Some("nomi".to_owned()),
        resolved_model,
        included_skills,
        excluded_auto_skills: Vec::new(),
        enabled_capabilities,
        enabled_capability_actions,
        required_resource_kinds,
        knowledge_policy: AgentKnowledgePolicy {
            enabled: knowledge_enabled,
            writeback: knowledge_enabled && knowledge_writeback,
            eagerness: (knowledge_enabled && knowledge_writeback)
                .then(|| knowledge_eagerness.to_owned()),
            grounded: knowledge_enabled,
        },
        warnings: Vec::new(),
    };
    let extra = json!({
        "system_prompt": instructions,
        "chat_config_revision_digest": route
            .as_ref()
            .map(|route| &route.primary.config_revision_digest),
        // Presentation-only deny-all fields for the retiring Conversation
        // adapter. The unified Driver builds its plan from the Snapshot.
        "allowed_tools": [],
        "enforce_tool_allowlist": true,
        "deferred_tools": [],
    });
    Ok(AgentBindingProjection {
        snapshot,
        request: CreateConversationRequest {
            r#type: AgentType::Nomi,
            name: Some(title),
            model: route.map(|route| ProviderWithModel {
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
    if input.binding.preset_revision_ref != input.revision.reference
        || input.snapshot.content.preset_revision_ref != input.revision.reference
    {
        return Err(AppError::Conflict(
            "Agent binding, Revision and Snapshot revision identities differ".into(),
        ));
    }
    if input.binding.resolved_snapshot_ref != input.snapshot.snapshot_ref {
        return Err(AppError::Conflict(
            "Agent binding and saved Snapshot identities differ".into(),
        ));
    }
    Ok(())
}

fn contract_error(error: nomifun_agent_contracts::PresetContractViolation) -> AppError {
    AppError::UnprocessableEntity(format!("{}: {}", error.code.as_ref(), error.message))
}

fn exact_chat_route(
    revision: &AgentPresetRevision,
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<Option<ChatRouteRecord>, AppError> {
    if !revision.payload.model_route_refs.contains_key(CHAT_TASK)
        && !revision.payload.chat_route_records.contains_key(CHAT_TASK)
    {
        if snapshot.content.chat_route_identity.is_some()
            || snapshot.content.model_route_refs.contains_key(CHAT_TASK)
        {
            return Err(AppError::Conflict(
                "task-only Agent snapshot unexpectedly contains a Chat route".into(),
            ));
        }
        return Ok(None);
    }
    let route_id = revision
        .payload
        .model_route_refs
        .get(CHAT_TASK)
        .ok_or_else(|| unsupported("model route", "agent_chat route is required"))?;
    let record = revision
        .payload
        .chat_route_records
        .get(CHAT_TASK)
        .ok_or_else(|| unsupported("model route", "agent_chat route record is required"))?;
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
            "Snapshot chat route identity does not match the Revision".into(),
        ));
    }
    Ok(Some(record.clone()))
}

/// Availability is structural: an Agent consumer must publish an actual
/// Action, Context, or Event contribution. No capability ID table participates.
pub(crate) fn native_capability_available(
    manifest: &nomifun_agent_contracts::CapabilityManifest,
) -> bool {
    manifest.validate_module_contract().is_ok()
        && manifest.supports_consumer(nomifun_agent_contracts::CapabilityConsumer::Agent)
        && (manifest.declares_actions()
            || manifest.contributes_context()
            || manifest.contributes_events())
}

/// Retained call-site name is removed: validation is now the generic Revision
/// contract only. Kernel compilation owns contribution/action availability.
pub(crate) fn validate_agent_revision_projection(
    revision: &AgentPresetRevision,
) -> Result<(), AppError> {
    revision.validate().map_err(contract_error)
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
    use nomifun_agent_contracts::*;
    use std::collections::{BTreeMap, BTreeSet};

    const DIGEST: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000003";

    fn route() -> ChatRouteRecord {
        ChatRouteRecord {
            schema: ChatRouteRecordSchema::V1,
            task: ChatRouteTask::AgentChat,
            primary: ChatRouteCandidate {
                model_route_id: ModelRouteId::from("route-1"),
                model_route_revision: 1,
                provider_id: OWNER.to_owned(),
                model: "model".to_owned(),
                protocol: ChatRouteProtocol::OpenaiChat,
                connection_config_ref: ConnectionConfigRef::from("config-1"),
                config_revision_digest: DigestHex::from(DIGEST),
                credential_ref: "credential-1".to_owned(),
                features: BTreeSet::from([ChatRouteFeature::TextOutput]),
                activation_features: BTreeSet::new(),
            },
            failovers: Vec::new(),
        }
    }

    fn resolved_module(id: &str) -> ResolvedCapability {
        let capability_id = CapabilityId::from(id);
        let contribution_id = ContributionId::from(format!("capability:{id}"));
        ResolvedCapability {
            consumption: CapabilityConsumption::Contribution,
            dependency_refs: Vec::new(),
            capability: CapabilityRef {
                id: capability_id.clone(),
            },
            source_package: PackageRef {
                id: "future.package".into(),
                version: "1.0.0".into(),
            },
            contribution_id: contribution_id.clone(),
            contribution_lock: ContributionLock {
                source_kind: ContributionSourceKind::PlatformBuiltin,
                source_identity: StableSourceIdentity::from("future.package"),
                mount_id: None,
                mcp_binding_id: None,
                contribution_id,
                contract_digest: DIGEST.into(),
            },
            resolved_mount_id: Some("future-mount".into()),
            resolved_source: PluginSourceMetadata {
                source_kind: PluginSourceKind::Bundled,
                source_identity: "future.package".into(),
                source_digest: Some(DIGEST.into()),
            },
            target_artifact_digest: DIGEST.into(),
            schema_digest: DIGEST.into(),
            dependency_path: vec![capability_id],
            required_runtime_features: BTreeSet::new(),
            display_name: Some("Future module".into()),
            description: None,
            actions: Vec::new(),
            required_resource_kinds: BTreeSet::from(["workspace".into()]),
            action_allowlist: BTreeSet::new(),
        }
    }

    fn fixture() -> (
        nomifun_common::UserId,
        AgentBindingValue,
        AgentPresetRevision,
        ResolvedSnapshotEnvelope,
    ) {
        let capability = CapabilityRef {
            id: "future.module".into(),
        };
        let payload = AgentPresetRevisionPayload {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: "1.0.0".into(),
            model_route_refs: BTreeMap::from([(CHAT_TASK.into(), "route-1".into())]),
            chat_route_records: BTreeMap::from([(CHAT_TASK.into(), route())]),
            enabled_capabilities: vec![CapabilitySelection {
                capability,
                action_allowlist: BTreeSet::new(),
            }],
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "Persona".into(),
            instructions: "Instructions".into(),
            starter_prompts: Vec::new(),
            runtime_policy: Default::default(),
        };
        let reference = PresetRevisionRef {
            preset_id: "preset".into(),
            revision: 1,
            revision_digest: digest_payload(&AgentPresetRevisionDigestInput {
                payload: payload.clone(),
                contribution_locks: Vec::new(),
            })
            .unwrap(),
        };
        let revision = AgentPresetRevision {
            reference: reference.clone(),
            payload,
            contribution_locks: Vec::new(),
            created_by: OWNER.into(),
            created_at_ms: 1,
            reason: None,
        };
        let content = ResolvedSnapshotContent {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: "1.0.0".into(),
            resolver_version: "1.0.0".into(),
            preset_revision_ref: reference.clone(),
            required_runtime_protocol_version: "1.0.0".into(),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: DIGEST.into(),
            required_runtime_features: BTreeSet::new(),
            compiled_runtime_profile_digest: DIGEST.into(),
            model_route_refs: revision.payload.model_route_refs.clone(),
            chat_route_identity: Some(
                revision.payload.chat_route_records[CHAT_TASK]
                    .identity_for(reference.revision_id(), CHAT_TASK)
                    .unwrap(),
            ),
            enabled_capabilities: vec![resolved_module("future.module")],
            required_resource_kinds: BTreeSet::from(["workspace".into()]),
            capability_allowlist: BTreeSet::from(["future.module".into()]),
            skill_locks: Vec::new(),
            mcp_tool_locks: Vec::new(),
            resolved_role_providers: BTreeMap::new(),
            canonical_schema_manifest_digest: DIGEST.into(),
            target_contribution_manifest_digest: DIGEST.into(),
        };
        let snapshot_ref = ResolvedSnapshotRef {
            snapshot_id: "snapshot".into(),
            snapshot_digest: digest_payload(&content).unwrap(),
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
            resolver_run_id: "run".into(),
            availability_evidence_revision: "evidence".into(),
        };
        let binding = AgentBindingValue {
            preset_revision_ref: reference,
            resolved_snapshot_ref: snapshot_ref,
            typed_resource_bindings: Vec::new(),
            binding_version: 1,
        };
        (
            nomifun_common::UserId::parse(OWNER).unwrap(),
            binding,
            revision,
            snapshot,
        )
    }

    fn input<'a>(
        fixture: &'a (
            nomifun_common::UserId,
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
            title: Some("Future"),
        }
    }

    #[test]
    fn arbitrary_module_projects_from_snapshot_without_an_id_mapping() {
        let projection = project(input(&fixture())).unwrap();
        assert_eq!(
            projection.snapshot.enabled_capabilities,
            vec!["future.module"]
        );
        assert_eq!(projection.request.extra["allowed_tools"], json!([]));
        for retired in ["browser_use", "computer_use", "runtime_profile", "mcp_capabilities"] {
            assert!(projection.request.extra.get(retired).is_none(), "{retired}");
        }
    }

    #[test]
    fn identity_mismatch_is_rejected_before_projection() {
        let mut fixture = fixture();
        fixture.1.binding_version += 1;
        fixture.1.resolved_snapshot_ref.snapshot_digest = "f".repeat(64).into();
        assert!(project(input(&fixture)).is_err());
    }

    #[test]
    fn task_only_agent_needs_no_chat_route() {
        let mut fixture = fixture();
        fixture.2.payload.model_route_refs.clear();
        fixture.2.payload.chat_route_records.clear();
        fixture.2.reference.revision_digest = fixture.2.revision_digest().unwrap();
        fixture.3.content.preset_revision_ref = fixture.2.reference.clone();
        fixture.3.content.model_route_refs.clear();
        fixture.3.content.chat_route_identity = None;
        fixture.3.snapshot_ref.snapshot_digest = digest_payload(&fixture.3.content).unwrap();
        fixture.1.preset_revision_ref = fixture.2.reference.clone();
        fixture.1.resolved_snapshot_ref = fixture.3.snapshot_ref.clone();
        let projection = project(input(&fixture)).unwrap();
        assert!(projection.request.model.is_none());
    }

    #[test]
    fn availability_depends_on_contributions_not_capability_id() {
        let action = CapabilityActionDescriptor {
            action_id: "future.invoke".into(),
            input_schema: "schema://future/input@1".into(),
            output_schema: "schema://future/output@1".into(),
            effect_class: EffectClass::Pure,
            presentation: ToolPresentationKind::FunctionTool,
        };
        let mut manifest = CapabilityManifest {
            id: "never-seen-before".into(),
            contribution_id: "capability:never-seen-before".into(),
            kind: CapabilityKind::Tool,
            package: PackageRef {
                id: "future.package".into(),
                version: "1.0.0".into(),
            },
            display: LocalizedMetadata {
                name: "Future".into(),
                description: "Future".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces: capability_surface_declarations(
                ["desktop"],
                [CapabilityConsumer::Agent],
            ),
            requires_runtime_features: Vec::new(),
            supported_platforms: vec![PlatformConstraint::Any],
            config_schema: StrictJsonValue(json!({"type":"object"})),
            contributions: CapabilityContributions {
                actions: vec![action],
                ..Default::default()
            },
        };
        assert!(native_capability_available(&manifest));
        manifest.contributions = CapabilityContributions::default();
        assert!(!native_capability_available(&manifest));
    }
}
