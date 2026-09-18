//! Target-generation Web Research, Knowledge, and Memory Modules.
//!
//! Provider mechanics, citation rendering, Knowledge retrieval internals,
//! attachment ingestion, memory maintenance, and resource discovery are host
//! services. Agent authoring receives only the four product Modules and their
//! exact Actions. Browser is owned independently by the Wave 2 `browser`
//! Module and never appears as a parallel Wave 1 capability.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, ArtifactEnvelope, CapabilityActionDescriptor,
    CapabilityAuthoringPolicy, CapabilityConsumer, CapabilityContributions, CapabilityId,
    CapabilityKind, CapabilityManifest, CancellationDescriptor, CanonicalErrorCode,
    CanonicalSchemaRef, CorrelationId, EffectClass, HostPortBindingDescriptor, IdempotencyKey,
    InProcessEntrypointMetadata, LocalizedMetadata, ManagedTaskRegistrationDescriptor,
    OperationId, PackageContributions, PackageId, PackageManifest, PackageRef,
    PlatformConstraint, PluginBootCriticality, PluginBootState, PluginDesiredState,
    PluginEffectiveState, PluginIdentityDescriptor, PluginMountId, PluginRegistrarDescriptor,
    PluginRegistrarOperation, PluginRegistrationMetadata, PluginSourceKind,
    PluginSourceMetadata, PluginStateCompareAndSwapOutcome, PluginStateEntry,
    PluginStateHandleDescriptor, PluginStateMethod, PrincipalRef, ResolvedSnapshotRef,
    ResourceBindingId, ResourceId, ResourceKind, ScopeKey, StateKey, StrictJsonValue,
    ToolPresentationKind, TypedResourceBinding, TypedResourceBindings, ValidatedPluginConfig,
    VersionString, capability_module_surface_declarations, digest_payload,
    CAPABILITY_UNAVAILABLE_ON_PLATFORM,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, HostPluginStateApi, KernelError,
    PluginRegistration, PluginStateError, PluginStateHandle,
};
use serde_json::{Value, json};

pub const CONTRACT_VERSION: &str = "1.0.0";
pub const VERSION: &str = CONTRACT_VERSION;
pub const PACKAGE_VERSION: &str = CONTRACT_VERSION;

pub const WEB_RESEARCH_PACKAGE_ID: &str = "nomifun.web-research";
pub const KNOWLEDGE_PACKAGE_ID: &str = "nomifun.knowledge";
pub const PROJECT_MEMORY_PACKAGE_ID: &str = "nomifun.project-memory";
pub const COMPANION_MEMORY_PACKAGE_ID: &str = "nomifun.companion-memory";

pub const WEB_RESEARCH_MOUNT_ID: &str = "domain-web-research";
pub const KNOWLEDGE_MOUNT_ID: &str = "domain-knowledge";
pub const PROJECT_MEMORY_MOUNT_ID: &str = "domain-project-memory";
pub const COMPANION_MEMORY_MOUNT_ID: &str = "domain-companion-memory";

pub const WEB_RESEARCH_MODULE_ID: &str = "web.research";
pub const KNOWLEDGE_MODULE_ID: &str = "knowledge";
pub const PROJECT_MEMORY_MODULE_ID: &str = "project.memory";
pub const COMPANION_MEMORY_MODULE_ID: &str = "companion.memory";

pub const WEB_RESEARCH_SEARCH_ACTION_ID: &str = "web.research/search";
pub const WEB_RESEARCH_FETCH_ACTION_ID: &str = "web.research/fetch";
pub const KNOWLEDGE_SEARCH_ACTION_ID: &str = "knowledge/search";
pub const KNOWLEDGE_READ_ACTION_ID: &str = "knowledge/read";
pub const KNOWLEDGE_WRITE_ACTION_ID: &str = "knowledge/write";
pub const KNOWLEDGE_AUTOGEN_ACTION_ID: &str = "knowledge/autogen";
pub const PROJECT_MEMORY_READ_ACTION_ID: &str = "project.memory/read";
pub const PROJECT_MEMORY_WRITE_ACTION_ID: &str = "project.memory/write";
pub const COMPANION_MEMORY_RECALL_ACTION_ID: &str = "companion.memory/recall";
pub const COMPANION_MEMORY_WRITE_ACTION_ID: &str = "companion.memory/write";

pub const WEB_RESEARCH_ACTION_IDS: &[&str] = &[
    WEB_RESEARCH_SEARCH_ACTION_ID,
    WEB_RESEARCH_FETCH_ACTION_ID,
];
pub const KNOWLEDGE_ACTION_IDS: &[&str] = &[
    KNOWLEDGE_SEARCH_ACTION_ID,
    KNOWLEDGE_READ_ACTION_ID,
    KNOWLEDGE_WRITE_ACTION_ID,
    KNOWLEDGE_AUTOGEN_ACTION_ID,
];
pub const PROJECT_MEMORY_ACTION_IDS: &[&str] = &[
    PROJECT_MEMORY_READ_ACTION_ID,
    PROJECT_MEMORY_WRITE_ACTION_ID,
];
pub const COMPANION_MEMORY_ACTION_IDS: &[&str] = &[
    COMPANION_MEMORY_RECALL_ACTION_ID,
    COMPANION_MEMORY_WRITE_ACTION_ID,
];

pub const PACKAGE_IDS: [&str; 4] = [
    WEB_RESEARCH_PACKAGE_ID,
    KNOWLEDGE_PACKAGE_ID,
    PROJECT_MEMORY_PACKAGE_ID,
    COMPANION_MEMORY_PACKAGE_ID,
];
pub const TARGET_PACKAGE_IDS: [&str; 4] = PACKAGE_IDS;
pub const CAPABILITY_IDS: [&str; 4] = [
    WEB_RESEARCH_MODULE_ID,
    KNOWLEDGE_MODULE_ID,
    PROJECT_MEMORY_MODULE_ID,
    COMPANION_MEMORY_MODULE_ID,
];
pub const TARGET_CAPABILITY_IDS: [&str; 4] = CAPABILITY_IDS;
pub const ALL_CAPABILITY_IDS: [&str; 4] = CAPABILITY_IDS;
pub const TARGET_ACTION_IDS: [&str; 10] = [
    WEB_RESEARCH_SEARCH_ACTION_ID,
    WEB_RESEARCH_FETCH_ACTION_ID,
    KNOWLEDGE_SEARCH_ACTION_ID,
    KNOWLEDGE_READ_ACTION_ID,
    KNOWLEDGE_WRITE_ACTION_ID,
    KNOWLEDGE_AUTOGEN_ACTION_ID,
    PROJECT_MEMORY_READ_ACTION_ID,
    PROJECT_MEMORY_WRITE_ACTION_ID,
    COMPANION_MEMORY_RECALL_ACTION_ID,
    COMPANION_MEMORY_WRITE_ACTION_ID,
];

pub const AGENT_SURFACES: &[&str] = &["desktop", "headless"];
pub const KNOWLEDGE_BASE_RESOURCE_KIND: &str = "knowledge_base";
pub const PROJECT_MEMORY_RESOURCE_KIND: &str = "project_memory";
pub const COMPANION_MEMORY_RESOURCE_KIND: &str = "companion_memory";
pub const WAVE1_CAPABILITY_HOST_PORT_ID: &str = "host.wave1.capability.invoke";
const CAPABILITY_UNAVAILABLE_CODE: &str = "CAPABILITY_UNAVAILABLE";

#[derive(Clone, Copy)]
struct ResourceRequirement {
    resource_kind: &'static str,
    operation: &'static str,
}

#[derive(Clone, Copy)]
struct ActionSpec {
    id: &'static str,
    effect_class: EffectClass,
    resource_kinds: &'static [&'static str],
    requirements: &'static [ResourceRequirement],
}

#[derive(Clone, Copy)]
struct PackageSpec {
    id: &'static str,
    mount_id: &'static str,
    module_id: &'static str,
    display_name: &'static str,
    description: &'static str,
    actions: &'static [ActionSpec],
}

const KNOWLEDGE_RESOURCE: &[&str] = &[KNOWLEDGE_BASE_RESOURCE_KIND];
const PROJECT_MEMORY_RESOURCE: &[&str] = &[PROJECT_MEMORY_RESOURCE_KIND];
const COMPANION_MEMORY_RESOURCE: &[&str] = &[COMPANION_MEMORY_RESOURCE_KIND];
const KNOWLEDGE_SEARCH_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: KNOWLEDGE_BASE_RESOURCE_KIND,
    operation: "search",
}];
const KNOWLEDGE_READ_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: KNOWLEDGE_BASE_RESOURCE_KIND,
    operation: "read",
}];
const KNOWLEDGE_WRITE_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: KNOWLEDGE_BASE_RESOURCE_KIND,
    operation: "write",
}];
const PROJECT_MEMORY_READ_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: PROJECT_MEMORY_RESOURCE_KIND,
    operation: "read",
}];
const PROJECT_MEMORY_WRITE_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: PROJECT_MEMORY_RESOURCE_KIND,
    operation: "write",
}];
const COMPANION_MEMORY_READ_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: COMPANION_MEMORY_RESOURCE_KIND,
    operation: "read",
}];
const COMPANION_MEMORY_WRITE_REQUIREMENTS: &[ResourceRequirement] = &[ResourceRequirement {
    resource_kind: COMPANION_MEMORY_RESOURCE_KIND,
    operation: "write",
}];

const WEB_RESEARCH_ACTIONS: [ActionSpec; 2] = [
    ActionSpec {
        id: WEB_RESEARCH_SEARCH_ACTION_ID,
        effect_class: EffectClass::ExternalTransmit,
        resource_kinds: &[],
        requirements: &[],
    },
    ActionSpec {
        id: WEB_RESEARCH_FETCH_ACTION_ID,
        effect_class: EffectClass::ExternalTransmit,
        resource_kinds: &[],
        requirements: &[],
    },
];

const KNOWLEDGE_ACTIONS: [ActionSpec; 4] = [
    ActionSpec {
        id: KNOWLEDGE_SEARCH_ACTION_ID,
        effect_class: EffectClass::ReadSensitive,
        resource_kinds: KNOWLEDGE_RESOURCE,
        requirements: KNOWLEDGE_SEARCH_REQUIREMENTS,
    },
    ActionSpec {
        id: KNOWLEDGE_READ_ACTION_ID,
        effect_class: EffectClass::ReadSensitive,
        resource_kinds: KNOWLEDGE_RESOURCE,
        requirements: KNOWLEDGE_READ_REQUIREMENTS,
    },
    ActionSpec {
        id: KNOWLEDGE_WRITE_ACTION_ID,
        effect_class: EffectClass::WriteDurable,
        resource_kinds: KNOWLEDGE_RESOURCE,
        requirements: KNOWLEDGE_WRITE_REQUIREMENTS,
    },
    ActionSpec {
        id: KNOWLEDGE_AUTOGEN_ACTION_ID,
        effect_class: EffectClass::WriteDurable,
        resource_kinds: KNOWLEDGE_RESOURCE,
        requirements: KNOWLEDGE_WRITE_REQUIREMENTS,
    },
];

const PROJECT_MEMORY_ACTIONS: [ActionSpec; 2] = [
    ActionSpec {
        id: PROJECT_MEMORY_READ_ACTION_ID,
        effect_class: EffectClass::ReadSensitive,
        resource_kinds: PROJECT_MEMORY_RESOURCE,
        requirements: PROJECT_MEMORY_READ_REQUIREMENTS,
    },
    ActionSpec {
        id: PROJECT_MEMORY_WRITE_ACTION_ID,
        effect_class: EffectClass::WriteDurable,
        resource_kinds: PROJECT_MEMORY_RESOURCE,
        requirements: PROJECT_MEMORY_WRITE_REQUIREMENTS,
    },
];

const COMPANION_MEMORY_ACTIONS: [ActionSpec; 2] = [
    ActionSpec {
        id: COMPANION_MEMORY_RECALL_ACTION_ID,
        effect_class: EffectClass::ReadSensitive,
        resource_kinds: COMPANION_MEMORY_RESOURCE,
        requirements: COMPANION_MEMORY_READ_REQUIREMENTS,
    },
    ActionSpec {
        id: COMPANION_MEMORY_WRITE_ACTION_ID,
        effect_class: EffectClass::WriteDurable,
        resource_kinds: COMPANION_MEMORY_RESOURCE,
        requirements: COMPANION_MEMORY_WRITE_REQUIREMENTS,
    },
];

const PACKAGES: [PackageSpec; 4] = [
    PackageSpec {
        id: WEB_RESEARCH_PACKAGE_ID,
        mount_id: WEB_RESEARCH_MOUNT_ID,
        module_id: WEB_RESEARCH_MODULE_ID,
        display_name: "Web Research",
        description: "Search and fetch public web sources with derived citations.",
        actions: &WEB_RESEARCH_ACTIONS,
    },
    PackageSpec {
        id: KNOWLEDGE_PACKAGE_ID,
        mount_id: KNOWLEDGE_MOUNT_ID,
        module_id: KNOWLEDGE_MODULE_ID,
        display_name: "Knowledge",
        description: "Search, read, write, and generate owned Knowledge content.",
        actions: &KNOWLEDGE_ACTIONS,
    },
    PackageSpec {
        id: PROJECT_MEMORY_PACKAGE_ID,
        mount_id: PROJECT_MEMORY_MOUNT_ID,
        module_id: PROJECT_MEMORY_MODULE_ID,
        display_name: "Project Memory",
        description: "Read and write the selected project memory.",
        actions: &PROJECT_MEMORY_ACTIONS,
    },
    PackageSpec {
        id: COMPANION_MEMORY_PACKAGE_ID,
        mount_id: COMPANION_MEMORY_MOUNT_ID,
        module_id: COMPANION_MEMORY_MODULE_ID,
        display_name: "Companion Memory",
        description: "Recall and write memory for the selected Companion.",
        actions: &COMPANION_MEMORY_ACTIONS,
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedResourceDescriptor {
    pub slot_key: &'static str,
    pub resource_kind: ResourceKind,
    pub required: bool,
    pub operations: BTreeSet<String>,
    pub binding_policy: &'static str,
}

#[derive(Clone)]
pub struct Wave1StateHandle(PluginStateHandle);

impl Wave1StateHandle {
    fn new(handle: PluginStateHandle) -> Self {
        Self(handle)
    }

    pub fn descriptor(&self) -> &PluginStateHandleDescriptor {
        self.0.descriptor()
    }

    pub async fn get(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
    ) -> Result<Option<PluginStateEntry>, PluginStateError> {
        self.0.get(scope_key, state_key).await
    }

    pub async fn compare_and_swap(
        &self,
        scope_key: &ScopeKey,
        state_key: &StateKey,
        expected_revision: u64,
        state_format_version: &VersionString,
        value: Option<StrictJsonValue>,
    ) -> Result<PluginStateCompareAndSwapOutcome, PluginStateError> {
        self.0
            .compare_and_swap(
                scope_key,
                state_key,
                expected_revision,
                state_format_version,
                value,
            )
            .await
    }
}

#[derive(Clone)]
pub struct Wave1HostContext {
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub state_scope_key: ScopeKey,
    pub state: Wave1StateHandle,
    pub resource_bindings: TypedResourceBindings,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1SearchRequest {
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1FetchRequest {
    pub url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1KnowledgeReadRequest {
    pub handle: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1KnowledgeWriteRequest {
    pub handle: Option<String>,
    pub base: Option<String>,
    pub rel_path: Option<String>,
    pub content: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1KnowledgeAutogenRequest {
    pub overwrite_readme: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Wave1ProjectMemoryReadRequest {
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1MemoryMutationRequest {
    pub content: Option<String>,
    pub title: Option<String>,
    pub items: Option<Vec<Value>>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Wave1CompanionMemoryRecallRequest {
    pub per_kind: Option<usize>,
    pub char_budget: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1CompanionMemoryWriteRequest {
    pub kind: String,
    pub content: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wave1CapabilityOperation {
    ResearchSearch(Wave1SearchRequest),
    ResearchFetch(Wave1FetchRequest),
    KnowledgeSearch(Wave1SearchRequest),
    KnowledgeRead(Wave1KnowledgeReadRequest),
    KnowledgeWrite(Wave1KnowledgeWriteRequest),
    KnowledgeAutogen(Wave1KnowledgeAutogenRequest),
    ProjectMemoryRead(Wave1ProjectMemoryReadRequest),
    ProjectMemoryWrite(Wave1MemoryMutationRequest),
    CompanionMemoryRecall(Wave1CompanionMemoryRecallRequest),
    CompanionMemoryWrite(Wave1CompanionMemoryWriteRequest),
}

impl Wave1CapabilityOperation {
    pub fn capability_id(&self) -> CapabilityId {
        CapabilityId::from(match self {
            Self::ResearchSearch(_) | Self::ResearchFetch(_) => WEB_RESEARCH_MODULE_ID,
            Self::KnowledgeSearch(_)
            | Self::KnowledgeRead(_)
            | Self::KnowledgeWrite(_)
            | Self::KnowledgeAutogen(_) => KNOWLEDGE_MODULE_ID,
            Self::ProjectMemoryRead(_) | Self::ProjectMemoryWrite(_) => PROJECT_MEMORY_MODULE_ID,
            Self::CompanionMemoryRecall(_) | Self::CompanionMemoryWrite(_) => {
                COMPANION_MEMORY_MODULE_ID
            }
        })
    }

    pub fn action_id(&self) -> ActionId {
        ActionId::from(match self {
            Self::ResearchSearch(_) => WEB_RESEARCH_SEARCH_ACTION_ID,
            Self::ResearchFetch(_) => WEB_RESEARCH_FETCH_ACTION_ID,
            Self::KnowledgeSearch(_) => KNOWLEDGE_SEARCH_ACTION_ID,
            Self::KnowledgeRead(_) => KNOWLEDGE_READ_ACTION_ID,
            Self::KnowledgeWrite(_) => KNOWLEDGE_WRITE_ACTION_ID,
            Self::KnowledgeAutogen(_) => KNOWLEDGE_AUTOGEN_ACTION_ID,
            Self::ProjectMemoryRead(_) => PROJECT_MEMORY_READ_ACTION_ID,
            Self::ProjectMemoryWrite(_) => PROJECT_MEMORY_WRITE_ACTION_ID,
            Self::CompanionMemoryRecall(_) => COMPANION_MEMORY_RECALL_ACTION_ID,
            Self::CompanionMemoryWrite(_) => COMPANION_MEMORY_WRITE_ACTION_ID,
        })
    }
}

#[derive(Clone)]
pub struct Wave1HostRequest {
    pub context: Wave1HostContext,
    pub operation: Wave1CapabilityOperation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave1HostPortError {
    pub code: CanonicalErrorCode,
    pub message: String,
}

impl Wave1HostPortError {
    pub fn new(code: impl Into<CanonicalErrorCode>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(CAPABILITY_UNAVAILABLE_CODE, message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new("INVALID_PAYLOAD", message)
    }
}

impl fmt::Display for Wave1HostPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_ref(), self.message)
    }
}

impl std::error::Error for Wave1HostPortError {}

#[async_trait]
pub trait Wave1HostPort: Send + Sync {
    async fn invoke(
        &self,
        request: Wave1HostRequest,
    ) -> Result<StrictJsonValue, Wave1HostPortError>;
}

struct UnconfiguredWave1HostPort;

#[async_trait]
impl Wave1HostPort for UnconfiguredWave1HostPort {
    async fn invoke(
        &self,
        request: Wave1HostRequest,
    ) -> Result<StrictJsonValue, Wave1HostPortError> {
        Err(Wave1HostPortError::unavailable(format!(
            "no production host adapter is bound for {}/{}",
            request.context.capability_id.as_ref(),
            request.context.action_id.as_ref()
        )))
    }
}

pub fn unconfigured_host_port() -> Arc<dyn Wave1HostPort> {
    Arc::new(UnconfiguredWave1HostPort)
}

pub fn registrations() -> Result<Vec<PluginRegistration>, String> {
    registrations_with_host_port(unconfigured_host_port())
}

pub fn registrations_with_host_port(
    host_port: Arc<dyn Wave1HostPort>,
) -> Result<Vec<PluginRegistration>, String> {
    let registrations = PACKAGES
        .iter()
        .map(|package| registration_for(package, Arc::clone(&host_port)))
        .collect::<Result<Vec<_>, _>>()?;
    let actual = registrations
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
    if actual != capability_ids() {
        return Err("Wave 1 Module inventory does not match the target contract".to_owned());
    }
    Ok(registrations)
}

fn registration_for(
    spec: &PackageSpec,
    host: Arc<dyn Wave1HostPort>,
) -> Result<PluginRegistration, String> {
    let package = PackageRef {
        id: PackageId::from(spec.id),
        version: VersionString::from(PACKAGE_VERSION),
    };
    let config_schema = StrictJsonValue(object_schema(false));
    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::Bundled,
        source_identity: spec.id.to_owned(),
        source_digest: None,
    };
    let mount_id = PluginMountId::from(spec.mount_id);
    let identity = PluginIdentityDescriptor {
        package: package.clone(),
        mount_id: mount_id.clone(),
    };
    let cancellation_port = host_port_ref("host.plugin.cancel");
    let task_port = host_port_ref("host.plugin.tasks");
    let action_port = host_port_ref(WAVE1_CAPABILITY_HOST_PORT_ID);
    let manifest = PackageManifest {
        schema_version: VersionString::from(PACKAGE_VERSION),
        host_contract_version: VersionString::from(CONTRACT_VERSION),
        package_id: package.id.clone(),
        package_version: package.version.clone(),
        display: display(spec.display_name, spec.description),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: format!("{}.entrypoint", spec.id),
            contract_version: VersionString::from(CONTRACT_VERSION),
        }
        .into(),
        contributions: PackageContributions {
            capabilities: vec![capability_manifest(&package, spec)?],
            skills: Vec::new(),
            mcp_tools: Vec::new(),
            role_contracts: Vec::new(),
            role_providers: Vec::new(),
        },
    };
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest).map_err(|error| error.to_string())?,
        mount_id: mount_id.clone(),
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
            declared_capability_ids: BTreeSet::from([CapabilityId::from(spec.module_id)]),
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
            declared_role_ids: BTreeSet::new(),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports: BTreeSet::from([
                cancellation_port.id.clone(),
                task_port.id.clone(),
                action_port.id.clone(),
            ]),
        },
        context: nomifun_agent_contracts::PluginContextDescriptor {
            identity,
            source,
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema).map_err(|error| error.to_string())?,
                config_revision: 1,
                value: StrictJsonValue(json!({})),
            },
            state: PluginStateHandleDescriptor {
                package_id: PackageId::from(spec.id),
                mount_id: mount_id.clone(),
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: Default::default(),
            host_ports: vec![host_port_binding(spec)?],
            typed_command_ports: Vec::new(),
            domain_outbox_ports: Vec::new(),
            cancellation: CancellationDescriptor {
                cancellation_port,
                scope_key: ScopeKey::from(format!("mount:{}", spec.mount_id)),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: task_port,
                scope_key: ScopeKey::from(format!("mount:{}", spec.mount_id)),
            },
        },
    };
    let mut registration = PluginRegistration::new(metadata);
    registration
        .add_capability_handler(
            CapabilityId::from(spec.module_id),
            Arc::new(Wave1CapabilityHandler {
                capability_id: CapabilityId::from(spec.module_id),
                host_port: host,
            }),
        )
        .map_err(|error| error.to_string())?;
    Ok(registration)
}

fn capability_manifest(
    package: &PackageRef,
    spec: &PackageSpec,
) -> Result<CapabilityManifest, String> {
    let actions = spec
        .actions
        .iter()
        .map(|action| {
            Ok(CapabilityActionDescriptor {
                action_id: ActionId::from(action.id),
                input_schema: schema_ref(action.id, "input", &action_input_schema(action.id)?)?,
                output_schema: schema_ref(action.id, "output", &action_output_schema(action.id)?)?,
                effect_class: action.effect_class,
                presentation: ToolPresentationKind::FunctionTool,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let supported_platforms = vec![PlatformConstraint::Any];
    Ok(CapabilityManifest {
        id: CapabilityId::from(spec.module_id),
        contribution_id: nomifun_agent_contracts::ContributionId::from(format!(
            "module:{}",
            spec.module_id
        )),
        version: VersionString::from(PACKAGE_VERSION),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: display(spec.display_name, spec.description),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_module_surface_declarations(
            capability_surfaces(spec.module_id).iter().copied(),
            supported_consumers(spec.module_id),
            CapabilityAuthoringPolicy::Direct,
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms,
        config_schema: StrictJsonValue(object_schema(false)),
        contributions: CapabilityContributions {
            actions,
            context_schema_refs: Vec::new(),
            context_phase: Default::default(),
            ui_slot: None,
            event_schema_refs: Vec::new(),
            resource_kinds: spec
                .actions
                .iter()
                .flat_map(|action| action.resource_kinds.iter())
                .map(|kind| ResourceKind::from(*kind))
                .collect(),
            host_ports: vec![host_port_ref(WAVE1_CAPABILITY_HOST_PORT_ID)],
        },
    })
}

struct Wave1CapabilityHandler {
    capability_id: CapabilityId,
    host_port: Arc<dyn Wave1HostPort>,
}

#[async_trait]
impl CapabilityHandler for Wave1CapabilityHandler {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        if context.capability_id != self.capability_id {
            return Err(KernelError::ActionNotDeclared {
                capability_id: context.capability_id,
                action_id: context.action_id,
            });
        }
        let action = find_action(self.capability_id.as_ref(), context.action_id.as_ref())
            .ok_or_else(|| KernelError::ActionNotDeclared {
                capability_id: context.capability_id.clone(),
                action_id: context.action_id.clone(),
            })?;
        let bindings = validate_resource_bindings(
            &self.capability_id,
            &context.principal.principal_id,
            action.requirements,
            &context.resource_bindings,
        )?;
        let operation = operation_from_action(&context.action_id, input)
            .map_err(wave1_input_error_to_kernel)?;
        self.host_port
            .invoke(Wave1HostRequest {
                context: Wave1HostContext {
                    principal: context.principal,
                    agent_session_id: context.agent_session_id,
                    operation_id: context.operation_id,
                    idempotency_key: context.idempotency_key,
                    correlation_id: context.correlation_id,
                    resolved_snapshot_ref: context.resolved_snapshot_ref,
                    registry_generation: context.registry_generation,
                    capability_id: self.capability_id.clone(),
                    action_id: context.action_id,
                    state_scope_key: context.state_scope_key,
                    state: Wave1StateHandle::new(context.state),
                    resource_bindings: bindings.into_iter().cloned().collect(),
                },
                operation,
            })
            .await
            .map_err(wave1_host_error_to_kernel)
    }
}

fn wave1_host_error_to_kernel(error: Wave1HostPortError) -> KernelError {
    KernelError::capability_execution_failed(error.code, error.message)
}

fn wave1_input_error_to_kernel(error: KernelError) -> KernelError {
    match error {
        KernelError::CapabilityExecution { reason } => {
            KernelError::capability_execution_failed("INVALID_PAYLOAD", reason)
        }
        other => other,
    }
}

pub fn operation_from_action(
    action: &ActionId,
    input: StrictJsonValue,
) -> Result<Wave1CapabilityOperation, KernelError> {
    validate_action_input(action.as_ref(), &input.0)?;
    let required = |field: &str| {
        input
            .0
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| KernelError::CapabilityExecution {
                reason: format!("{} requires non-empty `{field}`", action.as_ref()),
            })
    };
    let optional = |field: &str| {
        input
            .0
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|value| !value.trim().is_empty())
    };
    let operation = match action.as_ref() {
        WEB_RESEARCH_SEARCH_ACTION_ID | KNOWLEDGE_SEARCH_ACTION_ID => {
            let request = Wave1SearchRequest {
                query: required("query")?,
                limit: input
                    .0
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map(|limit| limit as usize),
            };
            if action.as_ref() == WEB_RESEARCH_SEARCH_ACTION_ID {
                Wave1CapabilityOperation::ResearchSearch(request)
            } else {
                Wave1CapabilityOperation::KnowledgeSearch(request)
            }
        }
        WEB_RESEARCH_FETCH_ACTION_ID => {
            Wave1CapabilityOperation::ResearchFetch(Wave1FetchRequest { url: required("url")? })
        }
        KNOWLEDGE_READ_ACTION_ID => {
            Wave1CapabilityOperation::KnowledgeRead(Wave1KnowledgeReadRequest {
                handle: required("handle")?,
            })
        }
        KNOWLEDGE_WRITE_ACTION_ID => {
            Wave1CapabilityOperation::KnowledgeWrite(Wave1KnowledgeWriteRequest {
                handle: optional("handle"),
                base: optional("base"),
                rel_path: optional("rel_path"),
                content: required("content")?,
                title: optional("title"),
            })
        }
        KNOWLEDGE_AUTOGEN_ACTION_ID => {
            Wave1CapabilityOperation::KnowledgeAutogen(Wave1KnowledgeAutogenRequest {
                overwrite_readme: input
                    .0
                    .get("overwrite_readme")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        }
        PROJECT_MEMORY_READ_ACTION_ID => {
            Wave1CapabilityOperation::ProjectMemoryRead(Wave1ProjectMemoryReadRequest {
                limit: input
                    .0
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map(|limit| limit as usize),
            })
        }
        PROJECT_MEMORY_WRITE_ACTION_ID => {
            Wave1CapabilityOperation::ProjectMemoryWrite(Wave1MemoryMutationRequest {
                content: optional("content"),
                title: optional("title"),
                items: input.0.get("items").and_then(Value::as_array).cloned(),
            })
        }
        COMPANION_MEMORY_RECALL_ACTION_ID => {
            Wave1CapabilityOperation::CompanionMemoryRecall(
                Wave1CompanionMemoryRecallRequest {
                    per_kind: input
                        .0
                        .get("per_kind")
                        .and_then(Value::as_u64)
                        .map(|value| value as usize),
                    char_budget: input
                        .0
                        .get("char_budget")
                        .and_then(Value::as_u64)
                        .map(|value| value as usize),
                },
            )
        }
        COMPANION_MEMORY_WRITE_ACTION_ID => {
            let tags = input
                .0
                .get("tags")
                .and_then(Value::as_array)
                .map(|tags| {
                    tags.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            Wave1CapabilityOperation::CompanionMemoryWrite(
                Wave1CompanionMemoryWriteRequest {
                    kind: required("kind")?,
                    content: required("content")?,
                    tags,
                },
            )
        }
        _ => {
            return Err(KernelError::CapabilityExecution {
                reason: format!("unknown Wave 1 Action {}", action.as_ref()),
            });
        }
    };
    Ok(operation)
}

pub fn capability_ids() -> BTreeSet<CapabilityId> {
    CAPABILITY_IDS
        .iter()
        .map(|id| CapabilityId::from(*id))
        .collect()
}

pub fn package_ids() -> BTreeSet<PackageId> {
    PACKAGE_IDS.iter().map(|id| PackageId::from(*id)).collect()
}

pub fn capability_ids_by_package() -> BTreeMap<PackageId, BTreeSet<CapabilityId>> {
    PACKAGES
        .iter()
        .map(|package| {
            (
                PackageId::from(package.id),
                BTreeSet::from([CapabilityId::from(package.module_id)]),
            )
        })
        .collect()
}

pub fn action_ids(capability_id: &str) -> BTreeSet<ActionId> {
    find_module(capability_id)
        .map(|package| {
            package
                .actions
                .iter()
                .map(|action| ActionId::from(action.id))
                .collect()
        })
        .unwrap_or_default()
}

pub fn required_resource_kinds(capability_id: &str) -> Option<BTreeSet<ResourceKind>> {
    find_module(capability_id).map(|package| {
        package
            .actions
            .iter()
            .flat_map(|action| action.resource_kinds.iter())
            .map(|kind| ResourceKind::from(*kind))
            .collect()
    })
}

pub fn required_action_resource_operations(
    capability_id: &str,
    action_id: &str,
) -> Option<Vec<(ResourceKind, String)>> {
    let action = find_action(capability_id, action_id)?;
    Some(
        action
            .requirements
            .iter()
            .map(|requirement| {
                (
                    ResourceKind::from(requirement.resource_kind),
                    requirement.operation.to_owned(),
                )
            })
            .collect(),
    )
}

pub fn resolve_canonical_schema(
    capability_id: &str,
    reference: &CanonicalSchemaRef,
) -> Result<StrictJsonValue, String> {
    let package = find_module(capability_id)
        .ok_or_else(|| format!("unknown Wave 1 Module {capability_id}"))?;
    for action in package.actions {
        for (facet, schema) in [
            ("input", action_input_schema(action.id)?),
            ("output", action_output_schema(action.id)?),
        ] {
            if schema_ref(action.id, facet, &schema)?.as_ref() == reference.as_ref() {
                return Ok(StrictJsonValue(schema));
            }
        }
    }
    Err(format!(
        "schema {} is not owned by Wave 1 Module {capability_id}",
        reference.as_ref()
    ))
}

pub fn typed_resource_descriptors() -> Vec<TypedResourceDescriptor> {
    vec![
        descriptor(
            "knowledge",
            KNOWLEDGE_BASE_RESOURCE_KIND,
            false,
            ["read", "search", "write"],
        ),
        descriptor(
            "project_memory",
            PROJECT_MEMORY_RESOURCE_KIND,
            false,
            ["read", "write"],
        ),
        descriptor(
            "companion_memory",
            COMPANION_MEMORY_RESOURCE_KIND,
            false,
            ["read", "write"],
        ),
    ]
}

pub fn all_resource_descriptors() -> Vec<TypedResourceDescriptor> {
    typed_resource_descriptors()
}

pub fn resource_descriptors() -> Vec<TypedResourceDescriptor> {
    typed_resource_descriptors()
}

pub fn resource_binding_metadata() -> BTreeMap<ResourceKind, BTreeSet<String>> {
    typed_resource_descriptors()
        .into_iter()
        .map(|descriptor| (descriptor.resource_kind, descriptor.operations))
        .collect()
}

pub fn canonical_resource_bindings(owner_id: impl Into<String>) -> Vec<TypedResourceBinding> {
    let owner_id = owner_id.into();
    vec![
        typed_resource_binding(
            "wave1-knowledge",
            KNOWLEDGE_BASE_RESOURCE_KIND,
            "knowledge",
            &owner_id,
            ["read", "search", "write"],
        ),
        typed_resource_binding(
            "wave1-project-memory",
            PROJECT_MEMORY_RESOURCE_KIND,
            "project-memory",
            &owner_id,
            ["read", "write"],
        ),
        typed_resource_binding(
            "wave1-companion-memory",
            COMPANION_MEMORY_RESOURCE_KIND,
            "companion-memory",
            &owner_id,
            ["read", "write"],
        ),
    ]
}

pub fn resource_bindings(owner_id: impl Into<String>) -> Vec<TypedResourceBinding> {
    canonical_resource_bindings(owner_id)
}

pub fn typed_resource_binding<I, S>(
    binding_id: impl Into<ResourceBindingId>,
    resource_kind: impl Into<ResourceKind>,
    resource_id: impl Into<ResourceId>,
    owner_id: impl Into<String>,
    operations: I,
) -> TypedResourceBinding
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    TypedResourceBinding {
        binding_id: binding_id.into(),
        resource_kind: resource_kind.into(),
        resource_id: resource_id.into(),
        owner_id: owner_id.into(),
        operations: operations.into_iter().map(Into::into).collect(),
        connection_config_ref: None,
        typed_parameters: BTreeMap::new(),
    }
}

pub fn check_platform_availability(
    capability_id: &CapabilityId,
    _host_target: &nomifun_agent_contracts::RuntimeTarget,
    host_surface: &str,
) -> Result<(), KernelError> {
    if find_module(capability_id.as_ref()).is_none() {
        return Err(KernelError::CapabilityExecution {
            reason: format!("unknown Wave 1 Module {}", capability_id.as_ref()),
        });
    }
    if capability_surfaces(capability_id.as_ref()).contains(&host_surface) {
        Ok(())
    } else {
        Err(KernelError::CapabilityUnavailableOnSurface {
            capability_id: capability_id.clone(),
            surface: host_surface.to_owned(),
        })
    }
}

pub fn is_available_on_platform(
    capability_id: &str,
    host_target: &str,
    host_surface: &str,
) -> Result<bool, String> {
    if find_module(capability_id).is_none() {
        return Err(format!("unknown Wave 1 Module {capability_id}"));
    }
    let _ = host_target;
    Ok(capability_surfaces(capability_id).contains(&host_surface))
}

pub fn unavailable_on_platform_code() -> CanonicalErrorCode {
    CanonicalErrorCode::from(CAPABILITY_UNAVAILABLE_ON_PLATFORM)
}

pub fn web_research_registration() -> Result<PluginRegistration, String> {
    registration_for_package(WEB_RESEARCH_PACKAGE_ID)
}

pub fn knowledge_registration() -> Result<PluginRegistration, String> {
    registration_for_package(KNOWLEDGE_PACKAGE_ID)
}

pub fn project_memory_registration() -> Result<PluginRegistration, String> {
    registration_for_package(PROJECT_MEMORY_PACKAGE_ID)
}

pub fn companion_memory_registration() -> Result<PluginRegistration, String> {
    registration_for_package(COMPANION_MEMORY_PACKAGE_ID)
}

fn registration_for_package(package_id: &str) -> Result<PluginRegistration, String> {
    let package = PACKAGES
        .iter()
        .find(|package| package.id == package_id)
        .ok_or_else(|| format!("unknown Wave 1 package {package_id}"))?;
    registration_for(package, unconfigured_host_port())
}

fn find_module(capability_id: &str) -> Option<&'static PackageSpec> {
    PACKAGES
        .iter()
        .find(|package| package.module_id == capability_id)
}

fn find_action(capability_id: &str, action_id: &str) -> Option<&'static ActionSpec> {
    find_module(capability_id)?
        .actions
        .iter()
        .find(|action| action.id == action_id)
}

fn validate_resource_bindings<'a>(
    capability_id: &CapabilityId,
    principal_id: &str,
    requirements: &[ResourceRequirement],
    bindings: &'a [TypedResourceBinding],
) -> Result<Vec<&'a TypedResourceBinding>, KernelError> {
    let expected_kinds = requirements
        .iter()
        .map(|requirement| ResourceKind::from(requirement.resource_kind))
        .collect::<BTreeSet<_>>();
    let mut seen_ids = BTreeSet::new();
    for binding in bindings {
        if binding.binding_id.as_ref().trim().is_empty()
            || binding.resource_id.as_ref().trim().is_empty()
        {
            return Err(KernelError::CapabilityExecution {
                reason: format!("{} received a blank resource identity", capability_id.as_ref()),
            });
        }
        if !seen_ids.insert(binding.binding_id.clone()) {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} received duplicate resource binding {}",
                    capability_id.as_ref(),
                    binding.binding_id.as_ref()
                ),
            });
        }
        if binding.owner_id != principal_id {
            return Err(KernelError::ResourceOwnerMismatch {
                binding_id: binding.binding_id.clone(),
            });
        }
        if !expected_kinds.contains(&binding.resource_kind) {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} received unexpected resource kind {}",
                    capability_id.as_ref(),
                    binding.resource_kind.as_ref()
                ),
            });
        }
    }
    for requirement in requirements {
        let matches = bindings
            .iter()
            .filter(|binding| binding.resource_kind.as_ref() == requirement.resource_kind)
            .collect::<Vec<_>>();
        let binding = match matches.as_slice() {
            [binding] => *binding,
            [] => {
                return Err(KernelError::CapabilityResourceNotBound {
                    capability_id: capability_id.clone(),
                    resource_kind: requirement.resource_kind.to_owned(),
                });
            }
            _ => {
                return Err(KernelError::CapabilityExecution {
                    reason: format!(
                        "{} requires exactly one {} resource binding",
                        capability_id.as_ref(),
                        requirement.resource_kind
                    ),
                });
            }
        };
        if !binding.operations.contains(requirement.operation) {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} requires operation {} on {}",
                    capability_id.as_ref(),
                    requirement.operation,
                    requirement.resource_kind
                ),
            });
        }
    }
    let mut selected = bindings.iter().collect::<Vec<_>>();
    selected.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
    Ok(selected)
}

pub fn action_input_schema(action_id: &str) -> Result<Value, String> {
    let schema = match action_id {
        WEB_RESEARCH_SEARCH_ACTION_ID | KNOWLEDGE_SEARCH_ACTION_ID => json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "query": {"type": "string", "minLength": 1, "maxLength": 2048},
                "limit": {"type": "integer", "minimum": 1, "maximum": 20}
            },
            "required": ["query"]
        }),
        WEB_RESEARCH_FETCH_ACTION_ID => json!({
            "type": "object", "additionalProperties": false,
            "properties": {"url": {"type": "string", "minLength": 1, "maxLength": 4096}},
            "required": ["url"]
        }),
        KNOWLEDGE_READ_ACTION_ID => json!({
            "type": "object", "additionalProperties": false,
            "properties": {"handle": {"type": "string", "minLength": 1, "maxLength": 512}},
            "required": ["handle"]
        }),
        KNOWLEDGE_WRITE_ACTION_ID => json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "handle": {"type": "string", "minLength": 1, "maxLength": 512},
                "base": {"type": "string", "minLength": 1, "maxLength": 256},
                "rel_path": {"type": "string", "minLength": 1, "maxLength": 1024},
                "content": {"type": "string", "minLength": 1, "maxLength": 65536},
                "title": {"type": "string", "minLength": 1, "maxLength": 512}
            },
            "required": ["content"]
        }),
        KNOWLEDGE_AUTOGEN_ACTION_ID => json!({
            "type": "object", "additionalProperties": false,
            "properties": {"overwrite_readme": {"type": "boolean"}}
        }),
        PROJECT_MEMORY_READ_ACTION_ID => json!({
            "type": "object", "additionalProperties": false,
            "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 128}}
        }),
        PROJECT_MEMORY_WRITE_ACTION_ID => json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "content": {"type": "string", "minLength": 1, "maxLength": 65536},
                "title": {"type": "string", "minLength": 1, "maxLength": 512},
                "items": {"type": "array", "minItems": 1, "maxItems": 128}
            },
            "anyOf": [{"required": ["content"]}, {"required": ["items"]}]
        }),
        COMPANION_MEMORY_RECALL_ACTION_ID => json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "per_kind": {"type": "integer", "minimum": 1, "maximum": 20},
                "char_budget": {"type": "integer", "minimum": 1, "maximum": 65536}
            }
        }),
        COMPANION_MEMORY_WRITE_ACTION_ID => json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "kind": {"type": "string", "enum": ["profile", "preference", "knowledge", "episode", "task", "affective"]},
                "content": {"type": "string", "minLength": 1, "maxLength": 16384},
                "tags": {"type": "array", "maxItems": 32, "uniqueItems": true, "items": {"type": "string", "minLength": 1, "maxLength": 64}}
            },
            "required": ["kind", "content"]
        }),
        _ => return Err(format!("unknown Wave 1 Action {action_id}")),
    };
    Ok(schema)
}

pub fn action_output_schema(action_id: &str) -> Result<Value, String> {
    if !TARGET_ACTION_IDS.contains(&action_id) {
        return Err(format!("unknown Wave 1 Action {action_id}"));
    }
    Ok(object_schema(true))
}

fn validate_action_input(action_id: &str, input: &Value) -> Result<(), KernelError> {
    let Some(object) = input.as_object() else {
        return Err(KernelError::CapabilityExecution {
            reason: format!("{action_id} input must be a JSON object"),
        });
    };
    let allowed = match action_id {
        WEB_RESEARCH_SEARCH_ACTION_ID | KNOWLEDGE_SEARCH_ACTION_ID => &["query", "limit"][..],
        WEB_RESEARCH_FETCH_ACTION_ID => &["url"][..],
        KNOWLEDGE_READ_ACTION_ID => &["handle"][..],
        KNOWLEDGE_WRITE_ACTION_ID => &["handle", "base", "rel_path", "content", "title"][..],
        KNOWLEDGE_AUTOGEN_ACTION_ID => &["overwrite_readme"][..],
        PROJECT_MEMORY_READ_ACTION_ID => &["limit"][..],
        PROJECT_MEMORY_WRITE_ACTION_ID => &["content", "title", "items"][..],
        COMPANION_MEMORY_RECALL_ACTION_ID => &["per_kind", "char_budget"][..],
        COMPANION_MEMORY_WRITE_ACTION_ID => &["kind", "content", "tags"][..],
        _ => {
            return Err(KernelError::CapabilityExecution {
                reason: format!("unknown Wave 1 Action {action_id}"),
            });
        }
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(KernelError::CapabilityExecution {
            reason: format!("{action_id} input contains an unknown field"),
        });
    }
    let required_string = |field: &str, maximum: usize| -> Result<(), KernelError> {
        let value = object
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| KernelError::CapabilityExecution {
                reason: format!("{action_id} requires non-empty `{field}`"),
            })?;
        if value.chars().count() > maximum {
            return Err(KernelError::CapabilityExecution {
                reason: format!("{action_id} field `{field}` exceeds {maximum} characters"),
            });
        }
        Ok(())
    };
    match action_id {
        WEB_RESEARCH_SEARCH_ACTION_ID | KNOWLEDGE_SEARCH_ACTION_ID => {
            required_string("query", 2048)?;
            validate_optional_integer(object, action_id, "limit", 1, 20)?;
        }
        WEB_RESEARCH_FETCH_ACTION_ID => required_string("url", 4096)?,
        KNOWLEDGE_READ_ACTION_ID => required_string("handle", 512)?,
        KNOWLEDGE_WRITE_ACTION_ID => {
            required_string("content", 65_536)?;
            validate_optional_string(object, action_id, "handle", 512)?;
            validate_optional_string(object, action_id, "base", 256)?;
            validate_optional_string(object, action_id, "rel_path", 1024)?;
            validate_optional_string(object, action_id, "title", 512)?;
        }
        KNOWLEDGE_AUTOGEN_ACTION_ID => {
            if object.get("overwrite_readme").is_some_and(|value| !value.is_boolean()) {
                return Err(KernelError::CapabilityExecution {
                    reason: format!("{action_id} field `overwrite_readme` must be a boolean"),
                });
            }
        }
        PROJECT_MEMORY_READ_ACTION_ID => {
            validate_optional_integer(object, action_id, "limit", 1, 128)?;
        }
        PROJECT_MEMORY_WRITE_ACTION_ID => {
            validate_optional_string(object, action_id, "content", 65_536)?;
            validate_optional_string(object, action_id, "title", 512)?;
            let has_content = object
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty());
            let has_items = object
                .get("items")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty() && items.len() <= 128);
            if !has_content && !has_items {
                return Err(KernelError::CapabilityExecution {
                    reason: format!("{action_id} requires non-empty `content` or `items`"),
                });
            }
        }
        COMPANION_MEMORY_RECALL_ACTION_ID => {
            validate_optional_integer(object, action_id, "per_kind", 1, 20)?;
            validate_optional_integer(object, action_id, "char_budget", 1, 65_536)?;
        }
        COMPANION_MEMORY_WRITE_ACTION_ID => {
            required_string("kind", 64)?;
            required_string("content", 16_384)?;
            let kind = object.get("kind").and_then(Value::as_str).unwrap_or_default();
            if !matches!(kind, "profile" | "preference" | "knowledge" | "episode" | "task" | "affective") {
                return Err(KernelError::CapabilityExecution {
                    reason: format!("{action_id} field `kind` is unsupported"),
                });
            }
            if let Some(tags) = object.get("tags") {
                let tags = tags.as_array().ok_or_else(|| KernelError::CapabilityExecution {
                    reason: format!("{action_id} field `tags` must be an array"),
                })?;
                if tags.len() > 32 {
                    return Err(KernelError::CapabilityExecution {
                        reason: format!("{action_id} field `tags` exceeds 32 entries"),
                    });
                }
                let mut unique = BTreeSet::new();
                for tag in tags {
                    let tag = tag.as_str().filter(|tag| !tag.trim().is_empty()).ok_or_else(|| KernelError::CapabilityExecution {
                        reason: format!("{action_id} tags must be non-empty strings"),
                    })?;
                    if tag.chars().count() > 64 || !unique.insert(tag.trim()) {
                        return Err(KernelError::CapabilityExecution {
                            reason: format!("{action_id} contains an invalid or duplicate tag"),
                        });
                    }
                }
            }
        }
        _ => unreachable!("known Action matched above"),
    }
    Ok(())
}

fn validate_optional_string(
    object: &serde_json::Map<String, Value>,
    action_id: &str,
    field: &str,
    maximum: usize,
) -> Result<(), KernelError> {
    if let Some(value) = object.get(field) {
        let value = value.as_str().filter(|value| !value.trim().is_empty()).ok_or_else(|| {
            KernelError::CapabilityExecution {
                reason: format!("{action_id} field `{field}` must be a non-empty string"),
            }
        })?;
        if value.chars().count() > maximum {
            return Err(KernelError::CapabilityExecution {
                reason: format!("{action_id} field `{field}` exceeds {maximum} characters"),
            });
        }
    }
    Ok(())
}

fn validate_optional_integer(
    object: &serde_json::Map<String, Value>,
    action_id: &str,
    field: &str,
    minimum: u64,
    maximum: u64,
) -> Result<(), KernelError> {
    if let Some(value) = object.get(field) {
        value.as_u64().filter(|value| (minimum..=maximum).contains(value)).ok_or_else(|| KernelError::CapabilityExecution {
            reason: format!("{action_id} field `{field}` must be an integer from {minimum} to {maximum}"),
        })?;
    }
    Ok(())
}

fn capability_surfaces(capability_id: &str) -> &'static [&'static str] {
    let _ = capability_id;
    AGENT_SURFACES
}

pub fn supported_consumers(capability_id: &str) -> BTreeSet<CapabilityConsumer> {
    if capability_id == KNOWLEDGE_MODULE_ID {
        BTreeSet::from([CapabilityConsumer::Agent, CapabilityConsumer::Gateway])
    } else {
        BTreeSet::from([CapabilityConsumer::Agent])
    }
}

fn descriptor<const N: usize>(
    slot_key: &'static str,
    resource_kind: &'static str,
    required: bool,
    operations: [&'static str; N],
) -> TypedResourceDescriptor {
    TypedResourceDescriptor {
        slot_key,
        resource_kind: ResourceKind::from(resource_kind),
        required,
        operations: operations.into_iter().map(str::to_owned).collect(),
        binding_policy: "select_only_owned_resource",
    }
}

fn schema_ref(id: &str, facet: &str, schema: &Value) -> Result<CanonicalSchemaRef, String> {
    let digest = digest_payload(schema).map_err(|error| error.to_string())?;
    Ok(CanonicalSchemaRef::from(format!(
        "schema://{id}/{facet}@1#{}",
        digest.as_ref()
    )))
}

fn object_schema(additional_properties: bool) -> Value {
    json!({"type": "object", "additionalProperties": additional_properties})
}

fn display(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

fn host_port_ref(id: &str) -> nomifun_agent_contracts::HostPortRef {
    nomifun_agent_contracts::HostPortRef {
        id: nomifun_agent_contracts::HostPortId::from(id),
        version: VersionString::from(CONTRACT_VERSION),
    }
}

fn host_port_binding(spec: &PackageSpec) -> Result<HostPortBindingDescriptor, String> {
    let request_schema = json!({
        "anyOf": spec.actions.iter().map(|action| action_input_schema(action.id)).collect::<Result<Vec<_>, _>>()?
    });
    let response_schema = json!({
        "anyOf": spec.actions.iter().map(|action| action_output_schema(action.id)).collect::<Result<Vec<_>, _>>()?
    });
    Ok(HostPortBindingDescriptor {
        port: host_port_ref(WAVE1_CAPABILITY_HOST_PORT_ID),
        request_schema: schema_ref(WAVE1_CAPABILITY_HOST_PORT_ID, "request", &request_schema)?,
        response_schema: schema_ref(WAVE1_CAPABILITY_HOST_PORT_ID, "response", &response_schema)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_inventory_contains_only_wave1_modules() {
        assert_eq!(capability_ids().len(), 4);
        assert_eq!(action_ids(WEB_RESEARCH_MODULE_ID).len(), 2);
        assert_eq!(action_ids(KNOWLEDGE_MODULE_ID).len(), 4);
        assert_eq!(action_ids(PROJECT_MEMORY_MODULE_ID).len(), 2);
        assert_eq!(action_ids(COMPANION_MEMORY_MODULE_ID).len(), 2);
    }

    #[test]
    fn action_decoder_requires_exact_module_action_identity() {
        let operation = operation_from_action(
            &ActionId::from(WEB_RESEARCH_SEARCH_ACTION_ID),
            StrictJsonValue(json!({"query": "UARC", "limit": 3})),
        )
        .unwrap();
        assert_eq!(operation.capability_id().as_ref(), WEB_RESEARCH_MODULE_ID);
        assert_eq!(operation.action_id().as_ref(), WEB_RESEARCH_SEARCH_ACTION_ID);
        assert!(operation_from_action(
            &ActionId::from("web.search.invoke"),
            StrictJsonValue(json!({"query": "legacy"}))
        )
        .is_err());
    }

    #[test]
    fn registrations_publish_one_module_per_package() {
        let registrations = registrations().unwrap();
        assert_eq!(registrations.len(), PACKAGE_IDS.len());
        for registration in registrations {
            let capabilities = &registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities;
            assert_eq!(capabilities.len(), 1);
            assert!(!capabilities[0].contributions.actions.is_empty());
        }
    }

    #[test]
    fn resource_metadata_excludes_provider_and_maintenance_operations() {
        let metadata = resource_binding_metadata();
        assert_eq!(
            metadata[&ResourceKind::from(KNOWLEDGE_BASE_RESOURCE_KIND)],
            BTreeSet::from(["read".into(), "search".into(), "write".into()])
        );
        assert!(!metadata
            .values()
            .flatten()
            .any(|operation| matches!(operation.as_str(), "embed" | "rerank" | "mount" | "merge" | "evolve")));
    }
}
