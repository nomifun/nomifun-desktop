//! Shared construction and validation helpers for the C7 bundled domain waves.
//!
//! Domain crates use this module to build the same vendor-neutral
//! `PackageManifest`/`PluginRegistration` shape.  The helper deliberately
//! exposes no application service bag, database pool, legacy router, runtime
//! selector, or approval state.  A domain implementation can therefore add
//! real typed ports later without changing the registration boundary.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, ArtifactEnvelope, CapabilityActionDescriptor, CapabilityContributions,
    CapabilityId, CapabilityKind, CapabilityManifest, CancellationDescriptor,
    CanonicalSchemaRef, DigestHex, EffectClass, HostPortId, HostPortRef,
    InProcessEntrypointMetadata, LocalizedMetadata, ManagedTaskRegistrationDescriptor,
    PackageContributions, PackageId, PackageManifest, PackageRef, PlatformConstraint,
    PluginBootCriticality, PluginBootState, PluginContextDescriptor, PluginDesiredState,
    PluginEffectiveState, PluginIdentityDescriptor, PluginMountId, PluginRegistrarDescriptor,
    PluginRegistrarOperation, PluginRegistrationMetadata, PluginSourceKind,
    PluginSourceMetadata, PluginStateHandleDescriptor, PluginStateMethod, ResourceKind, ScopeKey,
    RuntimeTarget, SkillDefinition, StrictJsonValue, ToolPresentationKind,
    TypedResourceBindings, ValidatedPluginConfig, VersionString, digest_payload,
};
use nomifun_agent_kernel::{
    CapabilityHandler, CapabilityInvocationContext, KernelError, PluginRegistration,
};
use serde_json::json;
use thiserror::Error;

pub const CONTRACT_VERSION: &str = "1.0.0";

#[derive(Debug, Error)]
pub enum DomainRegistrationError {
    #[error("domain package {package_id:?} declares duplicate capability {capability_id:?}")]
    DuplicateCapability {
        package_id: PackageId,
        capability_id: CapabilityId,
    },
    #[error("domain package {package_id:?} declares no capabilities")]
    EmptyPackage { package_id: PackageId },
    #[error("invalid domain registration for {package_id:?}: {reason}")]
    Invalid { package_id: PackageId, reason: String },
    #[error("canonical digest failed: {0}")]
    Digest(String),
    #[error("kernel registration failed: {0}")]
    Kernel(#[from] KernelError),
}

#[derive(Clone, Copy, Debug)]
pub struct CapabilitySpec {
    pub id: &'static str,
    pub kind: CapabilityKind,
    pub effect_class: Option<EffectClass>,
    pub resource_kinds: &'static [&'static str],
    pub presentation: ToolPresentationKind,
    pub host_targets: &'static [&'static str],
    pub host_surfaces: &'static [&'static str],
}

impl CapabilitySpec {
    pub const fn context(id: &'static str) -> Self {
        Self {
            id,
            kind: CapabilityKind::ContextContributor,
            effect_class: None,
            resource_kinds: &[],
            presentation: ToolPresentationKind::Hidden,
            host_targets: &[],
            host_surfaces: &[],
        }
    }

    pub const fn tool(
        id: &'static str,
        effect_class: EffectClass,
        resource_kinds: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            kind: CapabilityKind::Tool,
            effect_class: Some(effect_class),
            resource_kinds,
            presentation: ToolPresentationKind::FunctionTool,
            host_targets: &[],
            host_surfaces: &[],
        }
    }

    pub const fn resource_provider(
        id: &'static str,
        resource_kinds: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            kind: CapabilityKind::ResourceProvider,
            effect_class: None,
            resource_kinds,
            presentation: ToolPresentationKind::Hidden,
            host_targets: &[],
            host_surfaces: &[],
        }
    }

    pub const fn scheduler(id: &'static str) -> Self {
        Self {
            id,
            kind: CapabilityKind::Scheduler,
            effect_class: None,
            resource_kinds: &[],
            presentation: ToolPresentationKind::Hidden,
            host_targets: &[],
            host_surfaces: &[],
        }
    }

    pub const fn middleware(id: &'static str) -> Self {
        Self {
            id,
            kind: CapabilityKind::TurnMiddleware,
            effect_class: None,
            resource_kinds: &[],
            presentation: ToolPresentationKind::Hidden,
            host_targets: &[],
            host_surfaces: &[],
        }
    }

    pub const fn transport(id: &'static str) -> Self {
        Self {
            id,
            kind: CapabilityKind::Transport,
            effect_class: None,
            resource_kinds: &[],
            presentation: ToolPresentationKind::Hidden,
            host_targets: &[],
            host_surfaces: &[],
        }
    }

    pub const fn background(id: &'static str) -> Self {
        Self {
            id,
            kind: CapabilityKind::BackgroundService,
            effect_class: None,
            resource_kinds: &[],
            presentation: ToolPresentationKind::Hidden,
            host_targets: &[],
            host_surfaces: &[],
        }
    }

    pub const fn event_source(id: &'static str) -> Self {
        Self {
            id,
            kind: CapabilityKind::EventSource,
            effect_class: None,
            resource_kinds: &[],
            presentation: ToolPresentationKind::Hidden,
            host_targets: &[],
            host_surfaces: &[],
        }
    }

    pub const fn event_consumer(id: &'static str) -> Self {
        Self {
            id,
            kind: CapabilityKind::EventConsumer,
            effect_class: None,
            resource_kinds: &[],
            presentation: ToolPresentationKind::Hidden,
            host_targets: &[],
            host_surfaces: &[],
        }
    }

    /// Mark a capability as available only on an explicit host target/surface
    /// set.  An empty set means the capability is platform-neutral.
    pub const fn on_hosts(
        mut self,
        host_targets: &'static [&'static str],
        host_surfaces: &'static [&'static str],
    ) -> Self {
        self.host_targets = host_targets;
        self.host_surfaces = host_surfaces;
        self
    }

    fn has_action(&self) -> bool {
        self.kind == CapabilityKind::Tool
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PackageSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub mount_id: &'static str,
    pub capabilities: &'static [CapabilitySpec],
    pub supported_surfaces: &'static [&'static str],
}

#[derive(Clone)]
struct DeclarativeCapabilityHandler {
    capability_id: CapabilityId,
    action_id: ActionId,
}

#[async_trait]
impl CapabilityHandler for DeclarativeCapabilityHandler {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        // This is intentionally a deterministic host boundary, not a fake
        // domain implementation. Real domain crates replace this handler with
        // their typed resource/port implementation while preserving the same
        // registration metadata and invocation contract.
        Ok(StrictJsonValue(json!({
            "accepted": true,
            "capability_id": self.capability_id,
            "action_id": self.action_id,
            "registry_generation": context.registry_generation,
            "resource_binding_ids": context
                .resource_bindings
                .iter()
                .map(|binding| binding.binding_id.as_ref())
                .collect::<Vec<_>>(),
            "input": input.0,
        })))
    }
}

pub fn registration(spec: PackageSpec) -> Result<PluginRegistration, DomainRegistrationError> {
    let package_id = PackageId::from(spec.id);
    if spec.capabilities.is_empty() {
        return Err(DomainRegistrationError::EmptyPackage { package_id });
    }

    let package_ref = PackageRef {
        id: package_id.clone(),
        version: VersionString::from(CONTRACT_VERSION),
    };
    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::Bundled,
        source_identity: spec.id.to_owned(),
        source_digest: None,
    };
    let identity = PluginIdentityDescriptor {
        package: package_ref.clone(),
        mount_id: PluginMountId::from(spec.mount_id),
    };
    let config_schema = StrictJsonValue(json!({
        "type": "object",
        "additionalProperties": false,
    }));

    let mut seen = BTreeSet::new();
    let mut capability_manifests = Vec::with_capacity(spec.capabilities.len());
    let mut handler_specs = Vec::new();

    for capability in spec.capabilities {
        let capability_id = CapabilityId::from(capability.id);
        if !seen.insert(capability_id.clone()) {
            return Err(DomainRegistrationError::DuplicateCapability {
                package_id: package_id.clone(),
                capability_id,
            });
        }

        let input_schema = json!({
            "type": "object",
            "additionalProperties": true,
        });
        let output_schema = json!({
            "type": "object",
            "additionalProperties": true,
        });
        let input_digest = digest_payload(&input_schema).map_err(digest_error)?;
        let output_digest = digest_payload(&output_schema).map_err(digest_error)?;
        let actions = match (capability.has_action(), capability.effect_class) {
            (true, Some(effect_class)) => {
                let action_id = ActionId::from(format!("{}.invoke", capability.id));
                handler_specs.push((capability_id.clone(), action_id.clone()));
                vec![CapabilityActionDescriptor {
                    action_id,
                    input_schema: schema_ref(capability.id, "input", &input_digest),
                    output_schema: schema_ref(capability.id, "output", &output_digest),
                    effect_class,
                    presentation: capability.presentation,
                }]
            }
            (false, None) => Vec::new(),
            _ => {
                return Err(DomainRegistrationError::Invalid {
                    package_id: package_id.clone(),
                    reason: format!(
                        "capability {} must have an effect class iff it is a Tool",
                        capability.id
                    ),
                });
            }
        };

        let supported_surfaces = if capability.host_surfaces.is_empty() {
            spec.supported_surfaces
                .iter()
                .map(|surface| (*surface).to_owned())
                .collect()
        } else {
            capability
                .host_surfaces
                .iter()
                .map(|surface| (*surface).to_owned())
                .collect()
        };
        let supported_platforms = if capability.host_targets.is_empty() {
            vec![PlatformConstraint::Any]
        } else {
            vec![PlatformConstraint::Targets {
                host_targets: capability
                    .host_targets
                    .iter()
                    .map(|target| (*target).to_owned().into())
                    .collect(),
                host_surfaces: capability
                    .host_surfaces
                    .iter()
                    .map(|surface| (*surface).to_owned())
                    .collect(),
            }]
        };
        let resource_kinds = capability
            .resource_kinds
            .iter()
            .map(|kind| ResourceKind::from(*kind))
            .collect::<BTreeSet<_>>();
        capability_manifests.push(CapabilityManifest {
            id: capability_id,
            version: VersionString::from(CONTRACT_VERSION),
            kind: capability.kind,
            package: package_ref.clone(),
            display: localized(capability.id, capability.id),
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces,
            requires_runtime_features: Vec::new(),
            supported_platforms,
            config_schema: StrictJsonValue(json!({
                "type": "object",
                "additionalProperties": false,
            })),
            contributions: CapabilityContributions {
                actions,
                context_schema_refs: Vec::new(),
                event_schema_refs: Vec::new(),
                resource_kinds,
                host_ports: Vec::new(),
            },
        });
    }

    let manifest = PackageManifest {
        schema_version: VersionString::from(CONTRACT_VERSION),
        host_contract_version: VersionString::from(CONTRACT_VERSION),
        package_id: package_id.clone(),
        package_version: VersionString::from(CONTRACT_VERSION),
        display: localized(spec.display_name, spec.description),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: format!("{}.entrypoint", spec.id),
            contract_version: VersionString::from(CONTRACT_VERSION),
        },
        contributions: PackageContributions {
            capabilities: capability_manifests,
            skills: Vec::<SkillDefinition>::new(),
            mcp_tools: Vec::new(),
            role_contracts: Vec::new(),
            role_providers: Vec::new(),
        },
    };
    let manifest_artifact = ArtifactEnvelope::new(manifest).map_err(digest_error)?;
    let cancellation_port = host_port(&format!("{}.cancel", spec.id));
    let task_port = host_port(&format!("{}.tasks", spec.id));
    let state = PluginStateHandleDescriptor {
        package_id: package_id.clone(),
        mount_id: identity.mount_id.clone(),
        methods: PluginStateMethod::REQUIRED.into_iter().collect(),
    };
    let metadata = PluginRegistrationMetadata {
        manifest: manifest_artifact,
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
            declared_capability_ids: seen,
            declared_skill_ids: BTreeSet::new(),
            declared_mcp_tool_keys: BTreeSet::new(),
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
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema).map_err(digest_error)?,
                config_revision: 1,
                value: StrictJsonValue(json!({})),
            },
            state,
            declared_services: Default::default(),
            host_ports: Vec::new(),
            typed_command_ports: Vec::new(),
            domain_outbox_ports: Vec::new(),
            cancellation: CancellationDescriptor {
                cancellation_port,
                scope_key: ScopeKey::from(format!("mount:{}", spec.mount_id)),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: task_port,
                scope_key: ScopeKey::from(format!("mount:{}", spec.mount_id)),
            },
        },
    };

    let mut registration = PluginRegistration::new(metadata);
    for (capability_id, action_id) in handler_specs {
        registration.add_capability_handler(
            capability_id.clone(),
            Arc::new(DeclarativeCapabilityHandler {
                capability_id,
                action_id,
            }),
        )?;
    }
    Ok(registration)
}

pub fn registrations(
    specs: impl IntoIterator<Item = PackageSpec>,
) -> Result<Vec<PluginRegistration>, DomainRegistrationError> {
    let mut result = Vec::new();
    let mut packages = BTreeSet::new();
    let mut capabilities = BTreeMap::<CapabilityId, PackageId>::new();
    for spec in specs {
        if !packages.insert(spec.id) {
            return Err(DomainRegistrationError::Invalid {
                package_id: PackageId::from(spec.id),
                reason: "duplicate package id".to_owned(),
            });
        }
        let registration = registration(spec)?;
        for capability in &registration.metadata.manifest.payload.contributions.capabilities {
            if let Some(previous) =
                capabilities.insert(capability.id.clone(), PackageId::from(spec.id))
            {
                return Err(DomainRegistrationError::Invalid {
                    package_id: PackageId::from(spec.id),
                    reason: format!(
                        "capability {} is already owned by {}",
                        capability.id.as_ref(),
                        previous.as_ref()
                    ),
                });
            }
        }
        result.push(registration);
    }
    Ok(result)
}

pub fn validate_inventory(
    registrations: &[PluginRegistration],
) -> Result<(), DomainRegistrationError> {
    let mut packages = BTreeSet::new();
    let mut mounts = BTreeSet::new();
    let mut capabilities = BTreeSet::new();
    for registration in registrations {
        let metadata = &registration.metadata;
        let manifest = &metadata.manifest.payload;
        if !metadata.manifest.verify().map_err(digest_error)? {
            return Err(invalid_registration(
                &manifest.package_id,
                "package manifest digest verification failed",
            ));
        }
        let package_ref = PackageRef {
            id: manifest.package_id.clone(),
            version: manifest.package_version.clone(),
        };
        if metadata.source.source_identity.trim().is_empty()
            || metadata.source.source_identity != manifest.package_id.as_ref()
            || metadata.context.source != metadata.source
        {
            return Err(invalid_registration(
                &manifest.package_id,
                "source metadata does not match the manifest package identity and context",
            ));
        }
        if metadata.mount_id != metadata.registrar.identity.mount_id
            || metadata.mount_id != metadata.context.identity.mount_id
            || metadata.registrar.identity.package != package_ref
            || metadata.context.identity.package != package_ref
            || metadata.context.state.package_id != manifest.package_id
            || metadata.context.state.mount_id != metadata.mount_id
            || metadata.context.state.methods
                != PluginStateMethod::REQUIRED.into_iter().collect()
        {
            return Err(invalid_registration(
                &manifest.package_id,
                "package identity, mount, or mandatory state metadata drifted",
            ));
        }
        let expected_config_digest =
            digest_payload(&manifest.config_schema).map_err(digest_error)?;
        if metadata.context.validated_config.schema_digest != expected_config_digest {
            return Err(invalid_registration(
                &manifest.package_id,
                "validated config schema digest does not match the manifest",
            ));
        }
        if !packages.insert(manifest.package_id.clone()) {
            return Err(invalid_registration(
                &manifest.package_id,
                "duplicate package in registration inventory",
            ));
        }
        if !mounts.insert(metadata.mount_id.clone()) {
            return Err(invalid_registration(
                &manifest.package_id,
                "duplicate plugin mount in registration inventory",
            ));
        }
        let mut expected_handlers = BTreeSet::new();
        let mut manifest_capability_ids = BTreeSet::new();
        let mut manifest_skill_ids = BTreeSet::new();
        let mut manifest_mcp_tool_keys = BTreeSet::new();
        let mut manifest_role_ids = BTreeSet::new();
        let mut manifest_service_ids = BTreeSet::new();
        let role_member_ids = manifest
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
        for capability in &manifest.contributions.capabilities {
            if !capabilities.insert(capability.id.clone()) {
                return Err(invalid_registration(
                    &manifest.package_id,
                    format!(
                        "duplicate capability {} in registration inventory",
                        capability.id.as_ref()
                    ),
                ));
            }
            if !manifest_capability_ids.insert(capability.id.clone())
                || capability.package != package_ref
            {
                return Err(invalid_registration(
                    &manifest.package_id,
                    format!(
                        "capability {} does not belong to the package manifest",
                        capability.id.as_ref()
                    ),
                ));
            }
            let action_ids = capability
                .contributions
                .actions
                .iter()
                .map(|action| action.action_id.clone())
                .collect::<BTreeSet<_>>();
            if action_ids.len() != capability.contributions.actions.len() {
                return Err(invalid_registration(
                    &manifest.package_id,
                    format!(
                        "capability {} declares duplicate action IDs",
                        capability.id.as_ref()
                    ),
                ));
            }
            match capability.kind {
                CapabilityKind::Tool if capability.contributions.actions.len() == 1 => {
                    if !role_member_ids.contains(&capability.id) {
                        expected_handlers.insert(capability.id.clone());
                    }
                }
                CapabilityKind::Tool => {
                    return Err(invalid_registration(
                        &manifest.package_id,
                        format!(
                            "tool capability {} must declare exactly one action",
                            capability.id.as_ref()
                        ),
                    ));
                }
                _ if capability.contributions.actions.is_empty() => {}
                _ => {
                    return Err(invalid_registration(
                        &manifest.package_id,
                        format!(
                            "non-tool capability {} declares an action",
                            capability.id.as_ref()
                        ),
                    ));
                }
            }
        }
        for contract in &manifest.contributions.role_contracts {
            if !manifest_role_ids.insert(contract.key.role_id.clone()) {
                return Err(invalid_registration(
                    &manifest.package_id,
                    "execution-role contract declarations are duplicated",
                ));
            }
            if contract.members.is_empty() {
                return Err(invalid_registration(
                    &manifest.package_id,
                    "execution-role contract must contain members",
                ));
            }
            for member in &contract.members {
                if !manifest
                    .contributions
                    .capabilities
                    .iter()
                    .any(|capability| capability.id == member.capability.id)
                {
                    return Err(invalid_registration(
                        &manifest.package_id,
                        format!(
                            "role member {} is not declared by the package",
                            member.capability.id.as_ref()
                        ),
                    ));
                }
            }
        }
        for provider in &manifest.contributions.role_providers {
            if !manifest_role_ids.contains(&provider.role.key.role_id) {
                return Err(invalid_registration(
                    &manifest.package_id,
                    "role provider references an undeclared role contract",
                ));
            }
        }
        let provider_role_ids = manifest
            .contributions
            .role_providers
            .iter()
            .map(|provider| provider.role.key.role_id.clone())
            .collect::<BTreeSet<_>>();
        if provider_role_ids != manifest_role_ids {
            return Err(invalid_registration(
                &manifest.package_id,
                "role contracts and role providers must have the same role identity set",
            ));
        }
        for skill in &manifest.contributions.skills {
            if !manifest_skill_ids.insert(skill.id.clone()) || skill.package != package_ref {
                return Err(invalid_registration(
                    &manifest.package_id,
                    format!(
                        "skill {} does not belong to the package manifest",
                        skill.id.as_ref()
                    ),
                ));
            }
        }
        for mapping in &manifest.contributions.mcp_tools {
            if !manifest_mcp_tool_keys.insert(mapping.canonical_tool_key.clone())
                || mapping.package != package_ref
                || !manifest
                    .contributions
                    .capabilities
                    .iter()
                    .any(|capability| {
                        capability.id == mapping.capability.id
                            && capability.version == mapping.capability.version
                    })
            {
                return Err(invalid_registration(
                    &manifest.package_id,
                    format!(
                        "MCP mapping {} does not belong to the package manifest",
                        mapping.canonical_tool_key.as_ref()
                    ),
                ));
            }
        }
        for provision in &manifest.provides_services {
            if !manifest_service_ids.insert(provision.service.id.clone()) {
                return Err(invalid_registration(
                    &manifest.package_id,
                    "provided service declarations are duplicated",
                ));
            }
        }
        let declared_host_ports = declared_host_ports(metadata);
        let expected_registrar_operations = required_registrar_operations(
            !manifest_capability_ids.is_empty(),
            !manifest_skill_ids.is_empty(),
            !manifest_mcp_tool_keys.is_empty(),
            !manifest_role_ids.is_empty(),
            !manifest_service_ids.is_empty(),
            !declared_host_ports.is_empty(),
        );
        if metadata.registrar.declared_capability_ids != manifest_capability_ids
            || metadata.registrar.declared_skill_ids != manifest_skill_ids
            || metadata.registrar.declared_mcp_tool_keys != manifest_mcp_tool_keys
            || metadata.registrar.declared_role_ids != manifest_role_ids
            || metadata.registrar.declared_service_keys != manifest_service_ids
            || metadata.registrar.declared_host_ports != declared_host_ports
            || metadata.registrar.allowed_operations != expected_registrar_operations
        {
            return Err(invalid_registration(
                &manifest.package_id,
                "registrar declarations do not match manifest and context",
            ));
        }
        let actual_role_handlers = registration
            .role_action_handler_ids()
            .into_iter()
            .map(|(_, capability_id)| capability_id)
            .collect::<BTreeSet<_>>();
        let expected_role_handlers = role_member_ids
            .iter()
            .filter(|capability_id| {
                manifest
                    .contributions
                    .capabilities
                    .iter()
                    .any(|capability| {
                        &capability.id == *capability_id
                            && !capability.contributions.actions.is_empty()
                    })
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        if registration.handler_ids() != expected_handlers
            || actual_role_handlers != expected_role_handlers
        {
            return Err(invalid_registration(
                &manifest.package_id,
                "capability handler exports differ from Tool actions",
            ));
        }
    }
    Ok(())
}

fn declared_host_ports(metadata: &PluginRegistrationMetadata) -> BTreeSet<HostPortId> {
    metadata
        .context
        .host_ports
        .iter()
        .map(|binding| binding.port.id.clone())
        .chain(
            metadata
                .context
                .typed_command_ports
                .iter()
                .map(|binding| binding.port.id.clone()),
        )
        .chain(
            metadata
                .context
                .domain_outbox_ports
                .iter()
                .map(|binding| binding.port.id.clone()),
        )
        .chain([metadata.context.cancellation.cancellation_port.id.clone()])
        .chain([metadata.context.managed_task_registration.registrar_port.id.clone()])
        .collect()
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

pub fn typed_resource_bindings_for<'a>(
    owner_id: &str,
    entries: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a [&'a str])>,
) -> TypedResourceBindings {
    entries
        .into_iter()
        .map(|(binding_id, resource_kind, resource_id, operations)| {
            nomifun_agent_contracts::TypedResourceBinding {
                binding_id: binding_id.into(),
                resource_kind: resource_kind.into(),
                resource_id: resource_id.into(),
                owner_id: owner_id.to_owned(),
                operations: operations.iter().map(|operation| (*operation).to_owned()).collect(),
                connection_config_ref: None,
                typed_parameters: BTreeMap::new(),
            }
        })
        .collect()
}

fn localized(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

fn host_port(id: &str) -> HostPortRef {
    HostPortRef {
        id: HostPortId::from(id.to_owned()),
        version: VersionString::from(CONTRACT_VERSION),
    }
}

fn schema_ref(capability: &str, direction: &str, digest: &DigestHex) -> CanonicalSchemaRef {
    CanonicalSchemaRef::from(format!(
        "schema://{capability}/{direction}@1#{}",
        digest.as_ref()
    ))
}

fn digest_error(error: impl std::fmt::Display) -> DomainRegistrationError {
    DomainRegistrationError::Digest(error.to_string())
}

fn invalid_registration(
    package_id: &PackageId,
    reason: impl Into<String>,
) -> DomainRegistrationError {
    DomainRegistrationError::Invalid {
        package_id: package_id.clone(),
        reason: reason.into(),
    }
}

/// A small helper for callers that need to make an explicit, typed
/// availability decision before resolving a capability.
pub fn capability_available_on_host(
    host_target: &str,
    host_surface: &str,
    capability: &CapabilitySpec,
) -> bool {
    check_platform_availability(
        capability,
        &RuntimeTarget::from(host_target),
        host_surface,
    )
    .is_ok()
}

/// Check a capability against its typed host target and surface contract.
///
/// Empty host identities fail closed with the canonical Kernel error.
/// Surface-only restrictions remain represented by the capability's
/// `supported_surfaces`; `PlatformConstraint::Any` is only used for the
/// absence of a target restriction.
pub fn check_platform_availability(
    capability: &CapabilitySpec,
    host_target: &RuntimeTarget,
    host_surface: &str,
) -> Result<(), KernelError> {
    if host_target.as_ref().trim().is_empty() {
        return Err(KernelError::CapabilityUnavailableOnPlatform {
            capability_id: CapabilityId::from(capability.id),
            target: host_target.as_ref().to_owned(),
            surface: host_surface.to_owned(),
        });
    }
    if host_surface.trim().is_empty() {
        return Err(KernelError::CapabilityUnavailableOnSurface {
            capability_id: CapabilityId::from(capability.id),
            surface: host_surface.to_owned(),
        });
    }
    if !capability.host_targets.is_empty()
        && !capability
            .host_targets
            .iter()
            .any(|target| *target == host_target.as_ref())
    {
        return Err(KernelError::CapabilityUnavailableOnPlatform {
            capability_id: CapabilityId::from(capability.id),
            target: host_target.as_ref().to_owned(),
            surface: host_surface.to_owned(),
        });
    }
    if !capability.host_surfaces.is_empty()
        && !capability
            .host_surfaces
            .iter()
            .any(|surface| *surface == host_surface)
    {
        return Err(KernelError::CapabilityUnavailableOnSurface {
            capability_id: CapabilityId::from(capability.id),
            surface: host_surface.to_owned(),
        });
    }
    Ok(())
}

/// The C7 target inventory, expressed as declarative registration specs.
///
/// This is intentionally kept in the shared support crate as a reviewable
/// inventory table. Owning wave crates may expose narrower slices of it, while
/// the application composition root can validate the complete set without
/// learning domain implementation details.
pub fn c7_package_specs() -> Vec<PackageSpec> {
    vec![
        PackageSpec {
            id: "nomifun.model-media",
            display_name: "Model Media",
            description: "Provider-backed multimodal model capabilities.",
            mount_id: "domain-model-media",
            capabilities: &MODEL_MEDIA_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.web-research",
            display_name: "Web Research",
            description: "Search and fetch web sources for an AgentSession.",
            mount_id: "domain-web-research",
            capabilities: &WEB_RESEARCH_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.chat",
            display_name: "Chat Attachments",
            description: "Read attachment context for a chat Session.",
            mount_id: "domain-chat",
            capabilities: &CHAT_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.agent-execution",
            display_name: "Agent Execution",
            description: "Coordinate bounded AgentSession execution.",
            mount_id: "domain-agent-execution",
            capabilities: &AGENT_EXECUTION_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.workspace-execution",
            display_name: "Workspace Execution",
            description: "Operate on explicitly bound workspaces and processes.",
            mount_id: "domain-workspace-execution",
            capabilities: &WORKSPACE_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.ssh",
            display_name: "SSH",
            description: "Use explicitly bound remote SSH resources.",
            mount_id: "domain-ssh",
            capabilities: &SSH_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.knowledge",
            display_name: "Knowledge",
            description: "Search, read, and write owned knowledge bases.",
            mount_id: "domain-knowledge",
            capabilities: &KNOWLEDGE_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.project-memory",
            display_name: "Project Memory",
            description: "Read and maintain project-scoped memory.",
            mount_id: "domain-project-memory",
            capabilities: &PROJECT_MEMORY_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.companion-memory",
            display_name: "Companion Memory",
            description: "Read and maintain explicitly bound companion memory.",
            mount_id: "domain-companion-memory",
            capabilities: &COMPANION_MEMORY_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.skills",
            display_name: "Skills",
            description: "Expose catalogued skill guidance and hooks.",
            mount_id: "domain-skills",
            capabilities: &SKILL_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.mcp-connectors",
            display_name: "MCP Connectors",
            description: "Materialize MCP tools as canonical capabilities.",
            mount_id: "domain-mcp-connectors",
            capabilities: &MCP_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.browser",
            display_name: "Browser",
            description: "Use the process-wide managed browser resource.",
            mount_id: "domain-browser",
            capabilities: &BROWSER_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.computer-a11y",
            display_name: "Computer Accessibility",
            description: "Use the process-wide desktop accessibility resource.",
            mount_id: "domain-computer-a11y",
            capabilities: &COMPUTER_CAPABILITIES,
            supported_surfaces: &["desktop"],
        },
        PackageSpec {
            id: "nomifun.requirements",
            display_name: "Requirements",
            description: "Read and update owned requirements.",
            mount_id: "domain-requirements",
            capabilities: &REQUIREMENT_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.autowork-scheduler",
            display_name: "AutoWork Scheduler",
            description: "Schedule AgentSession triggers with durable bindings.",
            mount_id: "domain-autowork-scheduler",
            capabilities: &AUTOWORK_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.idmm",
            display_name: "IDMM",
            description: "Observe and steer bounded AgentSession turns.",
            mount_id: "domain-idmm",
            capabilities: &IDMM_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.companion",
            display_name: "Companion",
            description: "Bind persona and companion actions to a Session.",
            mount_id: "domain-companion",
            capabilities: &COMPANION_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.channel",
            display_name: "Channels",
            description: "Receive and send through paired channels.",
            mount_id: "domain-channel",
            capabilities: &CHANNEL_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.customer-service",
            display_name: "Customer Service",
            description: "Handle customer dialogue and owned notes.",
            mount_id: "domain-customer-service",
            capabilities: &CUSTOMER_SERVICE_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.robot",
            display_name: "Robot",
            description: "Connect to paired robot devices and media.",
            mount_id: "domain-robot",
            capabilities: &ROBOT_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.creation",
            display_name: "Creation",
            description: "Create text, image, video, and audio artifacts.",
            mount_id: "domain-creation",
            capabilities: &CREATION_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.workshop",
            display_name: "Creative Workshop",
            description: "Read and edit owned canvases and assets.",
            mount_id: "domain-workshop",
            capabilities: &WORKSHOP_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.office",
            display_name: "Office",
            description: "Preview and edit owned office artifacts.",
            mount_id: "domain-office",
            capabilities: &OFFICE_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.miniapp",
            display_name: "MiniApp",
            description: "Read, edit, publish, and serve owned MiniApps.",
            mount_id: "domain-miniapp",
            capabilities: &MINIAPP_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.notification",
            display_name: "Notifications",
            description: "Consume canonical Session and domain events.",
            mount_id: "domain-notification",
            capabilities: &NOTIFICATION_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
        PackageSpec {
            id: "nomifun.remote-ingress",
            display_name: "Remote Ingress",
            description: "Expose transport-only RemoteBinding ingress.",
            mount_id: "domain-remote-ingress",
            capabilities: &REMOTE_CAPABILITIES,
            supported_surfaces: &["desktop", "headless"],
        },
    ]
}

const WORKSPACE: &[&str] = &["workspace"];
const PROCESS_SESSION: &[&str] = &["process_session"];
const TERMINAL: &[&str] = &["terminal"];
const SSH_HOST: &[&str] = &["ssh_host"];
const BROWSER: &[&str] = &["browser"];
const COMPUTER: &[&str] = &["computer"];
const KNOWLEDGE_BASE: &[&str] = &["knowledge_base"];
const PROJECT_MEMORY: &[&str] = &["project_memory"];
const COMPANION: &[&str] = &["companion"];
const COMPANION_MEMORY: &[&str] = &["companion_memory"];
const CHANNEL: &[&str] = &["channel"];
const CUSTOMER: &[&str] = &["customer"];
const ROBOT: &[&str] = &["robot"];
const CANVAS: &[&str] = &["canvas"];
const ASSET_LIBRARY: &[&str] = &["asset_library"];
const GENERATION_PROVIDER: &[&str] = &["generation_provider"];
const MINIAPP: &[&str] = &["miniapp"];
const MCP_SERVER: &[&str] = &["mcp_server"];

const WEB_RESEARCH_CAPABILITIES: [CapabilitySpec; 3] = [
    CapabilitySpec::tool("web.search", EffectClass::ExternalTransmit, &[]),
    CapabilitySpec::tool("web.fetch", EffectClass::ExternalTransmit, &[]),
    CapabilitySpec::context("citation.render"),
];
const MODEL_MEDIA_CAPABILITIES: [CapabilitySpec; 9] = [
    CapabilitySpec::transport("llm.realtime"),
    CapabilitySpec::tool("llm.embedding", EffectClass::ExternalTransmit, &[]),
    CapabilitySpec::tool("llm.rerank", EffectClass::ExternalTransmit, &[]),
    CapabilitySpec::tool("llm.image.generate", EffectClass::ExternalTransmit, &[]),
    CapabilitySpec::tool("llm.image.edit", EffectClass::ExternalTransmit, &[]),
    CapabilitySpec::tool("llm.video.generate", EffectClass::ExternalTransmit, &[]),
    CapabilitySpec::tool("llm.audio.tts", EffectClass::ExternalTransmit, &[]),
    CapabilitySpec::tool("llm.audio.asr", EffectClass::ExternalTransmit, &[]),
    CapabilitySpec::context("llm.vision"),
];
const CHAT_CAPABILITIES: [CapabilitySpec; 1] = [
    CapabilitySpec::context("session.attachments.read"),
];
const AGENT_EXECUTION_CAPABILITIES: [CapabilitySpec; 5] = [
    CapabilitySpec::tool("agent.delegate", EffectClass::ExecuteLocal, PROCESS_SESSION),
    CapabilitySpec::tool("agent.fork", EffectClass::WriteDurable, &[]),
    CapabilitySpec::tool("agent.execution.plan", EffectClass::WriteDurable, &[]),
    CapabilitySpec::tool("agent.execution.steer", EffectClass::WriteDurable, PROCESS_SESSION),
    CapabilitySpec::tool("agent.execution.observe", EffectClass::ReadLocal, PROCESS_SESSION),
];
const WORKSPACE_CAPABILITIES: [CapabilitySpec; 17] = [
    CapabilitySpec::tool("fs.read", EffectClass::ReadLocal, WORKSPACE),
    CapabilitySpec::tool("fs.search", EffectClass::ReadLocal, WORKSPACE),
    CapabilitySpec::tool("fs.write", EffectClass::WriteDurable, WORKSPACE),
    CapabilitySpec::tool("fs.patch", EffectClass::WriteReversible, WORKSPACE),
    CapabilitySpec::tool("fs.delete", EffectClass::Destructive, WORKSPACE),
    CapabilitySpec::event_source("fs.watch"),
    CapabilitySpec::tool("fs.snapshot", EffectClass::ReadLocal, WORKSPACE),
    CapabilitySpec::resource_provider("workspace.bind", WORKSPACE),
    CapabilitySpec::resource_provider("workspace.artifacts", WORKSPACE),
    CapabilitySpec::tool("vcs.status", EffectClass::ReadLocal, WORKSPACE),
    CapabilitySpec::tool("vcs.diff", EffectClass::ReadLocal, WORKSPACE),
    CapabilitySpec::tool("vcs.stage", EffectClass::WriteReversible, WORKSPACE),
    CapabilitySpec::tool("vcs.commit", EffectClass::WriteDurable, WORKSPACE),
    CapabilitySpec::tool("vcs.push", EffectClass::ExternalTransmit, WORKSPACE),
    CapabilitySpec::tool("process.exec", EffectClass::ExecuteLocal, PROCESS_SESSION),
    CapabilitySpec::resource_provider("process.session", PROCESS_SESSION),
    CapabilitySpec::resource_provider("terminal.pty", TERMINAL),
];
const SSH_CAPABILITIES: [CapabilitySpec; 5] = [
    CapabilitySpec::resource_provider("ssh.connect", SSH_HOST),
    CapabilitySpec::tool("ssh.fs.read", EffectClass::ReadSensitive, SSH_HOST),
    CapabilitySpec::tool("ssh.fs.write", EffectClass::WriteDurable, SSH_HOST),
    CapabilitySpec::tool("ssh.exec", EffectClass::ExecuteLocal, SSH_HOST),
    CapabilitySpec::tool("ssh.sudo", EffectClass::ExecuteLocal, SSH_HOST),
];
const KNOWLEDGE_CAPABILITIES: [CapabilitySpec; 8] = [
    CapabilitySpec::tool("knowledge.search", EffectClass::ReadSensitive, KNOWLEDGE_BASE),
    CapabilitySpec::tool("knowledge.read", EffectClass::ReadSensitive, KNOWLEDGE_BASE),
    CapabilitySpec::tool("knowledge.write", EffectClass::WriteDurable, KNOWLEDGE_BASE),
    CapabilitySpec::resource_provider("knowledge.mount", KNOWLEDGE_BASE),
    CapabilitySpec::background("knowledge.source.sync"),
    CapabilitySpec::tool("knowledge.autogen", EffectClass::WriteDurable, KNOWLEDGE_BASE),
    CapabilitySpec::tool("knowledge.embedding", EffectClass::ReadSensitive, KNOWLEDGE_BASE),
    CapabilitySpec::tool("knowledge.rerank", EffectClass::ReadSensitive, KNOWLEDGE_BASE),
];
const PROJECT_MEMORY_CAPABILITIES: [CapabilitySpec; 5] = [
    CapabilitySpec::context("memory.project.read"),
    CapabilitySpec::tool("memory.project.write", EffectClass::WriteDurable, PROJECT_MEMORY),
    CapabilitySpec::tool("memory.project.distill", EffectClass::WriteDurable, PROJECT_MEMORY),
    CapabilitySpec::context("memory.project.citation"),
    CapabilitySpec::resource_provider("memory.session.scratch", PROJECT_MEMORY),
];
const COMPANION_MEMORY_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec::context("memory.companion.recall"),
    CapabilitySpec::tool("memory.companion.write", EffectClass::WriteDurable, COMPANION_MEMORY),
    CapabilitySpec::tool("memory.companion.merge", EffectClass::WriteDurable, COMPANION_MEMORY),
    CapabilitySpec::tool("memory.companion.evolve", EffectClass::WriteDurable, COMPANION_MEMORY),
];
const SKILL_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec::context("skill.catalog"),
    CapabilitySpec::context("skill.describe"),
    CapabilitySpec::tool("skill.invoke", EffectClass::ExecuteLocal, &[]),
    CapabilitySpec::middleware("skill.hooks"),
];
const MCP_CAPABILITIES: [CapabilitySpec; 6] = [
    CapabilitySpec::transport("mcp.connect"),
    CapabilitySpec::tool("mcp.tool_proxy", EffectClass::ExternalTransmit, MCP_SERVER),
    CapabilitySpec::resource_provider("mcp.resource", MCP_SERVER),
    CapabilitySpec::transport("mcp.oauth"),
    CapabilitySpec::tool("connector.data.read", EffectClass::ReadSensitive, MCP_SERVER),
    CapabilitySpec::tool("connector.data.write", EffectClass::WriteDurable, MCP_SERVER),
];
const BROWSER_CAPABILITIES: [CapabilitySpec; 10] = [
    CapabilitySpec::resource_provider("browser.identity", BROWSER),
    CapabilitySpec::context("browser.observe"),
    CapabilitySpec::tool("browser.navigate", EffectClass::ExternalTransmit, BROWSER),
    CapabilitySpec::tool("browser.act", EffectClass::WriteReversible, BROWSER),
    CapabilitySpec::tool(
        "browser.render_content",
        EffectClass::ExternalTransmit,
        BROWSER,
    ),
    CapabilitySpec::tool("browser.download", EffectClass::WriteDurable, BROWSER),
    CapabilitySpec::tool("browser.upload", EffectClass::ExternalTransmit, BROWSER),
    CapabilitySpec::tool("browser.evaluate", EffectClass::ExecuteLocal, BROWSER),
    CapabilitySpec::context("browser.site_memory"),
    CapabilitySpec::tool("browser.takeover", EffectClass::WriteReversible, BROWSER),
];
const COMPUTER_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec::context("computer.observe"),
    CapabilitySpec::tool("computer.input", EffectClass::Physical, COMPUTER),
    CapabilitySpec::tool("computer.launch", EffectClass::ExecuteLocal, COMPUTER),
    CapabilitySpec::context("a11y.observe"),
];
const REQUIREMENT_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec::tool("requirements.read", EffectClass::ReadSensitive, &[]),
    CapabilitySpec::tool("requirements.write", EffectClass::WriteDurable, &[]),
    CapabilitySpec::tool("requirements.status", EffectClass::ReadLocal, &[]),
    CapabilitySpec::tool("requirements.claim", EffectClass::WriteDurable, &[]),
];
const AUTOWORK_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec::scheduler("autowork.runner"),
    CapabilitySpec::tool("schedule.store", EffectClass::WriteDurable, &[]),
    CapabilitySpec::scheduler("schedule.timer"),
    CapabilitySpec::scheduler("schedule.agent_trigger"),
];
const IDMM_CAPABILITIES: [CapabilitySpec; 3] = [
    CapabilitySpec::middleware("idmm.observe"),
    CapabilitySpec::middleware("idmm.intervene"),
    CapabilitySpec::middleware("idmm.fallback_policy"),
];
const COMPANION_CAPABILITIES: [CapabilitySpec; 5] = [
    CapabilitySpec::context("companion.persona"),
    CapabilitySpec::context("companion.roster"),
    CapabilitySpec::tool("companion.summon", EffectClass::ReadSensitive, COMPANION),
    CapabilitySpec::tool("companion.learn", EffectClass::WriteDurable, COMPANION_MEMORY),
    CapabilitySpec::tool("companion.evolve", EffectClass::WriteDurable, COMPANION_MEMORY),
];
const CHANNEL_CAPABILITIES: [CapabilitySpec; 5] = [
    CapabilitySpec::event_source("channel.receive"),
    CapabilitySpec::tool("channel.reply", EffectClass::ExternalTransmit, CHANNEL),
    CapabilitySpec::tool("channel.send", EffectClass::ExternalTransmit, CHANNEL),
    CapabilitySpec::transport("channel.pairing"),
    CapabilitySpec::middleware("channel.group_policy"),
];
const CUSTOMER_SERVICE_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec::middleware("customer_service.dialogue"),
    CapabilitySpec::tool("customer_service.notes.read", EffectClass::ReadSensitive, CUSTOMER),
    CapabilitySpec::tool("customer_service.notes.write", EffectClass::WriteDurable, CUSTOMER),
    CapabilitySpec::tool("customer_service.handoff", EffectClass::ExternalTransmit, CUSTOMER),
];
const ROBOT_CAPABILITIES: [CapabilitySpec; 6] = [
    CapabilitySpec::resource_provider("robot.link", ROBOT),
    CapabilitySpec::background("robot.audio"),
    CapabilitySpec::context("robot.vision"),
    CapabilitySpec::tool("robot.display", EffectClass::Physical, ROBOT),
    CapabilitySpec::tool("robot.motion", EffectClass::Physical, ROBOT),
    CapabilitySpec::tool("robot.device_tools", EffectClass::Physical, ROBOT),
];
const CREATION_CAPABILITIES: [CapabilitySpec; 5] = [
    CapabilitySpec::tool("creation.text", EffectClass::WriteDurable, GENERATION_PROVIDER),
    CapabilitySpec::tool("creation.image", EffectClass::WriteDurable, GENERATION_PROVIDER),
    CapabilitySpec::tool("creation.image_edit", EffectClass::WriteDurable, GENERATION_PROVIDER),
    CapabilitySpec::tool("creation.video", EffectClass::WriteDurable, GENERATION_PROVIDER),
    CapabilitySpec::tool("creation.audio", EffectClass::WriteDurable, GENERATION_PROVIDER),
];
const WORKSHOP_CAPABILITIES: [CapabilitySpec; 6] = [
    CapabilitySpec::tool("workshop.canvas.read", EffectClass::ReadSensitive, CANVAS),
    CapabilitySpec::tool("workshop.canvas.edit", EffectClass::WriteReversible, CANVAS),
    CapabilitySpec::tool("workshop.asset.read", EffectClass::ReadSensitive, ASSET_LIBRARY),
    CapabilitySpec::tool("workshop.asset.write", EffectClass::WriteDurable, ASSET_LIBRARY),
    CapabilitySpec::tool("workshop.template.run", EffectClass::ExecuteLocal, CANVAS),
    CapabilitySpec::tool("workshop.director", EffectClass::WriteDurable, CANVAS),
];
const OFFICE_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec::tool("office.preview", EffectClass::ReadSensitive, ASSET_LIBRARY),
    CapabilitySpec::tool("office.document.edit", EffectClass::WriteReversible, ASSET_LIBRARY),
    CapabilitySpec::tool("office.sheet.edit", EffectClass::WriteReversible, ASSET_LIBRARY),
    CapabilitySpec::tool("office.slides.edit", EffectClass::WriteReversible, ASSET_LIBRARY),
];
const MINIAPP_CAPABILITIES: [CapabilitySpec; 4] = [
    CapabilitySpec::tool("miniapp.read", EffectClass::ReadSensitive, MINIAPP),
    CapabilitySpec::tool("miniapp.edit", EffectClass::WriteReversible, MINIAPP),
    CapabilitySpec::tool("miniapp.publish", EffectClass::ExternalTransmit, MINIAPP),
    CapabilitySpec::tool("miniapp.serve", EffectClass::ExternalTransmit, MINIAPP),
];
const NOTIFICATION_CAPABILITIES: [CapabilitySpec; 2] = [
    CapabilitySpec::event_consumer("notification.webhook"),
    CapabilitySpec::event_consumer("notification.desktop"),
];
const REMOTE_CAPABILITIES: [CapabilitySpec; 5] = [
    CapabilitySpec::transport("remote.mcp"),
    CapabilitySpec::transport("remote.rest"),
    CapabilitySpec::transport("ingress.web"),
    CapabilitySpec::transport("ingress.mobile"),
    CapabilitySpec::transport("ingress.channel"),
];

#[cfg(test)]
mod tests {
    use super::*;

    const CAPS: &[CapabilitySpec] = &[
        CapabilitySpec::tool("support.read", EffectClass::ReadLocal, &["workspace"]),
        CapabilitySpec::context("support.context"),
    ];

    #[test]
    fn declarative_registration_has_exact_handler_and_metadata_sets() {
        let registrations = registrations([PackageSpec {
            id: "domain.support",
            display_name: "Support",
            description: "Test support package",
            mount_id: "domain-support",
            capabilities: CAPS,
            supported_surfaces: &["desktop"],
        }])
        .unwrap();
        validate_inventory(&registrations).unwrap();
        let registration = &registrations[0];
        assert_eq!(registration.handler_ids().len(), 1);
        assert_eq!(
            registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities
                .len(),
            2
        );
    }

    #[test]
    fn non_tool_contributions_never_publish_actions_or_handlers() {
        let registrations = registrations(c7_package_specs()).unwrap();
        let mut tool_count = 0;
        for registration in &registrations {
            for capability in &registration
                .metadata
                .manifest
                .payload
                .contributions
                .capabilities
            {
                if capability.kind == CapabilityKind::Tool {
                    tool_count += 1;
                    assert_eq!(capability.contributions.actions.len(), 1);
                    assert!(registration.handler_ids().contains(&capability.id));
                } else {
                    assert!(capability.contributions.actions.is_empty());
                    assert!(!registration.handler_ids().contains(&capability.id));
                }
            }
        }
        assert_eq!(
            registrations
                .iter()
                .flat_map(|registration| registration.handler_ids())
                .count(),
            tool_count
        );
    }

    #[test]
    fn host_constraints_are_typed_and_fail_closed() {
        let capability = CapabilitySpec::tool(
            "support.windows",
            EffectClass::ExecuteLocal,
            &[],
        )
        .on_hosts(&["x86_64-pc-windows-msvc"], &["desktop"]);
        assert!(capability_available_on_host(
            "x86_64-pc-windows-msvc",
            "desktop",
            &capability
        ));
        assert!(!capability_available_on_host(
            "x86_64-unknown-linux-gnu",
            "headless",
            &capability
        ));
        assert!(matches!(
            check_platform_availability(
                &capability,
                &RuntimeTarget::from("x86_64-unknown-linux-gnu"),
                "headless",
            ),
            Err(KernelError::CapabilityUnavailableOnPlatform { .. })
        ));

        let surface_only =
            CapabilitySpec::context("support.desktop-only").on_hosts(&[], &["desktop"]);
        assert!(check_platform_availability(
            &surface_only,
            &RuntimeTarget::from("x86_64-unknown-linux-gnu"),
            "desktop",
        )
        .is_ok());
        assert!(matches!(
            check_platform_availability(
                &surface_only,
                &RuntimeTarget::from("x86_64-unknown-linux-gnu"),
                "headless",
            ),
            Err(KernelError::CapabilityUnavailableOnSurface { .. })
        ));
        assert!(!capability_available_on_host("", "desktop", &surface_only));
    }

    #[test]
    fn canonical_platform_inventory_matches_release_boundaries() {
        let specs = c7_package_specs();
        let browser = specs
            .iter()
            .find(|spec| spec.id == "nomifun.browser")
            .unwrap();
        let browser_capability = browser
            .capabilities
            .iter()
            .find(|capability| capability.id == "browser.navigate")
            .unwrap();
        assert!(capability_available_on_host(
            "x86_64-unknown-linux-gnu",
            "desktop",
            browser_capability,
        ));
        assert!(!capability_available_on_host(
            "x86_64-unknown-linux-gnu",
            "headless",
            browser_capability,
        ));

        let computer = specs
            .iter()
            .find(|spec| spec.id == "nomifun.computer-a11y")
            .unwrap();
        let computer_capability = computer
            .capabilities
            .iter()
            .find(|capability| capability.id == "computer.input")
            .unwrap();
        assert!(!capability_available_on_host(
            "x86_64-unknown-linux-gnu",
            "desktop",
            computer_capability,
        ));
        assert!(capability_available_on_host(
            "x86_64-pc-windows-msvc",
            "desktop",
            computer_capability,
        ));
    }

    #[test]
    fn c7_inventory_has_unique_packages_and_capabilities() {
        let registrations = registrations(c7_package_specs()).unwrap();
        assert_eq!(registrations.len(), 26);
        validate_inventory(&registrations).unwrap();
        let capability_count = registrations
            .iter()
            .map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .contributions
                    .capabilities
                    .len()
            })
            .sum::<usize>();
        assert_eq!(capability_count, 137);
    }

    #[test]
    fn canonical_registration_metadata_is_repeatable() {
        let first = registrations(c7_package_specs()).unwrap();
        let second = registrations(c7_package_specs()).unwrap();
        let first_metadata = first
            .iter()
            .map(|registration| registration.metadata.clone())
            .collect::<Vec<_>>();
        let second_metadata = second
            .iter()
            .map(|registration| registration.metadata.clone())
            .collect::<Vec<_>>();
        assert_eq!(first_metadata, second_metadata);
    }

    #[test]
    fn inventory_rejects_tampered_manifest_and_identity_metadata() {
        let mut manifest_tampered = registrations([PackageSpec {
            id: "domain.support",
            display_name: "Support",
            description: "Test support package",
            mount_id: "domain-support",
            capabilities: CAPS,
            supported_surfaces: &["desktop"],
        }])
        .unwrap();
        manifest_tampered[0]
            .metadata
            .manifest
            .payload
            .display
            .name = "Tampered".to_owned();
        assert!(validate_inventory(&manifest_tampered)
            .unwrap_err()
            .to_string()
            .contains("digest"));

        let mut identity_tampered = registrations([PackageSpec {
            id: "domain.support",
            display_name: "Support",
            description: "Test support package",
            mount_id: "domain-support",
            capabilities: CAPS,
            supported_surfaces: &["desktop"],
        }])
        .unwrap();
        identity_tampered[0].metadata.source.source_identity = "other".to_owned();
        assert!(validate_inventory(&identity_tampered)
            .unwrap_err()
            .to_string()
            .contains("source metadata"));
    }
}
