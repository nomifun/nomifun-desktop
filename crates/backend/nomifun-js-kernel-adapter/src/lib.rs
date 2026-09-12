//! Kernel adapters for JavaScript Plugin Package v1.
//!
//! This crate owns no registry and no package lifecycle. It turns one
//! validated immutable package artifact into one Kernel registration and
//! forwards execution to the shared JavaScript Host.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ArtifactEnvelope, CapabilityId, CapabilityKind, CancellationDescriptor,
    DeclaredServiceViewDescriptor, HostPortId, HostPortRef,
    ManagedTaskRegistrationDescriptor, PackageEntrypointMetadata, PluginBootCriticality,
    PluginBootState,
    PluginContextDescriptor, PluginDesiredState, PluginEffectiveState, PluginHostContributionRef,
    PluginHostTargetLock, PluginIdentityDescriptor, PluginMountId, PluginMountRuntimeContext,
    PluginPackageArtifactV1, PluginPackageV1Manifest, PluginRegistrationMetadata,
    PluginRegistrarDescriptor, PluginRegistrarOperation, PluginSourceKind, PluginSourceMetadata,
    PluginStateHandleDescriptor, PluginStateMethod, ScopeKey, StrictJsonValue,
    ValidatedPluginConfig, VersionString, digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityContextContributionFactory, CapabilityContextContributionRequest,
    CapabilityHandler, CapabilityInvocationContext, CapabilityResourceProviderFactory,
    CapabilityOperationHandler, CapabilityOperationInvocationContext,
    CapabilityResourceProviderRequest, ContextContributionResult, KernelError,
    PluginRegistration, ResolvedCapabilityContext, ResolvedCapabilityOperationContext,
    ResourceHandle, ResourceHandleIdentity, ResourceProviderResult,
};
use nomifun_js_host::{
    ExtensionHostDemandPort, ImmutablePluginModule, JavaScriptHostError,
    JavaScriptResourceHandle, MountLoadDemand,
};
use thiserror::Error;

const CONTRACT_VERSION: &str = "1.0.0";
const CANCEL_PORT: &str = "host.plugin.cancel";
const TASK_PORT: &str = "host.plugin.tasks";

#[derive(Debug, Error)]
pub enum JsKernelAdapterError {
    #[error("Plugin Package v1 is invalid: {0}")]
    InvalidPackage(String),
    #[error("Plugin Package v1 has no JavaScript main.mjs entrypoint")]
    MissingEntrypoint,
    #[error("Plugin Package v1 package {0:?} cannot publish Node Role Provider or Plugin Service")]
    UnsupportedNodeSurface(String),
    #[error("Plugin Package v1 capability {capability:?} cannot be registered by this Kernel")]
    UnsupportedCapability { capability: CapabilityId },
    #[error("Plugin Package v1 module path is invalid: {0}")]
    InvalidModule(String),
    #[error("Plugin provenance drift for {capability:?}: {reason}")]
    ProvenanceDrift { capability: CapabilityId, reason: String },
    #[error("JavaScript Host operation is unavailable: {0}")]
    HostUnavailable(String),
}

impl From<JavaScriptHostError> for JsKernelAdapterError {
    fn from(value: JavaScriptHostError) -> Self {
        Self::HostUnavailable(value.to_string())
    }
}

#[derive(Clone, Debug)]
pub struct PluginPackageInput {
    pub artifact: PluginPackageArtifactV1,
    pub mount_id: PluginMountId,
    pub package_root: PathBuf,
    pub config: ValidatedPluginConfig,
    pub credential_bindings: Vec<nomifun_agent_contracts::CredentialSlotBinding>,
    pub data_dir: PathBuf,
}

#[derive(Clone)]
pub struct JsKernelPluginAdapter {
    artifact: PluginPackageArtifactV1,
    mount_id: PluginMountId,
    context: PluginMountRuntimeContext,
    module: ImmutablePluginModule,
}

impl JsKernelPluginAdapter {
    pub fn new(input: PluginPackageInput) -> Result<Self, JsKernelAdapterError> {
        input
            .artifact
            .validate()
            .map_err(|error| JsKernelAdapterError::InvalidPackage(error.to_string()))?;
        if input.mount_id.as_ref().trim().is_empty() {
            return Err(JsKernelAdapterError::InvalidPackage(
                "mount_id must be non-empty".into(),
            ));
        }
        let manifest = &input.artifact.manifest.payload;
        if !manifest.package.provides_services.is_empty()
            || !manifest.package.requires_services.is_empty()
            || !manifest.package.contributions.role_contracts.is_empty()
            || !manifest.package.contributions.role_providers.is_empty()
        {
            return Err(JsKernelAdapterError::UnsupportedNodeSurface(
                manifest.package.package_id.as_ref().to_owned(),
            ));
        }
        let entrypoint = match &manifest.package.entrypoint {
            PackageEntrypointMetadata::JavaScript(value) => value,
            PackageEntrypointMetadata::InProcess(_) => {
                return Err(JsKernelAdapterError::MissingEntrypoint);
            }
        };
        let main_mjs = input.package_root.join(&entrypoint.normalized_relative_path);
        if !main_mjs.is_absolute()
            || main_mjs.file_name().and_then(|name| name.to_str()) != Some("main.mjs")
        {
            return Err(JsKernelAdapterError::InvalidModule(
                "package_root/main.mjs must be an absolute path".into(),
            ));
        }
        if !input.data_dir.is_absolute() {
            return Err(JsKernelAdapterError::InvalidModule(
                "data_dir must be absolute".into(),
            ));
        }
        let package = manifest.package_ref();
        let target = PluginHostTargetLock {
            mount_id: input.mount_id.clone(),
            package: package.clone(),
            artifact_digest: input.artifact.artifact_digest.clone(),
            manifest_digest: input.artifact.manifest.payload_digest.clone(),
        };
        let state = PluginStateHandleDescriptor {
            package_id: package.id.clone(),
            mount_id: input.mount_id.clone(),
            methods: PluginStateMethod::REQUIRED.into_iter().collect(),
        };
        let context = PluginMountRuntimeContext {
            target: target.clone(),
            mount_handle_id: format!("plugin-mount:{}", input.mount_id.as_ref()),
            config: input.config,
            credential_bindings: input.credential_bindings,
            state,
            data_dir: input.data_dir.display().to_string(),
        };
        context
            .validate()
            .map_err(|error| JsKernelAdapterError::InvalidPackage(error.to_string()))?;
        let module = ImmutablePluginModule::new(
            main_mjs,
            entrypoint.module_digest.clone(),
            target,
        );
        Ok(Self {
            artifact: input.artifact,
            mount_id: input.mount_id,
            context,
            module,
        })
    }

    pub fn artifact(&self) -> &PluginPackageArtifactV1 {
        &self.artifact
    }

    pub fn manifest(&self) -> &PluginPackageV1Manifest {
        &self.artifact.manifest.payload
    }

    pub fn mount_context(&self) -> &PluginMountRuntimeContext {
        &self.context
    }

    pub fn mount_demand(&self) -> MountLoadDemand {
        MountLoadDemand {
            context: self.context.clone(),
            module: self.module.clone(),
        }
    }

    pub fn target(&self) -> &PluginHostTargetLock {
        &self.context.target
    }

    pub fn registration(
        &self,
        host: Arc<dyn ExtensionHostDemandPort>,
    ) -> Result<PluginRegistration, JsKernelAdapterError> {
        let manifest = self.manifest();
        let package = manifest.package_ref();
        let identity = PluginIdentityDescriptor {
            package: package.clone(),
            mount_id: self.mount_id.clone(),
        };
        let source = PluginSourceMetadata {
            source_kind: PluginSourceKind::ManagedLocal,
            source_identity: self.mount_id.as_ref().to_owned(),
            source_digest: Some(self.artifact.artifact_digest.clone()),
        };
        let cancellation_port = host_port(CANCEL_PORT);
        let task_port = host_port(TASK_PORT);
        let metadata = PluginRegistrationMetadata {
            manifest: ArtifactEnvelope::new(manifest.package.clone())
                .map_err(|error| JsKernelAdapterError::InvalidPackage(error.to_string()))?,
            mount_id: self.mount_id.clone(),
            source: source.clone(),
            boot_state: PluginBootState {
                criticality: PluginBootCriticality::Required,
                desired_state: PluginDesiredState::Enabled,
                effective_state: PluginEffectiveState::Active,
                diagnostic_code: None,
            },
            registrar: PluginRegistrarDescriptor {
                identity: identity.clone(),
                allowed_operations: registrar_operations(manifest),
                declared_capability_ids: manifest
                    .package
                    .contributions
                    .capabilities
                    .iter()
                    .map(|capability| capability.id.clone())
                    .collect(),
                declared_skill_ids: manifest
                    .package
                    .contributions
                    .skills
                    .iter()
                    .map(|skill| skill.id.clone())
                    .collect(),
                declared_mcp_tool_keys: manifest
                    .package
                    .contributions
                    .mcp_tools
                    .iter()
                    .map(|mapping| mapping.canonical_tool_key.clone())
                    .collect(),
                declared_role_ids: BTreeSet::new(),
                declared_service_keys: BTreeSet::new(),
                declared_host_ports: BTreeSet::from([
                    cancellation_port.id.clone(),
                    task_port.id.clone(),
                ]),
            },
            context: PluginContextDescriptor {
                identity,
                source,
                validated_config: self.context.config.clone(),
                state: self.context.state.clone(),
                declared_services: DeclaredServiceViewDescriptor::default(),
                host_ports: Vec::new(),
                typed_command_ports: Vec::new(),
                domain_outbox_ports: Vec::new(),
                cancellation: CancellationDescriptor {
                    cancellation_port,
                    scope_key: ScopeKey::from(format!("mount:{}", self.mount_id.as_ref())),
                },
                managed_task_registration: ManagedTaskRegistrationDescriptor {
                    registrar_port: task_port,
                    scope_key: ScopeKey::from(format!("mount:{}", self.mount_id.as_ref())),
                },
            },
        };
        let mut registration = PluginRegistration::new(metadata);
        for capability in &manifest.package.contributions.capabilities {
            match capability.kind {
                CapabilityKind::Tool => {
                    let handler = Arc::new(NodeToolHandler {
                        host: Arc::clone(&host),
                        mount: MountLoadDemand {
                            context: self.context.clone(),
                            module: self.module.clone(),
                        },
                        contribution: contribution_ref(
                            self.target(),
                            capability.id.clone(),
                            capability.contribution_id.clone(),
                            capability,
                        )?,
                    });
                    registration
                        .add_capability_handler(
                            capability.id.clone(),
                            handler.clone(),
                        )
                        .and_then(|()| {
                            registration.add_capability_operation_handler(
                                capability.id.clone(),
                                handler,
                            )
                        })
                        .map_err(|error| {
                            JsKernelAdapterError::InvalidPackage(error.to_string())
                        })?;
                }
                CapabilityKind::ContextContributor => registration
                    .add_capability_context_factory(
                        capability.id.clone(),
                        Arc::new(NodeContextProxy {
                            host: Arc::clone(&host),
                            mount: MountLoadDemand {
                                context: self.context.clone(),
                                module: self.module.clone(),
                            },
                            contribution: contribution_ref(
                                self.target(),
                                capability.id.clone(),
                                capability.contribution_id.clone(),
                                capability,
                            )?,
                        }),
                    )
                    .map_err(|error| {
                        JsKernelAdapterError::InvalidPackage(error.to_string())
                    })?,
                CapabilityKind::ResourceProvider => registration
                    .add_capability_resource_factory(
                        capability.id.clone(),
                        Arc::new(NodeResourceProxy {
                            host: Arc::clone(&host),
                            mount: MountLoadDemand {
                                context: self.context.clone(),
                                module: self.module.clone(),
                            },
                            contribution: contribution_ref(
                                self.target(),
                                capability.id.clone(),
                                capability.contribution_id.clone(),
                                capability,
                            )?,
                        }),
                    )
                    .map_err(|error| {
                        JsKernelAdapterError::InvalidPackage(error.to_string())
                    })?,
                _ => {
                    return Err(JsKernelAdapterError::UnsupportedCapability {
                        capability: capability.id.clone(),
                    });
                }
            }
        }
        Ok(registration)
    }

    pub fn context_proxy(
        &self,
        host: Arc<dyn ExtensionHostDemandPort>,
        capability: CapabilityId,
    ) -> Result<Arc<dyn CapabilityContextContributionFactory>, JsKernelAdapterError> {
        let capability_manifest = self.capability(&capability)?;
        if capability_manifest.kind != CapabilityKind::ContextContributor {
            return Err(JsKernelAdapterError::UnsupportedCapability { capability });
        }
        Ok(Arc::new(NodeContextProxy {
            host,
            mount: MountLoadDemand {
                context: self.context.clone(),
                module: self.module.clone(),
            },
            contribution: contribution_ref(
                self.target(),
                capability,
                capability_manifest.contribution_id.clone(),
                capability_manifest,
            )?,
        }))
    }

    pub fn resource_proxy(
        &self,
        host: Arc<dyn ExtensionHostDemandPort>,
        capability: CapabilityId,
    ) -> Result<Arc<dyn CapabilityResourceProviderFactory>, JsKernelAdapterError> {
        let capability_manifest = self.capability(&capability)?;
        if capability_manifest.kind != CapabilityKind::ResourceProvider {
            return Err(JsKernelAdapterError::UnsupportedCapability { capability });
        }
        Ok(Arc::new(NodeResourceProxy {
            host,
            mount: MountLoadDemand {
                context: self.context.clone(),
                module: self.module.clone(),
            },
            contribution: contribution_ref(
                self.target(),
                capability,
                capability_manifest.contribution_id.clone(),
                capability_manifest,
            )?,
        }))
    }

    fn capability(
        &self,
        capability: &CapabilityId,
    ) -> Result<&nomifun_agent_contracts::CapabilityManifest, JsKernelAdapterError> {
        self.manifest()
            .package
            .contributions
            .capabilities
            .iter()
            .find(|value| &value.id == capability)
            .ok_or_else(|| JsKernelAdapterError::UnsupportedCapability {
                capability: capability.clone(),
            })
    }
}

fn host_port(id: &str) -> HostPortRef {
    HostPortRef {
        id: HostPortId::from(id),
        version: VersionString::from(CONTRACT_VERSION),
    }
}

fn registrar_operations(manifest: &PluginPackageV1Manifest) -> BTreeSet<PluginRegistrarOperation> {
    let contributions = &manifest.package.contributions;
    let mut operations = BTreeSet::from([PluginRegistrarOperation::BindHostPort]);
    if !contributions.capabilities.is_empty() {
        operations.insert(PluginRegistrarOperation::ContributeCapability);
    }
    if !contributions.skills.is_empty() {
        operations.insert(PluginRegistrarOperation::ContributeSkill);
    }
    if !contributions.mcp_tools.is_empty() {
        operations.insert(PluginRegistrarOperation::ContributeMcpToolMapping);
    }
    operations
}

fn contribution_ref(
    target: &PluginHostTargetLock,
    capability: CapabilityId,
    contribution_id: nomifun_agent_contracts::ContributionId,
    manifest: &nomifun_agent_contracts::CapabilityManifest,
) -> Result<PluginHostContributionRef, JsKernelAdapterError> {
    let contract_digest = digest_payload(manifest)
        .map_err(|error| JsKernelAdapterError::InvalidPackage(error.to_string()))?;
    Ok(PluginHostContributionRef {
        target: target.clone(),
        contribution_id,
        capability: nomifun_agent_contracts::CapabilityRef {
            id: capability,
            version: manifest.version.clone(),
        },
        contract_digest,
    })
}

fn drift(capability: &CapabilityId, reason: impl Into<String>) -> KernelError {
    KernelError::CapabilityProvenanceDrift {
        capability_id: capability.clone(),
        reason: reason.into(),
    }
}

fn validate_invocation_context(
    context: &CapabilityInvocationContext,
    expected: &PluginHostContributionRef,
) -> Result<(), KernelError> {
    if context.capability_id != expected.capability.id {
        return Err(drift(
            &expected.capability.id,
            "CapabilityInvocationContext capability identity differs from the Package lock",
        ));
    }
    validate_resolved_target(&context.resolved_capability, expected)?;
    if context.state.descriptor().package_id != expected.target.package.id
        || context.state.descriptor().mount_id != expected.target.mount_id
    {
        return Err(drift(
            &expected.capability.id,
            "Kernel state handle is bound to a different Mount or Package",
        ));
    }
    Ok(())
}

fn validate_capability_context(
    context: &ResolvedCapabilityContext,
    expected: &PluginHostContributionRef,
) -> Result<(), KernelError> {
    validate_resolved_target(&context.resolved_capability, expected)?;
    if context.mount.identity.package != expected.target.package
        || context.mount.identity.mount_id != expected.target.mount_id
    {
        return Err(drift(
            &expected.capability.id,
            "resolved Capability context is bound to a different Mount or Package",
        ));
    }
    Ok(())
}

fn validate_operation_context(
    context: &ResolvedCapabilityOperationContext,
    expected: &PluginHostContributionRef,
) -> Result<(), KernelError> {
    let lock = &context.operation_lock;
    if lock.capability != expected.capability
        || lock.contribution.contribution_id != expected.contribution_id
        || lock.contribution.contract_digest != expected.contract_digest
        || lock.contribution.mount_id.as_ref() != Some(&expected.target.mount_id)
        || lock.target_artifact_digest.as_ref() != Some(&expected.target.artifact_digest)
        || context.mount.identity.package != expected.target.package
        || context.mount.identity.mount_id != expected.target.mount_id
    {
        return Err(drift(
            &expected.capability.id,
            "non-Agent operation context differs from the immutable JavaScript Package lock",
        ));
    }
    Ok(())
}

fn validate_resolved_target(
    resolved: &nomifun_agent_contracts::ResolvedCapability,
    expected: &PluginHostContributionRef,
) -> Result<(), KernelError> {
    if resolved.capability != expected.capability
        || resolved.source_package != expected.target.package
        || resolved.contribution_id != expected.contribution_id
        || resolved.contribution_lock.contract_digest != expected.contract_digest
        || resolved.resolved_mount_id != expected.target.mount_id
        || resolved.target_artifact_digest != expected.target.artifact_digest
    {
        return Err(drift(
            &expected.capability.id,
            "resolved Capability target differs from the immutable JavaScript Package lock",
        ));
    }
    Ok(())
}

struct NodeToolHandler {
    host: Arc<dyn ExtensionHostDemandPort>,
    mount: MountLoadDemand,
    contribution: PluginHostContributionRef,
}

#[async_trait]
impl CapabilityHandler for NodeToolHandler {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        validate_invocation_context(&context, &self.contribution)?;
        let action = context.action_id.clone();
        self.host
            .invoke_demand(
                self.mount.clone(),
                self.contribution.clone(),
                action,
                input,
            )
            .await
            .map_err(|error| KernelError::CapabilityExecution {
                reason: error.to_string(),
            })
    }
}

#[async_trait]
impl CapabilityOperationHandler for NodeToolHandler {
    async fn invoke(
        &self,
        context: CapabilityOperationInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        validate_operation_context(&context.context, &self.contribution)?;
        self.host
            .invoke_demand(
                self.mount.clone(),
                self.contribution.clone(),
                context.action_id,
                input,
            )
            .await
            .map_err(|error| KernelError::CapabilityExecution {
                reason: error.to_string(),
            })
    }
}

struct NodeContextProxy {
    host: Arc<dyn ExtensionHostDemandPort>,
    mount: MountLoadDemand,
    contribution: PluginHostContributionRef,
}

#[async_trait]
impl CapabilityContextContributionFactory for NodeContextProxy {
    async fn contribute(
        &self,
        request: CapabilityContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        validate_capability_context(&request.context, &self.contribution)?;
        let value = self
            .host
            .contribute_context_demand(
                self.mount.clone(),
                self.contribution.clone(),
                request.schema_ref,
            )
            .await
            .map_err(|error| KernelError::CapabilityExecution {
                reason: error.to_string(),
            })?;
        Ok(ContextContributionResult {
            value: (!value.0.is_null()).then_some(value),
        })
    }
}

struct NodeResourceProxy {
    host: Arc<dyn ExtensionHostDemandPort>,
    mount: MountLoadDemand,
    contribution: PluginHostContributionRef,
}

#[async_trait]
impl CapabilityResourceProviderFactory for NodeResourceProxy {
    async fn acquire(
        &self,
        request: CapabilityResourceProviderRequest,
    ) -> Result<ResourceProviderResult, KernelError> {
        validate_capability_context(&request.context, &self.contribution)?;
        let [binding] = request.context.resource_bindings.as_slice() else {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "ResourceProvider {} requires exactly one typed resource binding",
                    self.contribution.capability.id.as_ref()
                ),
            });
        };
        let parameters = StrictJsonValue(
            serde_json::to_value(&binding.typed_parameters).map_err(|error| {
                KernelError::CapabilityExecution {
                    reason: error.to_string(),
                }
            })?,
        );
        let handle = self
            .host
            .acquire_resource_demand(
                self.mount.clone(),
                self.contribution.clone(),
                binding.binding_id.clone(),
                binding.resource_kind.clone(),
                parameters,
            )
            .await
            .map_err(|error| KernelError::CapabilityExecution {
                reason: error.to_string(),
            })?;
        Ok(ResourceProviderResult {
            handle: Arc::new(NodeResourceHandle {
                identity: ResourceHandleIdentity {
                    binding_id: binding.binding_id.clone(),
                    resource_kind: binding.resource_kind.clone(),
                    resource_id: binding.resource_id.clone(),
                },
                host: Arc::clone(&self.host),
                handle,
            }),
        })
    }
}

#[derive(Clone)]
struct NodeResourceHandle {
    identity: ResourceHandleIdentity,
    host: Arc<dyn ExtensionHostDemandPort>,
    handle: JavaScriptResourceHandle,
}

#[async_trait]
impl ResourceHandle for NodeResourceHandle {
    fn identity(&self) -> &ResourceHandleIdentity {
        &self.identity
    }

    async fn release(&self) -> Result<(), KernelError> {
        self.host
            .release_resource(&self.handle)
            .await
            .map_err(|error| KernelError::CapabilityExecution {
                reason: error.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{ResourceBindingId, ResourceId, ResourceKind};

    #[test]
    fn node_surface_is_explicitly_rejected() {
        let error = JsKernelAdapterError::UnsupportedNodeSurface("test".into());
        assert!(error.to_string().contains("Role Provider"));
    }

    #[test]
    fn resource_handle_identity_is_not_reconstructed() {
        let identity = ResourceHandleIdentity {
            binding_id: ResourceBindingId::from("binding"),
            resource_kind: ResourceKind::from("fixture.resource"),
            resource_id: ResourceId::from("resource"),
        };
        assert_eq!(identity.binding_id.as_ref(), "binding");
    }
}
