use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use nomifun_agent_contracts::{
    CanonicalSchemaRef, CapabilityId, CapabilityKind, DigestHex, ExecutionRoleId, PackageRef,
    PluginMountId, ResolvedRoleProviderLock, ResourceBindingId, ScopeKey,
    TypedResourceBinding,
};

use crate::service::build_service_bindings;
use crate::{
    ActiveCapabilitySetSnapshot, CapabilityHandler, CapabilityInvocationContext,
    CapabilityInvocationRequest, CompiledSnapshot, ContextContributionFactory,
    ContextContributionRequest, ContextContributionResult, DeclaredServiceView, KernelError,
    MaterializationPolicy, MaterializedRegistry, Materializer, PluginRegistration,
    PluginStateError, PluginStateHandle, PluginStatePersistence, PluginStateStore,
    ProviderMountContext, ResolvedRoleMemberContext, ResourceHandle,
    ResourceProviderFactory, ResourceProviderRequest, ResourceProviderResult,
    RoleMemberAdmission, RoleMemberInvocationRequest, ThinAuthority,
};

#[derive(Clone)]
struct HandlerBinding {
    mount_id: PluginMountId,
    handler: Arc<dyn CapabilityHandler>,
}

#[derive(Clone)]
struct PublishedRegistry {
    materialized: Arc<MaterializedRegistry>,
    handlers: BTreeMap<CapabilityId, HandlerBinding>,
    role_handlers:
        BTreeMap<(ExecutionRoleId, PluginMountId, CapabilityId), HandlerBinding>,
    role_context_factories:
        BTreeMap<(ExecutionRoleId, PluginMountId, CapabilityId), Arc<dyn ContextContributionFactory>>,
    role_resource_factories:
        BTreeMap<(ExecutionRoleId, PluginMountId, CapabilityId), Arc<dyn ResourceProviderFactory>>,
    service_views: BTreeMap<PluginMountId, DeclaredServiceView>,
    state_handles: BTreeMap<PluginMountId, PluginStateHandle>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ResourceHandleKey {
    scope_key: ScopeKey,
    role_id: ExecutionRoleId,
    mount_id: PluginMountId,
    contribution_digest: DigestHex,
    binding_id: ResourceBindingId,
}

impl PublishedRegistry {
    fn empty() -> Self {
        Self {
            materialized: Arc::new(MaterializedRegistry::empty()),
            handlers: BTreeMap::new(),
            role_handlers: BTreeMap::new(),
            role_context_factories: BTreeMap::new(),
            role_resource_factories: BTreeMap::new(),
            service_views: BTreeMap::new(),
            state_handles: BTreeMap::new(),
        }
    }
}

pub struct KernelRegistry {
    policy: MaterializationPolicy,
    state_store: Arc<PluginStateStore>,
    published: RwLock<Arc<PublishedRegistry>>,
    resource_handles: tokio::sync::Mutex<
        BTreeMap<ResourceHandleKey, Arc<dyn ResourceHandle>>,
    >,
}

impl KernelRegistry {
    pub fn new(
        policy: MaterializationPolicy,
        persistence: Arc<dyn PluginStatePersistence>,
    ) -> Result<Self, PluginStateError> {
        Ok(Self::from_state_store(
            policy,
            PluginStateStore::new(persistence)?,
        ))
    }

    pub(crate) fn from_state_store(
        policy: MaterializationPolicy,
        state_store: Arc<PluginStateStore>,
    ) -> Self {
        Self {
            policy,
            state_store,
            published: RwLock::new(Arc::new(PublishedRegistry::empty())),
            resource_handles: tokio::sync::Mutex::new(BTreeMap::new()),
        }
    }

    pub fn snapshot(&self) -> Result<Arc<MaterializedRegistry>, KernelError> {
        self.published
            .read()
            .map(|published| Arc::clone(&published.materialized))
            .map_err(|_| KernelError::RegistryPoisoned)
    }

    pub fn declared_service_view(
        &self,
        mount_id: &PluginMountId,
    ) -> Result<Option<DeclaredServiceView>, KernelError> {
        self.published
            .read()
            .map(|published| published.service_views.get(mount_id).cloned())
            .map_err(|_| KernelError::RegistryPoisoned)
    }

    /// Validate and materialize an entire host generation, then publish it with
    /// one lock swap. Any error leaves the previous generation untouched.
    pub fn replace_all(
        &self,
        registrations: Vec<PluginRegistration>,
    ) -> Result<Arc<MaterializedRegistry>, KernelError> {
        let registrations = registrations
            .iter()
            .map(PluginRegistration::canonicalized)
            .collect::<Result<Vec<_>, _>>()?;
        let mut guard = self
            .published
            .write()
            .map_err(|_| KernelError::RegistryPoisoned)?;
        let generation = guard
            .materialized
            .generation
            .checked_add(1)
            .ok_or(KernelError::ActivationGenerationExhausted)?;
        let materialized =
            Arc::new(Materializer::materialize_canonical(
                &self.policy,
                &registrations,
                generation,
            )?);

        let services = build_service_bindings(
            registrations.iter().map(|registration| {
                let manifest = &registration.metadata.manifest.payload;
                (
                    PackageRef {
                        id: manifest.package_id.clone(),
                        version: manifest.package_version.clone(),
                    },
                    registration.metadata.mount_id.clone(),
                    registration.services().clone(),
                )
            }),
        )?;

        let mut handlers = BTreeMap::new();
        let mut role_handlers = BTreeMap::new();
        let mut role_context_factories = BTreeMap::new();
        let mut role_resource_factories = BTreeMap::new();
        let mut state_handles = BTreeMap::new();
        let mut service_views = BTreeMap::new();
        for registration in &registrations {
            let manifest = &registration.metadata.manifest.payload;
            for (capability_id, handler) in registration.handlers() {
                if handlers
                    .insert(
                        capability_id.clone(),
                        HandlerBinding {
                            mount_id: registration.metadata.mount_id.clone(),
                            handler: Arc::clone(handler),
                        },
                    )
                    .is_some()
                {
                    return Err(KernelError::DuplicateCapability {
                        capability_id: capability_id.clone(),
                    });
                }
            }
            for ((role_id, capability_id), handler) in registration.role_action_handlers() {
                let key = (
                    role_id.clone(),
                    registration.metadata.mount_id.clone(),
                    capability_id.clone(),
                );
                if role_handlers
                    .insert(
                        key,
                        HandlerBinding {
                            mount_id: registration.metadata.mount_id.clone(),
                            handler: Arc::clone(handler),
                        },
                    )
                    .is_some()
                {
                    return Err(KernelError::DuplicateRoleProvider {
                        role_id: role_id.clone(),
                        mount_id: registration.metadata.mount_id.clone(),
                    });
                }
            }
            for ((role_id, capability_id), factory) in registration.role_context_factories() {
                let key = (
                    role_id.clone(),
                    registration.metadata.mount_id.clone(),
                    capability_id.clone(),
                );
                if role_context_factories
                    .insert(key, Arc::clone(factory))
                    .is_some()
                {
                    return Err(KernelError::DuplicateRoleProvider {
                        role_id: role_id.clone(),
                        mount_id: registration.metadata.mount_id.clone(),
                    });
                }
            }
            for ((role_id, capability_id), factory) in registration.role_resource_factories() {
                let key = (
                    role_id.clone(),
                    registration.metadata.mount_id.clone(),
                    capability_id.clone(),
                );
                if role_resource_factories
                    .insert(key, Arc::clone(factory))
                    .is_some()
                {
                    return Err(KernelError::DuplicateRoleProvider {
                        role_id: role_id.clone(),
                        mount_id: registration.metadata.mount_id.clone(),
                    });
                }
            }
            state_handles.insert(
                registration.metadata.mount_id.clone(),
                self.state_store.handle(
                    manifest.package_id.clone(),
                    registration.metadata.mount_id.clone(),
                    manifest.package_version.clone(),
                ),
            );
            service_views.insert(
                registration.metadata.mount_id.clone(),
                DeclaredServiceView::from_bindings(
                    &registration
                        .metadata
                        .context
                        .declared_services
                        .required_service_handles,
                    &services,
                )?,
            );
        }
        for ((role_id, mount_id), provider) in &materialized.role_providers {
            for capability_id in provider.contribution.members.keys() {
                let Some(capability) = materialized.capability(capability_id) else {
                    return Err(KernelError::RoleProviderMemberUnavailable {
                        role_id: role_id.clone(),
                        capability_id: capability_id.clone(),
                    });
                };
                if !capability.manifest.contributions.actions.is_empty()
                    && !role_handlers.contains_key(&(
                        role_id.clone(),
                        mount_id.clone(),
                        capability_id.clone(),
                    ))
                {
                    return Err(KernelError::RoleProviderMemberUnavailable {
                        role_id: role_id.clone(),
                        capability_id: capability_id.clone(),
                    });
                }
            }
        }
        validate_role_exports(
            &materialized,
            &handlers,
            &role_handlers,
            &role_context_factories,
            &role_resource_factories,
        )?;

        let next = Arc::new(PublishedRegistry {
            materialized: Arc::clone(&materialized),
            handlers,
            role_handlers,
            role_context_factories,
            role_resource_factories,
            service_views,
            state_handles,
        });
        *guard = next;
        Ok(materialized)
    }

    pub async fn invoke(
        &self,
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        request: CapabilityInvocationRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, KernelError> {
        ThinAuthority::enforce(snapshot, active, &request)?;
        let (handler, context) = {
            let published = self
                .published
                .read()
                .map_err(|_| KernelError::RegistryPoisoned)?;
            if published
                .materialized
                .role_for_capability(&request.capability_id)
                .is_some()
            {
                let resolved = resolve_role_member(
                    &published,
                    snapshot,
                    active,
                    &RoleMemberInvocationRequest {
                        principal: request.principal.clone(),
                        session_owner: request.session_owner.clone(),
                        operation_id: request.operation_id.clone(),
                        correlation_id: request.correlation_id.clone(),
                        capability_id: request.capability_id.clone(),
                        resource_binding_ids: request.resource_binding_ids.clone(),
                        state_scope_key: request.state_scope_key.clone(),
                        admission: RoleMemberAdmission::Agent {
                            agent_session_id: request.agent_session_id.clone(),
                            resolved_snapshot_ref: request.resolved_snapshot_ref.clone(),
                            active_set_generation: request.active_set_generation,
                        },
                    },
                    CapabilityKind::Tool,
                )?;
                let binding = published
                    .role_handlers
                    .get(&(
                        resolved.role_id.clone(),
                        resolved.provider_lock.provider.mount_id.clone(),
                        request.capability_id.clone(),
                    ))
                    .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                        role_id: resolved.role_id,
                        capability_id: request.capability_id.clone(),
                    })?;
                let member = resolved.context;
                (
                    Arc::clone(&binding.handler),
                    CapabilityInvocationContext {
                        principal: member.principal,
                        agent_session_id: member
                            .agent_session_id
                            .ok_or(KernelError::RegistryPoisoned)?,
                        operation_id: member.operation_id,
                        idempotency_key: request.idempotency_key.clone(),
                        correlation_id: member.correlation_id,
                        resolved_snapshot_ref: member
                            .resolved_snapshot_ref
                            .ok_or(KernelError::RegistryPoisoned)?,
                        registry_generation: member.registry_generation,
                        capability_id: member.member_id,
                        action_id: request.action_id.clone(),
                        resource_bindings: member.resource_bindings,
                        role_provider: Some(member.provider_lock),
                        state_scope_key: member.state_scope_key,
                        state: member.mount.state,
                        services: member.mount.services,
                    },
                )
            } else {
                if snapshot.registry_generation != published.materialized.generation
                    || snapshot.registry_digest != published.materialized.registry_digest
                {
                    return Err(KernelError::RegistryGenerationMismatch {
                        expected_generation: snapshot.registry_generation,
                        expected_digest: snapshot.registry_digest.clone(),
                        actual_generation: published.materialized.generation,
                        actual_digest: published.materialized.registry_digest.clone(),
                    });
                }
                let binding = published.handlers.get(&request.capability_id).ok_or_else(|| {
                    KernelError::MissingCapabilityHandler {
                        mount_id: published.materialized.capabilities[&request.capability_id]
                            .mount_id
                            .clone(),
                        capability_id: request.capability_id.clone(),
                    }
                })?;
                let state = published
                    .state_handles
                    .get(&binding.mount_id)
                    .cloned()
                    .ok_or(KernelError::RegistryPoisoned)?;
                let services = published
                    .service_views
                    .get(&binding.mount_id)
                    .cloned()
                    .unwrap_or_default();
                let mut resource_bindings = request
                    .resource_binding_ids
                    .iter()
                    .filter_map(|binding_id| snapshot.binding(binding_id).cloned())
                    .collect::<Vec<TypedResourceBinding>>();
                resource_bindings.sort_by(|left, right| {
                    left.binding_id.cmp(&right.binding_id)
                });
                (
                    Arc::clone(&binding.handler),
                    CapabilityInvocationContext {
                        principal: request.principal.clone(),
                        agent_session_id: request.agent_session_id.clone(),
                        operation_id: request.operation_id.clone(),
                        idempotency_key: request.idempotency_key.clone(),
                        correlation_id: request.correlation_id.clone(),
                        resolved_snapshot_ref: request.resolved_snapshot_ref.clone(),
                        registry_generation: published.materialized.generation,
                        capability_id: request.capability_id.clone(),
                        action_id: request.action_id.clone(),
                        resource_bindings,
                        role_provider: None,
                        state_scope_key: request.state_scope_key.clone(),
                        state,
                        services,
                    },
                )
            }
        };
        handler.invoke(context, request.input).await
    }

    /// Assemble one ContextContributor member through the exact frozen Role
    /// Provider. This is intentionally separate from action invocation: a
    /// context factory cannot be reached through the action handler map.
    pub async fn contribute_role_context(
        &self,
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        request: RoleMemberInvocationRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        let (factory, context, schema_ref) = {
            let published = self
                .published
                .read()
                .map_err(|_| KernelError::RegistryPoisoned)?;
            let resolved = resolve_role_member(
                &published,
                snapshot,
                active,
                &request,
                CapabilityKind::ContextContributor,
            )?;
            let factory = published
                .role_context_factories
                .get(&(
                    resolved.role_id.clone(),
                    resolved.provider_lock.provider.mount_id.clone(),
                    request.capability_id.clone(),
                ))
                .cloned()
                .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                    role_id: resolved.role_id.clone(),
                    capability_id: request.capability_id.clone(),
                })?;
            let schema_ref = resolved.context_schema_ref.ok_or_else(|| {
                KernelError::InvalidRoleProvider {
                    role_id: resolved.role_id.clone(),
                    mount_id: resolved.provider_lock.provider.mount_id.clone(),
                    reason: format!(
                        "context member {} does not declare exactly one context schema",
                        request.capability_id.as_ref()
                    ),
                }
            })?;
            (factory, resolved.context, schema_ref)
        };
        factory
            .contribute(ContextContributionRequest {
                context,
                schema_ref,
            })
            .await
    }

    /// Acquire one ResourceProvider member through the exact frozen Role
    /// Provider. The returned descriptor is provider-owned; lifecycle-specific
    /// handles remain inside the provider implementation and are not exposed
    /// as a second generic action route.
    pub async fn acquire_role_resource(
        &self,
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        request: RoleMemberInvocationRequest,
    ) -> Result<ResourceProviderResult, KernelError> {
        let (factory, context) = {
            let published = self
                .published
                .read()
                .map_err(|_| KernelError::RegistryPoisoned)?;
            let resolved = resolve_role_member(
                &published,
                snapshot,
                active,
                &request,
                CapabilityKind::ResourceProvider,
            )?;
            let factory = published
                .role_resource_factories
                .get(&(
                    resolved.role_id.clone(),
                    resolved.provider_lock.provider.mount_id.clone(),
                    request.capability_id.clone(),
                ))
                .cloned()
                .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                    role_id: resolved.role_id,
                    capability_id: request.capability_id.clone(),
                })?;
            (factory, resolved.context)
        };
        let result = factory
            .acquire(ResourceProviderRequest {
                context: context.clone(),
            })
            .await?;
        let identity = result.handle.identity();
        let Some(binding) = context
            .resource_bindings
            .iter()
            .find(|binding| binding.binding_id == identity.binding_id)
        else {
            return Err(KernelError::CapabilityExecution {
                reason: "resource provider returned a handle for an unbound resource"
                    .to_owned(),
            });
        };
        if binding.resource_kind != identity.resource_kind
            || binding.resource_id != identity.resource_id
        {
            return Err(KernelError::CapabilityExecution {
                reason: "resource provider returned a handle with mismatched resource identity"
                    .to_owned(),
            });
        }
        let key = ResourceHandleKey {
            scope_key: context.state_scope_key.clone(),
            role_id: context.role_id.clone(),
            mount_id: context.provider_lock.provider.mount_id.clone(),
            contribution_digest: context.provider_lock.provider.contribution_digest.clone(),
            binding_id: identity.binding_id.clone(),
        };
        let mut handles = self.resource_handles.lock().await;
        if let Some(existing) = handles.get(&key).cloned() {
            drop(handles);
            result.handle.release().await?;
            return Ok(ResourceProviderResult { handle: existing });
        }
        handles.insert(key, Arc::clone(&result.handle));
        Ok(result)
    }

    /// Release all lazily acquired handles for one session scope. Providers
    /// remain responsible for their concrete cleanup; the Kernel only owns
    /// exact-handle identity and de-duplication.
    pub async fn release_role_resources(
        &self,
        scope_key: &ScopeKey,
    ) -> Result<(), KernelError> {
        let handles = {
            let mut guard = self.resource_handles.lock().await;
            let keys = guard
                .keys()
                .filter(|key| &key.scope_key == scope_key)
                .cloned()
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| guard.remove(&key))
                .collect::<Vec<_>>()
        };
        for handle in handles {
            handle.release().await?;
        }
        Ok(())
    }

    pub async fn release_all_role_resources(&self) -> Result<(), KernelError> {
        let handles = {
            let mut guard = self.resource_handles.lock().await;
            std::mem::take(&mut *guard)
                .into_values()
                .collect::<Vec<_>>()
        };
        for handle in handles {
            handle.release().await?;
        }
        Ok(())
    }
}

fn validate_role_exports(
    materialized: &MaterializedRegistry,
    generic_handlers: &BTreeMap<CapabilityId, HandlerBinding>,
    action_handlers: &BTreeMap<
        (ExecutionRoleId, PluginMountId, CapabilityId),
        HandlerBinding,
    >,
    context_factories: &BTreeMap<
        (ExecutionRoleId, PluginMountId, CapabilityId),
        Arc<dyn ContextContributionFactory>,
    >,
    resource_factories: &BTreeMap<
        (ExecutionRoleId, PluginMountId, CapabilityId),
        Arc<dyn ResourceProviderFactory>,
    >,
) -> Result<(), KernelError> {
    for ((role_id, mount_id), provider) in &materialized.role_providers {
        for capability_id in provider.contribution.members.keys() {
            let capability = materialized.capability(capability_id).ok_or_else(|| {
                KernelError::RoleProviderMemberUnavailable {
                    role_id: role_id.clone(),
                    capability_id: capability_id.clone(),
                }
            })?;
            let key = (role_id.clone(), mount_id.clone(), capability_id.clone());
            let has_action = action_handlers.contains_key(&key);
            let has_context = context_factories.contains_key(&key);
            let has_resource = resource_factories.contains_key(&key);
            let expected = match capability.manifest.kind {
                CapabilityKind::Tool => {
                    if capability.manifest.contributions.actions.is_empty()
                        || !capability.manifest.contributions.context_schema_refs.is_empty()
                    {
                        return Err(KernelError::InvalidRoleProvider {
                            role_id: role_id.clone(),
                            mount_id: mount_id.clone(),
                            reason: format!(
                                "tool member {} has malformed action/context declarations",
                                capability_id.as_ref()
                            ),
                        });
                    }
                    (true, false, false)
                }
                CapabilityKind::ContextContributor => {
                    if capability.manifest.contributions.actions.len() != 0
                        || capability.manifest.contributions.context_schema_refs.is_empty()
                    {
                        return Err(KernelError::InvalidRoleProvider {
                            role_id: role_id.clone(),
                            mount_id: mount_id.clone(),
                            reason: format!(
                                "context member {} has malformed action/context declarations",
                                capability_id.as_ref()
                            ),
                        });
                    }
                    (false, true, false)
                }
                CapabilityKind::ResourceProvider => {
                    if !capability.manifest.contributions.actions.is_empty()
                        || !capability.manifest.contributions.context_schema_refs.is_empty()
                    {
                        return Err(KernelError::InvalidRoleProvider {
                            role_id: role_id.clone(),
                            mount_id: mount_id.clone(),
                            reason: format!(
                                "resource member {} has malformed action/context declarations",
                                capability_id.as_ref()
                            ),
                        });
                    }
                    (false, false, true)
                }
                _ => (false, false, false),
            };
            if (has_action, has_context, has_resource) != expected {
                return Err(KernelError::InvalidRoleProvider {
                    role_id: role_id.clone(),
                    mount_id: mount_id.clone(),
                    reason: format!(
                        "role member {} exports do not match its {:?} capability kind",
                        capability_id.as_ref(),
                        capability.manifest.kind
                    ),
                });
            }
        }
    }
    for (role_id, mount_id, capability_id) in action_handlers
        .keys()
        .chain(context_factories.keys())
        .chain(resource_factories.keys())
    {
        if !materialized.role_providers.contains_key(&(
            role_id.clone(),
            mount_id.clone(),
        )) {
            return Err(KernelError::InvalidRoleProvider {
                role_id: role_id.clone(),
                mount_id: mount_id.clone(),
                reason: format!(
                    "typed export {} has no materialized provider contribution",
                    capability_id.as_ref()
                ),
            });
        }
    }
    for (capability_id, capability) in &materialized.capabilities {
        if let Some(role_id) = materialized.role_for_capability(capability_id)
            && generic_handlers.contains_key(capability_id)
        {
            return Err(KernelError::InvalidRoleProvider {
                role_id: role_id.clone(),
                mount_id: capability.mount_id.clone(),
                reason: format!(
                    "role-backed capability {} is also registered in the generic handler map",
                    capability_id.as_ref()
                ),
            });
        }
    }
    Ok(())
}

struct ResolvedRoleMember {
    role_id: ExecutionRoleId,
    provider_lock: ResolvedRoleProviderLock,
    context: ResolvedRoleMemberContext,
    context_schema_ref: Option<CanonicalSchemaRef>,
}

fn resolve_role_member(
    published: &PublishedRegistry,
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    request: &RoleMemberInvocationRequest,
    expected_kind: CapabilityKind,
) -> Result<ResolvedRoleMember, KernelError> {
    if request.principal != request.session_owner {
        return Err(KernelError::ResourceOwnerMismatch {
            binding_id: ResourceBindingId::from("session-owner"),
        });
    }
    let (agent_session_id, resolved_snapshot_ref, active_set_generation) =
        match &request.admission {
            RoleMemberAdmission::Agent {
                agent_session_id,
                resolved_snapshot_ref,
                active_set_generation,
            } => (
                agent_session_id.clone(),
                resolved_snapshot_ref.clone(),
                *active_set_generation,
            ),
            RoleMemberAdmission::Operation { .. } => {
                return Err(KernelError::CapabilityExecution {
                    reason: "non-Agent Role member admission is not implemented for this path"
                        .to_owned(),
                });
            }
        };
    if resolved_snapshot_ref != *snapshot.snapshot_ref()
        || active.resolved_snapshot_ref != resolved_snapshot_ref
        || !snapshot
            .content()
            .capability_allowlist
            .contains(&request.capability_id)
    {
        return Err(KernelError::CapabilityNotInPreset {
            capability_id: request.capability_id.clone(),
        });
    }
    if active.generation != active_set_generation
        || !active.active.contains(&request.capability_id)
    {
        return Err(KernelError::CapabilityNotActive {
            capability_id: request.capability_id.clone(),
        });
    }
    if snapshot.registry_generation != published.materialized.generation
        || snapshot.registry_digest != published.materialized.registry_digest
    {
        return Err(KernelError::RegistryGenerationMismatch {
            expected_generation: snapshot.registry_generation,
            expected_digest: snapshot.registry_digest.clone(),
            actual_generation: published.materialized.generation,
            actual_digest: published.materialized.registry_digest.clone(),
        });
    }
    let capability = published
        .materialized
        .capability(&request.capability_id)
        .ok_or_else(|| KernelError::CapabilityNotMaterialized {
            capability_id: request.capability_id.clone(),
            version: nomifun_agent_contracts::VersionString::from("unknown"),
        })?;
    if capability.manifest.kind != expected_kind {
        return Err(KernelError::CapabilityExecution {
            reason: format!(
                "{} is not a {:?} role member",
                request.capability_id.as_ref(),
                expected_kind
            ),
        });
    }
    let role_id = published
        .materialized
        .role_for_capability(&request.capability_id)
        .cloned()
        .ok_or_else(|| KernelError::RoleProviderNotBound {
            role_id: ExecutionRoleId::from(request.capability_id.as_ref()),
        })?;
    let provider_lock = snapshot
        .role_provider(&role_id)
        .cloned()
        .ok_or_else(|| KernelError::RoleProviderNotBound {
            role_id: role_id.clone(),
        })?;
    if !provider_lock
        .supported_members
        .contains(&request.capability_id)
    {
        return Err(KernelError::RoleProviderMemberUnavailable {
            role_id,
            capability_id: request.capability_id.clone(),
        });
    }
    let provider = published
        .materialized
        .role_provider(&role_id, &provider_lock.provider.mount_id)
        .ok_or_else(|| KernelError::RoleProviderUnavailable {
            role_id: role_id.clone(),
            mount_id: provider_lock.provider.mount_id.clone(),
        })?;
    if provider.provider != provider_lock.provider {
        return Err(KernelError::RoleProviderUnavailable {
            role_id,
            mount_id: provider_lock.provider.mount_id.clone(),
        });
    }
    if provider.source != provider_lock.source
        || provider_lock.supported_members
            != provider.contribution.members.keys().cloned().collect()
    {
        return Err(KernelError::RoleProviderUnavailable {
            role_id: role_id.clone(),
            mount_id: provider_lock.provider.mount_id.clone(),
        });
    }
    let policy = snapshot
        .policy(&request.capability_id)
        .ok_or_else(|| KernelError::CapabilityNotInPreset {
            capability_id: request.capability_id.clone(),
        })?;
    if request.resource_binding_ids != policy.resource_binding_ids {
        let unexpected = request
            .resource_binding_ids
            .difference(&policy.resource_binding_ids)
            .next()
            .cloned()
            .unwrap_or_else(|| ResourceBindingId::from("missing"));
        let kind = snapshot
            .binding(&unexpected)
            .map(|binding| binding.resource_kind.as_ref().to_owned())
            .unwrap_or_else(|| "unknown".to_owned());
        return Err(KernelError::UnexpectedResourceBinding {
            capability_id: request.capability_id.clone(),
            binding_id: unexpected,
            resource_kind: kind,
        });
    }
    let frozen_role_bindings = provider_lock
        .resource_binding_refs
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if !request
        .resource_binding_ids
        .is_subset(&frozen_role_bindings)
    {
        return Err(KernelError::InvalidPresetRevision {
            reason: format!(
                "role member {} references a binding outside the frozen provider lock",
                request.capability_id.as_ref()
            ),
        });
    }
    let mut resource_bindings = request
        .resource_binding_ids
        .iter()
        .map(|binding_id| {
            snapshot
                .binding(binding_id)
                .cloned()
                .ok_or_else(|| KernelError::ResourceBindingMissing {
                    binding_id: binding_id.clone(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    for binding in &resource_bindings {
        if binding.owner_id != request.principal.principal_id {
            return Err(KernelError::ResourceOwnerMismatch {
                binding_id: binding.binding_id.clone(),
            });
        }
    }
    resource_bindings.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
    let state = published
        .state_handles
        .get(&provider_lock.provider.mount_id)
        .cloned()
        .ok_or(KernelError::RegistryPoisoned)?;
    let services = published
        .service_views
        .get(&provider_lock.provider.mount_id)
        .cloned()
        .unwrap_or_default();
    let metadata = published
        .materialized
        .plugins
        .get(&provider_lock.provider.mount_id)
        .ok_or(KernelError::RegistryPoisoned)?;
    let context_schema_ref = match expected_kind {
        CapabilityKind::ContextContributor => {
            let [schema_ref] = capability.manifest.contributions.context_schema_refs.as_slice()
            else {
                return Err(KernelError::InvalidRoleProvider {
                    role_id: role_id.clone(),
                    mount_id: provider_lock.provider.mount_id.clone(),
                    reason: format!(
                        "context member {} must declare exactly one context schema",
                        request.capability_id.as_ref()
                    ),
                });
            };
            Some(schema_ref.clone())
        }
        _ => None,
    };
    Ok(ResolvedRoleMember {
        role_id,
        provider_lock: provider_lock.clone(),
        context: ResolvedRoleMemberContext {
            role_id: provider_lock.provider.role.key.role_id.clone(),
            member_id: request.capability_id.clone(),
            provider_lock,
            principal: request.principal.clone(),
            agent_session_id: Some(agent_session_id),
            operation_id: request.operation_id.clone(),
            correlation_id: request.correlation_id.clone(),
            resolved_snapshot_ref: Some(resolved_snapshot_ref),
            registry_generation: published.materialized.generation,
            registry_digest: published.materialized.registry_digest.clone(),
            resource_bindings,
            state_scope_key: request.state_scope_key.clone(),
            mount: ProviderMountContext {
                identity: metadata.context.identity.clone(),
                config: metadata.context.validated_config.clone(),
                state,
                services,
            },
        },
        context_schema_ref,
    })
}
