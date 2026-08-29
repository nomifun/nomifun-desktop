use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, CapabilityId, PluginRegistrationMetadata, PrincipalRef, ResolvedSnapshotRef,
    ResourceBindingId, ScopeKey, StrictJsonValue, TypedResourceBindings,
};

use crate::{DeclaredServiceView, KernelError, PluginStateHandle, ServiceExports};

#[derive(Clone, Debug, PartialEq)]
pub struct CapabilityInvocationRequest {
    pub principal: PrincipalRef,
    pub session_owner: PrincipalRef,
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
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub registry_generation: u64,
    pub capability_id: CapabilityId,
    pub action_id: ActionId,
    pub resource_bindings: TypedResourceBindings,
    pub state_scope_key: ScopeKey,
    pub state: PluginStateHandle,
    pub services: DeclaredServiceView,
}

#[async_trait]
pub trait CapabilityHandler: Send + Sync {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError>;
}

#[derive(Clone)]
pub struct PluginRegistration {
    pub metadata: PluginRegistrationMetadata,
    handlers: BTreeMap<CapabilityId, Arc<dyn CapabilityHandler>>,
    services: ServiceExports,
}

impl PluginRegistration {
    pub fn new(metadata: PluginRegistrationMetadata) -> Self {
        Self {
            metadata,
            handlers: BTreeMap::new(),
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

    pub fn handler_ids(&self) -> BTreeSet<CapabilityId> {
        self.handlers.keys().cloned().collect()
    }

    pub fn service_refs(&self) -> BTreeSet<nomifun_agent_contracts::ServiceKeyRef> {
        self.services.provided_refs()
    }

    pub(crate) fn handlers(
        &self,
    ) -> impl Iterator<Item = (&CapabilityId, &Arc<dyn CapabilityHandler>)> {
        self.handlers.iter()
    }

    pub(crate) fn services(&self) -> &ServiceExports {
        &self.services
    }
}
