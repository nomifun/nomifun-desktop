use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use nomifun_agent_contracts::{
    CapabilityId, PackageRef, PluginMountId, TypedResourceBinding,
};

use crate::service::build_service_bindings;
use crate::{
    ActiveCapabilitySetSnapshot, CapabilityHandler, CapabilityInvocationContext,
    CapabilityInvocationRequest, CompiledSnapshot, DeclaredServiceView, KernelError,
    MaterializationPolicy, MaterializedRegistry, Materializer, PluginRegistration,
    PluginStateError, PluginStateHandle, PluginStatePersistence, PluginStateStore, ThinAuthority,
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
    service_views: BTreeMap<PluginMountId, DeclaredServiceView>,
    state_handles: BTreeMap<PluginMountId, PluginStateHandle>,
}

impl PublishedRegistry {
    fn empty() -> Self {
        Self {
            materialized: Arc::new(MaterializedRegistry::empty()),
            handlers: BTreeMap::new(),
            service_views: BTreeMap::new(),
            state_handles: BTreeMap::new(),
        }
    }
}

pub struct KernelRegistry {
    policy: MaterializationPolicy,
    state_store: Arc<PluginStateStore>,
    published: RwLock<Arc<PublishedRegistry>>,
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
            Arc::new(Materializer::materialize(
                &self.policy,
                &registrations,
                generation,
            )?);

        validate_runtime_exports(&registrations)?;
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

        let next = Arc::new(PublishedRegistry {
            materialized: Arc::clone(&materialized),
            handlers,
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
                    resolved_snapshot_ref: request.resolved_snapshot_ref.clone(),
                    registry_generation: published.materialized.generation,
                    capability_id: request.capability_id.clone(),
                    action_id: request.action_id.clone(),
                    resource_bindings,
                    state_scope_key: request.state_scope_key.clone(),
                    state,
                    services,
                },
            )
        };
        handler.invoke(context, request.input).await
    }
}

fn validate_runtime_exports(
    registrations: &[PluginRegistration],
) -> Result<(), KernelError> {
    for registration in registrations {
        let manifest = &registration.metadata.manifest.payload;
        let expected_handlers = manifest
            .contributions
            .capabilities
            .iter()
            .filter(|capability| !capability.contributions.actions.is_empty())
            .map(|capability| capability.id.clone())
            .collect::<BTreeSet<_>>();
        let actual_handlers = registration.handler_ids();
        if let Some(capability_id) =
            expected_handlers.difference(&actual_handlers).next()
        {
            return Err(KernelError::MissingCapabilityHandler {
                mount_id: registration.metadata.mount_id.clone(),
                capability_id: capability_id.clone(),
            });
        }
        if let Some(capability_id) =
            actual_handlers.difference(&expected_handlers).next()
        {
            return Err(KernelError::UndeclaredCapabilityHandler {
                mount_id: registration.metadata.mount_id.clone(),
                capability_id: capability_id.clone(),
            });
        }

        let expected_services = manifest
            .provides_services
            .iter()
            .map(|provision| provision.service.clone())
            .collect::<BTreeSet<_>>();
        let actual_services = registration.service_refs();
        if let Some(service) = expected_services.difference(&actual_services).next() {
            return Err(KernelError::MissingRuntimeServiceExport {
                mount_id: registration.metadata.mount_id.clone(),
                service_id: service.id.clone(),
            });
        }
        if let Some(service) = actual_services.difference(&expected_services).next() {
            return Err(KernelError::UndeclaredRuntimeServiceExport {
                mount_id: registration.metadata.mount_id.clone(),
                service_id: service.id.clone(),
            });
        }
    }
    Ok(())
}
