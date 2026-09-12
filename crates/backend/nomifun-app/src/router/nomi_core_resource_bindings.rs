//! Server-owned projection from product resource choices to Kernel bindings.
//!
//! The HTTP contract deliberately accepts only `(resource_kind, resource_id)`.
//! This module resolves those two product facts against the authoritative
//! domain service, derives the minimum operation grants from the frozen
//! capability set, and is the only place that creates `TypedResourceBinding`
//! values for a Nomi-core AgentSession.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ConnectionConfigRef, ResourceBindingId, ResourceId, ResourceKind, TypedResourceBinding,
};
use nomifun_api_types::{
    AgentBindingValueDto, AgentResourceSelectionDto, TypedResourceBindingDto,
};
use nomifun_agent_control_plane::AgentControlPlane;
pub(crate) use nomifun_agent_domain_wave3::CREATIVE_ASSET_LIBRARY_RESOURCE_ID;
use nomifun_db::{
    IChannelRepository, IMcpServerRepository, IProviderModelCapabilityRepository,
    IProviderModelRepository, IProviderRepository,
};
use serde_json::{Value, json};

use crate::services::AppServices;

pub(crate) const DEFAULT_WORKSPACE_RESOURCE_ID: &str = "default-workspace";
pub(crate) const DEFAULT_PROJECT_MEMORY_RESOURCE_ID: &str = "default-project-memory";
pub(crate) const MANAGED_PROCESS_SESSION_RESOURCE_ID: &str = "managed-process-session";
pub(crate) const MANAGED_TERMINAL_RESOURCE_ID: &str = "managed-terminal";

const MAX_RESOURCE_SELECTIONS: usize = 32;
const MAX_RESOURCE_FIELD_BYTES: usize = 512;

#[derive(Debug, Clone)]
pub(crate) struct ResourceSelectionResolutionError {
    code: &'static str,
    message: String,
    details: Value,
}

impl ResourceSelectionResolutionError {
    fn new(code: &'static str, message: impl Into<String>, details: Value) -> Self {
        Self {
            code,
            message: message.into(),
            details,
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn details(&self) -> &Value {
        &self.details
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new("RESOURCE_SELECTION_INVALID", message, Value::Null)
    }

    fn forbidden(kind: &str) -> Self {
        Self::new(
            "RESOURCE_OWNER_MISMATCH",
            "the selected resource does not belong to the authenticated owner",
            json!({ "resource_kind": kind }),
        )
    }

    fn not_found(kind: &str, id: &str) -> Self {
        Self::new(
            "RESOURCE_SELECTION_NOT_FOUND",
            "the selected resource does not exist",
            json!({ "resource_kind": kind, "resource_id": id }),
        )
    }

    fn unavailable(kind: &str, id: &str, reason: impl Into<String>) -> Self {
        Self::new(
            "RESOURCE_SELECTION_UNAVAILABLE",
            reason,
            json!({ "resource_kind": kind, "resource_id": id }),
        )
    }
}

#[derive(Clone, Debug)]
struct ResourceAuthorityRequest {
    owner_id: String,
    resource_id: String,
    required_operations: BTreeSet<String>,
    selected_capability_ids: BTreeSet<String>,
    selections_by_kind: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct ServerResolvedResource {
    resource_id: String,
    allowed_operations: BTreeSet<String>,
    connection_config_ref: Option<ConnectionConfigRef>,
    typed_parameters: BTreeMap<String, String>,
}

#[async_trait]
trait NomiCoreResourceAuthority: Send + Sync {
    async fn resolve(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError>;
}

/// Typed registry used by Nomi-core create/switch admission.
///
/// One resolver owns one canonical resource kind. More than one selection for
/// the same kind is rejected because Kernel target bindings intentionally have
/// cardinality one per kind.
#[derive(Clone, Default)]
pub(crate) struct NomiCoreResourceBindingResolverRegistry {
    authorities: Arc<BTreeMap<String, Arc<dyn NomiCoreResourceAuthority>>>,
}

impl NomiCoreResourceBindingResolverRegistry {
    fn from_authorities(
        authorities: impl IntoIterator<Item = (String, Arc<dyn NomiCoreResourceAuthority>)>,
    ) -> Result<Self, ResourceSelectionResolutionError> {
        let mut registry = BTreeMap::new();
        for (kind, authority) in authorities {
            if registry.insert(kind.clone(), authority).is_some() {
                return Err(ResourceSelectionResolutionError::invalid(format!(
                    "duplicate resource authority for kind {kind}"
                )));
            }
        }
        Ok(Self {
            authorities: Arc::new(registry),
        })
    }

    pub(crate) fn product(services: &AppServices) -> Result<Self, ResourceSelectionResolutionError> {
        let dependencies = Arc::new(ProductResourceDependencies {
            authoritative_owner_id: Arc::clone(&services.authoritative_user_id),
            work_dir: services.work_dir.clone(),
            knowledge: Arc::clone(&services.knowledge_service),
            companion: Arc::clone(&services.companion_service),
            customer: Arc::new(
                nomifun_customer_service::CustomerServiceAgentCapabilityOwner::new(
                    Arc::clone(&services.authoritative_user_id),
                    Arc::clone(&services.customer_service_service),
                ),
            ),
            customer_service: Arc::clone(&services.customer_service_service),
            workshop: Arc::clone(&services.workshop_service),
            miniapp: Arc::clone(&services.miniapp_application),
            providers: Arc::clone(&services.provider_repo),
            provider_models: Arc::clone(&services.provider_model_repo),
            provider_capabilities: Arc::clone(&services.provider_model_capability_repo),
            channels: Arc::new(nomifun_db::SqliteChannelRepository::new(
                services.database.pool().clone(),
            )),
            mcp_servers: Arc::new(nomifun_db::SqliteMcpServerRepository::new(
                services.database.pool().clone(),
            )),
            robots: services.robot.as_ref().map(|robot| Arc::clone(&robot.registry)),
        });

        Self::from_authorities(SUPPORTED_RESOURCE_KINDS.into_iter().map(|kind| {
            let authority: Arc<dyn NomiCoreResourceAuthority> = Arc::new(ProductResourceAuthority {
                kind,
                dependencies: Arc::clone(&dependencies),
            });
            (kind.to_owned(), authority)
        }))
    }

    pub(crate) async fn resolve(
        &self,
        owner_id: &str,
        selections: &[AgentResourceSelectionDto],
        selected_capability_ids: &BTreeSet<String>,
    ) -> Result<Vec<TypedResourceBinding>, ResourceSelectionResolutionError> {
        if selections.len() > MAX_RESOURCE_SELECTIONS {
            return Err(ResourceSelectionResolutionError::invalid(format!(
                "resource_selections cannot contain more than {MAX_RESOURCE_SELECTIONS} entries"
            )));
        }

        let required = required_operations(selected_capability_ids);
        let mut selections_by_kind = BTreeMap::new();
        for selection in selections {
            validate_selection_field("resource_kind", &selection.resource_kind)?;
            validate_selection_field("resource_id", &selection.resource_id)?;
            if selections_by_kind
                .insert(
                    selection.resource_kind.clone(),
                    selection.resource_id.clone(),
                )
                .is_some()
            {
                return Err(ResourceSelectionResolutionError::invalid(format!(
                    "resource kind {} was selected more than once",
                    selection.resource_kind
                )));
            }
            if !required.contains_key(&selection.resource_kind) {
                return Err(ResourceSelectionResolutionError::new(
                    "RESOURCE_SELECTION_UNUSED",
                    "the selected resource kind is not required by this Agent",
                    json!({ "resource_kind": selection.resource_kind }),
                ));
            }
        }
        if let (Some(companion_id), Some(memory_id)) = (
            selections_by_kind.get("companion"),
            selections_by_kind.get("companion_memory"),
        ) && companion_id != memory_id
        {
            return Err(ResourceSelectionResolutionError::new(
                "RESOURCE_SELECTION_RELATIONSHIP_MISMATCH",
                "companion and companion-memory selections must identify the same Companion",
                json!({
                    "companion_resource_id": companion_id,
                    "companion_memory_resource_id": memory_id,
                }),
            ));
        }

        let missing = required
            .keys()
            .filter(|kind| !selections_by_kind.contains_key(*kind))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(ResourceSelectionResolutionError::new(
                "RESOURCE_SELECTION_REQUIRED",
                "the Agent requires additional product resources",
                json!({ "missing_resource_kinds": missing }),
            ));
        }

        let mut bindings = Vec::with_capacity(selections.len());
        for (kind, resource_id) in &selections_by_kind {
            let authority = self.authorities.get(kind).ok_or_else(|| {
                ResourceSelectionResolutionError::new(
                    "RESOURCE_SELECTION_KIND_UNSUPPORTED",
                    "the resource kind has no server resolver",
                    json!({ "resource_kind": kind }),
                )
            })?;
            let operations = required.get(kind).cloned().unwrap_or_default();
            let resolved = authority
                .resolve(ResourceAuthorityRequest {
                    owner_id: owner_id.to_owned(),
                    resource_id: resource_id.clone(),
                    required_operations: operations.clone(),
                    selected_capability_ids: selected_capability_ids.clone(),
                    selections_by_kind: selections_by_kind.clone(),
                })
                .await?;
            if !operations.is_subset(&resolved.allowed_operations) {
                return Err(ResourceSelectionResolutionError::new(
                    "RESOURCE_OPERATION_NOT_ALLOWED",
                    "the selected resource cannot satisfy the Agent capability grant",
                    json!({
                        "resource_kind": kind,
                        "resource_id": resource_id,
                        "required_operations": operations,
                    }),
                ));
            }
            bindings.push(TypedResourceBinding {
                binding_id: ResourceBindingId::from(format!("{kind}:{}", resolved.resource_id)),
                resource_kind: ResourceKind::from(kind.clone()),
                resource_id: ResourceId::from(resolved.resource_id),
                owner_id: owner_id.to_owned(),
                operations,
                connection_config_ref: resolved.connection_config_ref,
                typed_parameters: resolved.typed_parameters,
            });
        }
        bindings.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
        Ok(bindings)
    }

    /// Attach server-resolved product resources to an already frozen binding.
    ///
    /// The caller has just resolved `binding` from the authenticated owner's
    /// saved Preset. Reloading its immutable Revision/Snapshot here prevents an
    /// HTTP client from choosing operations or resource kinds independently of
    /// the exact capability ceiling it is about to launch.
    pub(crate) async fn resolve_for_saved_binding(
        &self,
        control_plane: &AgentControlPlane,
        owner: &nomifun_agent_contracts::UserId,
        mut binding: AgentBindingValueDto,
        selections: &[AgentResourceSelectionDto],
    ) -> Result<AgentBindingValueDto, ResourceSelectionResolutionError> {
        let (_, revision, snapshot) = control_plane
            .saved_binding_artifacts(owner, &binding)
            .await
            .map_err(|error| {
                ResourceSelectionResolutionError::new(
                    "RESOURCE_BINDING_ARTIFACT_INVALID",
                    "the saved Agent binding cannot be resolved",
                    json!({ "control_plane_code": error.code().as_ref() }),
                )
            })?;
        let capability_ids = revision
            .payload
            .initial_capabilities
            .iter()
            .chain(&revision.payload.on_demand_capabilities)
            .map(|selection| selection.capability.id.as_ref().to_owned())
            .collect::<BTreeSet<_>>();
        let derived_kinds = required_operations(&capability_ids)
            .into_keys()
            .map(ResourceKind::from)
            .collect::<BTreeSet<_>>();
        if derived_kinds != snapshot.content.required_resource_kinds {
            return Err(ResourceSelectionResolutionError::new(
                "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                "the capability resource requirements differ from the frozen Snapshot",
                json!({
                    "derived_resource_kinds": derived_kinds
                        .iter()
                        .map(|kind| kind.as_ref())
                        .collect::<Vec<_>>(),
                    "snapshot_resource_kinds": snapshot
                        .content
                        .required_resource_kinds
                        .iter()
                        .map(|kind| kind.as_ref())
                        .collect::<Vec<_>>(),
                }),
            ));
        }
        binding.typed_resource_bindings = self
            .resolve(owner.as_ref(), selections, &capability_ids)
            .await?
            .into_iter()
            .map(|binding| TypedResourceBindingDto {
                binding_id: binding.binding_id.as_ref().to_owned(),
                resource_kind: binding.resource_kind.as_ref().to_owned(),
                resource_id: binding.resource_id.as_ref().to_owned(),
                owner_id: binding.owner_id,
                operations: binding.operations,
                connection_config_ref: binding
                    .connection_config_ref
                    .map(|reference| reference.as_ref().to_owned()),
                typed_parameters: binding.typed_parameters,
            })
            .collect();
        Ok(binding)
    }
}

fn validate_selection_field(
    field: &str,
    value: &str,
) -> Result<(), ResourceSelectionResolutionError> {
    if value.trim().is_empty() || value.len() > MAX_RESOURCE_FIELD_BYTES || value != value.trim() {
        return Err(ResourceSelectionResolutionError::invalid(format!(
            "{field} must be a trimmed non-empty value of at most {MAX_RESOURCE_FIELD_BYTES} bytes"
        )));
    }
    Ok(())
}

const SUPPORTED_RESOURCE_KINDS: [&str; 15] = [
    "workspace",
    "knowledge_base",
    "project_memory",
    "process_session",
    "terminal",
    "mcp_server",
    "companion",
    "companion_memory",
    "channel",
    "robot",
    "customer",
    "canvas",
    "asset_library",
    "generation_provider",
    "miniapp",
];

fn required_operations(
    capability_ids: &BTreeSet<String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut required = BTreeMap::<String, BTreeSet<String>>::new();
    let mut grant = |kind: &str, operation: &str| {
        required
            .entry(kind.to_owned())
            .or_default()
            .insert(operation.to_owned());
    };
    for capability in capability_ids {
        match capability.as_str() {
            "fs.read" | "fs.search" | "fs.watch" | "fs.snapshot" | "vcs.status" | "vcs.diff"
            | "workspace.bind" => grant("workspace", "read"),
            "fs.write" | "fs.patch" | "fs.delete" | "vcs.stage" | "vcs.commit"
            | "vcs.push" | "workspace.artifacts" => grant("workspace", "write"),
            "process.exec" | "agent.delegate" | "agent.execution.steer" | "process.session" => {
                grant("process_session", "execute")
            }
            "agent.execution.observe" => grant("process_session", "observe"),
            "terminal.pty" => grant("terminal", "use"),
            "knowledge.search" => grant("knowledge_base", "search"),
            "knowledge.read" | "knowledge.embedding" => grant("knowledge_base", "read"),
            "knowledge.rerank" => {
                grant("knowledge_base", "read");
                grant("knowledge_base", "search");
            }
            "knowledge.write" | "knowledge.autogen" => grant("knowledge_base", "write"),
            "knowledge.mount" => grant("knowledge_base", "mount"),
            "memory.project.read" | "memory.project.citation" | "memory.session.scratch" => {
                grant("project_memory", "read")
            }
            "memory.project.write" | "memory.project.distill" => {
                grant("project_memory", "write")
            }
            "memory.companion.recall" => grant("companion_memory", "read"),
            "memory.companion.write" | "memory.companion.merge" | "memory.companion.evolve"
            | "companion.learn" | "companion.evolve" => grant("companion_memory", "write"),
            "mcp.tool_proxy" => {
                grant("mcp_server", "connect");
                grant("mcp_server", "invoke");
            }
            "mcp.resource" | "connector.data.read" => grant("mcp_server", "read"),
            "connector.data.write" => grant("mcp_server", "invoke"),
            "companion.persona" | "companion.roster" => grant("companion", "read"),
            "channel.receive" => grant("channel", "receive"),
            "channel.reply" => grant("channel", "reply"),
            "channel.send" => grant("channel", "send"),
            "channel.pairing" | "channel.group_policy" => grant("channel", "manage"),
            "customer_service.dialogue" | "customer_service.notes.read" => {
                grant("customer", "read")
            }
            "customer_service.notes.write" | "customer_service.handoff" => {
                grant("customer", "write")
            }
            "robot.link" | "robot.device_tools" => grant("robot", "link"),
            "robot.audio" => grant("robot", "audio"),
            "robot.vision" => grant("robot", "vision"),
            "robot.display" => grant("robot", "display"),
            "robot.motion" => grant("robot", "motion"),
            "workshop.canvas.read" => grant("canvas", "read"),
            "workshop.canvas.edit" | "workshop.template.run" => grant("canvas", "write"),
            "workshop.asset.read" | "office.preview" => grant("asset_library", "read"),
            "workshop.asset.write" | "office.document.edit" | "office.sheet.edit"
            | "office.slides.edit" => grant("asset_library", "write"),
            "creation.text" => grant("generation_provider", "text"),
            "creation.image" | "creation.image_edit" => grant("generation_provider", "image"),
            "creation.video" => grant("generation_provider", "video"),
            "creation.audio" => grant("generation_provider", "audio"),
            "miniapp.read" => grant("miniapp", "read"),
            "miniapp.edit" => grant("miniapp", "edit"),
            "miniapp.publish" => grant("miniapp", "publish"),
            "miniapp.serve" => grant("miniapp", "serve"),
            _ => {}
        }
    }
    required
}

struct ProductResourceDependencies {
    authoritative_owner_id: Arc<str>,
    work_dir: PathBuf,
    knowledge: Arc<nomifun_knowledge::KnowledgeService>,
    companion: Arc<nomifun_companion::CompanionService>,
    customer: Arc<nomifun_customer_service::CustomerServiceAgentCapabilityOwner>,
    customer_service: Arc<nomifun_customer_service::CustomerServiceService>,
    workshop: Arc<nomifun_workshop::WorkshopService>,
    miniapp: Arc<nomifun_miniapp_platform::MiniAppM1ApplicationService>,
    providers: Arc<dyn IProviderRepository>,
    provider_models: Arc<dyn IProviderModelRepository>,
    provider_capabilities: Arc<dyn IProviderModelCapabilityRepository>,
    channels: Arc<dyn IChannelRepository>,
    mcp_servers: Arc<dyn IMcpServerRepository>,
    robots: Option<Arc<nomifun_robot::registry::RobotRegistry>>,
}

struct ProductResourceAuthority {
    kind: &'static str,
    dependencies: Arc<ProductResourceDependencies>,
}

#[async_trait]
impl NomiCoreResourceAuthority for ProductResourceAuthority {
    async fn resolve(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        if request.owner_id != self.dependencies.authoritative_owner_id.as_ref() {
            return Err(ResourceSelectionResolutionError::forbidden(self.kind));
        }
        match self.kind {
            "workspace" => self.resolve_workspace(&request),
            "process_session" => self.resolve_process_session(&request),
            "terminal" => self.resolve_terminal(&request),
            "project_memory" => self.resolve_project_memory(&request),
            "knowledge_base" => self.resolve_knowledge(request).await,
            "companion" | "companion_memory" => self.resolve_companion(request).await,
            "channel" => self.resolve_channel(request).await,
            "mcp_server" => self.resolve_mcp(request).await,
            "robot" => self.resolve_robot(request).await,
            "customer" => self.resolve_customer(request).await,
            "canvas" | "asset_library" => self.resolve_workshop(request).await,
            "generation_provider" => self.resolve_generation_provider(request).await,
            "miniapp" => self.resolve_miniapp(request).await,
            _ => Err(ResourceSelectionResolutionError::invalid(format!(
                "unsupported product resource kind {}",
                self.kind
            ))),
        }
    }
}

impl ProductResourceAuthority {
    fn fixed(
        &self,
        request: &ResourceAuthorityRequest,
        expected_id: &str,
        allowed_operations: &[&str],
        typed_parameters: BTreeMap<String, String>,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        if request.resource_id != expected_id {
            return Err(ResourceSelectionResolutionError::not_found(
                self.kind,
                &request.resource_id,
            ));
        }
        Ok(ServerResolvedResource {
            resource_id: expected_id.to_owned(),
            allowed_operations: allowed_operations
                .iter()
                .map(|operation| (*operation).to_owned())
                .collect(),
            connection_config_ref: None,
            typed_parameters,
        })
    }

    fn resolve_workspace(
        &self,
        request: &ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        if !self.dependencies.work_dir.is_absolute() {
            return Err(ResourceSelectionResolutionError::unavailable(
                self.kind,
                &request.resource_id,
                "the configured workspace root is not absolute",
            ));
        }
        self.fixed(
            request,
            DEFAULT_WORKSPACE_RESOURCE_ID,
            &["read", "write"],
            BTreeMap::from([(
                "workspace_root".to_owned(),
                self.dependencies.work_dir.to_string_lossy().into_owned(),
            )]),
        )
    }

    fn resolve_process_session(
        &self,
        request: &ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        self.fixed(
            request,
            MANAGED_PROCESS_SESSION_RESOURCE_ID,
            &["execute", "observe"],
            BTreeMap::from([(
                "workspace_root".to_owned(),
                self.dependencies.work_dir.to_string_lossy().into_owned(),
            )]),
        )
    }

    fn resolve_terminal(
        &self,
        request: &ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        self.fixed(request, MANAGED_TERMINAL_RESOURCE_ID, &["use"], BTreeMap::new())
    }

    fn resolve_project_memory(
        &self,
        request: &ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        self.fixed(
            request,
            DEFAULT_PROJECT_MEMORY_RESOURCE_ID,
            &["read", "write"],
            BTreeMap::new(),
        )
    }

    async fn resolve_knowledge(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        let info = self
            .dependencies
            .knowledge
            .get_base_info(&request.resource_id)
            .await
            .map_err(|_| ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id))?;
        if !info.root_exists {
            return Err(ResourceSelectionResolutionError::unavailable(
                self.kind,
                &request.resource_id,
                "the selected knowledge-base root is unavailable",
            ));
        }
        let mut allowed = BTreeSet::from(["read".to_owned(), "search".to_owned()]);
        if info.tree_access == nomifun_api_types::KnowledgeTreeAccess::Editable {
            allowed.insert("mount".to_owned());
            allowed.insert("write".to_owned());
        }
        if info.source.is_some() {
            allowed.insert("sync".to_owned());
        }
        Ok(ServerResolvedResource {
            resource_id: info.knowledge_base_id.as_ref().to_owned(),
            allowed_operations: allowed,
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([
                ("knowledge_root".to_owned(), info.root_path),
                ("name".to_owned(), info.name),
            ]),
        })
    }

    async fn resolve_companion(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        let counterpart_kind = if self.kind == "companion" {
            "companion_memory"
        } else {
            "companion"
        };
        if request
            .selections_by_kind
            .get(counterpart_kind)
            .is_some_and(|counterpart_id| counterpart_id != &request.resource_id)
        {
            return Err(ResourceSelectionResolutionError::new(
                "RESOURCE_SELECTION_RELATIONSHIP_MISMATCH",
                "companion and companion-memory selections must identify the same Companion",
                json!({
                    "resource_kind": self.kind,
                    "resource_id": request.resource_id,
                    "related_resource_kind": counterpart_kind,
                }),
            ));
        }
        let companion = self
            .dependencies
            .companion
            .get_companion(&request.resource_id)
            .await
            .map_err(|_| ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id))?;
        Ok(ServerResolvedResource {
            resource_id: companion.companion_id,
            allowed_operations: BTreeSet::from(["read".to_owned(), "write".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        })
    }

    async fn resolve_channel(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        let channel = self
            .dependencies
            .channels
            .get_plugin(&request.resource_id)
            .await
            .map_err(|_| ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id))?
            .ok_or_else(|| ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id))?;
        if !channel.enabled {
            return Err(ResourceSelectionResolutionError::unavailable(
                self.kind,
                &request.resource_id,
                "the selected channel is disabled",
            ));
        }
        let customer_session = request
            .selected_capability_ids
            .iter()
            .any(|capability| capability.starts_with("customer_service."));
        let expected_domain = if customer_session {
            nomifun_db::models::CHANNEL_OWNER_DOMAIN_CUSTOMER_SERVICE
        } else {
            nomifun_db::models::CHANNEL_OWNER_DOMAIN_COMPANION
        };
        if channel.owner_domain != expected_domain {
            return Err(ResourceSelectionResolutionError::forbidden(self.kind));
        }
        let typed_parameters = if expected_domain
            == nomifun_db::models::CHANNEL_OWNER_DOMAIN_COMPANION
        {
            let companion_id = channel.companion_id.as_ref().ok_or_else(|| {
                ResourceSelectionResolutionError::unavailable(
                    self.kind,
                    &request.resource_id,
                    "the selected companion-owned channel has no Companion binding",
                )
            })?;
            if request
                .selections_by_kind
                .get("companion")
                .is_some_and(|selected| selected != companion_id)
            {
                return Err(ResourceSelectionResolutionError::forbidden(self.kind));
            }
            BTreeMap::from([("companion_id".to_owned(), companion_id.clone())])
        } else {
            let cs_agent_id = request
                .selections_by_kind
                .get("customer")
                .ok_or_else(|| {
                    ResourceSelectionResolutionError::invalid(
                        "a customer-service channel requires the same target customer-service Agent selection",
                    )
                })?;
            let current = self
                .dependencies
                .customer_service
                .binding_for_plugin(&channel.channel_plugin_id)
                .await
                .map_err(|error| {
                    ResourceSelectionResolutionError::unavailable(
                        self.kind,
                        &request.resource_id,
                        error.to_string(),
                    )
                })?;
            if current.as_deref() != Some(cs_agent_id.as_str()) {
                return Err(ResourceSelectionResolutionError::forbidden(self.kind));
            }
            BTreeMap::from([("cs_agent_id".to_owned(), cs_agent_id.clone())])
        };
        Ok(ServerResolvedResource {
            resource_id: channel.channel_plugin_id,
            allowed_operations: BTreeSet::from([
                "manage".to_owned(),
                "receive".to_owned(),
                "reply".to_owned(),
                "send".to_owned(),
            ]),
            connection_config_ref: None,
            typed_parameters,
        })
    }

    async fn resolve_mcp(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        let server = self
            .dependencies
            .mcp_servers
            .find_by_id(&request.resource_id)
            .await
            .map_err(|_| ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id))?
            .ok_or_else(|| ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id))?;
        if !server.enabled {
            return Err(ResourceSelectionResolutionError::unavailable(
                self.kind,
                &request.resource_id,
                "the selected MCP server is disabled",
            ));
        }
        Ok(ServerResolvedResource {
            resource_id: server.mcp_server_id.clone(),
            allowed_operations: BTreeSet::from([
                "connect".to_owned(),
                "invoke".to_owned(),
                "read".to_owned(),
            ]),
            connection_config_ref: Some(ConnectionConfigRef::from(format!(
                "mcp-server:{}@{}",
                server.mcp_server_id, server.updated_at
            ))),
            typed_parameters: BTreeMap::new(),
        })
    }

    async fn resolve_robot(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        let registry = self.dependencies.robots.as_ref().ok_or_else(|| {
            ResourceSelectionResolutionError::unavailable(
                self.kind,
                &request.resource_id,
                "the robot service is unavailable",
            )
        })?;
        let robot = registry
            .list()
            .await
            .into_iter()
            .find(|robot| robot.robot_id == request.resource_id)
            .ok_or_else(|| ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id))?;
        let companion = request.selections_by_kind.get("companion");
        let companion_memory = request.selections_by_kind.get("companion_memory");
        if matches!((companion, companion_memory), (Some(left), Some(right)) if left != right) {
            return Err(ResourceSelectionResolutionError::invalid(
                "robot companion and companion-memory selections must identify the same Companion",
            ));
        }
        let selected_companion = companion.or(companion_memory);
        if selected_companion.is_some()
            && robot.companion_id.as_ref() != selected_companion
        {
            return Err(ResourceSelectionResolutionError::forbidden(self.kind));
        }
        let typed_parameters = robot
            .companion_id
            .as_ref()
            .map(|companion_id| {
                BTreeMap::from([("companion_id".to_owned(), companion_id.to_string())])
            })
            .unwrap_or_default();
        Ok(ServerResolvedResource {
            resource_id: robot.robot_id,
            allowed_operations: BTreeSet::from([
                "audio".to_owned(),
                "display".to_owned(),
                "link".to_owned(),
                "motion".to_owned(),
                "vision".to_owned(),
            ]),
            connection_config_ref: None,
            typed_parameters,
        })
    }

    async fn resolve_customer(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        let binding = self
            .dependencies
            .customer
            .resolve_resource_binding(
                &request.owner_id,
                &request.resource_id,
                request.required_operations,
            )
            .await
            .map_err(|error| {
                match error.code.as_str() {
                    nomifun_agent_domain_wave4::WAVE4_RESOURCE_OWNER_MISMATCH => {
                        ResourceSelectionResolutionError::forbidden(self.kind)
                    }
                    code if code.ends_with("_NOT_FOUND") => {
                        ResourceSelectionResolutionError::not_found(
                            self.kind,
                            &request.resource_id,
                        )
                    }
                    nomifun_agent_domain_wave4::WAVE4_RESOURCE_BINDING_INVALID
                    | nomifun_agent_domain_wave4::WAVE4_INVALID_REQUEST => {
                        ResourceSelectionResolutionError::invalid(
                            "the selected customer resource cannot satisfy the requested capability operations",
                        )
                    }
                    _ => ResourceSelectionResolutionError::unavailable(
                        self.kind,
                        &request.resource_id,
                        error.to_string(),
                    ),
                }
            })?;
        Ok(ServerResolvedResource {
            resource_id: binding.resource_id.as_ref().to_owned(),
            allowed_operations: binding.operations,
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        })
    }

    async fn resolve_workshop(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        self.dependencies
            .workshop
            .require_creative_studio_owner(&request.owner_id)
            .await
            .map_err(|_| ResourceSelectionResolutionError::forbidden(self.kind))?;
        if self.kind == "canvas" {
            self.dependencies
                .workshop
                .get_creative_canvas(&request.resource_id)
                .await
                .map_err(|_| ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id))?;
            Ok(ServerResolvedResource {
                resource_id: request.resource_id,
                allowed_operations: BTreeSet::from(["read".to_owned(), "write".to_owned()]),
                connection_config_ref: None,
                typed_parameters: BTreeMap::new(),
            })
        } else {
            self.fixed(
                &request,
                CREATIVE_ASSET_LIBRARY_RESOURCE_ID,
                &["read", "write"],
                BTreeMap::new(),
            )
        }
    }

    async fn resolve_generation_provider(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        let provider = self
            .dependencies
            .providers
            .find_by_id(&request.resource_id)
            .await
            .map_err(|_| ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id))?
            .ok_or_else(|| ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id))?;
        if !provider.enabled {
            return Err(ResourceSelectionResolutionError::unavailable(
                self.kind,
                &request.resource_id,
                "the selected generation provider is disabled",
            ));
        }
        let models = self
            .dependencies
            .provider_models
            .list_for_provider(&provider.provider_id)
            .await
            .map_err(|error| ResourceSelectionResolutionError::unavailable(
                self.kind,
                &request.resource_id,
                format!("provider model inventory is unavailable: {error}"),
            ))?;
        let enabled_models = models
            .iter()
            .filter(|model| model.enabled)
            .map(|model| (model.model.clone(), model.sort_order))
            .collect::<BTreeMap<_, _>>();
        let capabilities = self
            .dependencies
            .provider_capabilities
            .list_for_provider(&provider.provider_id)
            .await
            .map_err(|error| ResourceSelectionResolutionError::unavailable(
                self.kind,
                &request.resource_id,
                format!("provider capability inventory is unavailable: {error}"),
            ))?;
        let mut tasks = Vec::new();
        for capability in &request.selected_capability_ids {
            let mapping = match capability.as_str() {
                "creation.text" => Some(("creation.text", "chat")),
                "creation.image" => Some(("creation.image", "image_generation")),
                "creation.image_edit" => Some(("creation.image_edit", "image_edit")),
                "creation.video" => Some(("creation.video", "video_generation")),
                "creation.audio" => Some(("creation.audio", "speech_synthesis")),
                _ => None,
            };
            if let Some(mapping) = mapping {
                tasks.push(mapping);
            }
        }
        let mut typed_parameters = BTreeMap::new();
        let mut chosen_models = BTreeSet::new();
        for (capability_id, task) in tasks {
            let selected_model = capabilities
                .iter()
                .filter(|capability| capability.task == task)
                .filter_map(|capability| {
                    enabled_models
                        .get(&capability.model)
                        .map(|sort_order| (sort_order, capability.model.as_str()))
                })
                .min_by(|left, right| left.cmp(right))
                .map(|(_, model)| model.to_owned())
                .ok_or_else(|| {
                    ResourceSelectionResolutionError::unavailable(
                        self.kind,
                        &request.resource_id,
                        format!("the provider has no enabled model for task {task}"),
                    )
                })?;
            typed_parameters.insert(format!("model.{capability_id}"), selected_model.clone());
            chosen_models.insert(selected_model);
        }
        if chosen_models.len() == 1 {
            typed_parameters.insert(
                "model".to_owned(),
                chosen_models.into_iter().next().expect("one model"),
            );
        }
        Ok(ServerResolvedResource {
            resource_id: provider.provider_id.clone(),
            allowed_operations: BTreeSet::from([
                "audio".to_owned(),
                "image".to_owned(),
                "text".to_owned(),
                "video".to_owned(),
            ]),
            connection_config_ref: Some(ConnectionConfigRef::from(format!(
                "provider:{}@{}",
                provider.provider_id, provider.config_revision
            ))),
            typed_parameters,
        })
    }

    async fn resolve_miniapp(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        self.dependencies
            .miniapp
            .workshop(&request.owner_id, &request.resource_id)
            .await
            .map_err(|_| ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id))?;
        Ok(ServerResolvedResource {
            resource_id: request.resource_id,
            allowed_operations: BTreeSet::from([
                "edit".to_owned(),
                "publish".to_owned(),
                "read".to_owned(),
                "serve".to_owned(),
            ]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingAuthority {
        allowed: BTreeSet<String>,
    }

    #[async_trait]
    impl NomiCoreResourceAuthority for RecordingAuthority {
        async fn resolve(
            &self,
            request: ResourceAuthorityRequest,
        ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
            Ok(ServerResolvedResource {
                resource_id: request.resource_id,
                allowed_operations: self.allowed.clone(),
                connection_config_ref: None,
                typed_parameters: BTreeMap::new(),
            })
        }
    }

    fn registry(kind: &str, operations: &[&str]) -> NomiCoreResourceBindingResolverRegistry {
        NomiCoreResourceBindingResolverRegistry::from_authorities([(
            kind.to_owned(),
            Arc::new(RecordingAuthority {
                allowed: operations.iter().map(|value| (*value).to_owned()).collect(),
            }) as Arc<dyn NomiCoreResourceAuthority>,
        )])
        .unwrap()
    }

    #[tokio::test]
    async fn client_selection_derives_owner_and_capability_operations() {
        let bindings = registry("customer", &["read", "write"])
            .resolve(
                "owner-1",
                &[AgentResourceSelectionDto {
                    resource_kind: "customer".into(),
                    resource_id: "customer-1".into(),
                }],
                &BTreeSet::from([
                    "customer_service.notes.read".into(),
                    "customer_service.handoff".into(),
                ]),
            )
            .await
            .unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].owner_id, "owner-1");
        assert_eq!(bindings[0].binding_id.as_ref(), "customer:customer-1");
        assert_eq!(bindings[0].operations, BTreeSet::from(["read".into(), "write".into()]));
    }

    #[tokio::test]
    async fn duplicate_unused_and_missing_selections_fail_closed() {
        let registry = registry("knowledge_base", &["search"]);
        let capabilities = BTreeSet::from(["knowledge.search".to_owned()]);
        let duplicate = registry
            .resolve(
                "owner-1",
                &[
                    AgentResourceSelectionDto {
                        resource_kind: "knowledge_base".into(),
                        resource_id: "one".into(),
                    },
                    AgentResourceSelectionDto {
                        resource_kind: "knowledge_base".into(),
                        resource_id: "two".into(),
                    },
                ],
                &capabilities,
            )
            .await
            .unwrap_err();
        assert_eq!(duplicate.code(), "RESOURCE_SELECTION_INVALID");

        let missing = registry
            .resolve("owner-1", &[], &capabilities)
            .await
            .unwrap_err();
        assert_eq!(missing.code(), "RESOURCE_SELECTION_REQUIRED");

        let unused = registry
            .resolve(
                "owner-1",
                &[AgentResourceSelectionDto {
                    resource_kind: "knowledge_base".into(),
                    resource_id: "one".into(),
                }],
                &BTreeSet::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(unused.code(), "RESOURCE_SELECTION_UNUSED");
    }

    #[tokio::test]
    async fn authority_cannot_grant_less_than_the_server_derived_requirement() {
        let error = registry("customer", &["read"])
            .resolve(
                "owner-1",
                &[AgentResourceSelectionDto {
                    resource_kind: "customer".into(),
                    resource_id: "customer-1".into(),
                }],
                &BTreeSet::from(["customer_service.notes.write".into()]),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "RESOURCE_OPERATION_NOT_ALLOWED");
    }

    #[tokio::test]
    async fn companion_and_memory_cannot_cross_bind_two_companions() {
        let authority = || {
            Arc::new(RecordingAuthority {
                allowed: BTreeSet::from(["read".to_owned(), "write".to_owned()]),
            }) as Arc<dyn NomiCoreResourceAuthority>
        };
        let registry = NomiCoreResourceBindingResolverRegistry::from_authorities([
            ("companion".to_owned(), authority()),
            ("companion_memory".to_owned(), authority()),
        ])
        .unwrap();
        let error = registry
            .resolve(
                "owner-1",
                &[
                    AgentResourceSelectionDto {
                        resource_kind: "companion".into(),
                        resource_id: "companion-a".into(),
                    },
                    AgentResourceSelectionDto {
                        resource_kind: "companion_memory".into(),
                        resource_id: "companion-b".into(),
                    },
                ],
                &BTreeSet::from([
                    "companion.persona".into(),
                    "memory.companion.recall".into(),
                ]),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "RESOURCE_SELECTION_RELATIONSHIP_MISMATCH");
    }

    #[test]
    fn all_published_resource_kinds_have_capability_operation_derivation() {
        let capabilities = BTreeSet::from([
            "fs.read".into(),
            "knowledge.search".into(),
            "memory.project.read".into(),
            "process.exec".into(),
            "terminal.pty".into(),
            "mcp.tool_proxy".into(),
            "companion.persona".into(),
            "memory.companion.recall".into(),
            "channel.receive".into(),
            "robot.link".into(),
            "customer_service.dialogue".into(),
            "workshop.canvas.read".into(),
            "workshop.asset.read".into(),
            "creation.image".into(),
            "miniapp.read".into(),
        ]);
        let derived = required_operations(&capabilities);
        assert_eq!(
            derived.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            SUPPORTED_RESOURCE_KINDS.into_iter().collect()
        );
    }
}
