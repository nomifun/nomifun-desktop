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
    ActionId, CapabilityId, ConnectionConfigRef, ResolvedMcpToolLock, ResourceBindingId,
    ResourceId, ResourceKind, TypedResourceBinding,
};
use nomifun_api_types::{
    AgentBindingValueDto, AgentResourceSelectionDto, TypedResourceBindingDto,
};
use nomifun_agent_control_plane::AgentControlPlane;
pub(crate) use nomifun_agent_domain_wave3::CREATIVE_ASSET_LIBRARY_RESOURCE_ID;
use nomifun_db::{
    IChannelRepository, IMcpServerRepository,
};
use serde_json::{Value, json};

use crate::services::AppServices;

pub(crate) const DEFAULT_WORKSPACE_RESOURCE_ID: &str = "default-workspace";
pub(crate) const DEFAULT_PROJECT_MEMORY_RESOURCE_ID: &str = "default-project-memory";
pub(crate) const MANAGED_PROCESS_SESSION_RESOURCE_ID: &str = "managed-process-session";
pub(crate) const MANAGED_TERMINAL_RESOURCE_ID: &str = "managed-terminal";
pub(crate) const MANAGED_BROWSER_RESOURCE_ID: &str = "managed-browser";
pub(crate) const ATTACHED_CHROME_RESOURCE_ID: &str = "attached-chrome";
pub(crate) const LOCAL_COMPUTER_RESOURCE_ID: &str = "local-desktop";
pub(crate) const INSTALLATION_SCHEDULER_RESOURCE_ID: &str = "installation-scheduler";

const MAX_RESOURCE_SELECTIONS: usize = 32;
const MAX_RESOURCE_FIELD_BYTES: usize = 512;
// Knowledge mounting is deliberately optional at session creation. A session
// may start without a base and gain its conversation-scoped, read/write policy
// through the knowledge binding control before the first task is delivered.
const OPTIONAL_UNBOUND_RESOURCE_KINDS: [&str; 1] = ["knowledge_base"];

type FrozenActionAllowlists = BTreeMap<String, BTreeSet<ActionId>>;

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
/// One resolver owns one canonical resource kind. Only frozen per-tool MCP
/// mappings permit multiple resources of one kind; all others remain singular.
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
            plugin_runtime: Arc::clone(&services.plugin_runtime),
            channels: Arc::new(nomifun_db::SqliteChannelRepository::new(
                services.database.pool().clone(),
            )),
            mcp_servers: Arc::new(nomifun_db::SqliteMcpServerRepository::new(
                services.database.pool().clone(),
            )),
            ssh_hosts: services.ssh_pool.host_service(),
            robots: services.robot.as_ref().map(|robot| Arc::clone(&robot.registry)),
            #[cfg(feature = "browser-use")]
            managed_browser_available: services.browser_resources.is_some(),
            #[cfg(feature = "browser-use")]
            attached_chrome: services.attached_chrome.clone(),
        });

        Self::from_authorities(SUPPORTED_RESOURCE_KINDS.into_iter().map(|kind| {
            let authority: Arc<dyn NomiCoreResourceAuthority> = Arc::new(ProductResourceAuthority {
                kind,
                dependencies: Arc::clone(&dependencies),
            });
            (kind.to_owned(), authority)
        }))
    }

    /// Single-resource resolution for capability-owned resources and explicit
    /// resource-only MCP bindings. Per-tool MCP invoke authority additionally
    /// requires saved Snapshot locks, supplied by `resolve_for_saved_binding`.
    #[cfg(test)]
    async fn resolve(
        &self,
        owner_id: &str,
        selections: &[AgentResourceSelectionDto],
        selected_capability_ids: &BTreeSet<String>,
    ) -> Result<Vec<TypedResourceBinding>, ResourceSelectionResolutionError> {
        self.resolve_selected(
            owner_id,
            selections,
            selected_capability_ids,
            &FrozenActionAllowlists::new(),
            &[],
        )
        .await
    }

    async fn resolve_selected(
        &self,
        owner_id: &str,
        selections: &[AgentResourceSelectionDto],
        selected_capability_ids: &BTreeSet<String>,
        action_allowlists: &FrozenActionAllowlists,
        mcp_locks: &[ResolvedMcpToolLock],
    ) -> Result<Vec<TypedResourceBinding>, ResourceSelectionResolutionError> {
        if selections.len() > MAX_RESOURCE_SELECTIONS {
            return Err(ResourceSelectionResolutionError::invalid(format!(
                "resource_selections cannot contain more than {MAX_RESOURCE_SELECTIONS} entries"
            )));
        }

        if selected_capability_ids
            .iter()
            .any(|id| nomifun_mcp::is_retired_mcp_authoring_capability(id))
        {
            return Err(ResourceSelectionResolutionError::new(
                "MCP_LEGACY_CAPABILITY_RETIRED",
                "MCP servers are bound resources and tools require namespaced per-tool Actions",
                Value::Null,
            ));
        }
        let mcp_lock_ids = mcp_locks
            .iter()
            .map(|lock| lock.capability_id.as_ref())
            .collect::<BTreeSet<_>>();
        let selected_mcp_tool_ids = selected_capability_ids
            .iter()
            .map(String::as_str)
            .filter(|id| super::nomi_core_mcp_catalog::is_product_tool(id))
            .collect::<BTreeSet<_>>();
        if mcp_lock_ids.len() != mcp_locks.len()
            || selected_mcp_tool_ids != mcp_lock_ids
            || mcp_locks.iter().any(|lock| {
                nomifun_api_types::McpServerId::parse(lock.server_id.as_ref().to_owned()).is_err()
                    || !super::nomi_core_mcp_catalog::is_product_tool(lock.capability_id.as_ref())
                    || lock.canonical_tool_key.as_ref() != lock.capability_id.as_ref()
                    || lock.materialization_revision
                        != nomifun_mcp::MCP_TOOL_MATERIALIZATION_REVISION
                    || !selected_capability_ids.contains(lock.capability_id.as_ref())
            })
        {
            return Err(ResourceSelectionResolutionError::new(
                "MCP_RESOURCE_SELECTION_MISMATCH",
                "MCP Snapshot locks must identify selected namespaced per-tool Actions",
                Value::Null,
            ));
        }

        let mut required = required_operations(selected_capability_ids, action_allowlists)?;
        let selected_mcp_servers = selections
            .iter()
            .filter(|selection| selection.resource_kind == "mcp_server")
            .count();
        if selected_mcp_servers > 0 {
            required
                .entry("mcp_server".to_owned())
                .or_default()
                .extend(["connect".to_owned(), "read".to_owned()]);
        }
        let mut selections_by_kind = BTreeMap::new();
        let mut selected_pairs = BTreeSet::new();
        for selection in selections {
            validate_selection_field("resource_kind", &selection.resource_kind)?;
            validate_selection_field("resource_id", &selection.resource_id)?;
            if !selected_pairs.insert((selection.resource_kind.clone(), selection.resource_id.clone())) {
                return Err(ResourceSelectionResolutionError::invalid("duplicate resource selection"));
            }
            if selections_by_kind
                .insert(
                    selection.resource_kind.clone(),
                    selection.resource_id.clone(),
                )
                .is_some() && selection.resource_kind != "mcp_server"
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
        if !mcp_locks.is_empty() || selected_mcp_servers > 0 {
            let expected = mcp_locks.iter().map(|lock| lock.server_id.as_ref()).collect::<BTreeSet<_>>();
            let actual = selections.iter().filter(|selection| selection.resource_kind == "mcp_server")
                .map(|selection| selection.resource_id.as_str()).collect::<BTreeSet<_>>();
            if !expected.is_subset(&actual)
                || actual.len() > super::nomi_core_mcp_catalog::MAX_SESSION_SERVERS {
                return Err(ResourceSelectionResolutionError::new("MCP_RESOURCE_SELECTION_MISMATCH",
                    "MCP selection must contain every frozen tool server and the total is bounded to 16", Value::Null));
            }
            // This map is only for cross-kind relationship checks. Never
            // expose an arbitrary last MCP server as the singular selection.
            selections_by_kind.remove("mcp_server");
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
            .filter(|kind| {
                !selections.iter().any(|selection| &selection.resource_kind == *kind)
                    && !OPTIONAL_UNBOUND_RESOURCE_KINDS.contains(&kind.as_str())
            })
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
        for (kind, resource_id) in &selected_pairs {
            let authority = self.authorities.get(kind).ok_or_else(|| {
                ResourceSelectionResolutionError::new(
                    "RESOURCE_SELECTION_KIND_UNSUPPORTED",
                    "the resource kind has no server resolver",
                    json!({ "resource_kind": kind }),
                )
            })?;
            let resource_capabilities = selected_capability_ids.iter().filter(|id|
                kind != "mcp_server" || mcp_locks.is_empty()
                || !super::nomi_core_mcp_catalog::is_product_tool(id)
                || mcp_locks.iter().any(|lock| lock.capability_id.as_ref() == id.as_str()
                    && lock.server_id.as_ref() == resource_id.as_str())).cloned().collect::<BTreeSet<_>>();
            // Resource-only members must not inherit invoke from tools frozen
            // to another server, even though per-tool policy also filters it.
            let mut operations = required_operations(&resource_capabilities, action_allowlists)?
                .get(kind).cloned().unwrap_or_default();
            if kind == "mcp_server" {
                operations.extend(["connect".to_owned(), "read".to_owned()]);
                if mcp_locks
                    .iter()
                    .any(|lock| lock.server_id.as_ref() == resource_id.as_str())
                {
                    operations.insert("invoke".to_owned());
                }
            }
            let resolved = authority
                .resolve(ResourceAuthorityRequest {
                    owner_id: owner_id.to_owned(),
                    resource_id: resource_id.clone(),
                    required_operations: operations.clone(),
                    selected_capability_ids: resource_capabilities,
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
        let (_, _revision, snapshot) = control_plane
            .saved_binding_artifacts(owner, &binding)
            .await
            .map_err(|error| {
                ResourceSelectionResolutionError::new(
                    "RESOURCE_BINDING_ARTIFACT_INVALID",
                    "the saved Agent binding cannot be resolved",
                    json!({ "control_plane_code": error.code().as_ref() }),
                )
            })?;
        let capability_ids = snapshot
            .content
            .enabled_capabilities
            .iter()
            .map(|capability| capability.capability.id.as_ref().to_owned())
            .collect::<BTreeSet<_>>();
        let action_allowlists = snapshot
            .content
            .enabled_capabilities
            .iter()
            .map(|capability| {
                (
                    capability.capability.id.as_ref().to_owned(),
                    capability.action_allowlist.clone(),
                )
            })
            .collect::<FrozenActionAllowlists>();
        let derived_kinds = required_operations(&capability_ids, &action_allowlists)?
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
            .resolve_selected(
                owner.as_ref(),
                selections,
                &capability_ids,
                &action_allowlists,
                &snapshot.content.mcp_tool_locks,
            )
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

const SUPPORTED_RESOURCE_KINDS: [&str; 18] = [
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
    "plugin",
    "ssh_host",
    "browser",
    "computer",
    "scheduler",
];

fn required_operations(
    capability_ids: &BTreeSet<String>,
    action_allowlists: &FrozenActionAllowlists,
) -> Result<BTreeMap<String, BTreeSet<String>>, ResourceSelectionResolutionError> {
    let mut required = BTreeMap::<String, BTreeSet<String>>::new();
    let mut grant = |kind: &str, operation: &str| {
        required
            .entry(kind.to_owned())
            .or_default()
            .insert(operation.to_owned());
    };
    for capability in capability_ids {
        if nomifun_agent_domain_wave2::WORKSPACE_EXECUTION_CAPABILITY_IDS
            .contains(&capability.as_str())
            || capability == nomifun_agent_domain_wave2::SSH_MODULE_ID
            || capability == nomifun_agent_domain_wave2::BROWSER_MODULE_ID
            || capability == nomifun_agent_domain_wave2::COMPUTER_MODULE_ID
        {
            let actions = action_allowlists.get(capability).ok_or_else(|| {
                ResourceSelectionResolutionError::new(
                    "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                    "a Wave 2 Module is missing its frozen exact Action grant",
                    json!({ "capability_id": capability }),
                )
            })?;
            let resource_kinds =
                nomifun_agent_domain_wave2::required_resource_kinds(capability).ok_or_else(|| {
                    ResourceSelectionResolutionError::new(
                        "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                        "a Wave 2 Module has no canonical resource declaration",
                        json!({ "capability_id": capability }),
                    )
                })?;
            if resource_kinds.len() != 1 {
                return Err(ResourceSelectionResolutionError::new(
                    "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                    "a Wave 2 Module must declare exactly one canonical resource kind",
                    json!({ "capability_id": capability }),
                ));
            }
            let resource_kind = resource_kinds
                .iter()
                .next()
                .expect("exactly one Wave 2 resource kind");
            let capability_id = CapabilityId::from(capability.clone());
            for action_id in actions {
                let operation =
                    nomifun_agent_domain_wave2::required_action_resource_operation(
                        &capability_id,
                        action_id,
                    )
                    .ok_or_else(|| {
                        ResourceSelectionResolutionError::new(
                            "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                            "a frozen Wave 2 Action has no canonical resource operation",
                            json!({
                                "capability_id": capability,
                                "action_id": action_id.as_ref(),
                            }),
                        )
                    })?;
                grant(resource_kind.as_ref(), operation);
            }
            continue;
        }
        let module_resource_operations = if nomifun_agent_domain_wave1::CAPABILITY_IDS
            .contains(&capability.as_str())
        {
            Some(
                action_allowlists
                    .get(capability)
                    .ok_or_else(|| {
                        ResourceSelectionResolutionError::new(
                            "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                            "a Wave 1 Module is missing its frozen exact Action grant",
                            json!({ "capability_id": capability }),
                        )
                    })?
                    .iter()
                    .map(|action_id| {
                        nomifun_agent_domain_wave1::required_action_resource_operations(
                            capability,
                            action_id.as_ref(),
                        )
                        .ok_or_else(|| {
                            ResourceSelectionResolutionError::new(
                                "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                                "a frozen Wave 1 Action has no canonical resource operation contract",
                                json!({ "capability_id": capability, "action_id": action_id.as_ref() }),
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
        } else if nomifun_agent_domain_wave5::TARGET_CAPABILITY_IDS
            .contains(&capability.as_str())
        {
            Some(
                action_allowlists
                    .get(capability)
                    .ok_or_else(|| {
                        ResourceSelectionResolutionError::new(
                            "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                            "a Wave 5 Module is missing its frozen exact Action grant",
                            json!({ "capability_id": capability }),
                        )
                    })?
                    .iter()
                    .map(|action_id| {
                        nomifun_agent_domain_wave5::required_action_resource_operations(
                            capability,
                            action_id.as_ref(),
                        )
                        .ok_or_else(|| {
                            ResourceSelectionResolutionError::new(
                                "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                                "a frozen Wave 5 Action has no canonical resource operation contract",
                                json!({ "capability_id": capability, "action_id": action_id.as_ref() }),
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
        } else if nomifun_agent_domain_wave3::TARGET_CAPABILITY_IDS
            .contains(&capability.as_str())
        {
            Some(
                action_allowlists
                    .get(capability)
                    .ok_or_else(|| {
                        ResourceSelectionResolutionError::new(
                            "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                            "a Wave 3 Module is missing its frozen exact Action grant",
                            json!({ "capability_id": capability }),
                        )
                    })?
                    .iter()
                    .map(|action_id| {
                        nomifun_agent_domain_wave3::required_action_resource_operations(
                            capability,
                            action_id.as_ref(),
                        )
                        .ok_or_else(|| {
                            ResourceSelectionResolutionError::new(
                                "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                                "a frozen Wave 3 Action has no canonical resource operation contract",
                                json!({ "capability_id": capability, "action_id": action_id.as_ref() }),
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
        } else if nomifun_agent_domain_wave4::CONVERSATION_MODULE_IDS
            .contains(&capability.as_str())
            || capability == nomifun_agent_domain_wave4::ROBOT_MODULE_ID
        {
            match capability.as_str() {
                nomifun_agent_domain_wave4::CHANNEL_MESSAGING_MODULE_ID => {
                    grant(nomifun_agent_domain_wave4::CHANNEL_RESOURCE_KIND, "receive");
                    grant(nomifun_agent_domain_wave4::CHANNEL_RESOURCE_KIND, "manage");
                }
                nomifun_agent_domain_wave4::COMPANION_MODULE_ID => {
                    grant(nomifun_agent_domain_wave4::COMPANION_RESOURCE_KIND, "read");
                }
                nomifun_agent_domain_wave4::CUSTOMER_SERVICE_MODULE_ID => {
                    grant(nomifun_agent_domain_wave4::CUSTOMER_RESOURCE_KIND, "read");
                }
                _ => {}
            }
            Some(
                action_allowlists
                    .get(capability)
                    .ok_or_else(|| {
                        ResourceSelectionResolutionError::new(
                            "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                            "a conversation Module is missing its frozen exact Action grant",
                            json!({ "capability_id": capability }),
                        )
                    })?
                    .iter()
                    .map(|action_id| {
                        nomifun_agent_domain_wave4::required_action_resource_operations(
                            capability,
                            action_id.as_ref(),
                        )
                        .ok_or_else(|| {
                            ResourceSelectionResolutionError::new(
                                "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH",
                                "a frozen conversation Action has no canonical resource operation contract",
                                json!({ "capability_id": capability, "action_id": action_id.as_ref() }),
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )
        } else {
            None
        };
        if let Some(operation_sets) = module_resource_operations {
            for operations in operation_sets {
                for (resource_kind, operation) in operations {
                    grant(resource_kind.as_ref(), &operation);
                }
            }
            continue;
        }
        match capability.as_str() {
            "agent.delegate" | "agent.execution.steer" => {
                grant("process_session", "execute")
            }
            "agent.execution.observe" => grant("process_session", "observe"),
            id if super::nomi_core_mcp_catalog::is_product_tool(id) => {
                grant("mcp_server", "connect");
                grant("mcp_server", "invoke");
            }
            _ => {}
        }
    }
    Ok(required)
}

struct ProductResourceDependencies {
    authoritative_owner_id: Arc<str>,
    work_dir: PathBuf,
    knowledge: Arc<nomifun_knowledge::KnowledgeService>,
    companion: Arc<nomifun_companion::CompanionService>,
    customer: Arc<nomifun_customer_service::CustomerServiceAgentCapabilityOwner>,
    customer_service: Arc<nomifun_customer_service::CustomerServiceService>,
    workshop: Arc<nomifun_workshop::WorkshopService>,
    plugin_runtime: Arc<nomifun_plugin_platform::runtime::PluginRuntimeApplicationService>,
    channels: Arc<dyn IChannelRepository>,
    mcp_servers: Arc<dyn IMcpServerRepository>,
    ssh_hosts: nomifun_ssh::SshHostService,
    robots: Option<Arc<nomifun_robot::registry::RobotRegistry>>,
    #[cfg(feature = "browser-use")]
    managed_browser_available: bool,
    #[cfg(feature = "browser-use")]
    attached_chrome: Option<Arc<crate::AttachedChromeProviderService>>,
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
            "scheduler" => self.resolve_scheduler(&request),
            "browser" => self.resolve_browser(&request),
            "computer" => self.resolve_computer(&request),
            "ssh_host" => self.resolve_ssh(request).await,
            "knowledge_base" => self.resolve_knowledge(request).await,
            "companion" | "companion_memory" => self.resolve_companion(request).await,
            "channel" => self.resolve_channel(request).await,
            "mcp_server" => self.resolve_mcp(request).await,
            "robot" => self.resolve_robot(request).await,
            "customer" => self.resolve_customer(request).await,
            "canvas" | "asset_library" => self.resolve_workshop(request).await,
            "plugin" => self.resolve_plugin(request).await,
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

    fn resolve_scheduler(
        &self,
        request: &ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        self.fixed(
            request,
            INSTALLATION_SCHEDULER_RESOURCE_ID,
            &["read", "write", "delete"],
            BTreeMap::new(),
        )
    }

    fn resolve_computer(
        &self,
        request: &ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        self.fixed(
            request,
            LOCAL_COMPUTER_RESOURCE_ID,
            &["observe", "input", "launch"],
            BTreeMap::from([("scope".to_owned(), "local_desktop".to_owned())]),
        )
    }

    #[cfg(feature = "browser-use")]
    fn resolve_browser(
        &self,
        request: &ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        let provider_kind = match request.resource_id.as_str() {
            MANAGED_BROWSER_RESOURCE_ID if self.dependencies.managed_browser_available => {
                "managed"
            }
            MANAGED_BROWSER_RESOURCE_ID => {
                return Err(ResourceSelectionResolutionError::unavailable(
                    self.kind,
                    &request.resource_id,
                    "the managed Browser Provider is unavailable on this host",
                ));
            }
            ATTACHED_CHROME_RESOURCE_ID => {
                let service = self.dependencies.attached_chrome.as_ref().ok_or_else(|| {
                    ResourceSelectionResolutionError::unavailable(
                        self.kind,
                        &request.resource_id,
                        "the attached Chrome Provider is unavailable on this host",
                    )
                })?;
                if service
                    .snapshot(&request.owner_id)
                    .map_err(|error| {
                        ResourceSelectionResolutionError::unavailable(
                            self.kind,
                            &request.resource_id,
                            error.to_string(),
                        )
                    })?
                    .is_none()
                {
                    return Err(ResourceSelectionResolutionError::unavailable(
                        self.kind,
                        &request.resource_id,
                        "connect the installation-level attached Chrome Provider first",
                    ));
                }
                "attached_chrome"
            }
            _ => {
                return Err(ResourceSelectionResolutionError::not_found(
                    self.kind,
                    &request.resource_id,
                ));
            }
        };
        Ok(ServerResolvedResource {
            resource_id: request.resource_id.clone(),
            allowed_operations: nomifun_browser_platform::product::BrowserCapabilityAction::all()
                .map(|action| action.resource_operation().as_str().to_owned())
                .into_iter()
                .collect(),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([
                ("provider_kind".to_owned(), provider_kind.to_owned()),
                ("persistence".to_owned(), "persistent".to_owned()),
            ]),
        })
    }

    #[cfg(not(feature = "browser-use"))]
    fn resolve_browser(
        &self,
        request: &ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        Err(ResourceSelectionResolutionError::unavailable(
            self.kind,
            &request.resource_id,
            "Browser Providers are unavailable in this host build",
        ))
    }

    async fn resolve_ssh(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        let id = nomifun_common::SshHostId::parse(request.resource_id.clone()).map_err(|_| {
            ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id)
        })?;
        self.dependencies
            .ssh_hosts
            .get(&request.owner_id, &id)
            .await
            .map_err(|error| match error {
                nomifun_ssh::SshServiceError::NotFound => {
                    ResourceSelectionResolutionError::not_found(self.kind, &request.resource_id)
                }
                other => ResourceSelectionResolutionError::unavailable(
                    self.kind,
                    &request.resource_id,
                    other.to_string(),
                ),
            })?;
        Ok(ServerResolvedResource {
            resource_id: request.resource_id,
            allowed_operations: nomifun_ssh::SSH_HOST_RESOURCE_OPERATIONS
                .into_iter()
                .map(str::to_owned)
                .collect(),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([("remote_cwd".to_owned(), ".".to_owned())]),
        })
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
            .any(|capability| capability == "customer.service");
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
        let selected_tools = request.selected_capability_ids.iter()
            .filter(|id| super::nomi_core_mcp_catalog::is_product_tool(id)).collect::<BTreeSet<_>>();
        if !selected_tools.is_empty() {
            let tools = super::nomi_core_mcp_catalog::server_tools(server.clone()).map_err(|_| {
                ResourceSelectionResolutionError::unavailable(self.kind, &request.resource_id, "the selected MCP catalog cannot materialize exact tool contracts")
            })?;
            let current = tools.into_iter().map(|tool| tool.lock.capability_id.as_ref().to_owned()).collect::<BTreeSet<_>>();
            if selected_tools.iter().any(|id| !current.contains(id.as_str())) {
                return Err(ResourceSelectionResolutionError::unavailable(self.kind, &request.resource_id, "selected MCP tools belong to another server or are no longer available"));
            }
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
                "device".to_owned(),
                "display".to_owned(),
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

    async fn resolve_plugin(
        &self,
        request: ResourceAuthorityRequest,
    ) -> Result<ServerResolvedResource, ResourceSelectionResolutionError> {
        self.dependencies
            .plugin_runtime
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

    const MCP_SERVER_A: &str = "0195f7c0-7b6a-7c21-8f4a-1234567890ab";
    const MCP_SERVER_B: &str = "0195f7c0-7b6a-7c21-8f4a-1234567890ac";

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
            .resolve_selected(
                "owner-1",
                &[AgentResourceSelectionDto {
                    resource_kind: "customer".into(),
                    resource_id: "customer-1".into(),
                }],
                &BTreeSet::from(["customer.service".into()]),
                &BTreeMap::from([(
                    "customer.service".into(),
                    BTreeSet::from([
                        ActionId::from("customer.service/notes.read"),
                        ActionId::from("customer.service/handoff"),
                    ]),
                )]),
                &[],
            )
            .await
            .unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].owner_id, "owner-1");
        assert_eq!(bindings[0].binding_id.as_ref(), "customer:customer-1");
        assert_eq!(bindings[0].operations, BTreeSet::from(["read".into(), "write".into()]));
    }

    #[test]
    fn conversation_modules_derive_scene_operations_outside_action_grants() {
        let capabilities = BTreeSet::from([
            "channel.messaging".to_owned(),
            "companion".to_owned(),
            "customer.service".to_owned(),
        ]);
        let actions = BTreeMap::from([
            (
                "channel.messaging".to_owned(),
                BTreeSet::from([ActionId::from("channel.messaging/send")]),
            ),
            (
                "companion".to_owned(),
                BTreeSet::from([ActionId::from("companion/evolve")]),
            ),
            (
                "customer.service".to_owned(),
                BTreeSet::from([ActionId::from("customer.service/handoff")]),
            ),
        ]);
        let derived = required_operations(&capabilities, &actions).unwrap();
        assert_eq!(
            derived["channel"],
            BTreeSet::from(["manage".into(), "receive".into(), "send".into()]),
        );
        assert_eq!(
            derived["companion"],
            BTreeSet::from(["read".into(), "write".into()]),
        );
        assert_eq!(
            derived["customer"],
            BTreeSet::from(["read".into(), "write".into()]),
        );
    }

    #[tokio::test]
    async fn duplicate_unused_and_missing_mandatory_selections_fail_closed() {
        let knowledge_registry = registry("knowledge_base", &["search"]);
        let capabilities = BTreeSet::from(["knowledge".to_owned()]);
        let actions = BTreeMap::from([(
            "knowledge".to_owned(),
            BTreeSet::from([ActionId::from("knowledge/search")]),
        )]);
        let duplicate = knowledge_registry
            .resolve_selected(
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
                &actions,
                &[],
            )
            .await
            .unwrap_err();
        assert_eq!(duplicate.code(), "RESOURCE_SELECTION_INVALID");

        let workspace_module = nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID.to_owned();
        let missing = registry("workspace", &["read"])
            .resolve_selected(
                "owner-1",
                &[],
                &BTreeSet::from([workspace_module.clone()]),
                &BTreeMap::from([(
                    workspace_module,
                    BTreeSet::from([ActionId::from("workspace.files/read")]),
                )]),
                &[],
            )
            .await
            .unwrap_err();
        assert_eq!(missing.code(), "RESOURCE_SELECTION_REQUIRED");

        let unused = knowledge_registry
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
    async fn knowledge_base_may_remain_unbound_until_the_session_mount_is_applied() {
        let bindings = registry("knowledge_base", &["search"])
            .resolve_selected(
                "owner-1",
                &[],
                &BTreeSet::from(["knowledge".to_owned()]),
                &BTreeMap::from([(
                    "knowledge".to_owned(),
                    BTreeSet::from([ActionId::from("knowledge/search")]),
                )]),
                &[],
            )
            .await
            .unwrap();

        assert!(bindings.is_empty());
    }

    #[tokio::test]
    async fn authority_cannot_grant_less_than_the_server_derived_requirement() {
        let error = registry("customer", &["read"])
            .resolve_selected(
                "owner-1",
                &[AgentResourceSelectionDto {
                    resource_kind: "customer".into(),
                    resource_id: "customer-1".into(),
                }],
                &BTreeSet::from(["customer.service".into()]),
                &BTreeMap::from([(
                    "customer.service".into(),
                    BTreeSet::from([ActionId::from("customer.service/notes.write")]),
                )]),
                &[],
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
            .resolve_selected(
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
                &BTreeSet::from(["companion".into(), "companion.memory".into()]),
                &BTreeMap::from([
                    (
                        "companion".into(),
                        BTreeSet::from([ActionId::from("companion/learn")]),
                    ),
                    (
                        "companion.memory".into(),
                        BTreeSet::from([ActionId::from("companion.memory/recall")]),
                    ),
                ]),
                &[],
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "RESOURCE_SELECTION_RELATIONSHIP_MISMATCH");
    }

    #[tokio::test]
    async fn mcp_bindings_derive_read_and_exact_per_tool_invoke_without_legacy_capabilities() {
        let server_id = nomifun_api_types::McpServerId::parse(MCP_SERVER_A).unwrap();
        let capability_id =
            nomifun_mcp::canonical_mcp_tool_capability_id(&server_id, "lookup").unwrap();
        let lock = ResolvedMcpToolLock {
            server_id: nomifun_agent_contracts::McpServerId::from(MCP_SERVER_A),
            canonical_tool_key: capability_id.clone().into(),
            capability_id: capability_id.clone().into(),
            schema_digest: "a".repeat(64).into(),
            materialization_revision: nomifun_mcp::MCP_TOOL_MATERIALIZATION_REVISION,
        };
        let resolver = registry("mcp_server", &["connect", "invoke", "read"]);
        let selections = [MCP_SERVER_A, MCP_SERVER_B].map(|resource_id| {
            AgentResourceSelectionDto {
                resource_kind: "mcp_server".into(),
                resource_id: resource_id.into(),
            }
        });
        let bindings = resolver
            .resolve_selected(
                "owner-1",
                &selections,
                &BTreeSet::from([capability_id]),
                &FrozenActionAllowlists::new(),
                &[lock],
            )
            .await
            .unwrap();
        let by_server = bindings
            .iter()
            .map(|binding| (binding.resource_id.as_ref(), &binding.operations))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            by_server[MCP_SERVER_A],
            &BTreeSet::from(["connect".into(), "invoke".into(), "read".into()])
        );
        assert_eq!(
            by_server[MCP_SERVER_B],
            &BTreeSet::from(["connect".into(), "read".into()])
        );

        for legacy in nomifun_mcp::RETIRED_MCP_AUTHORING_CAPABILITY_IDS {
            let error = resolver
                .resolve("owner-1", &[], &BTreeSet::from([legacy.to_owned()]))
                .await
                .unwrap_err();
            assert_eq!(error.code(), "MCP_LEGACY_CAPABILITY_RETIRED");
        }
    }

    #[tokio::test]
    async fn workspace_modules_derive_only_their_frozen_exact_action_operations() {
        for (module_id, action_id, resource_kind, operation) in [
            (
                nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID,
                "workspace.files/read",
                "workspace",
                "read",
            ),
            (
                nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID,
                "workspace.files/delete",
                "workspace",
                "write",
            ),
            (
                nomifun_agent_domain_wave2::WORKSPACE_VCS_MODULE_ID,
                "workspace.vcs/status",
                "workspace",
                "read",
            ),
            (
                nomifun_agent_domain_wave2::WORKSPACE_VCS_MODULE_ID,
                "workspace.vcs/push",
                "workspace",
                "write",
            ),
            (
                nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID,
                "workspace.process/poll",
                "process_session",
                "execute",
            ),
            (
                nomifun_agent_domain_wave2::WORKSPACE_ARTIFACTS_MODULE_ID,
                "workspace.artifacts/read",
                "workspace",
                "read",
            ),
            (
                nomifun_agent_domain_wave2::WORKSPACE_ARTIFACTS_MODULE_ID,
                "workspace.artifacts/publish",
                "workspace",
                "write",
            ),
        ] {
            let capability_ids = BTreeSet::from([module_id.to_owned()]);
            let action_allowlists = BTreeMap::from([(
                module_id.to_owned(),
                BTreeSet::from([ActionId::from(action_id)]),
            )]);
            let bindings = registry(resource_kind, &[operation])
                .resolve_selected(
                    "owner-1",
                    &[AgentResourceSelectionDto {
                        resource_kind: resource_kind.into(),
                        resource_id: "resource-1".into(),
                    }],
                    &capability_ids,
                    &action_allowlists,
                    &[],
                )
                .await
                .unwrap();
            assert_eq!(bindings.len(), 1);
            assert_eq!(bindings[0].operations, BTreeSet::from([operation.into()]));
        }

        let module_id = nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID;
        let capability_ids = BTreeSet::from([module_id.to_owned()]);
        let bindings = registry("workspace", &["read", "write"])
            .resolve_selected(
                "owner-1",
                &[AgentResourceSelectionDto {
                    resource_kind: "workspace".into(),
                    resource_id: "resource-1".into(),
                }],
                &capability_ids,
                &BTreeMap::from([(
                    module_id.to_owned(),
                    BTreeSet::from([
                        ActionId::from("workspace.files/read"),
                        ActionId::from("workspace.files/delete"),
                    ]),
                )]),
                &[],
            )
            .await
            .unwrap();
        assert_eq!(
            bindings[0].operations,
            BTreeSet::from(["read".into(), "write".into()])
        );
    }

    #[test]
    fn retired_workspace_capability_ids_never_derive_resource_authority() {
        let retired = BTreeSet::from(
            [
                "fs.read",
                "fs.search",
                "fs.watch",
                "fs.snapshot",
                "fs.write",
                "fs.patch",
                "fs.delete",
                "vcs.status",
                "vcs.diff",
                "vcs.stage",
                "vcs.commit",
                "vcs.push",
                "workspace.bind",
                "process.exec",
                "process.session",
                "terminal.pty",
            ]
            .map(str::to_owned),
        );
        assert!(
            required_operations(&retired, &FrozenActionAllowlists::new())
                .unwrap()
                .is_empty()
        );

        let error = required_operations(
            &BTreeSet::from([
                nomifun_agent_domain_wave2::WORKSPACE_ARTIFACTS_MODULE_ID.to_owned(),
            ]),
            &FrozenActionAllowlists::new(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "RESOURCE_REQUIREMENT_CONTRACT_MISMATCH");
    }

    #[test]
    fn all_published_resource_kinds_have_capability_operation_derivation() {
        let server_id = nomifun_api_types::McpServerId::parse(MCP_SERVER_A).unwrap();
        let mcp_tool =
            nomifun_mcp::canonical_mcp_tool_capability_id(&server_id, "lookup").unwrap();
        let capabilities = BTreeSet::from([
            nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID.into(),
            "knowledge".into(),
            "project.memory".into(),
            nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID.into(),
            nomifun_agent_domain_wave2::SSH_MODULE_ID.into(),
            nomifun_agent_domain_wave2::BROWSER_MODULE_ID.into(),
            nomifun_agent_domain_wave2::COMPUTER_MODULE_ID.into(),
            nomifun_agent_domain_wave5::AUTOMATION_SCHEDULE_MODULE_ID.into(),
            mcp_tool,
            "companion".into(),
            "companion.memory".into(),
            "channel.messaging".into(),
            nomifun_agent_domain_wave4::ROBOT_MODULE_ID.into(),
            "customer.service".into(),
            "creative.workshop".into(),
            "creation.media".into(),
            "plugin.development".into(),
        ]);
        let action_allowlists = BTreeMap::from([
            (
                nomifun_agent_domain_wave2::WORKSPACE_FILES_MODULE_ID.into(),
                BTreeSet::from([ActionId::from("workspace.files/read")]),
            ),
            (
                nomifun_agent_domain_wave2::WORKSPACE_PROCESS_MODULE_ID.into(),
                BTreeSet::from([ActionId::from("workspace.process/exec")]),
            ),
            (
                nomifun_agent_domain_wave2::SSH_MODULE_ID.into(),
                BTreeSet::from([ActionId::from("ssh/exec")]),
            ),
            (
                nomifun_agent_domain_wave2::BROWSER_MODULE_ID.into(),
                BTreeSet::from([ActionId::from("browser/observe")]),
            ),
            (
                nomifun_agent_domain_wave2::COMPUTER_MODULE_ID.into(),
                BTreeSet::from([ActionId::from("computer/observe")]),
            ),
            (
                nomifun_agent_domain_wave5::AUTOMATION_SCHEDULE_MODULE_ID.into(),
                BTreeSet::from([ActionId::from(
                    nomifun_agent_domain_wave5::SCHEDULE_LIST_ACTION_ID,
                )]),
            ),
            (
                "knowledge".into(),
                BTreeSet::from([ActionId::from("knowledge/search")]),
            ),
            (
                "project.memory".into(),
                BTreeSet::from([ActionId::from("project.memory/read")]),
            ),
            (
                "companion".into(),
                BTreeSet::from([ActionId::from("companion/learn")]),
            ),
            (
                "companion.memory".into(),
                BTreeSet::from([ActionId::from("companion.memory/recall")]),
            ),
            (
                "channel.messaging".into(),
                BTreeSet::from([ActionId::from("channel.messaging/reply")]),
            ),
            (
                nomifun_agent_domain_wave4::ROBOT_MODULE_ID.into(),
                BTreeSet::from([ActionId::from(
                    nomifun_agent_domain_wave4::ROBOT_VISION_ACTION_ID,
                )]),
            ),
            (
                "customer.service".into(),
                BTreeSet::from([ActionId::from("customer.service/notes.read")]),
            ),
            (
                "creative.workshop".into(),
                BTreeSet::from([
                    ActionId::from("creative.workshop/canvas.read"),
                    ActionId::from("creative.workshop/asset.read"),
                ]),
            ),
            (
                "creation.media".into(),
                BTreeSet::from([ActionId::from("creation.media/image")]),
            ),
            (
                "plugin.development".into(),
                BTreeSet::from([ActionId::from("plugin.development/read")]),
            ),
        ]);
        let derived = required_operations(&capabilities, &action_allowlists).unwrap();
        let expected = SUPPORTED_RESOURCE_KINDS
            .into_iter()
            .filter(|kind| *kind != "terminal")
            .collect::<BTreeSet<_>>();
        assert_eq!(
            derived.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            expected
        );
    }

    #[tokio::test]
    async fn browser_module_requires_one_server_resolved_resource_binding() {
        let capabilities = BTreeSet::from([
            nomifun_agent_domain_wave2::BROWSER_MODULE_ID.to_owned(),
        ]);
        let actions = BTreeMap::from([(
            nomifun_agent_domain_wave2::BROWSER_MODULE_ID.to_owned(),
            BTreeSet::from([
                ActionId::from("browser/observe"),
                ActionId::from("browser/navigate"),
            ]),
        )]);
        assert_eq!(
            nomifun_agent_domain_wave2::required_resource_kinds(
                nomifun_agent_domain_wave2::BROWSER_MODULE_ID,
            ),
            Some(BTreeSet::from([ResourceKind::from("browser")]))
        );
        let registry = registry("browser", &["observe", "navigate"]);
        let missing = registry
            .resolve_selected("owner-1", &[], &capabilities, &actions, &[])
            .await
            .unwrap_err();
        assert_eq!(missing.code(), "RESOURCE_SELECTION_REQUIRED");
        let bindings = registry
            .resolve_selected(
                "owner-1",
                &[AgentResourceSelectionDto {
                    resource_kind: "browser".into(),
                    resource_id: "managed-browser".into(),
                }],
                &capabilities,
                &actions,
                &[],
            )
            .await
            .unwrap();
        assert_eq!(bindings[0].resource_kind.as_ref(), "browser");
        assert_eq!(
            bindings[0].operations,
            BTreeSet::from(["navigate".into(), "observe".into()])
        );
    }
}
