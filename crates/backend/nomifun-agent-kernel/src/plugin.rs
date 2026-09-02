use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, AgentSessionId, CanonicalSchemaRef, CapabilityId, CorrelationId, DigestHex,
    ExecutionRoleId, HostPortId, IdempotencyKey, OperationId, PackageRef,
    PluginContextDescriptor, PluginIdentityDescriptor, PluginRegistrarDescriptor,
    PluginRegistrarOperation, PluginRegistrationMetadata, PrincipalRef,
    ResolvedRoleProviderLock, ResolvedSnapshotRef, ResourceBindingId, ResourceId, ResourceKind,
    ScopeKey, ServiceKeyId, ServiceKeyRef, SkillId, StrictJsonValue, TypedResourceBindings,
    ValidatedPluginConfig,
};

use crate::{DeclaredServiceView, KernelError, PluginStateHandle, ServiceExports};

#[derive(Clone, Debug, PartialEq)]
pub struct CapabilityInvocationRequest {
    pub principal: PrincipalRef,
    pub session_owner: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub active_set_generation: u64,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub resource_binding_ids: BTreeSet<ResourceBindingId>,
    pub state_scope_key: ScopeKey,
    pub input: StrictJsonValue,
}

#[derive(Clone)]
pub struct CapabilityInvocationContext {
    pub principal: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub idempotency_key: IdempotencyKey,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub resource_bindings: TypedResourceBindings,
    pub role_provider: Option<ResolvedRoleProviderLock>,
    pub state_scope_key: ScopeKey,
    pub state: PluginStateHandle,
    pub services: DeclaredServiceView,
}

/// Host-owned admission for a non-action role member.
///
/// Context assembly and resource acquisition have no model-selected action or
/// idempotency key. They still carry the same owner, frozen Snapshot,
/// activation generation, and exact resource-binding set used by regular
/// capability calls.
#[derive(Clone, Debug, PartialEq)]
pub enum RoleMemberAdmission {
    Agent {
        agent_session_id: AgentSessionId,
        resolved_snapshot_ref: ResolvedSnapshotRef,
        active_set_generation: u64,
    },
    Operation {
        provider_lock: ResolvedRoleProviderLock,
        registry_generation: u64,
        registry_digest: DigestHex,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoleMemberInvocationRequest {
    pub principal: PrincipalRef,
    pub session_owner: PrincipalRef,
    pub operation_id: OperationId,
    pub correlation_id: CorrelationId,
    pub capability_id: CapabilityId,
    pub resource_binding_ids: BTreeSet<ResourceBindingId>,
    pub state_scope_key: ScopeKey,
    pub admission: RoleMemberAdmission,
}

/// Exact Provider-Mount facts projected after the Registry resolves the frozen
/// Role lock.
#[derive(Clone)]
pub struct ProviderMountContext {
    pub identity: PluginIdentityDescriptor,
    pub config: ValidatedPluginConfig,
    pub state: PluginStateHandle,
    pub services: DeclaredServiceView,
}

/// Provider-Mount context projected by the Kernel after exact role dispatch.
#[derive(Clone)]
pub struct ResolvedRoleMemberContext {
    pub role_id: ExecutionRoleId,
    pub member_id: CapabilityId,
    pub provider_lock: ResolvedRoleProviderLock,
    pub principal: PrincipalRef,
    pub agent_session_id: Option<AgentSessionId>,
    pub operation_id: OperationId,
    pub correlation_id: CorrelationId,
    pub resolved_snapshot_ref: Option<ResolvedSnapshotRef>,
    pub registry_generation: u64,
    pub registry_digest: DigestHex,
    pub resource_bindings: TypedResourceBindings,
    pub state_scope_key: ScopeKey,
    pub mount: ProviderMountContext,
}

#[async_trait]
pub trait CapabilityHandler: Send + Sync {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError>;
}

/// Typed export for a `CapabilityKind::ContextContributor` role member.
#[derive(Clone)]
pub struct ContextContributionRequest {
    pub context: ResolvedRoleMemberContext,
    pub schema_ref: CanonicalSchemaRef,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContextContributionResult {
    pub value: Option<StrictJsonValue>,
}

#[async_trait]
pub trait ContextContributionFactory: Send + Sync {
    async fn contribute(
        &self,
        request: ContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError>;
}

/// Typed export for a `CapabilityKind::ResourceProvider` role member.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceHandleIdentity {
    pub binding_id: ResourceBindingId,
    pub resource_kind: ResourceKind,
    pub resource_id: ResourceId,
}

#[async_trait]
pub trait ResourceHandle: Send + Sync {
    fn identity(&self) -> &ResourceHandleIdentity;

    async fn release(&self) -> Result<(), KernelError>;
}

#[derive(Clone)]
pub struct ResourceProviderRequest {
    pub context: ResolvedRoleMemberContext,
}

#[derive(Clone)]
pub struct ResourceProviderResult {
    pub handle: Arc<dyn ResourceHandle>,
}

#[async_trait]
pub trait ResourceProviderFactory: Send + Sync {
    async fn acquire(
        &self,
        request: ResourceProviderRequest,
    ) -> Result<ResourceProviderResult, KernelError>;
}

#[derive(Clone)]
pub struct PluginRegistration {
    pub metadata: PluginRegistrationMetadata,
    handlers: BTreeMap<CapabilityId, Arc<dyn CapabilityHandler>>,
    role_action_handlers:
        BTreeMap<(ExecutionRoleId, CapabilityId), Arc<dyn CapabilityHandler>>,
    role_context_factories:
        BTreeMap<(ExecutionRoleId, CapabilityId), Arc<dyn ContextContributionFactory>>,
    role_resource_factories:
        BTreeMap<(ExecutionRoleId, CapabilityId), Arc<dyn ResourceProviderFactory>>,
    services: ServiceExports,
}

impl PluginRegistration {
    pub fn new(metadata: PluginRegistrationMetadata) -> Self {
        Self {
            metadata,
            handlers: BTreeMap::new(),
            role_action_handlers: BTreeMap::new(),
            role_context_factories: BTreeMap::new(),
            role_resource_factories: BTreeMap::new(),
            services: ServiceExports::new(),
        }
    }

    pub fn add_capability_handler(
        &mut self,
        capability_id: CapabilityId,
        handler: Arc<dyn CapabilityHandler>,
    ) -> Result<(), KernelError> {
        if self
            .handlers
            .insert(capability_id.clone(), handler)
            .is_some()
        {
            return Err(KernelError::DuplicateCapability { capability_id });
        }
        Ok(())
    }

    pub fn provide_service<T>(
        &mut self,
        key: &crate::ServiceKey<T>,
        service: Arc<T>,
    ) -> Result<(), KernelError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.services.provide(key, service)
    }

    pub fn add_role_action_handler(
        &mut self,
        role_id: ExecutionRoleId,
        capability_id: CapabilityId,
        handler: Arc<dyn CapabilityHandler>,
    ) -> Result<(), KernelError> {
        if self
            .role_action_handlers
            .insert((role_id.clone(), capability_id.clone()), handler)
            .is_some()
        {
            return Err(KernelError::DuplicateRoleProvider {
                role_id,
                mount_id: self.metadata.mount_id.clone(),
            });
        }
        Ok(())
    }

    pub fn add_role_context_factory(
        &mut self,
        role_id: ExecutionRoleId,
        capability_id: CapabilityId,
        factory: Arc<dyn ContextContributionFactory>,
    ) -> Result<(), KernelError> {
        if self
            .role_context_factories
            .insert((role_id.clone(), capability_id.clone()), factory)
            .is_some()
        {
            return Err(KernelError::DuplicateRoleProvider {
                role_id,
                mount_id: self.metadata.mount_id.clone(),
            });
        }
        Ok(())
    }

    pub fn add_role_resource_factory(
        &mut self,
        role_id: ExecutionRoleId,
        capability_id: CapabilityId,
        factory: Arc<dyn ResourceProviderFactory>,
    ) -> Result<(), KernelError> {
        if self
            .role_resource_factories
            .insert((role_id.clone(), capability_id.clone()), factory)
            .is_some()
        {
            return Err(KernelError::DuplicateRoleProvider {
                role_id,
                mount_id: self.metadata.mount_id.clone(),
            });
        }
        Ok(())
    }

    pub fn handler_ids(&self) -> BTreeSet<CapabilityId> {
        self.handlers.keys().cloned().collect()
    }

    pub fn service_refs(&self) -> BTreeSet<nomifun_agent_contracts::ServiceKeyRef> {
        self.services.provided_refs()
    }

    pub fn role_action_handler_ids(
        &self,
    ) -> BTreeSet<(ExecutionRoleId, CapabilityId)> {
        self.role_action_handlers.keys().cloned().collect()
    }

    pub fn role_context_factory_ids(
        &self,
    ) -> BTreeSet<(ExecutionRoleId, CapabilityId)> {
        self.role_context_factories.keys().cloned().collect()
    }

    pub fn role_resource_factory_ids(
        &self,
    ) -> BTreeSet<(ExecutionRoleId, CapabilityId)> {
        self.role_resource_factories.keys().cloned().collect()
    }

    pub(crate) fn handlers(
        &self,
    ) -> impl Iterator<Item = (&CapabilityId, &Arc<dyn CapabilityHandler>)> {
        self.handlers.iter()
    }

    pub(crate) fn services(&self) -> &ServiceExports {
        &self.services
    }

    pub(crate) fn role_action_handlers(
        &self,
    ) -> impl Iterator<
        Item = (
            &(ExecutionRoleId, CapabilityId),
            &Arc<dyn CapabilityHandler>,
        ),
    > {
        self.role_action_handlers.iter()
    }

    pub(crate) fn role_context_factories(
        &self,
    ) -> impl Iterator<
        Item = (
            &(ExecutionRoleId, CapabilityId),
            &Arc<dyn ContextContributionFactory>,
        ),
    > {
        self.role_context_factories.iter()
    }

    pub(crate) fn role_resource_factories(
        &self,
    ) -> impl Iterator<
        Item = (
            &(ExecutionRoleId, CapabilityId),
            &Arc<dyn ResourceProviderFactory>,
        ),
    > {
        self.role_resource_factories.iter()
    }

    /// Rebuild registration metadata from the manifest and actual runtime
    /// exports before a generation is published.
    pub fn canonicalized(&self) -> Result<Self, KernelError> {
        let mut registration = self.clone();
        registration.metadata = self.canonical_metadata()?;
        Ok(registration)
    }

    /// Return the registration metadata that the kernel will publish.
    ///
    /// The public metadata field is kept as an input compatibility surface for
    /// current package builders. Duplicated registrar/context fields are
    /// canonicalized here from the manifest, actual exports, and context port
    /// descriptors before materialization; callers cannot publish a stale
    /// hand-written declaration.
    pub(crate) fn canonical_metadata(&self) -> Result<PluginRegistrationMetadata, KernelError> {
        let mut metadata = self.metadata.clone();
        let manifest = &metadata.manifest.payload;
        let package = PackageRef {
            id: manifest.package_id.clone(),
            version: manifest.package_version.clone(),
        };
        let role_backed_capabilities = manifest
            .contributions
            .role_contracts
            .iter()
            .flat_map(|contract| {
                contract
                    .members
                    .iter()
                    .map(|member| member.capability.id.clone())
            })
            .collect::<BTreeSet<_>>();
        let expected_handlers = manifest
            .contributions
            .capabilities
            .iter()
            .filter(|capability| {
                !capability.contributions.actions.is_empty()
                    && !role_backed_capabilities.contains(&capability.id)
            })
            .map(|capability| capability.id.clone())
            .collect::<BTreeSet<_>>();
        let actual_handlers = self.handler_ids();
        if let Some(capability_id) = expected_handlers.difference(&actual_handlers).next() {
            return Err(KernelError::MissingCapabilityHandler {
                mount_id: metadata.mount_id.clone(),
                capability_id: capability_id.clone(),
            });
        }
        if let Some(capability_id) = actual_handlers.difference(&expected_handlers).next() {
            return Err(KernelError::UndeclaredCapabilityHandler {
                mount_id: metadata.mount_id.clone(),
                capability_id: capability_id.clone(),
            });
        }
        let role_members = manifest
            .contributions
            .role_providers
            .iter()
            .flat_map(|provider| {
                provider
                    .members
                    .keys()
                    .cloned()
                    .map(|capability_id| {
                        (provider.role.key.role_id.clone(), capability_id)
                    })
            })
            .collect::<BTreeSet<_>>();
        let action_capabilities = manifest
            .contributions
            .capabilities
            .iter()
            .filter(|capability| {
                capability.kind == nomifun_agent_contracts::CapabilityKind::Tool
            })
            .map(|capability| capability.id.clone())
            .collect::<BTreeSet<_>>();
        let expected_role_action_handlers = role_members
            .iter()
            .filter(|(_, capability_id)| action_capabilities.contains(capability_id))
            .cloned()
            .collect::<BTreeSet<_>>();
        let actual_role_action_handlers = self.role_action_handler_ids();
        if let Some((role_id, capability_id)) = expected_role_action_handlers
            .difference(&actual_role_action_handlers)
            .next()
        {
            return Err(KernelError::InvalidRoleProvider {
                role_id: role_id.clone(),
                mount_id: metadata.mount_id.clone(),
                reason: format!(
                    "role provider did not export action handler {}",
                    capability_id.as_ref()
                ),
            });
        }
        if let Some((role_id, capability_id)) = self
            .role_action_handler_ids()
            .difference(&role_members)
            .next()
        {
            return Err(KernelError::InvalidRoleProvider {
                role_id: role_id.clone(),
                mount_id: metadata.mount_id.clone(),
                reason: format!(
                    "role action handler {} is not declared by the provider contribution",
                    capability_id.as_ref()
                ),
            });
        }
        let actual_role_context_factories = self.role_context_factory_ids();
        if let Some((role_id, capability_id)) = actual_role_context_factories
            .difference(&role_members)
            .next()
        {
            return Err(KernelError::InvalidRoleProvider {
                role_id: role_id.clone(),
                mount_id: metadata.mount_id.clone(),
                reason: format!(
                    "role context factory {} is not declared by the provider contribution",
                    capability_id.as_ref()
                ),
            });
        }
        let actual_role_resource_factories = self.role_resource_factory_ids();
        if let Some((role_id, capability_id)) = actual_role_resource_factories
            .difference(&role_members)
            .next()
        {
            return Err(KernelError::InvalidRoleProvider {
                role_id: role_id.clone(),
                mount_id: metadata.mount_id.clone(),
                reason: format!(
                    "role resource factory {} is not declared by the provider contribution",
                    capability_id.as_ref()
                ),
            });
        }
        let expected_service_refs = manifest
            .provides_services
            .iter()
            .map(|provision| provision.service.clone())
            .collect::<BTreeSet<_>>();
        let actual_service_refs = self.service_refs();
        if let Some(service) = expected_service_refs.difference(&actual_service_refs).next() {
            return Err(KernelError::MissingRuntimeServiceExport {
                mount_id: metadata.mount_id.clone(),
                service_id: service.id.clone(),
            });
        }
        if let Some(service) = actual_service_refs.difference(&expected_service_refs).next() {
            return Err(KernelError::UndeclaredRuntimeServiceExport {
                mount_id: metadata.mount_id.clone(),
                service_id: service.id.clone(),
            });
        }

        let host_ports = declared_host_ports(&metadata.context, &metadata.mount_id)?;
        let capability_ids = unique_capability_ids(manifest, &metadata.mount_id)?;
        let skill_ids = unique_skill_ids(manifest, &metadata.mount_id)?;
        let mcp_tool_keys = unique_mcp_tool_keys(manifest, &metadata.mount_id)?;
        let role_ids = unique_role_ids(manifest, &metadata.mount_id)?;
        let declared_service_ids =
            unique_service_ids(&manifest.provides_services, &metadata.mount_id)?;
        let service_ids = unique_service_ids_from_refs(
            &actual_service_refs,
            &metadata.mount_id,
        )?;
        if service_ids != declared_service_ids {
            return Err(invalid_registration(
                &metadata.mount_id,
                "runtime service exports do not match the manifest declarations",
            ));
        }
        let identity = nomifun_agent_contracts::PluginIdentityDescriptor {
            package,
            mount_id: metadata.mount_id.clone(),
        };
        metadata.registrar = PluginRegistrarDescriptor {
            identity: identity.clone(),
            allowed_operations: required_registrar_operations(
                !capability_ids.is_empty(),
                !skill_ids.is_empty(),
                !mcp_tool_keys.is_empty(),
                !role_ids.is_empty(),
                !service_ids.is_empty(),
                !host_ports.is_empty(),
            ),
            declared_capability_ids: capability_ids,
            declared_skill_ids: skill_ids,
            declared_mcp_tool_keys: mcp_tool_keys,
            declared_role_ids: role_ids,
            declared_service_keys: service_ids,
            declared_host_ports: host_ports,
        };
        metadata.context.identity = identity;
        metadata.context.source = metadata.source.clone();
        metadata.context.state.package_id = manifest.package_id.clone();
        metadata.context.state.mount_id = metadata.mount_id.clone();
        metadata.context.state.methods =
            nomifun_agent_contracts::PluginStateMethod::REQUIRED
                .into_iter()
                .collect();
        metadata.context.declared_services.provided_services =
            actual_service_refs.into_iter().collect();
        Ok(metadata)
    }
}

fn unique_capability_ids(
    manifest: &nomifun_agent_contracts::PackageManifest,
    mount_id: &nomifun_agent_contracts::PluginMountId,
) -> Result<BTreeSet<CapabilityId>, KernelError> {
    let mut ids = BTreeSet::new();
    for capability in &manifest.contributions.capabilities {
        if !ids.insert(capability.id.clone()) {
            return Err(invalid_registration(
                mount_id,
                format!("duplicate capability declaration {}", capability.id.as_ref()),
            ));
        }
    }
    Ok(ids)
}

fn unique_skill_ids(
    manifest: &nomifun_agent_contracts::PackageManifest,
    mount_id: &nomifun_agent_contracts::PluginMountId,
) -> Result<BTreeSet<SkillId>, KernelError> {
    let mut ids = BTreeSet::new();
    for skill in &manifest.contributions.skills {
        if !ids.insert(skill.id.clone()) {
            return Err(invalid_registration(
                mount_id,
                format!("duplicate skill declaration {}", skill.id.as_ref()),
            ));
        }
    }
    Ok(ids)
}

fn unique_mcp_tool_keys(
    manifest: &nomifun_agent_contracts::PackageManifest,
    mount_id: &nomifun_agent_contracts::PluginMountId,
) -> Result<BTreeSet<nomifun_agent_contracts::McpToolKey>, KernelError> {
    let mut keys = BTreeSet::new();
    for mapping in &manifest.contributions.mcp_tools {
        let key = (
            mapping.server_id.clone(),
            mapping.canonical_tool_key.clone(),
        );
        if !keys.insert(key.clone()) {
            return Err(invalid_registration(
                mount_id,
                format!(
                    "duplicate MCP tool declaration {}/{}",
                    key.0.as_ref(),
                    key.1.as_ref()
                ),
            ));
        }
    }
    Ok(keys
        .into_iter()
        .map(|(_, tool_key)| tool_key)
        .collect())
}

fn unique_role_ids(
    manifest: &nomifun_agent_contracts::PackageManifest,
    mount_id: &nomifun_agent_contracts::PluginMountId,
) -> Result<BTreeSet<ExecutionRoleId>, KernelError> {
    let mut ids = BTreeSet::new();
    for contribution in &manifest.contributions.role_providers {
        if !ids.insert(contribution.role.key.role_id.clone()) {
            return Err(invalid_registration(
                mount_id,
                format!(
                    "duplicate role provider declaration {}",
                    contribution.role.key.role_id.as_ref()
                ),
            ));
        }
    }
    Ok(ids)
}

fn unique_service_ids(
    provisions: &[nomifun_agent_contracts::ServiceProvision],
    mount_id: &nomifun_agent_contracts::PluginMountId,
) -> Result<BTreeSet<ServiceKeyId>, KernelError> {
    let mut ids = BTreeSet::new();
    for provision in provisions {
        if !ids.insert(provision.service.id.clone()) {
            return Err(invalid_registration(
                mount_id,
                format!(
                    "duplicate provided service declaration {}",
                    provision.service.id.as_ref()
                ),
            ));
        }
    }
    Ok(ids)
}

fn unique_service_ids_from_refs(
    service_refs: &BTreeSet<ServiceKeyRef>,
    mount_id: &nomifun_agent_contracts::PluginMountId,
) -> Result<BTreeSet<ServiceKeyId>, KernelError> {
    let mut ids = BTreeSet::new();
    for service in service_refs {
        if !ids.insert(service.id.clone()) {
            return Err(invalid_registration(
                mount_id,
                format!(
                    "duplicate exported service identity {}",
                    service.id.as_ref()
                ),
            ));
        }
    }
    Ok(ids)
}

fn declared_host_ports(
    context: &PluginContextDescriptor,
    mount_id: &nomifun_agent_contracts::PluginMountId,
) -> Result<BTreeSet<HostPortId>, KernelError> {
    let mut ports = BTreeSet::new();
    for port_id in context
        .host_ports
        .iter()
        .map(|binding| binding.port.id.clone())
        .chain(
            context
                .typed_command_ports
                .iter()
                .map(|binding| binding.port.id.clone()),
        )
        .chain(
            context
                .domain_outbox_ports
                .iter()
                .map(|binding| binding.port.id.clone()),
        )
        .chain([context.cancellation.cancellation_port.id.clone()])
        .chain([context.managed_task_registration.registrar_port.id.clone()])
    {
        if !ports.insert(port_id.clone()) {
            return Err(invalid_registration(
                mount_id,
                format!("duplicate host port declaration {}", port_id.as_ref()),
            ));
        }
    }
    Ok(ports)
}

fn required_registrar_operations(
    capabilities: bool,
    skills: bool,
    mcp: bool,
    role_providers: bool,
    services: bool,
    host_ports: bool,
) -> BTreeSet<PluginRegistrarOperation> {
    let mut operations = BTreeSet::new();
    if capabilities {
        operations.insert(PluginRegistrarOperation::ContributeCapability);
    }
    if skills {
        operations.insert(PluginRegistrarOperation::ContributeSkill);
    }
    if mcp {
        operations.insert(PluginRegistrarOperation::ContributeMcpToolMapping);
    }
    if role_providers {
        operations.insert(PluginRegistrarOperation::ContributeRoleProvider);
    }
    if services {
        operations.insert(PluginRegistrarOperation::ProvideService);
    }
    if host_ports {
        operations.insert(PluginRegistrarOperation::BindHostPort);
    }
    operations
}

fn invalid_registration(
    mount_id: &nomifun_agent_contracts::PluginMountId,
    reason: impl Into<String>,
) -> KernelError {
    KernelError::InvalidRegistration {
        mount_id: mount_id.clone(),
        reason: reason.into(),
    }
}
