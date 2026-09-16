//! Typed Role adapters reuse the direct JS execution path after validating the
//! frozen Provider identity. They never resolve defaults or re-enter Kernel.

use super::*;
use nomifun_agent_contracts::{ExactRoleProviderRef, ResolvedRoleProviderLock};
use nomifun_agent_kernel::{
    ContextContributionFactory, ContextContributionRequest, ResolvedRoleMemberContext,
    ResourceProviderFactory, ResourceProviderRequest, RoleToolHandler, RoleToolInvocationContext,
};

pub(super) fn register(
    adapter: &JsKernelPluginAdapter,
    host: Arc<dyn ExtensionHostDemandPort>,
    registration: &mut PluginRegistration,
) -> Result<(), JsKernelAdapterError> {
    for provider in &adapter.manifest().package.contributions.role_providers {
        let expected = ExactRoleProviderRef {
            role: provider.role.clone(),
            package: adapter.manifest().package_ref(),
            mount_id: adapter.mount_id.clone(),
            contribution_digest: digest_payload(provider)
                .map_err(|error| JsKernelAdapterError::InvalidPackage(error.to_string()))?,
        };
        for (member_id, member) in &provider.members {
            let implementation = member.implementation.as_ref().ok_or_else(|| {
                JsKernelAdapterError::InvalidPackage("JS Role member has no implementation".into())
            })?;
            let capability = adapter.capability(&implementation.id)?;
            let export = Arc::new(NodeRoleExport {
                provider: expected.clone(),
                member_id: member_id.clone(),
                host: Arc::clone(&host),
                mount: adapter.mount_demand(),
                contribution: contribution_ref(
                    adapter.target(),
                    capability.id.clone(),
                    capability.contribution_id.clone(),
                    capability,
                )?,
            });
            let role_id = provider.role.key.role_id.clone();
            let result = match capability.kind {
                CapabilityKind::Tool => registration
                    .add_role_action_handler(role_id.clone(), member_id.clone(), export.clone())
                    .and_then(|()| {
                        registration.add_role_tool_handler(role_id, member_id.clone(), export)
                    }),
                CapabilityKind::ContextContributor => {
                    registration.add_role_context_factory(role_id, member_id.clone(), export)
                }
                CapabilityKind::ResourceProvider => {
                    registration.add_role_resource_factory(role_id, member_id.clone(), export)
                }
                _ => {
                    return Err(JsKernelAdapterError::UnsupportedCapability {
                        capability: capability.id.clone(),
                    });
                }
            };
            result.map_err(|error| JsKernelAdapterError::InvalidPackage(error.to_string()))?;
        }
    }
    Ok(())
}

struct NodeRoleExport {
    provider: ExactRoleProviderRef,
    member_id: CapabilityId,
    host: Arc<dyn ExtensionHostDemandPort>,
    mount: MountLoadDemand,
    contribution: PluginHostContributionRef,
}

impl NodeRoleExport {
    fn validate_lock(
        &self,
        member: &CapabilityId,
        lock: &ResolvedRoleProviderLock,
    ) -> Result<(), KernelError> {
        if member != &self.member_id
            || lock.provider != self.provider
            || !lock.supported_members.contains(&self.member_id)
            || lock.source.source_kind != PluginSourceKind::ManagedLocal
            || lock.source.source_identity != self.contribution.target.mount_id.as_ref()
            || lock.source.source_digest.as_ref() != Some(&self.contribution.target.artifact_digest)
        {
            return Err(drift(
                &self.member_id,
                "Role Provider differs from the immutable JS implementation lock",
            ));
        }
        Ok(())
    }

    fn validate_context(&self, context: &ResolvedRoleMemberContext) -> Result<(), KernelError> {
        self.validate_lock(&context.member_id, &context.provider_lock)?;
        if context.role_id != self.provider.role.key.role_id
            || context.mount.identity.package != self.provider.package
            || context.mount.identity.mount_id != self.provider.mount_id
            || context.mount.state.descriptor().package_id != self.provider.package.id
            || context.mount.state.descriptor().mount_id != self.provider.mount_id
        {
            return Err(drift(
                &self.member_id,
                "Role context is bound to a different Provider Mount",
            ));
        }
        Ok(())
    }

    fn tool(&self) -> NodeToolHandler {
        NodeToolHandler {
            host: Arc::clone(&self.host),
            mount: self.mount.clone(),
            contribution: self.contribution.clone(),
        }
    }
}

#[async_trait]
impl CapabilityHandler for NodeRoleExport {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        let lock = context
            .role_provider
            .as_ref()
            .ok_or_else(|| drift(&self.member_id, "Role invocation has no frozen Provider"))?;
        self.validate_lock(&context.capability_id, lock)?;
        if context.resolved_capability.capability.id != self.member_id
            || context.state.descriptor().package_id != self.provider.package.id
            || context.state.descriptor().mount_id != self.provider.mount_id
        {
            return Err(drift(
                &self.member_id,
                "Role invocation state differs from the Provider Mount",
            ));
        }
        self.tool().invoke_scoped(context.action_id, input, context.dependencies).await
    }
}

#[async_trait]
impl RoleToolHandler for NodeRoleExport {
    async fn invoke(
        &self,
        context: RoleToolInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        self.validate_context(&context.context)?;
        self.tool().invoke_action(context.action_id, input).await
    }
}

#[async_trait]
impl ContextContributionFactory for NodeRoleExport {
    async fn contribute(
        &self,
        request: ContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        self.validate_context(&request.context)?;
        NodeContextProxy {
            host: Arc::clone(&self.host),
            mount: self.mount.clone(),
            contribution: self.contribution.clone(),
        }
        .contribute_schema(request.schema_ref, request.input, request.dependencies)
        .await
    }
}

#[async_trait]
impl ResourceProviderFactory for NodeRoleExport {
    async fn acquire(
        &self,
        request: ResourceProviderRequest,
    ) -> Result<ResourceProviderResult, KernelError> {
        self.validate_context(&request.context)?;
        NodeResourceProxy {
            host: Arc::clone(&self.host),
            mount: self.mount.clone(),
            contribution: self.contribution.clone(),
        }
        .acquire_bindings(&request.context.resource_bindings)
        .await
    }
}
