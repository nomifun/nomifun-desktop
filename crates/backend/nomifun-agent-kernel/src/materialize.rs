use std::collections::{BTreeMap, BTreeSet};

use jsonschema::Validator;
use nomifun_agent_contracts::{
    CapabilityId, CapabilityManifest, DigestHex, ExactRoleProviderRef, ExecutionRoleId,
    McpServerId, McpToolCapabilityMapping, McpToolKey, PackageId, PackageManifest, PackageRef,
    PluginBootCriticality, PluginDesiredState, PluginEffectiveState, PluginMountId,
    PluginRegistrarOperation, PluginRegistrationMetadata, PluginSourceKind, PluginSourceMetadata,
    PluginStateMethod, RoleContractManifest, RoleMemberRequirement, RoleProviderContribution,
    ServiceHandleDescriptor, ServiceKeyDagEdge, ServiceKeyDagNode, ServiceKeyDagPayload,
    ServiceKeyId, ServiceKeyRef, SkillDefinition, SkillId, VersionString, digest_payload,
};
use semver::Version;
use serde::Serialize;

use crate::{KernelError, PluginRegistration};

#[derive(Clone, Debug)]
pub struct MaterializationPolicy {
    pub host_contract_version: VersionString,
    pub available_runtime_features: BTreeSet<nomifun_agent_contracts::RuntimeFeatureId>,
    pub allowed_sources: BTreeSet<PluginSourceKind>,
}

impl MaterializationPolicy {
    pub fn stable(host_contract_version: impl Into<VersionString>) -> Self {
        Self {
            host_contract_version: host_contract_version.into(),
            available_runtime_features: BTreeSet::new(),
            allowed_sources: BTreeSet::from([PluginSourceKind::Bundled]),
        }
    }

    pub fn stable_with_test_fixtures(host_contract_version: impl Into<VersionString>) -> Self {
        Self {
            host_contract_version: host_contract_version.into(),
            available_runtime_features: BTreeSet::new(),
            allowed_sources: BTreeSet::from([
                PluginSourceKind::Bundled,
                PluginSourceKind::TestFixture,
            ]),
        }
    }
}

#[derive(Clone, Debug)]
pub struct MaterializedPackage {
    pub manifest: PackageManifest,
    pub manifest_digest: DigestHex,
    pub mount_id: PluginMountId,
    pub source: PluginSourceMetadata,
}

#[derive(Clone, Debug)]
pub struct MaterializedCapability {
    pub manifest: CapabilityManifest,
    pub schema_digest: DigestHex,
    pub mount_id: PluginMountId,
    pub source: PluginSourceMetadata,
}

#[derive(Clone, Debug)]
pub struct MaterializedSkill {
    pub definition: SkillDefinition,
    pub mount_id: PluginMountId,
    pub source: PluginSourceMetadata,
}

#[derive(Clone, Debug)]
pub struct MaterializedMcpTool {
    pub mapping: McpToolCapabilityMapping,
    pub mount_id: PluginMountId,
    pub source: PluginSourceMetadata,
}

#[derive(Clone, Debug)]
pub struct MaterializedRoleContract {
    pub manifest: RoleContractManifest,
    pub contract_digest: DigestHex,
    pub mount_id: PluginMountId,
}

#[derive(Clone, Debug)]
pub struct MaterializedRoleProvider {
    pub provider: ExactRoleProviderRef,
    pub contribution: RoleProviderContribution,
    pub source: PluginSourceMetadata,
}

#[derive(Clone, Debug)]
pub struct MaterializedRegistry {
    pub generation: u64,
    pub registry_digest: DigestHex,
    pub packages: BTreeMap<PackageId, MaterializedPackage>,
    pub plugins: BTreeMap<PluginMountId, PluginRegistrationMetadata>,
    pub capabilities: BTreeMap<CapabilityId, MaterializedCapability>,
    pub skills: BTreeMap<SkillId, MaterializedSkill>,
    pub mcp_tools: BTreeMap<(McpServerId, McpToolKey), MaterializedMcpTool>,
    pub mcp_by_capability: BTreeMap<CapabilityId, (McpServerId, McpToolKey)>,
    pub role_contracts: BTreeMap<ExecutionRoleId, MaterializedRoleContract>,
    pub role_providers:
        BTreeMap<(ExecutionRoleId, PluginMountId), MaterializedRoleProvider>,
    pub capability_roles: BTreeMap<CapabilityId, ExecutionRoleId>,
    pub package_start_order: Vec<PluginMountId>,
    pub service_dag: ServiceKeyDagPayload,
}

impl MaterializedRegistry {
    pub fn empty() -> Self {
        let service_dag = ServiceKeyDagPayload {
            schema_version: VersionString::from("1.0.0"),
            nodes: Vec::new(),
            edges: Vec::new(),
            topological_start_order: Vec::new(),
            reverse_stop_order: Vec::new(),
        };
        let registry_digest = digest_payload(&EmptyRegistryDigest {
            schema_version: VersionString::from("1.0.0"),
        })
        .expect("empty registry digest is infallible");
        Self {
            generation: 0,
            registry_digest,
            packages: BTreeMap::new(),
            plugins: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            skills: BTreeMap::new(),
            mcp_tools: BTreeMap::new(),
            mcp_by_capability: BTreeMap::new(),
            role_contracts: BTreeMap::new(),
            role_providers: BTreeMap::new(),
            capability_roles: BTreeMap::new(),
            package_start_order: Vec::new(),
            service_dag,
        }
    }

    pub fn package(&self, package_id: &PackageId) -> Option<&MaterializedPackage> {
        self.packages.get(package_id)
    }

    pub fn capability(
        &self,
        capability_id: &CapabilityId,
    ) -> Option<&MaterializedCapability> {
        self.capabilities.get(capability_id)
    }

    pub fn skill(&self, skill_id: &SkillId) -> Option<&MaterializedSkill> {
        self.skills.get(skill_id)
    }

    pub fn mcp_for_capability(
        &self,
        capability_id: &CapabilityId,
    ) -> Option<&MaterializedMcpTool> {
        self.mcp_by_capability
            .get(capability_id)
            .and_then(|key| self.mcp_tools.get(key))
    }

    pub fn role_for_capability(
        &self,
        capability_id: &CapabilityId,
    ) -> Option<&ExecutionRoleId> {
        self.capability_roles.get(capability_id)
    }

    pub fn role_contract(
        &self,
        role_id: &ExecutionRoleId,
    ) -> Option<&MaterializedRoleContract> {
        self.role_contracts.get(role_id)
    }

    pub fn role_provider(
        &self,
        role_id: &ExecutionRoleId,
        mount_id: &PluginMountId,
    ) -> Option<&MaterializedRoleProvider> {
        self.role_providers
            .get(&(role_id.clone(), mount_id.clone()))
    }
}

#[derive(Serialize)]
struct EmptyRegistryDigest {
    schema_version: VersionString,
}

#[derive(Serialize)]
struct RegistryDigestPayload {
    schema_version: VersionString,
    registrations: Vec<PluginRegistrationMetadata>,
    package_start_order: Vec<PluginMountId>,
    service_dag: ServiceKeyDagPayload,
}

pub struct Materializer;

impl Materializer {
    pub fn materialize(
        policy: &MaterializationPolicy,
        registrations: &[PluginRegistration],
        generation: u64,
    ) -> Result<MaterializedRegistry, KernelError> {
        let canonical_registrations = registrations
            .iter()
            .map(PluginRegistration::canonicalized)
            .collect::<Result<Vec<_>, _>>()?;
        Self::materialize_canonical(policy, &canonical_registrations, generation)
    }

    pub(crate) fn materialize_canonical(
        policy: &MaterializationPolicy,
        registrations: &[PluginRegistration],
        generation: u64,
    ) -> Result<MaterializedRegistry, KernelError> {
        let mut ordered = registrations
            .iter()
            .map(|registration| registration.metadata.clone())
            .collect::<Vec<_>>();
        ordered.sort_by(|left, right| {
            registration_sort_key(left).cmp(&registration_sort_key(right))
        });
        for registration in &ordered {
            validate_registration(policy, registration)?;
        }

        let mut packages = BTreeMap::new();
        let mut plugins = BTreeMap::new();
        for registration in &ordered {
            let manifest = &registration.manifest.payload;
            if plugins
                .insert(registration.mount_id.clone(), registration.clone())
                .is_some()
            {
                return Err(KernelError::DuplicateMount {
                    mount_id: registration.mount_id.clone(),
                });
            }
            if packages
                .insert(
                    manifest.package_id.clone(),
                    MaterializedPackage {
                        manifest: manifest.clone(),
                        manifest_digest: registration.manifest.payload_digest.clone(),
                        mount_id: registration.mount_id.clone(),
                        source: registration.source.clone(),
                    },
                )
                .is_some()
            {
                return Err(KernelError::DuplicatePackage {
                    package_id: manifest.package_id.clone(),
                });
            }
        }

        let package_edges = validate_package_dependencies(&packages)?;
        let package_mounts = packages
            .values()
            .map(|package| package.mount_id.clone())
            .collect::<BTreeSet<_>>();
        let package_start_order =
            topological_order(package_mounts, package_edges).ok_or(
                KernelError::PackageDependencyCycle,
            )?;

        let mut capabilities = BTreeMap::new();
        let mut skills = BTreeMap::new();
        let mut mcp_tools = BTreeMap::new();
        let mut mcp_by_capability = BTreeMap::new();
        for registration in &ordered {
            let manifest = &registration.manifest.payload;
            for capability in &manifest.contributions.capabilities {
                if capabilities
                    .insert(
                        capability.id.clone(),
                        MaterializedCapability {
                            manifest: capability.clone(),
                            schema_digest: digest_payload(capability)
                                .map_err(|error| KernelError::Digest {
                                    reason: error.to_string(),
                                })?,
                            mount_id: registration.mount_id.clone(),
                            source: registration.source.clone(),
                        },
                    )
                    .is_some()
                {
                    return Err(KernelError::DuplicateCapability {
                        capability_id: capability.id.clone(),
                    });
                }
            }
            for skill in &manifest.contributions.skills {
                if skills
                    .insert(
                        skill.id.clone(),
                        MaterializedSkill {
                            definition: skill.clone(),
                            mount_id: registration.mount_id.clone(),
                            source: registration.source.clone(),
                        },
                    )
                    .is_some()
                {
                    return Err(KernelError::DuplicateSkill {
                        skill_id: skill.id.clone(),
                    });
                }
            }
            for mapping in &manifest.contributions.mcp_tools {
                let key = (
                    mapping.server_id.clone(),
                    mapping.canonical_tool_key.clone(),
                );
                if mcp_tools
                    .insert(
                        key.clone(),
                        MaterializedMcpTool {
                            mapping: mapping.clone(),
                            mount_id: registration.mount_id.clone(),
                            source: registration.source.clone(),
                        },
                    )
                    .is_some()
                {
                    return Err(KernelError::DuplicateMcpTool {
                        server_id: key.0,
                        tool_key: key.1,
                    });
                }
                if mcp_by_capability
                    .insert(mapping.capability.id.clone(), key)
                    .is_some()
                {
                    return Err(KernelError::DuplicateMcpCapability {
                        capability_id: mapping.capability.id.clone(),
                    });
                }
            }
        }

        validate_capability_dependencies(&capabilities)?;
        validate_skills(&skills, &capabilities)?;
        validate_mcp_mappings(&mcp_tools, &capabilities)?;
        let (role_contracts, capability_roles) =
            materialize_role_contracts(&ordered, &capabilities)?;
        let role_providers =
            materialize_role_providers(&ordered, &role_contracts)?;
        let service_dag = build_service_dag(&ordered)?;

        let registry_digest = digest_payload(&RegistryDigestPayload {
            schema_version: VersionString::from("1.0.0"),
            registrations: ordered,
            package_start_order: package_start_order.clone(),
            service_dag: service_dag.clone(),
        })
        .map_err(|error| KernelError::Digest {
            reason: error.to_string(),
        })?;

        Ok(MaterializedRegistry {
            generation,
            registry_digest,
            packages,
            plugins,
            capabilities,
            skills,
            mcp_tools,
            mcp_by_capability,
            role_contracts,
            role_providers,
            capability_roles,
            package_start_order,
            service_dag,
        })
    }
}

fn registration_sort_key(
    registration: &PluginRegistrationMetadata,
) -> (&str, &str, &str) {
    (
        registration.manifest.payload.package_id.as_ref(),
        registration.manifest.payload.package_version.as_ref(),
        registration.mount_id.as_ref(),
    )
}

fn validate_registration(
    policy: &MaterializationPolicy,
    registration: &PluginRegistrationMetadata,
) -> Result<(), KernelError> {
    let manifest = &registration.manifest.payload;
    if !registration
        .manifest
        .verify()
        .map_err(|error| KernelError::Digest {
            reason: error.to_string(),
        })?
    {
        return Err(KernelError::InvalidManifestDigest {
            package_id: manifest.package_id.clone(),
        });
    }
    if !policy.allowed_sources.contains(&registration.source.source_kind) {
        return Err(KernelError::SourceNotAllowed {
            mount_id: registration.mount_id.clone(),
        });
    }
    validate_version("schema_version", &manifest.schema_version)?;
    validate_version("host_contract_version", &manifest.host_contract_version)?;
    validate_version("package_version", &manifest.package_version)?;
    validate_version(
        "entrypoint.contract_version",
        &manifest.entrypoint.contract_version,
    )?;
    if manifest.host_contract_version != policy.host_contract_version {
        return Err(KernelError::HostContractVersionMismatch {
            package_id: manifest.package_id.clone(),
            required: manifest.host_contract_version.clone(),
            actual: policy.host_contract_version.clone(),
        });
    }
    if registration.boot_state.criticality != PluginBootCriticality::Required
        && registration.boot_state.criticality != PluginBootCriticality::Optional
    {
        return invalid_registration(registration, "unknown boot criticality");
    }
    if registration.boot_state.desired_state != PluginDesiredState::Enabled
        || registration.boot_state.effective_state != PluginEffectiveState::Active
        || registration.boot_state.diagnostic_code.is_some()
    {
        return invalid_registration(
            registration,
            "only enabled, active registrations without diagnostics may publish",
        );
    }

    let package_ref = PackageRef {
        id: manifest.package_id.clone(),
        version: manifest.package_version.clone(),
    };
    if registration.registrar.identity.package != package_ref
        || registration.context.identity.package != package_ref
        || registration.registrar.identity.mount_id != registration.mount_id
        || registration.context.identity.mount_id != registration.mount_id
        || registration.context.source != registration.source
        || registration.context.state.package_id != manifest.package_id
        || registration.context.state.mount_id != registration.mount_id
        || registration.context.state.methods
            != PluginStateMethod::REQUIRED.into_iter().collect()
    {
        return invalid_registration(
            registration,
            "identity, source, or mandatory PluginState descriptor drifted",
        );
    }

    let schema = &manifest.config_schema.0;
    let validator = compile_schema(
        &format!("package {} config", manifest.package_id.as_ref()),
        schema,
    )?;
    let expected_schema_digest =
        digest_payload(&manifest.config_schema).map_err(|error| KernelError::Digest {
            reason: error.to_string(),
        })?;
    if registration.context.validated_config.schema_digest != expected_schema_digest {
        return invalid_registration(registration, "validated config schema digest mismatch");
    }
    let errors = validator
        .iter_errors(&registration.context.validated_config.value.0)
        .take(8)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(KernelError::InvalidPluginConfig {
            mount_id: registration.mount_id.clone(),
            reason: errors.join("; "),
        });
    }

    let capability_ids = manifest
        .contributions
        .capabilities
        .iter()
        .map(|capability| capability.id.clone())
        .collect::<BTreeSet<_>>();
    let skill_ids = manifest
        .contributions
        .skills
        .iter()
        .map(|skill| skill.id.clone())
        .collect::<BTreeSet<_>>();
    let mcp_keys = manifest
        .contributions
        .mcp_tools
        .iter()
        .map(|mapping| mapping.canonical_tool_key.clone())
        .collect::<BTreeSet<_>>();
    let role_ids = manifest
        .contributions
        .role_providers
        .iter()
        .map(|provider| provider.role.key.role_id.clone())
        .collect::<BTreeSet<_>>();
    let provided_service_refs = manifest
        .provides_services
        .iter()
        .map(|provision| provision.service.clone())
        .collect::<BTreeSet<_>>();
    let provided_service_ids = provided_service_refs
        .iter()
        .map(|service| service.id.clone())
        .collect::<BTreeSet<_>>();
    if capability_ids.len() != manifest.contributions.capabilities.len()
        || skill_ids.len() != manifest.contributions.skills.len()
        || mcp_keys.len() != manifest.contributions.mcp_tools.len()
        || role_ids.len() != manifest.contributions.role_providers.len()
        || provided_service_refs.len() != manifest.provides_services.len()
    {
        return invalid_registration(registration, "duplicate declaration within manifest");
    }

    let required_operations = required_registrar_operations(
        !capability_ids.is_empty(),
        !skill_ids.is_empty(),
        !mcp_keys.is_empty(),
        !role_ids.is_empty(),
        !provided_service_ids.is_empty(),
        !declared_host_ports(&registration.context).is_empty(),
    );
    if registration.registrar.declared_capability_ids != capability_ids
        || registration.registrar.declared_skill_ids != skill_ids
        || registration.registrar.declared_mcp_tool_keys != mcp_keys
        || registration.registrar.declared_role_ids != role_ids
        || registration.registrar.declared_service_keys != provided_service_ids
        || registration.registrar.declared_host_ports
            != declared_host_ports(&registration.context)
        || registration.registrar.allowed_operations != required_operations
    {
        return invalid_registration(
            registration,
            "registrar declarations must exactly match manifest and context",
        );
    }

    let context_provided = registration
        .context
        .declared_services
        .provided_services
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let required_service_refs = manifest
        .requires_services
        .iter()
        .map(|requirement| requirement.service.clone())
        .collect::<BTreeSet<_>>();
    let context_required = registration
        .context
        .declared_services
        .required_service_handles
        .iter()
        .map(|handle| handle.service.clone())
        .collect::<BTreeSet<_>>();
    if context_provided != provided_service_refs
        || context_required != required_service_refs
        || required_service_refs.len() != manifest.requires_services.len()
    {
        return invalid_registration(
            registration,
            "declared service view must exactly match manifest requirements",
        );
    }

    let host_ports = declared_host_ports(&registration.context);
    for capability in &manifest.contributions.capabilities {
        validate_version("capability.version", &capability.version)?;
        if capability.package != package_ref {
            return invalid_registration(
                registration,
                "capability package reference does not match its manifest",
            );
        }
        compile_schema(
            &format!("capability {} config", capability.id.as_ref()),
            &capability.config_schema.0,
        )?;
        let action_ids = capability
            .contributions
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect::<BTreeSet<_>>();
        if action_ids.len() != capability.contributions.actions.len() {
            return invalid_registration(registration, "duplicate capability action id");
        }
        if capability
            .contributions
            .host_ports
            .iter()
            .any(|port| !host_ports.contains(&port.id))
        {
            return invalid_registration(
                registration,
                "capability references an undeclared host port",
            );
        }
        for requirement in &capability.requires_runtime_features {
            validate_version("runtime feature version", &requirement.version)?;
            if !policy.available_runtime_features.contains(&requirement.id) {
                return Err(KernelError::RuntimeFeatureUnavailable {
                    capability_id: capability.id.clone(),
                    feature: requirement.id.as_ref().to_owned(),
                });
            }
        }
    }
    for skill in &manifest.contributions.skills {
        validate_version("skill.version", &skill.version)?;
        if skill.package != package_ref {
            return invalid_registration(
                registration,
                "skill package reference does not match its manifest",
            );
        }
    }
    for mapping in &manifest.contributions.mcp_tools {
        validate_version(
            "mcp.materialization_version",
            &mapping.materialization_version,
        )?;
        if mapping.package != package_ref {
            return invalid_registration(
                registration,
                "MCP mapping package reference does not match its manifest",
            );
        }
    }
    for dependency in &manifest.package_dependencies {
        validate_version("package dependency version", &dependency.version)?;
    }
    for requirement in &manifest.requires_runtime_features {
        validate_version("package runtime feature version", &requirement.version)?;
        if !policy.available_runtime_features.contains(&requirement.id) {
            return Err(KernelError::RuntimeFeatureUnavailable {
                capability_id: CapabilityId::from(format!(
                    "package:{}",
                    manifest.package_id.as_ref()
                )),
                feature: requirement.id.as_ref().to_owned(),
            });
        }
    }
    Ok(())
}

fn invalid_registration<T>(
    registration: &PluginRegistrationMetadata,
    reason: impl Into<String>,
) -> Result<T, KernelError> {
    Err(KernelError::InvalidRegistration {
        mount_id: registration.mount_id.clone(),
        reason: reason.into(),
    })
}

fn validate_version(field: &'static str, value: &VersionString) -> Result<(), KernelError> {
    Version::parse(value.as_ref())
        .map(|_| ())
        .map_err(|_| KernelError::InvalidVersion {
            field,
            value: value.clone(),
        })
}

fn compile_schema(subject: &str, schema: &serde_json::Value) -> Result<Validator, KernelError> {
    jsonschema::options()
        .build(schema)
        .map_err(|error| KernelError::InvalidJsonSchema {
            subject: subject.to_owned(),
            reason: error.to_string(),
        })
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

fn declared_host_ports(
    context: &nomifun_agent_contracts::PluginContextDescriptor,
) -> BTreeSet<nomifun_agent_contracts::HostPortId> {
    context
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
        .collect()
}

fn validate_package_dependencies(
    packages: &BTreeMap<PackageId, MaterializedPackage>,
) -> Result<Vec<(PluginMountId, PluginMountId)>, KernelError> {
    let mut edges = Vec::new();
    for package in packages.values() {
        for dependency in &package.manifest.package_dependencies {
            let Some(provider) = packages.get(&dependency.id) else {
                return Err(KernelError::MissingPackageDependency {
                    package_id: package.manifest.package_id.clone(),
                    dependency_id: dependency.id.clone(),
                    dependency_version: dependency.version.clone(),
                });
            };
            if provider.manifest.package_version != dependency.version {
                return Err(KernelError::MissingPackageDependency {
                    package_id: package.manifest.package_id.clone(),
                    dependency_id: dependency.id.clone(),
                    dependency_version: dependency.version.clone(),
                });
            }
            edges.push((provider.mount_id.clone(), package.mount_id.clone()));
        }
    }
    Ok(edges)
}

fn validate_capability_dependencies(
    capabilities: &BTreeMap<CapabilityId, MaterializedCapability>,
) -> Result<(), KernelError> {
    let nodes = capabilities.keys().cloned().collect::<BTreeSet<_>>();
    let mut edges = Vec::new();
    for capability in capabilities.values() {
        for dependency in &capability.manifest.requires {
            let Some(materialized) = capabilities.get(&dependency.id) else {
                return Err(KernelError::MissingCapabilityDependency {
                    capability_id: capability.manifest.id.clone(),
                    dependency_id: dependency.id.clone(),
                    dependency_version: dependency.version.clone(),
                });
            };
            if materialized.manifest.version != dependency.version {
                return Err(KernelError::MissingCapabilityDependency {
                    capability_id: capability.manifest.id.clone(),
                    dependency_id: dependency.id.clone(),
                    dependency_version: dependency.version.clone(),
                });
            }
            edges.push((dependency.id.clone(), capability.manifest.id.clone()));
        }
    }
    topological_order(nodes, edges)
        .map(|_| ())
        .ok_or(KernelError::CapabilityDependencyCycle)
}

fn validate_skills(
    skills: &BTreeMap<SkillId, MaterializedSkill>,
    capabilities: &BTreeMap<CapabilityId, MaterializedCapability>,
) -> Result<(), KernelError> {
    for skill in skills.values() {
        for requirement in &skill.definition.requires_capabilities {
            if !capabilities.get(&requirement.id).is_some_and(|capability| {
                capability.manifest.version == requirement.version
            }) {
                return Err(KernelError::MissingSkillCapability {
                    skill_id: skill.definition.id.clone(),
                    capability_id: requirement.id.clone(),
                });
            }
        }
    }
    Ok(())
}

fn validate_mcp_mappings(
    mappings: &BTreeMap<(McpServerId, McpToolKey), MaterializedMcpTool>,
    capabilities: &BTreeMap<CapabilityId, MaterializedCapability>,
) -> Result<(), KernelError> {
    for ((server_id, tool_key), mapping) in mappings {
        if !capabilities
            .get(&mapping.mapping.capability.id)
            .is_some_and(|capability| {
                capability.manifest.version == mapping.mapping.capability.version
            })
        {
            return Err(KernelError::MissingMcpCapability {
                server_id: server_id.clone(),
                tool_key: tool_key.clone(),
                capability_id: mapping.mapping.capability.id.clone(),
            });
        }
    }
    Ok(())
}

fn materialize_role_contracts(
    registrations: &[PluginRegistrationMetadata],
    capabilities: &BTreeMap<CapabilityId, MaterializedCapability>,
) -> Result<
    (
        BTreeMap<ExecutionRoleId, MaterializedRoleContract>,
        BTreeMap<CapabilityId, ExecutionRoleId>,
    ),
    KernelError,
> {
    let mut contracts = BTreeMap::new();
    let mut capability_roles = BTreeMap::new();
    for registration in registrations {
        for contract in &registration.manifest.payload.contributions.role_contracts {
            let role_id = contract.key.role_id.clone();
            validate_version("role contract version", &contract.key.contract_version)?;
            if role_id.as_ref().trim().is_empty() || !role_id.as_ref().contains('.') {
                return Err(KernelError::InvalidRoleContract {
                    role_id,
                    reason: "execution-role IDs must be non-empty and namespaced".to_owned(),
                });
            }
            if contract.members.is_empty() {
                return Err(KernelError::InvalidRoleContract {
                    role_id,
                    reason: "role contract must declare at least one member".to_owned(),
                });
            }

            let mut member_ids = BTreeSet::new();
            let mut required = 0usize;
            for member in &contract.members {
                if !member_ids.insert(member.capability.id.clone()) {
                    return Err(KernelError::InvalidRoleContract {
                        role_id: role_id.clone(),
                        reason: format!(
                            "duplicate role member {}",
                            member.capability.id.as_ref()
                        ),
                    });
                }
                if member.requirement == RoleMemberRequirement::Required {
                    required += 1;
                }
                let Some(capability) = capabilities.get(&member.capability.id) else {
                    return Err(KernelError::InvalidRoleContract {
                        role_id: role_id.clone(),
                        reason: format!(
                            "role member {} is not materialized",
                            member.capability.id.as_ref()
                        ),
                    });
                };
                if capability.manifest.version != member.capability.version
                    || capability.schema_digest != member.capability_manifest_digest
                {
                    return Err(KernelError::InvalidRoleContract {
                        role_id: role_id.clone(),
                        reason: format!(
                            "role member {} does not match its exact capability manifest",
                            member.capability.id.as_ref()
                        ),
                    });
                }
                if let Some(existing) = capability_roles
                    .insert(member.capability.id.clone(), role_id.clone())
                {
                    return Err(KernelError::InvalidRoleContract {
                        role_id: role_id.clone(),
                        reason: format!(
                            "capability {} is already owned by execution role {}",
                            member.capability.id.as_ref(),
                            existing.as_ref()
                        ),
                    });
                }
            }
            if required == 0 {
                return Err(KernelError::InvalidRoleContract {
                    role_id: role_id.clone(),
                    reason: "role contract must declare at least one required member".to_owned(),
                });
            }
            if let Some(resource_kind) = &contract.serialized_target_resource_kind {
                if !contract.members.iter().any(|member| {
                    capabilities[&member.capability.id]
                        .manifest
                        .contributions
                        .resource_kinds
                        .contains(resource_kind)
                }) {
                    return Err(KernelError::InvalidRoleContract {
                        role_id: role_id.clone(),
                        reason: format!(
                            "serialized target resource kind {} is unused by every role member",
                            resource_kind.as_ref()
                        ),
                    });
                }
            }
            let contract_digest =
                digest_payload(contract).map_err(|error| KernelError::Digest {
                    reason: error.to_string(),
                })?;
            if contracts
                .insert(
                    role_id.clone(),
                    MaterializedRoleContract {
                        manifest: contract.clone(),
                        contract_digest,
                        mount_id: registration.mount_id.clone(),
                    },
                )
                .is_some()
            {
                return Err(KernelError::DuplicateRoleContract { role_id });
            }
        }
    }
    Ok((contracts, capability_roles))
}

fn materialize_role_providers(
    registrations: &[PluginRegistrationMetadata],
    contracts: &BTreeMap<ExecutionRoleId, MaterializedRoleContract>,
) -> Result<
    BTreeMap<(ExecutionRoleId, PluginMountId), MaterializedRoleProvider>,
    KernelError,
> {
    let mut providers = BTreeMap::new();
    for registration in registrations {
        let manifest = &registration.manifest.payload;
        let package = PackageRef {
            id: manifest.package_id.clone(),
            version: manifest.package_version.clone(),
        };
        for contribution in &manifest.contributions.role_providers {
            let role_id = contribution.role.key.role_id.clone();
            let Some(contract) = contracts.get(&role_id) else {
                return Err(KernelError::InvalidRoleProvider {
                    role_id,
                    mount_id: registration.mount_id.clone(),
                    reason: "referenced role contract is not materialized".to_owned(),
                });
            };
            if contribution.role.key != contract.manifest.key
                || contribution.role.contract_digest != contract.contract_digest
            {
                return Err(KernelError::InvalidRoleProvider {
                    role_id,
                    mount_id: registration.mount_id.clone(),
                    reason: "provider references a different role contract".to_owned(),
                });
            }
            let contract_members = contract
                .manifest
                .members
                .iter()
                .map(|member| (member.capability.id.clone(), member.requirement))
                .collect::<BTreeMap<_, _>>();
            if contribution
                .members
                .keys()
                .any(|capability_id| !contract_members.contains_key(capability_id))
            {
                return Err(KernelError::InvalidRoleProvider {
                    role_id,
                    mount_id: registration.mount_id.clone(),
                    reason: "provider contributes a capability outside the role contract"
                        .to_owned(),
                });
            }
            if let Some(missing) = contract_members.iter().find_map(
                |(capability_id, requirement)| {
                    (*requirement == RoleMemberRequirement::Required
                        && !contribution.members.contains_key(capability_id))
                    .then_some(capability_id)
                },
            ) {
                return Err(KernelError::InvalidRoleProvider {
                    role_id,
                    mount_id: registration.mount_id.clone(),
                    reason: format!(
                        "provider is missing required role member {}",
                        missing.as_ref()
                    ),
                });
            }
            let contribution_digest =
                digest_payload(contribution).map_err(|error| KernelError::Digest {
                    reason: error.to_string(),
                })?;
            let provider = ExactRoleProviderRef {
                role: contribution.role.clone(),
                package: package.clone(),
                mount_id: registration.mount_id.clone(),
                contribution_digest,
            };
            let key = (role_id.clone(), registration.mount_id.clone());
            if providers
                .insert(
                    key,
                    MaterializedRoleProvider {
                        provider,
                        contribution: contribution.clone(),
                        source: registration.source.clone(),
                    },
                )
                .is_some()
            {
                return Err(KernelError::DuplicateRoleProvider {
                    role_id,
                    mount_id: registration.mount_id.clone(),
                });
            }
        }
    }
    Ok(providers)
}

fn build_service_dag(
    registrations: &[PluginRegistrationMetadata],
) -> Result<ServiceKeyDagPayload, KernelError> {
    let mut providers = BTreeMap::<ServiceKeyId, (ServiceKeyRef, PackageRef, PluginMountId)>::new();
    for registration in registrations {
        let manifest = &registration.manifest.payload;
        let package_ref = PackageRef {
            id: manifest.package_id.clone(),
            version: manifest.package_version.clone(),
        };
        for provision in &manifest.provides_services {
            if providers
                .insert(
                    provision.service.id.clone(),
                    (
                        provision.service.clone(),
                        package_ref.clone(),
                        registration.mount_id.clone(),
                    ),
                )
                .is_some()
            {
                return Err(KernelError::DuplicateServiceProvider {
                    service_id: provision.service.id.clone(),
                });
            }
        }
    }

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut graph_edges = Vec::new();
    for registration in registrations {
        let manifest = &registration.manifest.payload;
        let package_ref = PackageRef {
            id: manifest.package_id.clone(),
            version: manifest.package_version.clone(),
        };
        let provides = manifest
            .provides_services
            .iter()
            .map(|provision| provision.service.clone())
            .collect::<Vec<_>>();
        let requires = manifest
            .requires_services
            .iter()
            .map(|requirement| requirement.service.clone())
            .collect::<Vec<_>>();
        nodes.push(ServiceKeyDagNode {
            package: package_ref,
            mount_id: registration.mount_id.clone(),
            provides,
            requires,
        });

        let actual_handles = registration
            .context
            .declared_services
            .required_service_handles
            .iter()
            .map(handle_identity)
            .collect::<BTreeSet<_>>();
        let mut expected_handles = BTreeSet::new();
        for requirement in &manifest.requires_services {
            let Some((provided, provider_package, provider_mount)) =
                providers.get(&requirement.service.id)
            else {
                return Err(KernelError::MissingService {
                    mount_id: registration.mount_id.clone(),
                    service_id: requirement.service.id.clone(),
                    version: requirement.service.version.clone(),
                });
            };
            if provided.version != requirement.service.version {
                return Err(KernelError::ServiceVersionMismatch {
                    mount_id: registration.mount_id.clone(),
                    service_id: requirement.service.id.clone(),
                    required: requirement.service.version.clone(),
                    actual: provided.version.clone(),
                });
            }
            expected_handles.insert((
                requirement.service.clone(),
                provider_package.clone(),
                provider_mount.clone(),
            ));
            edges.push(ServiceKeyDagEdge {
                service: requirement.service.clone(),
                provider_mount_id: provider_mount.clone(),
                consumer_mount_id: registration.mount_id.clone(),
            });
            graph_edges.push((provider_mount.clone(), registration.mount_id.clone()));
        }
        if actual_handles != expected_handles {
            return invalid_registration(
                registration,
                "required service handles do not match the resolved provider",
            );
        }
    }
    nodes.sort_by(|left, right| left.mount_id.cmp(&right.mount_id));
    edges.sort_by(|left, right| {
        (
            &left.provider_mount_id,
            &left.consumer_mount_id,
            &left.service.id,
            &left.service.version,
        )
            .cmp(&(
                &right.provider_mount_id,
                &right.consumer_mount_id,
                &right.service.id,
                &right.service.version,
            ))
    });
    let mounts = registrations
        .iter()
        .map(|registration| registration.mount_id.clone())
        .collect::<BTreeSet<_>>();
    let topological_start_order =
        topological_order(mounts, graph_edges).ok_or(KernelError::ServiceDependencyCycle)?;
    let reverse_stop_order = topological_start_order.iter().rev().cloned().collect();
    Ok(ServiceKeyDagPayload {
        schema_version: VersionString::from("1.0.0"),
        nodes,
        edges,
        topological_start_order,
        reverse_stop_order,
    })
}

fn handle_identity(
    handle: &ServiceHandleDescriptor,
) -> (ServiceKeyRef, PackageRef, PluginMountId) {
    (
        handle.service.clone(),
        handle.provider_package.clone(),
        handle.provider_mount_id.clone(),
    )
}

fn topological_order<K>(
    nodes: BTreeSet<K>,
    edges: Vec<(K, K)>,
) -> Option<Vec<K>>
where
    K: Clone + Ord,
{
    let mut outgoing = nodes
        .iter()
        .cloned()
        .map(|node| (node, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut indegree = nodes
        .iter()
        .cloned()
        .map(|node| (node, 0usize))
        .collect::<BTreeMap<_, _>>();
    for (from, to) in edges {
        if outgoing.get_mut(&from)?.insert(to.clone()) {
            *indegree.get_mut(&to)? += 1;
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(node, degree)| (*degree == 0).then_some(node.clone()))
        .collect::<BTreeSet<_>>();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(node) = ready.pop_first() {
        order.push(node.clone());
        for next in outgoing.get(&node)? {
            let degree = indegree.get_mut(next)?;
            *degree -= 1;
            if *degree == 0 {
                ready.insert(next.clone());
            }
        }
    }
    (order.len() == nodes.len()).then_some(order)
}
