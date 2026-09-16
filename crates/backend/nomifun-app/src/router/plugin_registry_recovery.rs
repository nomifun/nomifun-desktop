//! Attribute a Kernel validation failure to the declarations that own its typed
//! subject. This is not a dependency resolver: it neither checks compatibility
//! nor orders registrations. The caller retries the remaining complete batch.
use std::collections::BTreeMap;

use nomifun_agent_contracts::PluginMountId;
use nomifun_agent_kernel::{KernelError, PluginRegistration};

pub(super) fn rejected_mounts(
    error: &KernelError,
    candidates: &BTreeMap<PluginMountId, PluginRegistration>,
) -> Vec<PluginMountId> {
    candidates
        .iter()
        .filter_map(|(mount, registration)| {
            let manifest = &registration.metadata.manifest.payload;
            let contributions = &manifest.contributions;
            let rejected = match error {
                KernelError::InvalidRegistration { mount_id, .. }
                | KernelError::InvalidPluginConfig { mount_id, .. }
                | KernelError::SourceNotAllowed { mount_id }
                | KernelError::InvalidRoleProvider { mount_id, .. }
                | KernelError::DuplicateRoleProvider { mount_id, .. }
                | KernelError::DuplicateMount { mount_id }
                | KernelError::MissingService { mount_id, .. }
                | KernelError::ServiceVersionMismatch { mount_id, .. }
                | KernelError::MissingRuntimeServiceExport { mount_id, .. }
                | KernelError::UndeclaredRuntimeServiceExport { mount_id, .. }
                | KernelError::MissingCapabilityHandler { mount_id, .. }
                | KernelError::UndeclaredCapabilityHandler { mount_id, .. }
                | KernelError::MissingCapabilityContextFactory { mount_id, .. }
                | KernelError::UndeclaredCapabilityContextFactory { mount_id, .. }
                | KernelError::MissingCapabilityResourceFactory { mount_id, .. }
                | KernelError::UndeclaredCapabilityResourceFactory { mount_id, .. } => {
                    mount == mount_id
                }
                KernelError::InvalidManifestDigest { package_id }
                | KernelError::HostContractVersionMismatch { package_id, .. }
                | KernelError::MissingPackageDependency { package_id, .. }
                | KernelError::DuplicatePackage { package_id } => {
                    &manifest.package_id == package_id
                }
                KernelError::MissingCapabilityDependency { capability_id, .. }
                | KernelError::DuplicateCapability { capability_id }
                | KernelError::DuplicateMcpCapability { capability_id } => contributions
                    .capabilities
                    .iter()
                    .any(|c| &c.id == capability_id),
                KernelError::MissingSkillCapability { skill_id, .. }
                | KernelError::DuplicateSkill { skill_id } => {
                    contributions.skills.iter().any(|s| &s.id == skill_id)
                }
                KernelError::MissingMcpCapability {
                    server_id,
                    tool_key,
                    ..
                }
                | KernelError::InvalidMcpMaterialization {
                    server_id,
                    tool_key,
                    ..
                }
                | KernelError::DuplicateMcpTool {
                    server_id,
                    tool_key,
                } => contributions
                    .mcp_tools
                    .iter()
                    .any(|m| &m.server_id == server_id && &m.canonical_tool_key == tool_key),
                KernelError::InvalidRoleContract { role_id, .. }
                | KernelError::DuplicateRoleContract { role_id } => contributions
                    .role_contracts
                    .iter()
                    .any(|r| &r.key.role_id == role_id),
                KernelError::DuplicateServiceProvider { service_id } => manifest
                    .provides_services
                    .iter()
                    .any(|s| &s.service.id == service_id),
                KernelError::DuplicateContribution { contribution_id } => contributions
                    .capabilities
                    .iter()
                    .any(|c| &c.contribution_id == contribution_id),
                // Do not infer ownership from diagnostic strings, choose a winner
                // for an unscoped error, or remove trusted base registrations.
                _ => false,
            };
            rejected.then(|| mount.clone())
        })
        .collect()
}
