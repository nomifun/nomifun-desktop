//! Bundled Wave 1 read-capability registrations.
//!
//! The six package identities and 25 capabilities below are the Wave 1 slice
//! of the frozen first-party contribution inventory.  Customer-service
//! dialogue/identity is owned by Wave 4 and is intentionally absent here.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, ArtifactEnvelope, CapabilityActionDescriptor, CapabilityContributions,
    CapabilityId, CapabilityKind, CapabilityManifest, CancellationDescriptor,
    CanonicalSchemaRef, DeclaredServiceViewDescriptor, EffectClass,
    InProcessEntrypointMetadata, LocalizedMetadata, ManagedTaskRegistrationDescriptor,
    PackageContributions, PackageId, PackageManifest, PackageRef, PlatformConstraint,
    PluginBootCriticality, PluginBootState, PluginContextDescriptor, PluginDesiredState,
    PluginEffectiveState, PluginIdentityDescriptor, PluginMountId, PluginRegistrarDescriptor,
    PluginRegistrarOperation, PluginRegistrationMetadata, PluginSourceKind,
    PluginSourceMetadata, PluginStateHandleDescriptor, PluginStateMethod, ResourceKind,
    SkillDefinition, StrictJsonValue, ToolPresentationKind, ValidatedPluginConfig,
    VersionString, digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, KernelError, PluginRegistration,
};
use serde_json::{Value, json};

pub const CONTRACT_VERSION: &str = "1.0.0";

/// Bundled package identities owned by Wave 1.
pub const PACKAGE_IDS: [&str; 6] = [
    "nomifun.web-research",
    "nomifun.chat",
    "nomifun.knowledge",
    "nomifun.project-memory",
    "nomifun.companion-memory",
    "nomifun.skills",
];

/// Capability IDs present in the six Wave 1 packages.
pub const CAPABILITY_IDS: [&str; 25] = [
    "web.search",
    "web.fetch",
    "citation.render",
    "session.attachments.read",
    "knowledge.search",
    "knowledge.read",
    "knowledge.write",
    "knowledge.mount",
    "knowledge.source.sync",
    "knowledge.autogen",
    "knowledge.embedding",
    "knowledge.rerank",
    "memory.project.read",
    "memory.project.write",
    "memory.project.distill",
    "memory.project.citation",
    "memory.session.scratch",
    "memory.companion.recall",
    "memory.companion.write",
    "memory.companion.merge",
    "memory.companion.evolve",
    "skill.catalog",
    "skill.describe",
    "skill.invoke",
    "skill.hooks",
];

const SURFACES: &[&str] = &["desktop", "headless", "remote", "web"];
const KNOWLEDGE: &[&str] = &["knowledge_base"];
const PROJECT_MEMORY: &[&str] = &["project_memory"];
const COMPANION_MEMORY: &[&str] = &["companion_memory"];

#[derive(Clone, Copy)]
struct CapabilitySpec {
    id: &'static str,
    kind: CapabilityKind,
    effect: Option<EffectClass>,
    resources: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct PackageSpec {
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    mount_id: &'static str,
    capabilities: &'static [CapabilitySpec],
}

const WEB_RESEARCH: &[CapabilitySpec] = &[
    tool("web.search", EffectClass::ExternalTransmit, &[]),
    tool("web.fetch", EffectClass::ExternalTransmit, &[]),
    context("citation.render", &[]),
];

const CHAT: &[CapabilitySpec] = &[context("session.attachments.read", &[])];

const KNOWLEDGE_CAPABILITIES: &[CapabilitySpec] = &[
    tool("knowledge.search", EffectClass::ReadSensitive, KNOWLEDGE),
    tool("knowledge.read", EffectClass::ReadSensitive, KNOWLEDGE),
    tool("knowledge.write", EffectClass::WriteDurable, KNOWLEDGE),
    resource_provider("knowledge.mount", KNOWLEDGE),
    background("knowledge.source.sync"),
    tool("knowledge.autogen", EffectClass::WriteDurable, KNOWLEDGE),
    tool("knowledge.embedding", EffectClass::ReadSensitive, KNOWLEDGE),
    tool("knowledge.rerank", EffectClass::ReadSensitive, KNOWLEDGE),
];

const PROJECT_MEMORY_CAPABILITIES: &[CapabilitySpec] = &[
    context("memory.project.read", PROJECT_MEMORY),
    tool(
        "memory.project.write",
        EffectClass::WriteDurable,
        PROJECT_MEMORY,
    ),
    tool(
        "memory.project.distill",
        EffectClass::WriteDurable,
        PROJECT_MEMORY,
    ),
    context("memory.project.citation", PROJECT_MEMORY),
    resource_provider("memory.session.scratch", PROJECT_MEMORY),
];

const COMPANION_MEMORY_CAPABILITIES: &[CapabilitySpec] = &[
    context("memory.companion.recall", COMPANION_MEMORY),
    tool(
        "memory.companion.write",
        EffectClass::WriteDurable,
        COMPANION_MEMORY,
    ),
    tool(
        "memory.companion.merge",
        EffectClass::WriteDurable,
        COMPANION_MEMORY,
    ),
    tool(
        "memory.companion.evolve",
        EffectClass::WriteDurable,
        COMPANION_MEMORY,
    ),
];

const SKILLS: &[CapabilitySpec] = &[
    context("skill.catalog", &[]),
    context("skill.describe", &[]),
    tool("skill.invoke", EffectClass::ExecuteLocal, &[]),
    middleware("skill.hooks"),
];

const PACKAGES: &[PackageSpec] = &[
    PackageSpec {
        id: "nomifun.web-research",
        display_name: "Web Research",
        description: "Search and fetch web sources for an AgentSession.",
        mount_id: "domain-web-research",
        capabilities: WEB_RESEARCH,
    },
    PackageSpec {
        id: "nomifun.chat",
        display_name: "Chat Attachments",
        description: "Read attachment context for a chat Session.",
        mount_id: "domain-chat",
        capabilities: CHAT,
    },
    PackageSpec {
        id: "nomifun.knowledge",
        display_name: "Knowledge",
        description: "Search, read, and maintain owned knowledge bases.",
        mount_id: "domain-knowledge",
        capabilities: KNOWLEDGE_CAPABILITIES,
    },
    PackageSpec {
        id: "nomifun.project-memory",
        display_name: "Project Memory",
        description: "Read and maintain project-scoped memory.",
        mount_id: "domain-project-memory",
        capabilities: PROJECT_MEMORY_CAPABILITIES,
    },
    PackageSpec {
        id: "nomifun.companion-memory",
        display_name: "Companion Memory",
        description: "Read and maintain explicitly bound companion memory.",
        mount_id: "domain-companion-memory",
        capabilities: COMPANION_MEMORY_CAPABILITIES,
    },
    PackageSpec {
        id: "nomifun.skills",
        display_name: "Skills",
        description: "Expose catalogued skill guidance and invocation.",
        mount_id: "domain-skills",
        capabilities: SKILLS,
    },
];

const fn tool(
    id: &'static str,
    effect: EffectClass,
    resources: &'static [&'static str],
) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::Tool,
        effect: Some(effect),
        resources,
    }
}

const fn context(id: &'static str, resources: &'static [&'static str]) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::ContextContributor,
        effect: None,
        resources,
    }
}

const fn resource_provider(
    id: &'static str,
    resources: &'static [&'static str],
) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::ResourceProvider,
        effect: None,
        resources,
    }
}

const fn background(id: &'static str) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::BackgroundService,
        effect: None,
        resources: &[],
    }
}

const fn middleware(id: &'static str) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::TurnMiddleware,
        effect: None,
        resources: &[],
    }
}

/// Build the six trusted Wave 1 registrations.
pub fn registrations() -> Result<Vec<PluginRegistration>, String> {
    let mut package_ids = BTreeSet::new();
    let mut declared_capability_ids = BTreeSet::new();
    let mut output = Vec::with_capacity(PACKAGES.len());

    for package in PACKAGES {
        if !package_ids.insert(package.id) {
            return Err(format!("duplicate Wave 1 package {}", package.id));
        }
        let registration = registration_for(package)?;
        for capability in &registration.metadata.manifest.payload.contributions.capabilities {
            if !declared_capability_ids.insert(capability.id.clone()) {
                return Err(format!(
                    "duplicate Wave 1 capability {}",
                    capability.id.as_ref()
                ));
            }
        }
        output.push(registration);
    }

    let expected = capability_ids();
    if declared_capability_ids != expected {
        return Err(format!(
            "Wave 1 capability inventory mismatch: expected {}, got {}",
            expected.len(),
            declared_capability_ids.len()
        ));
    }
    Ok(output)
}

/// Return the exact Wave 1 capability ID set.
pub fn capability_ids() -> BTreeSet<CapabilityId> {
    CAPABILITY_IDS
        .iter()
        .map(|id| CapabilityId::from(*id))
        .collect()
}

/// Return the typed resource kinds declared by a known capability.
pub fn required_resource_kinds(capability_id: &str) -> Option<BTreeSet<ResourceKind>> {
    PACKAGES
        .iter()
        .flat_map(|package| package.capabilities.iter())
        .find(|capability| capability.id == capability_id)
        .map(|capability| {
            capability
                .resources
                .iter()
                .map(|resource| ResourceKind::from(*resource))
                .collect()
        })
}

fn registration_for(spec: &PackageSpec) -> Result<PluginRegistration, String> {
    let package = PackageRef {
        id: PackageId::from(spec.id),
        version: VersionString::from(CONTRACT_VERSION),
    };
    let config_schema = object_schema(false);
    let capabilities = spec
        .capabilities
        .iter()
        .map(|capability| capability_manifest(capability, &package))
        .collect::<Result<Vec<_>, _>>()?;
    let manifest = PackageManifest {
        schema_version: VersionString::from(CONTRACT_VERSION),
        host_contract_version: VersionString::from(CONTRACT_VERSION),
        package_id: package.id.clone(),
        package_version: package.version.clone(),
        display: display(spec.display_name, spec.description),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: StrictJsonValue(config_schema.clone()),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: format!("{}.entrypoint", spec.id),
            contract_version: VersionString::from(CONTRACT_VERSION),
        },
        contributions: PackageContributions {
            capabilities,
            skills: Vec::<SkillDefinition>::new(),
            mcp_tools: Vec::new(),
        },
    };

    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::Bundled,
        source_identity: spec.id.to_owned(),
        source_digest: None,
    };
    let identity = PluginIdentityDescriptor {
        package: package.clone(),
        mount_id: PluginMountId::from(spec.mount_id),
    };
    let cancellation_port = host_port("host.plugin.cancel");
    let task_port = host_port("host.plugin.tasks");
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest).map_err(|error| error.to_string())?,
        mount_id: identity.mount_id.clone(),
        source: source.clone(),
        boot_state: PluginBootState {
            criticality: PluginBootCriticality::Required,
            desired_state: PluginDesiredState::Enabled,
            effective_state: PluginEffectiveState::Active,
            diagnostic_code: None,
        },
        registrar: PluginRegistrarDescriptor {
            identity: identity.clone(),
            allowed_operations: BTreeSet::from([
                PluginRegistrarOperation::BindHostPort,
                PluginRegistrarOperation::ContributeCapability,
            ]),
            declared_capability_ids: spec
                .capabilities
                .iter()
                .map(|capability| CapabilityId::from(capability.id))
                .collect(),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports: BTreeSet::from([
                cancellation_port.id.clone(),
                task_port.id.clone(),
            ]),
        },
        context: PluginContextDescriptor {
            identity,
            source,
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&StrictJsonValue(config_schema))
                    .map_err(|error| error.to_string())?,
                config_revision: 1,
                value: StrictJsonValue(json!({})),
            },
            state: PluginStateHandleDescriptor {
                package_id: package.id,
                mount_id: PluginMountId::from(spec.mount_id),
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: DeclaredServiceViewDescriptor::default(),
            host_ports: Vec::new(),
            typed_command_ports: Vec::new(),
            domain_outbox_ports: Vec::new(),
            cancellation: CancellationDescriptor {
                cancellation_port,
                scope_key: nomifun_agent_contracts::ScopeKey::from(format!(
                    "mount:{}",
                    spec.mount_id
                )),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: task_port,
                scope_key: nomifun_agent_contracts::ScopeKey::from(format!(
                    "mount:{}",
                    spec.mount_id
                )),
            },
        },
    };

    let mut registration = PluginRegistration::new(metadata);
    for capability in spec.capabilities.iter().filter(|capability| {
        capability.kind == CapabilityKind::Tool && capability.effect.is_some()
    }) {
        registration
            .add_capability_handler(
                CapabilityId::from(capability.id),
                Arc::new(DeterministicHandler {
                    capability_id: CapabilityId::from(capability.id),
                    action_id: action_id(capability.id),
                    resources: capability
                        .resources
                        .iter()
                        .map(|resource| ResourceKind::from(*resource))
                        .collect(),
                }),
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(registration)
}

fn capability_manifest(
    spec: &CapabilitySpec,
    package: &PackageRef,
) -> Result<CapabilityManifest, String> {
    let contributions = match spec.kind {
        CapabilityKind::Tool => {
            let input = tool_input_schema();
            let output = tool_output_schema();
            CapabilityContributions {
                actions: vec![CapabilityActionDescriptor {
                    action_id: action_id(spec.id),
                    input_schema: schema_ref(spec.id, "input", &input)?,
                    output_schema: schema_ref(spec.id, "output", &output)?,
                    effect_class: spec.effect.expect("tool effect"),
                    presentation: ToolPresentationKind::FunctionTool,
                }],
                resource_kinds: resource_kinds(spec),
                ..CapabilityContributions::default()
            }
        }
        CapabilityKind::ContextContributor | CapabilityKind::TurnMiddleware => {
            let schema = context_schema();
            CapabilityContributions {
                context_schema_refs: vec![schema_ref(spec.id, "context", &schema)?],
                resource_kinds: resource_kinds(spec),
                ..CapabilityContributions::default()
            }
        }
        CapabilityKind::EventSource
        | CapabilityKind::EventConsumer
        | CapabilityKind::BackgroundService => {
            let schema = event_schema();
            CapabilityContributions {
                event_schema_refs: vec![schema_ref(spec.id, "event", &schema)?],
                resource_kinds: resource_kinds(spec),
                ..CapabilityContributions::default()
            }
        }
        _ => CapabilityContributions {
            resource_kinds: resource_kinds(spec),
            ..CapabilityContributions::default()
        },
    };
    Ok(CapabilityManifest {
        id: CapabilityId::from(spec.id),
        version: VersionString::from(CONTRACT_VERSION),
        kind: spec.kind,
        package: package.clone(),
        display: display(spec.id, "Bundled Wave 1 capability."),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: SURFACES.iter().map(|surface| (*surface).to_owned()).collect(),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(object_schema(false)),
        contributions,
    })
}

struct DeterministicHandler {
    capability_id: CapabilityId,
    action_id: ActionId,
    resources: BTreeSet<ResourceKind>,
}

#[async_trait]
impl CapabilityHandler for DeterministicHandler {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        if context.capability_id != self.capability_id || context.action_id != self.action_id {
            return Err(KernelError::ActionNotDeclared {
                capability_id: context.capability_id,
                action_id: context.action_id,
            });
        }
        if !input.0.is_object() {
            return Err(KernelError::CapabilityExecution {
                reason: format!("{} input must be an object", self.capability_id.as_ref()),
            });
        }
        for resource_kind in &self.resources {
            if !context
                .resource_bindings
                .iter()
                .any(|binding| binding.resource_kind == *resource_kind)
            {
                return Err(KernelError::CapabilityResourceNotBound {
                    capability_id: self.capability_id.clone(),
                    resource_kind: resource_kind.as_ref().to_owned(),
                });
            }
        }
        let mut resource_binding_ids = context
            .resource_bindings
            .iter()
            .map(|binding| binding.binding_id.as_ref().to_owned())
            .collect::<Vec<_>>();
        resource_binding_ids.sort();
        Ok(StrictJsonValue(json!({
            "accepted": true,
            "capability_id": self.capability_id.as_ref(),
            "action_id": self.action_id.as_ref(),
            "resource_binding_ids": resource_binding_ids,
            "input": input.0
        })))
    }
}

fn action_id(capability_id: &str) -> ActionId {
    ActionId::from(format!("{capability_id}.invoke"))
}

fn resource_kinds(spec: &CapabilitySpec) -> BTreeSet<ResourceKind> {
    spec.resources
        .iter()
        .map(|resource| ResourceKind::from(*resource))
        .collect()
}

fn schema_ref(
    capability_id: &str,
    facet: &str,
    schema: &Value,
) -> Result<CanonicalSchemaRef, String> {
    let digest = digest_payload(schema).map_err(|error| error.to_string())?;
    Ok(CanonicalSchemaRef::from(format!(
        "schema://{capability_id}/{facet}@1#{}",
        digest.as_ref()
    )))
}

fn object_schema(additional_properties: bool) -> Value {
    json!({
        "type": "object",
        "additionalProperties": additional_properties
    })
}

fn tool_input_schema() -> Value {
    object_schema(true)
}

fn tool_output_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "accepted": {"type": "boolean"},
            "capability_id": {"type": "string"},
            "action_id": {"type": "string"},
            "resource_binding_ids": {
                "type": "array",
                "items": {"type": "string"}
            },
            "input": {"type": "object"}
        },
        "required": [
            "accepted",
            "capability_id",
            "action_id",
            "resource_binding_ids",
            "input"
        ]
    })
}

fn context_schema() -> Value {
    object_schema(true)
}

fn event_schema() -> Value {
    object_schema(true)
}

fn display(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

fn host_port(id: &str) -> nomifun_agent_contracts::HostPortRef {
    nomifun_agent_contracts::HostPortRef {
        id: nomifun_agent_contracts::HostPortId::from(id),
        version: VersionString::from(CONTRACT_VERSION),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    use super::*;
    use nomifun_agent_contracts::{
        AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload, CapabilityExposure,
        CapabilityRef, CapabilitySelection, DigestHex, OperationId, PresetRevisionRef,
        PrincipalRef, ResourceBindingId, RuntimeProfileKind, RuntimeTarget, ScopeKey,
        TypedResourceBinding, UserId,
    };
    use nomifun_agent_kernel::{
        AgentPresetCompiler, CapabilityInvocationRequest, CompileRequest, CompilerEnvironment,
        InMemoryPluginStatePersistence, KernelRegistry, MaterializationPolicy, Materializer,
        SessionCapabilityState,
    };

    fn principal() -> PrincipalRef {
        PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "wave1-test-owner".to_owned(),
        }
    }

    #[test]
    fn registrations_match_the_frozen_wave1_slice() {
        let registrations = registrations().expect("Wave 1 registrations");
        let packages = registrations
            .iter()
            .map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .package_id
                    .as_ref()
                    .to_owned()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            packages,
            PACKAGE_IDS
                .iter()
                .map(|id| (*id).to_owned())
                .collect::<BTreeSet<_>>()
        );

        let capabilities = registrations
            .iter()
            .flat_map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .contributions
                    .capabilities
                    .iter()
                    .map(|capability| capability.id.clone())
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(capabilities, capability_ids());
        assert_eq!(capabilities.len(), CAPABILITY_IDS.len());
        for registration in &registrations {
            assert!(
                registration
                    .metadata
                    .manifest
                    .verify()
                    .expect("manifest digest")
            );
            let expected_handlers = registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities
                .iter()
                .filter(|capability| !capability.contributions.actions.is_empty())
                .map(|capability| capability.id.clone())
                .collect::<BTreeSet<_>>();
            assert_eq!(registration.handler_ids(), expected_handlers);
        }
        let materialized = Materializer::materialize(
            &MaterializationPolicy::stable(CONTRACT_VERSION),
            &registrations,
            1,
        )
        .expect("Wave 1 registrations materialize");
        assert_eq!(materialized.packages.len(), PACKAGE_IDS.len());
        assert_eq!(materialized.capabilities.len(), CAPABILITY_IDS.len());
    }

    #[tokio::test]
    async fn knowledge_search_invocation_is_deterministic_and_typed() {
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("kernel registry");
        let materialized = registry
            .replace_all(registrations().expect("Wave 1 registrations"))
            .expect("publish Wave 1 registrations");
        let test_principal = principal();
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("knowledge"),
            resource_kind: ResourceKind::from("knowledge_base"),
            resource_id: nomifun_agent_contracts::ResourceId::from("kb-1"),
            owner_id: test_principal.principal_id.clone(),
            operations: BTreeSet::from(["read".to_owned(), "search".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let test_action_id = action_id("knowledge.search");
        let payload = AgentPresetRevisionPayload {
            schema_version: VersionString::from(CONTRACT_VERSION),
            surfaces: BTreeSet::from(["desktop".to_owned()]),
            model_route_refs: BTreeMap::new(),
            initial_capabilities: vec![CapabilitySelection {
                capability: CapabilityRef {
                    id: CapabilityId::from("knowledge.search"),
                    version: VersionString::from(CONTRACT_VERSION),
                },
                required: true,
                exposure: CapabilityExposure::Advertised,
                action_allowlist: BTreeSet::from([test_action_id.clone()]),
                resource_binding_refs: vec![binding.binding_id.clone()],
                destination_constraints: BTreeSet::new(),
                context_budget_override: None,
                tool_budget_override: None,
                config: StrictJsonValue(json!({})),
            }],
            on_demand_capabilities: Vec::new(),
            skill_bindings: Vec::new(),
            resource_bindings: vec![binding],
            persona: "Wave 1 test".to_owned(),
            instructions: "Invoke the selected capability.".to_owned(),
            context_policy: StrictJsonValue(json!({})),
            execution_constraints: StrictJsonValue(json!({})),
            runtime_budget: StrictJsonValue(json!({})),
        };
        let revision = AgentPresetRevision {
            reference: PresetRevisionRef {
                preset_id: AgentPresetId::from("wave1-test"),
                revision: 1,
                revision_digest: digest_payload(&payload).expect("revision digest"),
            },
            payload,
            created_by: UserId::from(test_principal.principal_id.clone()),
            created_at_ms: 1,
            reason: None,
        };
        let snapshot = AgentPresetCompiler::compile(
            &materialized,
            &CompilerEnvironment {
                resolver_version: VersionString::from(CONTRACT_VERSION),
                required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: DigestHex::from("runtime"),
                available_runtime_features: BTreeSet::new(),
                canonical_schema_manifest_digest: DigestHex::from("schema"),
                target_contribution_manifest_digest: DigestHex::from("target"),
                host_target: RuntimeTarget::from("windows-desktop-x64"),
                host_surface: "desktop".to_owned(),
                availability_evidence_revision: "wave1-test".to_owned(),
            },
            CompileRequest {
                revision,
                principal: test_principal.clone(),
                scene: "wave1-test".to_owned(),
                surface: "desktop".to_owned(),
                audience: "test".to_owned(),
                created_at_ms: 2,
                resolver_run_id: OperationId::from("wave1-resolve"),
            },
        )
        .expect("compile selected capability");
        let active = SessionCapabilityState::new(&snapshot)
            .snapshot()
            .expect("initial active set");
        let request = CapabilityInvocationRequest {
            principal: test_principal.clone(),
            session_owner: test_principal,
            resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
            active_set_generation: active.generation,
            capability_id: CapabilityId::from("knowledge.search"),
            action_id: test_action_id,
            resource_binding_ids: BTreeSet::from([ResourceBindingId::from("knowledge")]),
            state_scope_key: ScopeKey::from("session:wave1-test"),
            input: StrictJsonValue(json!({"query": "rust"})),
        };
        let first = registry
            .invoke(&snapshot, &active, request.clone())
            .await
            .expect("first invocation");
        let second = registry
            .invoke(&snapshot, &active, request)
            .await
            .expect("second invocation");
        assert_eq!(first, second);
        assert_eq!(first.0["accepted"], json!(true));
        assert_eq!(first.0["capability_id"], json!("knowledge.search"));
        assert_eq!(first.0["resource_binding_ids"], json!(["knowledge"]));

        let invalid = CapabilityInvocationRequest {
            input: StrictJsonValue(json!("not-an-object")),
            ..CapabilityInvocationRequest {
                principal: principal(),
                session_owner: principal(),
                resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from("knowledge.search"),
                action_id: action_id("knowledge.search"),
                resource_binding_ids: BTreeSet::from([ResourceBindingId::from("knowledge")]),
                state_scope_key: ScopeKey::from("session:wave1-test"),
                input: StrictJsonValue(json!({})),
            }
        };
        assert!(registry.invoke(&snapshot, &active, invalid).await.is_err());
    }
}
