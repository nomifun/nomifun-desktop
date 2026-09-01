//! Bundled Wave 1 read-capability registrations.
//!
//! The six package identities and 25 capabilities below are the Wave 1 slice
//! of the frozen first-party contribution inventory.  Customer-service
//! dialogue/identity is owned by Wave 4 and is intentionally absent here.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, ArtifactEnvelope, CapabilityActionDescriptor,
    CapabilityContributions, CapabilityId, CapabilityKind, CapabilityManifest,
    CancellationDescriptor, CanonicalErrorCode, CanonicalSchemaRef, CorrelationId,
    DeclaredServiceViewDescriptor, EffectClass, HostPortBindingDescriptor,
    IdempotencyKey, InProcessEntrypointMetadata, LocalizedMetadata,
    ManagedTaskRegistrationDescriptor, OperationId, PackageContributions, PackageId,
    PackageManifest, PackageRef, PlatformConstraint, PluginBootCriticality, PluginBootState,
    PluginContextDescriptor, PluginDesiredState, PluginEffectiveState, PluginIdentityDescriptor,
    PluginMountId, PluginRegistrarDescriptor, PluginRegistrarOperation,
    PluginRegistrationMetadata, PluginSourceKind, PluginSourceMetadata,
    PluginStateHandleDescriptor, PluginStateMethod, PrincipalRef, ResolvedSnapshotRef,
    ResourceBindingId, ResourceId, ResourceKind, ScopeKey, SkillDefinition, StrictJsonValue,
    ToolPresentationKind, TypedResourceBinding, TypedResourceBindings, ValidatedPluginConfig,
    VersionString,
    CAPABILITY_UNAVAILABLE_ON_PLATFORM, digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, KernelError, PluginRegistration,
};
use serde_json::{Value, json};

pub const CONTRACT_VERSION: &str = "1.0.0";
pub const VERSION: &str = CONTRACT_VERSION;
pub const PACKAGE_VERSION: &str = CONTRACT_VERSION;

pub const WEB_RESEARCH_PACKAGE_ID: &str = "nomifun.web-research";
pub const CHAT_PACKAGE_ID: &str = "nomifun.chat";
pub const KNOWLEDGE_PACKAGE_ID: &str = "nomifun.knowledge";
pub const PROJECT_MEMORY_PACKAGE_ID: &str = "nomifun.project-memory";
pub const COMPANION_MEMORY_PACKAGE_ID: &str = "nomifun.companion-memory";
pub const SKILLS_PACKAGE_ID: &str = "nomifun.skills";

pub const WEB_RESEARCH_MOUNT_ID: &str = "domain-web-research";
pub const CHAT_MOUNT_ID: &str = "domain-chat";
pub const KNOWLEDGE_MOUNT_ID: &str = "domain-knowledge";
pub const PROJECT_MEMORY_MOUNT_ID: &str = "domain-project-memory";
pub const COMPANION_MEMORY_MOUNT_ID: &str = "domain-companion-memory";
pub const SKILLS_MOUNT_ID: &str = "domain-skills";

/// Bundled package identities owned by Wave 1.
pub const PACKAGE_IDS: [&str; 6] = [
    WEB_RESEARCH_PACKAGE_ID,
    CHAT_PACKAGE_ID,
    KNOWLEDGE_PACKAGE_ID,
    PROJECT_MEMORY_PACKAGE_ID,
    COMPANION_MEMORY_PACKAGE_ID,
    SKILLS_PACKAGE_ID,
];
pub const TARGET_PACKAGE_IDS: [&str; 6] = PACKAGE_IDS;

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
pub const TARGET_CAPABILITY_IDS: [&str; 25] = CAPABILITY_IDS;
pub const ALL_CAPABILITY_IDS: [&str; 25] = CAPABILITY_IDS;

pub const WEB_SEARCH: &str = "web.search";
pub const WEB_FETCH: &str = "web.fetch";
pub const CITATION_RENDER: &str = "citation.render";
pub const SESSION_ATTACHMENTS_READ: &str = "session.attachments.read";
pub const KNOWLEDGE_SEARCH: &str = "knowledge.search";
pub const KNOWLEDGE_READ: &str = "knowledge.read";
pub const KNOWLEDGE_WRITE: &str = "knowledge.write";
pub const KNOWLEDGE_MOUNT: &str = "knowledge.mount";
pub const KNOWLEDGE_SOURCE_SYNC: &str = "knowledge.source.sync";
pub const KNOWLEDGE_AUTOGEN: &str = "knowledge.autogen";
pub const KNOWLEDGE_EMBEDDING: &str = "knowledge.embedding";
pub const KNOWLEDGE_RERANK: &str = "knowledge.rerank";
pub const MEMORY_PROJECT_READ: &str = "memory.project.read";
pub const MEMORY_PROJECT_WRITE: &str = "memory.project.write";
pub const MEMORY_PROJECT_DISTILL: &str = "memory.project.distill";
pub const MEMORY_PROJECT_CITATION: &str = "memory.project.citation";
pub const MEMORY_SESSION_SCRATCH: &str = "memory.session.scratch";
pub const MEMORY_COMPANION_RECALL: &str = "memory.companion.recall";
pub const MEMORY_COMPANION_WRITE: &str = "memory.companion.write";
pub const MEMORY_COMPANION_MERGE: &str = "memory.companion.merge";
pub const MEMORY_COMPANION_EVOLVE: &str = "memory.companion.evolve";
pub const SKILL_CATALOG: &str = "skill.catalog";
pub const SKILL_DESCRIBE: &str = "skill.describe";
pub const SKILL_INVOKE: &str = "skill.invoke";
pub const SKILL_HOOKS: &str = "skill.hooks";

pub const WEB_SEARCH_ACTION: &str = "web.search.invoke";
pub const WEB_FETCH_ACTION: &str = "web.fetch.invoke";
pub const KNOWLEDGE_SEARCH_ACTION: &str = "knowledge.search.invoke";
pub const KNOWLEDGE_READ_ACTION: &str = "knowledge.read.invoke";
pub const KNOWLEDGE_WRITE_ACTION: &str = "knowledge.write.invoke";
pub const KNOWLEDGE_AUTOGEN_ACTION: &str = "knowledge.autogen.invoke";
pub const KNOWLEDGE_EMBEDDING_ACTION: &str = "knowledge.embedding.invoke";
pub const KNOWLEDGE_RERANK_ACTION: &str = "knowledge.rerank.invoke";
pub const MEMORY_PROJECT_WRITE_ACTION: &str = "memory.project.write.invoke";
pub const MEMORY_PROJECT_DISTILL_ACTION: &str = "memory.project.distill.invoke";
pub const MEMORY_COMPANION_WRITE_ACTION: &str = "memory.companion.write.invoke";
pub const MEMORY_COMPANION_MERGE_ACTION: &str = "memory.companion.merge.invoke";
pub const MEMORY_COMPANION_EVOLVE_ACTION: &str = "memory.companion.evolve.invoke";
pub const SKILL_INVOKE_ACTION: &str = "skill.invoke.invoke";

/// Family names owned by this bounded read-capability slice.
pub const TARGET_CAPABILITY_FAMILIES: [&str; 9] = [
    "attachments.read",
    "knowledge.read",
    "knowledge.search",
    "memory.read",
    "research.core",
    "skill.instructions",
    "web.fetch",
    "web.search",
    "customer-service.read",
];

pub const AGENT_SURFACES: &[&str] = &["desktop", "headless"];
pub const KNOWLEDGE_BASE_RESOURCE_KIND: &str = "knowledge_base";
pub const PROJECT_MEMORY_RESOURCE_KIND: &str = "project_memory";
pub const COMPANION_MEMORY_RESOURCE_KIND: &str = "companion_memory";
pub const RESEARCH_CORE_PACK_ID: &str = "research.core";

const SURFACES: &[&str] = AGENT_SURFACES;
const KNOWLEDGE: &[&str] = &[KNOWLEDGE_BASE_RESOURCE_KIND];
const PROJECT_MEMORY: &[&str] = &[PROJECT_MEMORY_RESOURCE_KIND];
const COMPANION_MEMORY: &[&str] = &[COMPANION_MEMORY_RESOURCE_KIND];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResourceRequirement {
    resource_kind: &'static str,
    operation: &'static str,
}

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
const KNOWLEDGE_READ_REQUIREMENTS_WITH_SEARCH: &[ResourceRequirement] = &[
    ResourceRequirement {
        resource_kind: KNOWLEDGE_BASE_RESOURCE_KIND,
        operation: "read",
    },
    ResourceRequirement {
        resource_kind: KNOWLEDGE_BASE_RESOURCE_KIND,
        operation: "search",
    },
];
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

#[derive(Clone, Copy)]
struct CapabilitySpec {
    id: &'static str,
    kind: CapabilityKind,
    effect: Option<EffectClass>,
    resources: &'static [&'static str],
    requirements: &'static [ResourceRequirement],
}

#[derive(Clone, Copy)]
struct PackageSpec {
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    mount_id: &'static str,
    capabilities: &'static [CapabilitySpec],
}

/// The typed resource slot metadata exported by this domain slice.
///
/// A descriptor describes the contract for a slot; it does not create or
/// resolve a product resource.  Concrete resource ownership is still checked
/// against the invocation principal by the host and handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedResourceDescriptor {
    pub slot_key: &'static str,
    pub resource_kind: ResourceKind,
    pub required: bool,
    pub operations: BTreeSet<String>,
    pub binding_policy: &'static str,
}

/// The single narrow host port used by Wave 1 action handlers.
///
/// This is an in-process Rust port, not a product service object or a
/// registry.  A production host adapts its existing domain owners
/// (Knowledge, Skills, Research, and memory stores) to this port and passes
/// the adapter to [`registrations_with_host_port`].  The Wave 1 crate owns
/// neither those stores nor their business facts.
pub const WAVE1_CAPABILITY_HOST_PORT_ID: &str = "host.wave1.capability.invoke";

/// Invocation metadata projected from the Kernel context into a domain port.
///
/// The projection intentionally excludes the application service bag,
/// Gateway state, legacy Conversation state, and the Kernel authority itself.
#[derive(Clone, Debug, PartialEq)]
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

/// The fields mirror the canonical Knowledge write input without resolving a
/// target in this crate.  Target resolution and write policy remain owned by
/// the injected Knowledge service.
#[derive(Clone, Debug, PartialEq)]
pub struct Wave1KnowledgeWriteRequest {
    pub handle: Option<String>,
    pub base: Option<String>,
    pub rel_path: Option<String>,
    pub content: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1KnowledgeEmbeddingRequest {
    pub text: Option<String>,
    pub query: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1KnowledgeAutogenRequest {
    pub overwrite_readme: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave1MemoryScope {
    Project,
    Companion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave1MemoryOperation {
    Write,
    Distill,
    Merge,
    Evolve,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1MemoryMutationRequest {
    pub scope: Wave1MemoryScope,
    pub operation: Wave1MemoryOperation,
    pub content: Option<String>,
    pub title: Option<String>,
    pub items: Option<Vec<Value>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1SkillInvokeRequest {
    pub skill_id: String,
    pub arguments: Option<StrictJsonValue>,
}

/// Typed operation variants understood by the Wave 1 host port.
///
/// The result returned by the port is passed through unchanged.  In
/// particular, this enum does not contain an acknowledgement, receipt, or
/// synthetic "accepted" result.
#[derive(Clone, Debug, PartialEq)]
pub enum Wave1CapabilityOperation {
    ResearchSearch(Wave1SearchRequest),
    ResearchFetch(Wave1FetchRequest),
    KnowledgeSearch(Wave1SearchRequest),
    KnowledgeRead(Wave1KnowledgeReadRequest),
    KnowledgeWrite(Wave1KnowledgeWriteRequest),
    KnowledgeAutogen(Wave1KnowledgeAutogenRequest),
    KnowledgeEmbedding(Wave1KnowledgeEmbeddingRequest),
    KnowledgeRerank(Wave1KnowledgeEmbeddingRequest),
    MemoryMutation(Wave1MemoryMutationRequest),
    SkillInvoke(Wave1SkillInvokeRequest),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Wave1HostRequest {
    pub context: Wave1HostContext,
    pub operation: Wave1CapabilityOperation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave1HostPortError {
    pub code: String,
    pub message: String,
}

impl Wave1HostPortError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new("WAVE1_HOST_PORT_UNAVAILABLE", message)
    }
}

impl fmt::Display for Wave1HostPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Wave1HostPortError {}

/// Production-owned implementation boundary for Wave 1 actions.
///
/// Implementations must call the owning domain service and return its
/// canonical action result.  They must not manufacture a receipt or use the
/// request as a successful result.
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
            "no production host adapter is bound for {}",
            request.context.capability_id.as_ref()
        )))
    }
}

const WEB_RESEARCH: &[CapabilitySpec] = &[
    tool("web.search", EffectClass::ExternalTransmit, &[]),
    tool("web.fetch", EffectClass::ExternalTransmit, &[]),
    context("citation.render", &[]),
];

const CHAT: &[CapabilitySpec] = &[context("session.attachments.read", &[])];

const KNOWLEDGE_CAPABILITIES: &[CapabilitySpec] = &[
    tool_with_requirements(
        "knowledge.search",
        EffectClass::ReadSensitive,
        KNOWLEDGE,
        KNOWLEDGE_SEARCH_REQUIREMENTS,
    ),
    tool_with_requirements(
        "knowledge.read",
        EffectClass::ReadSensitive,
        KNOWLEDGE,
        KNOWLEDGE_READ_REQUIREMENTS,
    ),
    tool_with_requirements(
        "knowledge.write",
        EffectClass::WriteDurable,
        KNOWLEDGE,
        KNOWLEDGE_WRITE_REQUIREMENTS,
    ),
    resource_provider("knowledge.mount", KNOWLEDGE),
    background("knowledge.source.sync"),
    tool_with_requirements(
        "knowledge.autogen",
        EffectClass::WriteDurable,
        KNOWLEDGE,
        KNOWLEDGE_WRITE_REQUIREMENTS,
    ),
    tool_with_requirements(
        "knowledge.embedding",
        EffectClass::ReadSensitive,
        KNOWLEDGE,
        KNOWLEDGE_READ_REQUIREMENTS,
    ),
    tool_with_requirements(
        "knowledge.rerank",
        EffectClass::ReadSensitive,
        KNOWLEDGE,
        KNOWLEDGE_READ_REQUIREMENTS_WITH_SEARCH,
    ),
];

const PROJECT_MEMORY_CAPABILITIES: &[CapabilitySpec] = &[
    context_with_requirements(
        "memory.project.read",
        PROJECT_MEMORY,
        PROJECT_MEMORY_READ_REQUIREMENTS,
    ),
    tool_with_requirements(
        "memory.project.write",
        EffectClass::WriteDurable,
        PROJECT_MEMORY,
        PROJECT_MEMORY_WRITE_REQUIREMENTS,
    ),
    tool_with_requirements(
        "memory.project.distill",
        EffectClass::WriteDurable,
        PROJECT_MEMORY,
        PROJECT_MEMORY_WRITE_REQUIREMENTS,
    ),
    context_with_requirements(
        "memory.project.citation",
        PROJECT_MEMORY,
        PROJECT_MEMORY_READ_REQUIREMENTS,
    ),
    resource_provider("memory.session.scratch", PROJECT_MEMORY),
];

const COMPANION_MEMORY_CAPABILITIES: &[CapabilitySpec] = &[
    context_with_requirements(
        "memory.companion.recall",
        COMPANION_MEMORY,
        COMPANION_MEMORY_READ_REQUIREMENTS,
    ),
    tool_with_requirements(
        "memory.companion.write",
        EffectClass::WriteDurable,
        COMPANION_MEMORY,
        COMPANION_MEMORY_WRITE_REQUIREMENTS,
    ),
    tool_with_requirements(
        "memory.companion.merge",
        EffectClass::WriteDurable,
        COMPANION_MEMORY,
        COMPANION_MEMORY_WRITE_REQUIREMENTS,
    ),
    tool_with_requirements(
        "memory.companion.evolve",
        EffectClass::WriteDurable,
        COMPANION_MEMORY,
        COMPANION_MEMORY_WRITE_REQUIREMENTS,
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
    tool_with_requirements(id, effect, resources, &[])
}

const fn tool_with_requirements(
    id: &'static str,
    effect: EffectClass,
    resources: &'static [&'static str],
    requirements: &'static [ResourceRequirement],
) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::Tool,
        effect: Some(effect),
        resources,
        requirements,
    }
}

const fn context(id: &'static str, resources: &'static [&'static str]) -> CapabilitySpec {
    context_with_requirements(id, resources, &[])
}

const fn context_with_requirements(
    id: &'static str,
    resources: &'static [&'static str],
    requirements: &'static [ResourceRequirement],
) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::ContextContributor,
        effect: None,
        resources,
        requirements,
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
        requirements: &[],
    }
}

const fn background(id: &'static str) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::BackgroundService,
        effect: None,
        resources: &[],
        requirements: &[],
    }
}

const fn middleware(id: &'static str) -> CapabilitySpec {
    CapabilitySpec {
        id,
        kind: CapabilityKind::TurnMiddleware,
        effect: None,
        resources: &[],
        requirements: &[],
    }
}

/// Build the six trusted Wave 1 registrations without a production adapter.
///
/// This preserves the metadata-only bootstrap path, but every action fails
/// closed until the host uses [`registrations_with_host_port`] with a real
/// adapter.  It deliberately does not install a successful test handler.
pub fn registrations() -> Result<Vec<PluginRegistration>, String> {
    registrations_with_host_port(unconfigured_host_port())
}

/// Return a host-port implementation that fails closed for unconfigured
/// metadata-only compositions and isolated contract tests.
pub fn unconfigured_host_port() -> Arc<dyn Wave1HostPort> {
    Arc::new(UnconfiguredWave1HostPort)
}

/// Build the six trusted Wave 1 registrations with the host-owned action port.
///
/// The host port is captured by each action handler as an explicit dependency.
/// This keeps the registration crate independent of concrete Knowledge,
/// Skills, Research, memory, Gateway, and application service types while
/// making the runtime path actually executable when the host supplies an
/// implementation.
pub fn registrations_with_host_port(
    host_port: Arc<dyn Wave1HostPort>,
) -> Result<Vec<PluginRegistration>, String> {
    let mut package_ids = BTreeSet::new();
    let mut declared_capability_ids = BTreeSet::new();
    let mut output = Vec::with_capacity(PACKAGES.len());

    for package in PACKAGES {
        if !package_ids.insert(package.id) {
            return Err(format!("duplicate Wave 1 package {}", package.id));
        }
        let registration = registration_for(package, Arc::clone(&host_port))?;
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

/// Return the exact Wave 1 package ID set.
pub fn package_ids() -> BTreeSet<PackageId> {
    PACKAGE_IDS
        .iter()
        .map(|id| PackageId::from(*id))
        .collect()
}

/// Return the canonical capability IDs contributed by each Wave 1 package.
pub fn capability_ids_by_package() -> BTreeMap<PackageId, BTreeSet<CapabilityId>> {
    PACKAGES
        .iter()
        .map(|package| {
            (
                PackageId::from(package.id),
                package
                    .capabilities
                    .iter()
                    .map(|capability| CapabilityId::from(capability.id))
                    .collect(),
            )
        })
        .collect()
}

/// Return the canonical action identity for an action-bearing capability.
pub fn action_id(capability_id: &str) -> Option<ActionId> {
    capability_for(capability_id)
        .filter(|capability| capability.kind == CapabilityKind::Tool)
        .map(|_| ActionId::from(format!("{capability_id}.invoke")))
}

/// Resolve a deletion-manifest family to the canonical IDs owned by this
/// slice.  `customer-service.read` is intentionally not returned: that
/// package and identity are owned by Wave 4.
pub fn canonical_capability_ids_for_family(family: &str) -> BTreeSet<CapabilityId> {
    let ids: &[&str] = match family {
        "attachments.read" => &[SESSION_ATTACHMENTS_READ],
        "knowledge.read" => &[KNOWLEDGE_READ],
        "knowledge.search" => &[KNOWLEDGE_SEARCH],
        "memory.read" => &[
            MEMORY_PROJECT_READ,
            MEMORY_PROJECT_CITATION,
            MEMORY_COMPANION_RECALL,
        ],
        "research.core" => &[WEB_SEARCH, WEB_FETCH, CITATION_RENDER],
        "skill.instructions" => &[SKILL_CATALOG, SKILL_DESCRIBE],
        "web.fetch" => &[WEB_FETCH],
        "web.search" => &[WEB_SEARCH],
        _ => &[],
    };
    ids.iter().map(|id| CapabilityId::from(*id)).collect()
}

/// Return the sole canonical ID for a one-capability family.
pub fn canonical_capability_id(family: &str) -> Option<CapabilityId> {
    let ids = canonical_capability_ids_for_family(family);
    (ids.len() == 1).then(|| ids.into_iter().next().expect("length checked"))
}

/// Return the typed resource kinds declared by a known capability.
pub fn required_resource_kinds(capability_id: &str) -> Option<BTreeSet<ResourceKind>> {
    capability_for(capability_id).map(|capability| resource_kinds(capability))
}

/// Return the resource kinds and operations exposed by the Wave 1 slots.
pub fn resource_binding_metadata() -> BTreeMap<ResourceKind, BTreeSet<String>> {
    typed_resource_descriptors()
        .into_iter()
        .map(|descriptor| (descriptor.resource_kind, descriptor.operations))
        .collect()
}

/// Return the typed resource slots used by Wave 1 capabilities.
pub fn typed_resource_descriptors() -> Vec<TypedResourceDescriptor> {
    vec![
        descriptor(
            "knowledge",
            KNOWLEDGE_BASE_RESOURCE_KIND,
            false,
            ["embed", "mount", "read", "rerank", "search", "sync", "write"],
            "select_only_owned_resource",
        ),
        descriptor(
            "project_memory",
            PROJECT_MEMORY_RESOURCE_KIND,
            false,
            ["read", "write"],
            "select_only_owned_resource",
        ),
        descriptor(
            "companion_memory",
            COMPANION_MEMORY_RESOURCE_KIND,
            false,
            ["read", "write"],
            "select_only_owned_resource",
        ),
    ]
}

pub fn all_resource_descriptors() -> Vec<TypedResourceDescriptor> {
    typed_resource_descriptors()
}

pub fn resource_descriptors() -> Vec<TypedResourceDescriptor> {
    typed_resource_descriptors()
}

/// Build canonical owner-scoped bindings for Wave 1 resource slots.
///
/// These are contract fixtures only; callers may replace the concrete resource
/// IDs before persisting a Preset Revision.
pub fn canonical_resource_bindings(owner_id: impl Into<String>) -> Vec<TypedResourceBinding> {
    let owner_id = owner_id.into();
    vec![
        resource_binding(
            "wave1-knowledge",
            KNOWLEDGE_BASE_RESOURCE_KIND,
            "knowledge",
            &["embed", "mount", "read", "rerank", "search", "sync", "write"],
            &owner_id,
        ),
        resource_binding(
            "wave1-project-memory",
            PROJECT_MEMORY_RESOURCE_KIND,
            "project-memory",
            &["read", "write"],
            &owner_id,
        ),
        resource_binding(
            "wave1-companion-memory",
            COMPANION_MEMORY_RESOURCE_KIND,
            "companion-memory",
            &["read", "write"],
            &owner_id,
        ),
    ]
}

pub fn resource_bindings(owner_id: impl Into<String>) -> Vec<TypedResourceBinding> {
    canonical_resource_bindings(owner_id)
}

/// Construct one typed binding without creating or resolving a product
/// resource.
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

pub fn typed_resource_bindings_for<'a>(
    owner_id: &str,
    entries: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a [&'a str])>,
) -> TypedResourceBindings {
    entries
        .into_iter()
        .map(|(binding_id, resource_kind, resource_id, operations)| {
            typed_resource_binding(
                binding_id,
                resource_kind,
                resource_id,
                owner_id,
                operations.iter().copied(),
            )
        })
        .collect()
}

/// Check the host surface portion of Wave 1 availability.  Wave 1
/// capabilities run on the local Desktop/Headless host; Web and mobile
/// clients use Remote ingress and are not local capability surfaces.
pub fn check_platform_availability(
    capability_id: &CapabilityId,
    host_target: &nomifun_agent_contracts::RuntimeTarget,
    host_surface: &str,
) -> Result<(), KernelError> {
    if capability_for(capability_id.as_ref()).is_none() {
        return Err(KernelError::CapabilityExecution {
            reason: format!("unknown Wave 1 capability {}", capability_id.as_ref()),
        });
    }
    if SURFACES.contains(&host_surface) {
        let _ = host_target;
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
    _host_target: &str,
    host_surface: &str,
) -> Result<bool, String> {
    if capability_for(capability_id).is_none() {
        return Err(format!("unknown Wave 1 capability {capability_id}"));
    }
    Ok(SURFACES.contains(&host_surface))
}

pub fn unavailable_on_platform_code() -> CanonicalErrorCode {
    CanonicalErrorCode::from(CAPABILITY_UNAVAILABLE_ON_PLATFORM)
}

pub fn web_research_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGES[0], unconfigured_host_port())
}

pub fn chat_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGES[1], unconfigured_host_port())
}

pub fn knowledge_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGES[2], unconfigured_host_port())
}

pub fn project_memory_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGES[3], unconfigured_host_port())
}

pub fn companion_memory_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGES[4], unconfigured_host_port())
}

pub fn skills_registration() -> Result<PluginRegistration, String> {
    registration_for(&PACKAGES[5], unconfigured_host_port())
}

fn registration_for(
    spec: &PackageSpec,
    action_host: Arc<dyn Wave1HostPort>,
) -> Result<PluginRegistration, String> {
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
    let has_action_handler = spec.capabilities.iter().any(|capability| {
        capability.kind == CapabilityKind::Tool && capability.effect.is_some()
    });
    let action_host_port = host_port(WAVE1_CAPABILITY_HOST_PORT_ID);
    let mut declared_host_ports =
        BTreeSet::from([cancellation_port.id.clone(), task_port.id.clone()]);
    let host_port_bindings = if has_action_handler {
        declared_host_ports.insert(action_host_port.id.clone());
        vec![host_port_binding()?]
    } else {
        Vec::new()
    };
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
            declared_host_ports,
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
            host_ports: host_port_bindings,
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
                Arc::new(Wave1CapabilityHandler {
                    capability_id: CapabilityId::from(capability.id),
                    action_id: action_id(capability.id)
                        .expect("action-bearing capability has an action identity"),
                    requirements: capability.requirements,
                    host_port: Arc::clone(&action_host),
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
            let input = tool_input_schema(spec.id);
            let output = tool_output_schema();
            CapabilityContributions {
                actions: vec![CapabilityActionDescriptor {
                    action_id: action_id(spec.id)
                        .expect("tool capability has an action identity"),
                    input_schema: schema_ref(spec.id, "input", &input)?,
                    output_schema: schema_ref(spec.id, "output", &output)?,
                    effect_class: spec.effect.expect("tool effect"),
                    presentation: ToolPresentationKind::FunctionTool,
                }],
                resource_kinds: resource_kinds(spec),
                host_ports: vec![host_port(WAVE1_CAPABILITY_HOST_PORT_ID)],
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
        CapabilityKind::EventSource | CapabilityKind::EventConsumer => {
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
        display: capability_display(spec.id),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: SURFACES.iter().map(|surface| (*surface).to_owned()).collect(),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(object_schema(false)),
        contributions,
    })
}

struct Wave1CapabilityHandler {
    capability_id: CapabilityId,
    action_id: ActionId,
    requirements: &'static [ResourceRequirement],
    host_port: Arc<dyn Wave1HostPort>,
}

#[async_trait]
impl CapabilityHandler for Wave1CapabilityHandler {
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
        validate_action_input(self.capability_id.as_ref(), &input.0)?;
        let resource_bindings = validate_resource_bindings(
            &self.capability_id,
            &context.principal.principal_id,
            self.requirements,
            &context.resource_bindings,
        )?;
        let operation = operation_from_input(&self.capability_id, &input.0)?;
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
                    action_id: self.action_id.clone(),
                    state_scope_key: context.state_scope_key,
                    resource_bindings: resource_bindings.into_iter().cloned().collect(),
                },
                operation,
            })
            .await
            .map_err(|error| KernelError::CapabilityExecution {
                reason: error.to_string(),
            })
    }
}

fn operation_from_input(
    capability_id: &CapabilityId,
    input: &Value,
) -> Result<Wave1CapabilityOperation, KernelError> {
    let id = capability_id.as_ref();
    let required = |field: &str| {
        input
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| KernelError::CapabilityExecution {
                reason: format!("{id} requires non-empty `{field}`"),
            })
    };
    let optional = |field: &str| {
        input
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|value| !value.trim().is_empty())
    };

    match id {
        WEB_SEARCH | KNOWLEDGE_SEARCH => {
            let request = Wave1SearchRequest {
                query: required("query")?,
                limit: input
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize),
            };
            if id == WEB_SEARCH {
                Ok(Wave1CapabilityOperation::ResearchSearch(request))
            } else {
                Ok(Wave1CapabilityOperation::KnowledgeSearch(request))
            }
        }
        WEB_FETCH => Ok(Wave1CapabilityOperation::ResearchFetch(Wave1FetchRequest {
            url: required("url")?,
        })),
        KNOWLEDGE_READ => Ok(Wave1CapabilityOperation::KnowledgeRead(
            Wave1KnowledgeReadRequest {
                handle: required("handle")?,
            },
        )),
        KNOWLEDGE_WRITE => {
            let request = Wave1KnowledgeWriteRequest {
                handle: optional("handle"),
                base: optional("base"),
                rel_path: optional("rel_path"),
                content: required("content")?,
                title: optional("title"),
            };
            Ok(Wave1CapabilityOperation::KnowledgeWrite(request))
        }
        KNOWLEDGE_AUTOGEN => {
            let overwrite_readme = input
                .get("overwrite_readme")
                .map(|value| {
                    value
                        .as_bool()
                        .ok_or_else(|| KernelError::CapabilityExecution {
                            reason: format!(
                                "{id} field `overwrite_readme` must be a boolean"
                            ),
                        })
                })
                .transpose()?
                .unwrap_or(false);
            Ok(Wave1CapabilityOperation::KnowledgeAutogen(
                Wave1KnowledgeAutogenRequest { overwrite_readme },
            ))
        }
        KNOWLEDGE_EMBEDDING | KNOWLEDGE_RERANK => {
            let request = Wave1KnowledgeEmbeddingRequest {
                text: optional("text"),
                query: optional("query"),
            };
            if id == KNOWLEDGE_RERANK {
                Ok(Wave1CapabilityOperation::KnowledgeRerank(request))
            } else {
                Ok(Wave1CapabilityOperation::KnowledgeEmbedding(request))
            }
        }
        MEMORY_PROJECT_WRITE
        | MEMORY_PROJECT_DISTILL
        | MEMORY_COMPANION_WRITE
        | MEMORY_COMPANION_MERGE
        | MEMORY_COMPANION_EVOLVE => {
            let scope = if id.starts_with("memory.project.") {
                Wave1MemoryScope::Project
            } else {
                Wave1MemoryScope::Companion
            };
            let operation = match id {
                MEMORY_PROJECT_WRITE | MEMORY_COMPANION_WRITE => Wave1MemoryOperation::Write,
                MEMORY_PROJECT_DISTILL => Wave1MemoryOperation::Distill,
                MEMORY_COMPANION_MERGE => Wave1MemoryOperation::Merge,
                MEMORY_COMPANION_EVOLVE => Wave1MemoryOperation::Evolve,
                _ => unreachable!("all memory mutation capabilities are listed above"),
            };
            Ok(Wave1CapabilityOperation::MemoryMutation(
                Wave1MemoryMutationRequest {
                    scope,
                    operation,
                    content: optional("content"),
                    title: optional("title"),
                    items: input.get("items").and_then(Value::as_array).cloned(),
                },
            ))
        }
        SKILL_INVOKE => Ok(Wave1CapabilityOperation::SkillInvoke(
            Wave1SkillInvokeRequest {
                skill_id: required("skill_id")?,
                arguments: input
                    .get("arguments")
                    .cloned()
                    .map(StrictJsonValue),
            },
        )),
        _ => Err(KernelError::CapabilityExecution {
            reason: format!("{id} does not expose an action host operation"),
        }),
    }
}

fn resource_kinds(spec: &CapabilitySpec) -> BTreeSet<ResourceKind> {
    spec.resources
        .iter()
        .map(|resource| ResourceKind::from(*resource))
        .collect()
}

fn capability_for(capability_id: &str) -> Option<&'static CapabilitySpec> {
    PACKAGES
        .iter()
        .flat_map(|package| package.capabilities.iter())
        .find(|capability| capability.id == capability_id)
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
        if binding.binding_id.as_ref().is_empty() || binding.resource_id.as_ref().is_empty() {
            return Err(KernelError::CapabilityExecution {
                reason: format!(
                    "{} requires non-empty binding and resource IDs",
                    capability_id.as_ref()
                ),
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
        let binding = bindings
            .iter()
            .find(|binding| binding.resource_kind.as_ref() == requirement.resource_kind)
            .ok_or_else(|| KernelError::CapabilityResourceNotBound {
                capability_id: capability_id.clone(),
                resource_kind: requirement.resource_kind.to_owned(),
            })?;
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

fn tool_input_schema(capability_id: &str) -> Value {
    match capability_id {
        WEB_SEARCH | KNOWLEDGE_SEARCH => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "query": {"type": "string", "minLength": 1, "maxLength": 2048},
                "limit": {"type": "integer", "minimum": 1, "maximum": 20}
            },
            "required": ["query"]
        }),
        WEB_FETCH => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "url": {"type": "string", "minLength": 1, "maxLength": 4096}
            },
            "required": ["url"]
        }),
        KNOWLEDGE_READ => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "handle": {"type": "string", "minLength": 1, "maxLength": 512}
            },
            "required": ["handle"]
        }),
        KNOWLEDGE_WRITE => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "handle": {"type": "string", "minLength": 1, "maxLength": 512},
                "base": {"type": "string", "minLength": 1, "maxLength": 256},
                "rel_path": {"type": "string", "minLength": 1, "maxLength": 1024},
                "content": {"type": "string", "minLength": 1, "maxLength": 65536},
                "title": {"type": "string", "minLength": 1, "maxLength": 512}
            },
            "required": ["content"]
        }),
        KNOWLEDGE_AUTOGEN => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "overwrite_readme": {"type": "boolean"}
            }
        }),
        KNOWLEDGE_EMBEDDING | KNOWLEDGE_RERANK => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "text": {"type": "string", "minLength": 1, "maxLength": 16384},
                "query": {"type": "string", "minLength": 1, "maxLength": 16384}
            },
            "anyOf": [
                {"required": ["text"]},
                {"required": ["query"]}
            ]
        }),
        MEMORY_PROJECT_WRITE
        | MEMORY_PROJECT_DISTILL
        | MEMORY_COMPANION_WRITE
        | MEMORY_COMPANION_MERGE
        | MEMORY_COMPANION_EVOLVE => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "content": {"type": "string", "minLength": 1, "maxLength": 65536},
                "title": {"type": "string", "minLength": 1, "maxLength": 512},
                "items": {"type": "array", "maxItems": 128}
            },
            "anyOf": [
                {"required": ["content"]},
                {"required": ["items"]}
            ]
        }),
        SKILL_INVOKE => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "skill_id": {"type": "string", "minLength": 1, "maxLength": 256},
                "arguments": {"type": "object"}
            },
            "required": ["skill_id"]
        }),
        _ => object_schema(false),
    }
}

fn validate_action_input(capability_id: &str, input: &Value) -> Result<(), KernelError> {
    let Some(object) = input.as_object() else {
        return Err(KernelError::CapabilityExecution {
            reason: format!("{capability_id} input must be a JSON object"),
        });
    };

    for (key, value) in object {
        if !allowed_input_key(capability_id, key) {
            return Err(KernelError::CapabilityExecution {
                reason: format!("{capability_id} input contains unknown field `{key}`"),
            });
        }
        match key.as_str() {
            "query" | "url" | "handle" | "base" | "rel_path" | "content" | "title"
            | "text" | "skill_id" => {
                let Some(value) = value.as_str() else {
                    return Err(KernelError::CapabilityExecution {
                        reason: format!("{capability_id} field `{key}` must be a string"),
                    });
                };
                let value = value.trim();
                if value.is_empty() {
                    return Err(KernelError::CapabilityExecution {
                        reason: format!("{capability_id} field `{key}` must not be empty"),
                    });
                }
                let max_chars = match key.as_str() {
                    "content" => 65_536,
                    "text" | "query" => 16_384,
                    "url" | "rel_path" => 4_096,
                    _ => 512,
                };
                if value.chars().count() > max_chars {
                    return Err(KernelError::CapabilityExecution {
                        reason: format!(
                            "{capability_id} field `{key}` exceeds {max_chars} characters"
                        ),
                    });
                }
            }
            "limit" => {
                let Some(value) = value.as_u64() else {
                    return Err(KernelError::CapabilityExecution {
                        reason: format!("{capability_id} field `limit` must be an integer"),
                    });
                };
                if !(1..=20).contains(&value) {
                    return Err(KernelError::CapabilityExecution {
                        reason: format!("{capability_id} field `limit` must be between 1 and 20"),
                    });
                }
            }
            "arguments" => {
                if !value.is_object() {
                    return Err(KernelError::CapabilityExecution {
                        reason: format!("{capability_id} field `arguments` must be an object"),
                    });
                }
            }
            "items" => {
                let Some(items) = value.as_array() else {
                    return Err(KernelError::CapabilityExecution {
                        reason: format!("{capability_id} field `items` must be an array"),
                    });
                };
                if items.is_empty() || items.len() > 128 {
                    return Err(KernelError::CapabilityExecution {
                        reason: format!(
                            "{capability_id} field `items` must contain between 1 and 128 entries"
                        ),
                    });
                }
            }
            _ => {}
        }
    }

    match capability_id {
        WEB_SEARCH | KNOWLEDGE_SEARCH => require_string(input, capability_id, "query")?,
        WEB_FETCH => require_string(input, capability_id, "url")?,
        KNOWLEDGE_READ => require_string(input, capability_id, "handle")?,
        KNOWLEDGE_WRITE => {
            require_string(input, capability_id, "content")?
        }
        KNOWLEDGE_AUTOGEN => {}
        KNOWLEDGE_EMBEDDING | KNOWLEDGE_RERANK => {
            if input.get("text").and_then(Value::as_str).is_none()
                && input.get("query").and_then(Value::as_str).is_none()
            {
                return Err(KernelError::CapabilityExecution {
                    reason: format!("{capability_id} requires `text` or `query`"),
                });
            }
        }
        MEMORY_PROJECT_WRITE
        | MEMORY_PROJECT_DISTILL
        | MEMORY_COMPANION_WRITE
        | MEMORY_COMPANION_MERGE
        | MEMORY_COMPANION_EVOLVE => {
            if input.get("content").and_then(Value::as_str).is_none()
                && input.get("items").and_then(Value::as_array).is_none()
            {
                return Err(KernelError::CapabilityExecution {
                    reason: format!("{capability_id} requires `content` or `items`"),
                });
            }
        }
        SKILL_INVOKE => require_string(input, capability_id, "skill_id")?,
        _ => {}
    }
    Ok(())
}

fn require_string(input: &Value, capability_id: &str, field: &str) -> Result<(), KernelError> {
    let valid = input
        .get(field)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    if valid {
        Ok(())
    } else {
        Err(KernelError::CapabilityExecution {
            reason: format!("{capability_id} requires non-empty `{field}`"),
        })
    }
}

fn allowed_input_key(capability_id: &str, key: &str) -> bool {
    match capability_id {
        WEB_SEARCH | KNOWLEDGE_SEARCH => matches!(key, "query" | "limit"),
        WEB_FETCH => key == "url",
        KNOWLEDGE_READ => key == "handle",
        KNOWLEDGE_WRITE => matches!(key, "handle" | "base" | "rel_path" | "content" | "title"),
        KNOWLEDGE_AUTOGEN => key == "overwrite_readme",
        KNOWLEDGE_EMBEDDING | KNOWLEDGE_RERANK => matches!(key, "text" | "query"),
        MEMORY_PROJECT_WRITE
        | MEMORY_PROJECT_DISTILL
        | MEMORY_COMPANION_WRITE
        | MEMORY_COMPANION_MERGE
        | MEMORY_COMPANION_EVOLVE => matches!(key, "content" | "title" | "items"),
        SKILL_INVOKE => matches!(key, "skill_id" | "arguments"),
        _ => false,
    }
}

fn tool_output_schema() -> Value {
    // The owning service defines the operation result. The registration only
    // constrains the wire to a JSON object; it must not publish a synthetic
    // "accepted" or "deterministic" receipt.
    object_schema(true)
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

fn capability_display(capability_id: &str) -> LocalizedMetadata {
    let (name, description) = match capability_id {
        WEB_SEARCH => (
            "Web search",
            "Search web sources through the selected research capability.",
        ),
        WEB_FETCH => (
            "Web fetch",
            "Fetch a selected web source for bounded Agent context.",
        ),
        CITATION_RENDER => (
            "Citation rendering",
            "Render stable citations for sources used by the Agent.",
        ),
        SESSION_ATTACHMENTS_READ => (
            "Attachment context",
            "Read attachments already bound to the current AgentSession.",
        ),
        KNOWLEDGE_SEARCH => (
            "Knowledge search",
            "Search the selected owned knowledge base.",
        ),
        KNOWLEDGE_READ => (
            "Knowledge read",
            "Read a selected document from the owned knowledge base.",
        ),
        KNOWLEDGE_WRITE => (
            "Knowledge write",
            "Write bounded material to the selected knowledge base.",
        ),
        KNOWLEDGE_MOUNT => (
            "Knowledge mount",
            "Provide the typed knowledge-base resource boundary.",
        ),
        KNOWLEDGE_SOURCE_SYNC => (
            "Knowledge source sync",
            "Synchronize an owned knowledge source through its domain owner.",
        ),
        KNOWLEDGE_AUTOGEN => (
            "Knowledge auto-generation",
            "Generate bounded knowledge material for an owned knowledge base.",
        ),
        KNOWLEDGE_EMBEDDING => (
            "Knowledge embedding",
            "Create a retrieval embedding for selected knowledge content.",
        ),
        KNOWLEDGE_RERANK => (
            "Knowledge reranking",
            "Rerank selected knowledge search material.",
        ),
        MEMORY_PROJECT_READ => (
            "Project memory context",
            "Provide current project-memory context for the Agent.",
        ),
        MEMORY_PROJECT_WRITE => (
            "Project memory write",
            "Write bounded material to the owned project memory.",
        ),
        MEMORY_PROJECT_DISTILL => (
            "Project memory distillation",
            "Distill bounded turn material into owned project memory.",
        ),
        MEMORY_PROJECT_CITATION => (
            "Project memory citations",
            "Provide citations for selected project-memory entries.",
        ),
        MEMORY_SESSION_SCRATCH => (
            "Session memory scratch",
            "Provide the typed session-scoped memory resource boundary.",
        ),
        MEMORY_COMPANION_RECALL => (
            "Companion memory recall",
            "Provide current context from explicitly bound companion memory.",
        ),
        MEMORY_COMPANION_WRITE => (
            "Companion memory write",
            "Write bounded material to explicitly bound companion memory.",
        ),
        MEMORY_COMPANION_MERGE => (
            "Companion memory merge",
            "Merge bounded material into explicitly bound companion memory.",
        ),
        MEMORY_COMPANION_EVOLVE => (
            "Companion memory evolution",
            "Submit bounded evolution material for companion memory.",
        ),
        SKILL_CATALOG => (
            "Skill catalog",
            "Expose the selected Skill catalog as Agent context.",
        ),
        SKILL_DESCRIBE => (
            "Skill description",
            "Expose selected Skill instructions and metadata as context.",
        ),
        SKILL_INVOKE => (
            "Skill invocation",
            "Invoke an explicitly selected Skill through the capability boundary.",
        ),
        SKILL_HOOKS => (
            "Skill turn hooks",
            "Apply selected Skill turn middleware.",
        ),
        _ => ("Wave 1 capability", "Bundled Wave 1 capability."),
    };
    display(name, description)
}

fn descriptor<const N: usize>(
    slot_key: &'static str,
    resource_kind: &'static str,
    required: bool,
    operations: [&'static str; N],
    binding_policy: &'static str,
) -> TypedResourceDescriptor {
    TypedResourceDescriptor {
        slot_key,
        resource_kind: ResourceKind::from(resource_kind),
        required,
        operations: operations.into_iter().map(str::to_owned).collect(),
        binding_policy,
    }
}

fn resource_binding(
    binding_id: &str,
    resource_kind: &str,
    resource_id: &str,
    operations: &[&str],
    owner_id: &str,
) -> TypedResourceBinding {
    TypedResourceBinding {
        binding_id: ResourceBindingId::from(binding_id),
        resource_kind: ResourceKind::from(resource_kind),
        resource_id: ResourceId::from(resource_id),
        owner_id: owner_id.to_owned(),
        operations: operations.iter().map(|operation| (*operation).to_owned()).collect(),
        connection_config_ref: None,
        typed_parameters: BTreeMap::new(),
    }
}

fn host_port(id: &str) -> nomifun_agent_contracts::HostPortRef {
    nomifun_agent_contracts::HostPortRef {
        id: nomifun_agent_contracts::HostPortId::from(id),
        version: VersionString::from(CONTRACT_VERSION),
    }
}

fn host_port_binding() -> Result<HostPortBindingDescriptor, String> {
    let request_schema = object_schema(true);
    let response_schema = object_schema(true);
    Ok(HostPortBindingDescriptor {
        port: host_port(WAVE1_CAPABILITY_HOST_PORT_ID),
        request_schema: schema_ref(WAVE1_CAPABILITY_HOST_PORT_ID, "request", &request_schema)?,
        response_schema: schema_ref(WAVE1_CAPABILITY_HOST_PORT_ID, "response", &response_schema)?,
    })
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

    struct TestHostPort;

    #[async_trait::async_trait]
    impl Wave1HostPort for TestHostPort {
        async fn invoke(
            &self,
            request: Wave1HostRequest,
        ) -> Result<StrictJsonValue, Wave1HostPortError> {
            let Wave1CapabilityOperation::KnowledgeSearch(search) = request.operation else {
                return Err(Wave1HostPortError::new(
                    "TEST_UNEXPECTED_OPERATION",
                    "test expects a knowledge.search operation",
                ));
            };
            Ok(StrictJsonValue(json!({
                "host_port_invoked": true,
                "capability_id": request.context.capability_id,
                "query": search.query,
                "limit": search.limit
            })))
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
    async fn knowledge_search_invocation_forwards_typed_request_to_host_port() {
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("kernel registry");
        let materialized = registry
            .replace_all(
                registrations_with_host_port(Arc::new(TestHostPort))
                    .expect("Wave 1 registrations"),
            )
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
        let test_action_id =
            action_id("knowledge.search").expect("knowledge.search has an action identity");
        let payload = AgentPresetRevisionPayload {
            schema_version: VersionString::from(CONTRACT_VERSION),
            surfaces: BTreeSet::from(["desktop".to_owned()]),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
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
            agent_session_id: AgentSessionId::from("wave1-test-session"),
            operation_id: OperationId::from("wave1-test-operation"),
            idempotency_key: IdempotencyKey::from("wave1-test-idempotency"),
            correlation_id: CorrelationId::from("wave1-test-correlation"),
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
        assert_eq!(first.0["host_port_invoked"], json!(true));
        assert_eq!(first.0["capability_id"], json!("knowledge.search"));
        assert_eq!(first.0["query"], json!("rust"));
        assert_eq!(first.0["limit"], Value::Null);

        let invalid = CapabilityInvocationRequest {
            input: StrictJsonValue(json!("not-an-object")),
            ..CapabilityInvocationRequest {
                principal: principal(),
                session_owner: principal(),
                agent_session_id: AgentSessionId::from("wave1-invalid-session"),
                operation_id: OperationId::from("wave1-invalid-operation"),
                idempotency_key: IdempotencyKey::from("wave1-invalid-idempotency"),
                correlation_id: CorrelationId::from("wave1-invalid-correlation"),
                resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from("knowledge.search"),
                action_id: action_id("knowledge.search")
                    .expect("knowledge.search has an action identity"),
                resource_binding_ids: BTreeSet::from([ResourceBindingId::from("knowledge")]),
                state_scope_key: ScopeKey::from("session:wave1-test"),
                input: StrictJsonValue(json!({})),
            }
        };
        assert!(registry.invoke(&snapshot, &active, invalid).await.is_err());
    }

    #[test]
    fn knowledge_autogen_requires_a_boolean_overwrite_flag() {
        let operation =
            operation_from_input(&CapabilityId::from(KNOWLEDGE_AUTOGEN), &json!({}))
                .expect("missing overwrite flag defaults to false");
        assert_eq!(
            operation,
            Wave1CapabilityOperation::KnowledgeAutogen(Wave1KnowledgeAutogenRequest {
                overwrite_readme: false,
            })
        );

        let operation = operation_from_input(
            &CapabilityId::from(KNOWLEDGE_AUTOGEN),
            &json!({"overwrite_readme": true}),
        )
        .expect("boolean overwrite flag is accepted");
        assert_eq!(
            operation,
            Wave1CapabilityOperation::KnowledgeAutogen(Wave1KnowledgeAutogenRequest {
                overwrite_readme: true,
            })
        );

        let error = operation_from_input(
            &CapabilityId::from(KNOWLEDGE_AUTOGEN),
            &json!({"overwrite_readme": "true"}),
        )
        .expect_err("a string must not be silently coerced to false");
        assert!(
            error
                .to_string()
                .contains("overwrite_readme` must be a boolean")
        );
    }
}
