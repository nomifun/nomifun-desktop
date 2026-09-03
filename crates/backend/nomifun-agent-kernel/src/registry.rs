use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use nomifun_agent_contracts::{
    ActionId, CanonicalSchemaRef, CapabilityId, CapabilityKind, DigestHex, ExecutionRoleId,
    PackageRef, PluginMountId, ResolvedRoleProviderLock, ResourceBindingId, ScopeKey,
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
    RoleMemberAdmission, RoleMemberInvocationRequest, RoleToolHandler,
    RoleToolInvocationContext, RoleToolOperationRequest, ThinAuthority,
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
    role_tool_handlers:
        BTreeMap<(ExecutionRoleId, PluginMountId, CapabilityId), Arc<dyn RoleToolHandler>>,
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
            role_tool_handlers: BTreeMap::new(),
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
        let mut role_tool_handlers = BTreeMap::new();
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
            for ((role_id, capability_id), handler) in registration.role_tool_handlers() {
                let key = (
                    role_id.clone(),
                    registration.metadata.mount_id.clone(),
                    capability_id.clone(),
                );
                if role_tool_handlers
                    .insert(key, Arc::clone(handler))
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
            &role_tool_handlers,
            &role_context_factories,
            &role_resource_factories,
        )?;

        let next = Arc::new(PublishedRegistry {
            materialized: Arc::clone(&materialized),
            handlers,
            role_handlers,
            role_tool_handlers,
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
        let role_request = RoleMemberInvocationRequest {
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
        };
        if snapshot
            .content()
            .resolved_role_providers
            .values()
            .any(|provider| {
                provider.supported_members.contains(&request.capability_id)
            })
        {
            self.ensure_role_resources_for_member(
                RoleAdmissionEvidence::Agent { snapshot, active },
                &role_request,
                CapabilityKind::Tool,
            )
            .await?;
        }
        let role_dispatch = {
            let published = self
                .published
                .read()
                .map_err(|_| KernelError::RegistryPoisoned)?;
            if published
                .materialized
                .role_for_capability(&request.capability_id)
                .is_some()
            {
                let resolved = resolve_role_member_dispatch(
                    &published,
                    RoleAdmissionEvidence::Agent { snapshot, active },
                    &role_request,
                    RoleMemberDispatchKind::AgentTool {
                        action_id: &request.action_id,
                    },
                )?;
                let RoleMemberDispatchTarget::AgentTool(handler) = resolved.target else {
                    return Err(KernelError::RegistryPoisoned);
                };
                Some((handler, resolved.member.context))
            } else {
                None
            }
        };
        if let Some((handler, context)) = role_dispatch {
            return dispatch_resolved_role_tool(
                RoleMemberDispatchTarget::AgentTool(handler),
                context,
                request.action_id,
                request.idempotency_key,
                request.input,
            )
            .await;
        }
        let (handler, context) = {
            let published = self
                .published
                .read()
                .map_err(|_| KernelError::RegistryPoisoned)?;
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
            resource_bindings.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
            let mcp_tool_lock = snapshot
                .content()
                .mcp_tool_locks
                .iter()
                .find(|lock| lock.capability_id == request.capability_id)
                .cloned();
            (
                Arc::clone(&binding.handler),
                CapabilityInvocationContext {
                    principal: request.principal,
                    agent_session_id: request.agent_session_id,
                    operation_id: request.operation_id,
                    idempotency_key: request.idempotency_key,
                    correlation_id: request.correlation_id,
                    resolved_snapshot_ref: request.resolved_snapshot_ref,
                    registry_generation: published.materialized.generation,
                    capability_id: request.capability_id,
                    action_id: request.action_id,
                    resource_bindings,
                    role_provider: None,
                    state_scope_key: request.state_scope_key,
                    state,
                    services,
                    mcp_tool_lock,
                },
            )
        };
        handler.invoke(context, request.input).await
    }

    /// Invoke a role-backed Tool from a non-Agent operation admission.
    ///
    /// Unlike [`Self::invoke`], this route does not accept or synthesize an
    /// AgentSession/Snapshot. The operation's exact Provider lock and typed
    /// resource projection are resolved once and passed to the operation
    /// handler.
    pub async fn invoke_role_tool(
        &self,
        request: RoleToolOperationRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, KernelError> {
        if !matches!(
            request.member.admission,
            RoleMemberAdmission::Operation { .. }
        ) {
            return Err(KernelError::CapabilityExecution {
                reason: "invoke_role_tool requires Operation admission".to_owned(),
            });
        }
        self.ensure_role_resources_for_member(
            RoleAdmissionEvidence::Operation,
            &request.member,
            CapabilityKind::Tool,
        )
        .await?;
        let (target, context) = {
            let published = self
                .published
                .read()
                .map_err(|_| KernelError::RegistryPoisoned)?;
            let resolved = resolve_role_member_dispatch(
                &published,
                RoleAdmissionEvidence::Operation,
                &request.member,
                RoleMemberDispatchKind::OperationTool {
                    action_id: &request.action_id,
                },
            )?;
            (resolved.target, resolved.member.context)
        };
        dispatch_resolved_role_tool(
            target,
            context,
            request.action_id,
            request.idempotency_key,
            request.input,
        )
        .await
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
        self.ensure_role_resources_for_member(
            RoleAdmissionEvidence::Agent { snapshot, active },
            &request,
            CapabilityKind::ContextContributor,
        )
        .await?;
        self.contribute_role_context_with_evidence(
            RoleAdmissionEvidence::Agent { snapshot, active },
            request,
        )
        .await
    }

    /// Assemble a ContextContributor through a non-Agent operation admission.
    pub async fn contribute_role_context_operation(
        &self,
        request: RoleMemberInvocationRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        if !matches!(
            request.admission,
            RoleMemberAdmission::Operation { .. }
        ) {
            return Err(KernelError::CapabilityExecution {
                reason: "contribute_role_context_operation requires Operation admission"
                    .to_owned(),
            });
        }
        self.ensure_role_resources_for_member(
            RoleAdmissionEvidence::Operation,
            &request,
            CapabilityKind::ContextContributor,
        )
        .await?;
        self.contribute_role_context_with_evidence(
            RoleAdmissionEvidence::Operation,
            request,
        )
        .await
    }

    async fn contribute_role_context_with_evidence(
        &self,
        evidence: RoleAdmissionEvidence<'_>,
        request: RoleMemberInvocationRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        let (factory, context, schema_ref) = {
            let published = self
                .published
                .read()
                .map_err(|_| KernelError::RegistryPoisoned)?;
            let resolved = resolve_role_member_dispatch(
                &published,
                evidence,
                &request,
                RoleMemberDispatchKind::Context,
            )?;
            let RoleMemberDispatchTarget::Context(factory) = resolved.target else {
                return Err(KernelError::RegistryPoisoned);
            };
            let schema_ref = resolved.member.context_schema_ref.ok_or_else(|| {
                KernelError::InvalidRoleProvider {
                    role_id: resolved.member.role_id.clone(),
                    mount_id: resolved
                        .member
                        .provider_lock
                        .provider
                        .mount_id
                        .clone(),
                    reason: format!(
                        "context member {} does not declare exactly one context schema",
                        request.capability_id.as_ref()
                    ),
                }
            })?;
            (factory, resolved.member.context, schema_ref)
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
        self.acquire_role_resource_with_evidence(
            RoleAdmissionEvidence::Agent { snapshot, active },
            request,
        )
        .await
    }

    /// Acquire a ResourceProvider through a non-Agent operation admission.
    pub async fn acquire_role_resource_operation(
        &self,
        request: RoleMemberInvocationRequest,
    ) -> Result<ResourceProviderResult, KernelError> {
        if !matches!(
            request.admission,
            RoleMemberAdmission::Operation { .. }
        ) {
            return Err(KernelError::CapabilityExecution {
                reason: "acquire_role_resource_operation requires Operation admission"
                    .to_owned(),
            });
        }
        self.acquire_role_resource_with_evidence(
            RoleAdmissionEvidence::Operation,
            request,
        )
        .await
    }

    async fn acquire_role_resource_with_evidence(
        &self,
        evidence: RoleAdmissionEvidence<'_>,
        request: RoleMemberInvocationRequest,
    ) -> Result<ResourceProviderResult, KernelError> {
        let (factory, context) = {
            let published = self
                .published
                .read()
                .map_err(|_| KernelError::RegistryPoisoned)?;
            let resolved = resolve_role_member_dispatch(
                &published,
                evidence,
                &request,
                RoleMemberDispatchKind::Resource,
            )?;
            let RoleMemberDispatchTarget::Resource(factory) = resolved.target else {
                return Err(KernelError::RegistryPoisoned);
            };
            (factory, resolved.member.context)
        };
        let key = resource_handle_key(&context)?;
        if let Some(handle) = self.resource_handles.lock().await.get(&key).cloned() {
            return Ok(ResourceProviderResult { handle });
        }
        let result = factory
            .acquire(ResourceProviderRequest {
                context: context.clone(),
            })
            .await?;
        self.retain_resource_handle(&context, result).await
    }

    async fn retain_resource_handle(
        &self,
        context: &ResolvedRoleMemberContext,
        result: ResourceProviderResult,
    ) -> Result<ResourceProviderResult, KernelError> {
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
        let key = resource_handle_key(context)?;
        let mut handles = self.resource_handles.lock().await;
        if let Some(existing) = handles.get(&key).cloned() {
            drop(handles);
            if !Arc::ptr_eq(&existing, &result.handle) {
                result.handle.release().await?;
            }
            return Ok(ResourceProviderResult { handle: existing });
        }
        handles.insert(key, Arc::clone(&result.handle));
        Ok(result)
    }

    async fn ensure_role_resources_for_member(
        &self,
        evidence: RoleAdmissionEvidence<'_>,
        request: &RoleMemberInvocationRequest,
        expected_kind: CapabilityKind,
    ) -> Result<(), KernelError> {
        let acquisitions = {
            let published = self
                .published
                .read()
                .map_err(|_| KernelError::RegistryPoisoned)?;
            let resolved = resolve_role_member(
                &published,
                evidence,
                request,
                expected_kind,
            )?;
            let provider = published
                .materialized
                .role_provider(
                    &resolved.role_id,
                    &resolved.provider_lock.provider.mount_id,
                )
                .ok_or_else(|| KernelError::RoleProviderUnavailable {
                    role_id: resolved.role_id.clone(),
                    mount_id: resolved.provider_lock.provider.mount_id.clone(),
                })?;
            let target_member = provider
                .contribution
                .members
                .get(&request.capability_id)
                .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                    role_id: resolved.role_id.clone(),
                    capability_id: request.capability_id.clone(),
                })?;
            let mut acquisitions = Vec::new();
            for (resource_capability_id, resource_member) in
                &provider.contribution.members
            {
                let Some(resource_capability) = published
                    .materialized
                    .capability(resource_capability_id)
                else {
                    continue;
                };
                if resource_capability.manifest.kind != CapabilityKind::ResourceProvider
                    || resource_member
                        .required_resource_kinds
                        .is_disjoint(&target_member.required_resource_kinds)
                {
                    continue;
                }
                let factory = published
                    .role_resource_factories
                    .get(&(
                        resolved.role_id.clone(),
                        resolved.provider_lock.provider.mount_id.clone(),
                        resource_capability_id.clone(),
                    ))
                    .cloned()
                    .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                        role_id: resolved.role_id.clone(),
                        capability_id: resource_capability_id.clone(),
                    })?;
                let resource_bindings = resolved
                    .context
                    .resource_bindings
                    .iter()
                    .filter(|binding| {
                        resource_member
                            .required_resource_kinds
                            .contains(&binding.resource_kind)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if resource_bindings.is_empty() {
                    return Err(KernelError::ResourceBindingMissing {
                        binding_id: ResourceBindingId::from(
                            resource_member
                                .required_resource_kinds
                                .iter()
                                .next()
                                .map(AsRef::as_ref)
                                .unwrap_or("resource"),
                        ),
                    });
                }
                let mut context = resolved.context.clone();
                context.member_id = resource_capability_id.clone();
                context.resource_bindings = resource_bindings;
                acquisitions.push((factory, context));
            }
            acquisitions
        };
        for (factory, context) in acquisitions {
            let key = resource_handle_key(&context)?;
            if self.resource_handles.lock().await.contains_key(&key) {
                continue;
            }
            let result = factory
                .acquire(ResourceProviderRequest {
                    context: context.clone(),
                })
                .await?;
            self.retain_resource_handle(&context, result).await?;
        }
        Ok(())
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

async fn dispatch_resolved_role_tool(
    target: RoleMemberDispatchTarget,
    context: ResolvedRoleMemberContext,
    action_id: ActionId,
    idempotency_key: nomifun_agent_contracts::IdempotencyKey,
    input: nomifun_agent_contracts::StrictJsonValue,
) -> Result<nomifun_agent_contracts::StrictJsonValue, KernelError> {
    match target {
        RoleMemberDispatchTarget::AgentTool(handler) => {
            let agent_session_id = context.agent_session_id.ok_or_else(|| {
                KernelError::CapabilityExecution {
                    reason: "Agent Tool dispatch resolved without an AgentSession".to_owned(),
                }
            })?;
            let resolved_snapshot_ref = context.resolved_snapshot_ref.ok_or_else(|| {
                KernelError::CapabilityExecution {
                    reason: "Agent Tool dispatch resolved without a Snapshot".to_owned(),
                }
            })?;
            handler
                .invoke(
                    CapabilityInvocationContext {
                        principal: context.principal,
                        agent_session_id,
                        operation_id: context.operation_id,
                        idempotency_key,
                        correlation_id: context.correlation_id,
                        resolved_snapshot_ref,
                        registry_generation: context.registry_generation,
                        capability_id: context.member_id,
                        action_id,
                        resource_bindings: context.resource_bindings,
                        role_provider: Some(context.provider_lock),
                        state_scope_key: context.state_scope_key,
                        state: context.mount.state,
                        services: context.mount.services,
                        mcp_tool_lock: None,
                    },
                    input,
                )
                .await
        }
        RoleMemberDispatchTarget::OperationTool(handler) => {
            handler
                .invoke(
                    RoleToolInvocationContext {
                        context,
                        action_id,
                        idempotency_key,
                    },
                    input,
                )
                .await
        }
        RoleMemberDispatchTarget::Context(_)
        | RoleMemberDispatchTarget::Resource(_) => Err(KernelError::CapabilityExecution {
            reason: "role member dispatch target is not a Tool".to_owned(),
        }),
    }
}

fn resource_handle_key(
    context: &ResolvedRoleMemberContext,
) -> Result<ResourceHandleKey, KernelError> {
    let [binding] = context.resource_bindings.as_slice() else {
        return Err(KernelError::InvalidPresetRevision {
            reason: format!(
                "resource provider {} requires exactly one frozen resource binding",
                context.member_id.as_ref()
            ),
        });
    };
    Ok(ResourceHandleKey {
        scope_key: context.state_scope_key.clone(),
        role_id: context.role_id.clone(),
        mount_id: context.provider_lock.provider.mount_id.clone(),
        contribution_digest: context.provider_lock.provider.contribution_digest.clone(),
        binding_id: binding.binding_id.clone(),
    })
}

fn validate_role_exports(
    materialized: &MaterializedRegistry,
    generic_handlers: &BTreeMap<CapabilityId, HandlerBinding>,
    action_handlers: &BTreeMap<
        (ExecutionRoleId, PluginMountId, CapabilityId),
        HandlerBinding,
    >,
    operation_tool_handlers: &BTreeMap<
        (ExecutionRoleId, PluginMountId, CapabilityId),
        Arc<dyn RoleToolHandler>,
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
            let has_operation_tool = operation_tool_handlers.contains_key(&key);
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
            if has_operation_tool && capability.manifest.kind != CapabilityKind::Tool {
                return Err(KernelError::InvalidRoleProvider {
                    role_id: role_id.clone(),
                    mount_id: mount_id.clone(),
                    reason: format!(
                        "operation Tool export {} does not target a Tool capability",
                        capability_id.as_ref()
                    ),
                });
            }
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
        .chain(operation_tool_handlers.keys())
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

#[derive(Clone, Copy)]
enum RoleAdmissionEvidence<'a> {
    Agent {
        snapshot: &'a CompiledSnapshot,
        active: &'a ActiveCapabilitySetSnapshot,
    },
    Operation,
}

enum RoleMemberDispatchKind<'a> {
    AgentTool { action_id: &'a ActionId },
    OperationTool { action_id: &'a ActionId },
    Context,
    Resource,
}

impl RoleMemberDispatchKind<'_> {
    fn capability_kind(&self) -> CapabilityKind {
        match self {
            Self::AgentTool { .. } | Self::OperationTool { .. } => CapabilityKind::Tool,
            Self::Context => CapabilityKind::ContextContributor,
            Self::Resource => CapabilityKind::ResourceProvider,
        }
    }
}

enum RoleMemberDispatchTarget {
    AgentTool(Arc<dyn CapabilityHandler>),
    OperationTool(Arc<dyn RoleToolHandler>),
    Context(Arc<dyn ContextContributionFactory>),
    Resource(Arc<dyn ResourceProviderFactory>),
}

struct ResolvedRoleMemberDispatch {
    member: ResolvedRoleMember,
    target: RoleMemberDispatchTarget,
}

fn resolve_role_member_dispatch(
    published: &PublishedRegistry,
    evidence: RoleAdmissionEvidence<'_>,
    request: &RoleMemberInvocationRequest,
    dispatch_kind: RoleMemberDispatchKind<'_>,
) -> Result<ResolvedRoleMemberDispatch, KernelError> {
    let member = resolve_role_member(
        published,
        evidence,
        request,
        dispatch_kind.capability_kind(),
    )?;
    if let RoleMemberDispatchKind::AgentTool { action_id }
    | RoleMemberDispatchKind::OperationTool { action_id } = &dispatch_kind
    {
        let capability = published
            .materialized
            .capability(&request.capability_id)
            .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                capability_id: request.capability_id.clone(),
                version: nomifun_agent_contracts::VersionString::from("unknown"),
            })?;
        if !capability
            .manifest
            .contributions
            .actions
            .iter()
            .any(|action| &action.action_id == *action_id)
        {
            return Err(KernelError::ActionNotDeclared {
                capability_id: request.capability_id.clone(),
                action_id: (*action_id).clone(),
            });
        }
    }
    let key = (
        member.role_id.clone(),
        member.provider_lock.provider.mount_id.clone(),
        request.capability_id.clone(),
    );
    let target = match dispatch_kind {
        RoleMemberDispatchKind::AgentTool { .. } => {
            let handler = published
                .role_handlers
                .get(&key)
                .map(|binding| Arc::clone(&binding.handler))
                .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                    role_id: member.role_id.clone(),
                    capability_id: request.capability_id.clone(),
                })?;
            RoleMemberDispatchTarget::AgentTool(handler)
        }
        RoleMemberDispatchKind::OperationTool { .. } => {
            let handler = published
                .role_tool_handlers
                .get(&key)
                .cloned()
                .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                    role_id: member.role_id.clone(),
                    capability_id: request.capability_id.clone(),
                })?;
            RoleMemberDispatchTarget::OperationTool(handler)
        }
        RoleMemberDispatchKind::Context => {
            let factory = published
                .role_context_factories
                .get(&key)
                .cloned()
                .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                    role_id: member.role_id.clone(),
                    capability_id: request.capability_id.clone(),
                })?;
            RoleMemberDispatchTarget::Context(factory)
        }
        RoleMemberDispatchKind::Resource => {
            let factory = published
                .role_resource_factories
                .get(&key)
                .cloned()
                .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                    role_id: member.role_id.clone(),
                    capability_id: request.capability_id.clone(),
                })?;
            RoleMemberDispatchTarget::Resource(factory)
        }
    };
    Ok(ResolvedRoleMemberDispatch { member, target })
}

fn resolve_role_member(
    published: &PublishedRegistry,
    evidence: RoleAdmissionEvidence<'_>,
    request: &RoleMemberInvocationRequest,
    expected_kind: CapabilityKind,
) -> Result<ResolvedRoleMember, KernelError> {
    if request.principal != request.session_owner {
        return Err(KernelError::ResourceOwnerMismatch {
            binding_id: ResourceBindingId::from("session-owner"),
        });
    }
    let (
        agent_session_id,
        resolved_snapshot_ref,
        agent_snapshot,
        operation_provider_lock,
        operation_resource_bindings,
        expected_registry_generation,
        expected_registry_digest,
    ) = match (evidence, &request.admission) {
        (
            RoleAdmissionEvidence::Agent { snapshot, active },
            RoleMemberAdmission::Agent {
                agent_session_id,
                resolved_snapshot_ref,
                active_set_generation,
            },
        ) => {
            if resolved_snapshot_ref != snapshot.snapshot_ref()
                || active.resolved_snapshot_ref != *resolved_snapshot_ref
                || !snapshot
                    .content()
                    .capability_allowlist
                    .contains(&request.capability_id)
            {
                return Err(KernelError::CapabilityNotInPreset {
                    capability_id: request.capability_id.clone(),
                });
            }
            if active.generation != *active_set_generation
                || !active.active.contains(&request.capability_id)
            {
                return Err(KernelError::CapabilityNotActive {
                    capability_id: request.capability_id.clone(),
                });
            }
            (
                Some(agent_session_id.clone()),
                Some(resolved_snapshot_ref.clone()),
                Some(snapshot),
                None,
                None,
                snapshot.registry_generation,
                snapshot.registry_digest.clone(),
            )
        }
        (
            RoleAdmissionEvidence::Operation,
            RoleMemberAdmission::Operation {
                provider_lock,
                registry_generation,
                registry_digest,
                resource_bindings,
            },
        ) => (
            None,
            None,
            None,
            Some(provider_lock.clone()),
            Some(resource_bindings.clone()),
            *registry_generation,
            registry_digest.clone(),
        ),
        _ => {
            return Err(KernelError::CapabilityExecution {
                reason: "role member admission does not match the requested dispatch path"
                    .to_owned(),
            });
        }
    };
    if expected_registry_generation != published.materialized.generation
        || expected_registry_digest != published.materialized.registry_digest
    {
        return Err(KernelError::RegistryGenerationMismatch {
            expected_generation: expected_registry_generation,
            expected_digest: expected_registry_digest,
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
    let provider_lock = match (agent_snapshot, operation_provider_lock) {
        (Some(snapshot), None) => snapshot
            .role_provider(&role_id)
            .cloned()
            .ok_or_else(|| KernelError::RoleProviderNotBound {
                role_id: role_id.clone(),
            })?,
        (None, Some(provider_lock)) => provider_lock,
        _ => {
            return Err(KernelError::CapabilityExecution {
                reason: "role member admission resolved inconsistent Provider evidence"
                    .to_owned(),
            });
        }
    };
    if provider_lock.provider.role.key.role_id != role_id {
        return Err(KernelError::RoleProviderUnavailable {
            role_id,
            mount_id: provider_lock.provider.mount_id.clone(),
        });
    }
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
    let provider_member = provider
        .contribution
        .members
        .get(&request.capability_id)
        .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
            role_id: role_id.clone(),
            capability_id: request.capability_id.clone(),
        })?;
    let frozen_role_bindings = provider_lock
        .resource_binding_refs
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut resource_bindings = if let Some(snapshot) = agent_snapshot {
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
        request
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
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let bindings = operation_resource_bindings.ok_or_else(|| {
            KernelError::InvalidPresetRevision {
                reason: "operation admission is missing typed resource bindings".to_owned(),
            }
        })?;
        let actual_binding_ids = bindings
            .iter()
            .map(|binding| binding.binding_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        if actual_binding_ids.len() != bindings.len() {
            return Err(KernelError::InvalidPresetRevision {
                reason: "operation admission contains duplicate resource binding IDs"
                    .to_owned(),
            });
        }
        if actual_binding_ids != request.resource_binding_ids {
            let unexpected = request
                .resource_binding_ids
                .difference(&actual_binding_ids)
                .next()
                .cloned()
                .or_else(|| {
                    actual_binding_ids
                        .difference(&request.resource_binding_ids)
                        .next()
                        .cloned()
                })
                .unwrap_or_else(|| ResourceBindingId::from("missing"));
            let kind = bindings
                .iter()
                .find(|binding| binding.binding_id == unexpected)
                .map(|binding| binding.resource_kind.as_ref().to_owned())
                .unwrap_or_else(|| "unknown".to_owned());
            return Err(KernelError::UnexpectedResourceBinding {
                capability_id: request.capability_id.clone(),
                binding_id: unexpected,
                resource_kind: kind,
            });
        }
        if request.resource_binding_ids != frozen_role_bindings {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "operation role member {} resource bindings do not match its exact Provider lock",
                    request.capability_id.as_ref()
                ),
            });
        }
        bindings
    };
    for binding in &resource_bindings {
        if binding.owner_id != request.principal.principal_id {
            return Err(KernelError::ResourceOwnerMismatch {
                binding_id: binding.binding_id.clone(),
            });
        }
    }
    if agent_snapshot.is_none() {
        for resource_kind in &provider_member.required_resource_kinds {
            let matches = resource_bindings
                .iter()
                .filter(|binding| &binding.resource_kind == resource_kind)
                .count();
            if matches == 0 {
                return Err(KernelError::CapabilityResourceNotBound {
                    capability_id: request.capability_id.clone(),
                    resource_kind: resource_kind.as_ref().to_owned(),
                });
            }
            if matches > 1 {
                return Err(KernelError::InvalidPresetRevision {
                    reason: format!(
                        "operation role member {} has multiple bindings for resource kind {}",
                        request.capability_id.as_ref(),
                        resource_kind.as_ref()
                    ),
                });
            }
        }
        if let Some(binding) = resource_bindings.iter().find(|binding| {
            !provider_member
                .required_resource_kinds
                .contains(&binding.resource_kind)
        }) {
            return Err(KernelError::UnexpectedResourceBinding {
                capability_id: request.capability_id.clone(),
                binding_id: binding.binding_id.clone(),
                resource_kind: binding.resource_kind.as_ref().to_owned(),
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
            agent_session_id,
            operation_id: request.operation_id.clone(),
            correlation_id: request.correlation_id.clone(),
            resolved_snapshot_ref,
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
