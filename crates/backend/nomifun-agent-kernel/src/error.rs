use nomifun_agent_contracts::{
    ActionId, CapabilityId, CanonicalErrorCode, DigestHex, ExecutionRoleId, McpServerId,
    McpToolKey, PackageId, PluginMountId, ResourceBindingId, ServiceKeyId, SkillId, VersionString,
};
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum KernelError {
    #[error("artifact envelope for package {package_id:?} failed digest verification")]
    InvalidManifestDigest { package_id: PackageId },
    #[error("{field} has invalid semantic version {value:?}")]
    InvalidVersion {
        field: &'static str,
        value: VersionString,
    },
    #[error(
        "package {package_id:?} requires host contract {required:?}, but the host provides {actual:?}"
    )]
    HostContractVersionMismatch {
        package_id: PackageId,
        required: VersionString,
        actual: VersionString,
    },
    #[error("plugin source kind is not enabled for mount {mount_id:?}")]
    SourceNotAllowed { mount_id: PluginMountId },
    #[error("plugin registration for mount {mount_id:?} is invalid: {reason}")]
    InvalidRegistration {
        mount_id: PluginMountId,
        reason: String,
    },
    #[error("JSON schema for {subject} is invalid: {reason}")]
    InvalidJsonSchema { subject: String, reason: String },
    #[error("configuration for mount {mount_id:?} is invalid: {reason}")]
    InvalidPluginConfig {
        mount_id: PluginMountId,
        reason: String,
    },
    #[error("duplicate package id {package_id:?}")]
    DuplicatePackage { package_id: PackageId },
    #[error("duplicate plugin mount id {mount_id:?}")]
    DuplicateMount { mount_id: PluginMountId },
    #[error("duplicate capability id {capability_id:?}")]
    DuplicateCapability { capability_id: CapabilityId },
    #[error("duplicate skill id {skill_id:?}")]
    DuplicateSkill { skill_id: SkillId },
    #[error("duplicate MCP tool mapping {server_id:?}/{tool_key:?}")]
    DuplicateMcpTool {
        server_id: McpServerId,
        tool_key: McpToolKey,
    },
    #[error("MCP capability {capability_id:?} has more than one tool mapping")]
    DuplicateMcpCapability { capability_id: CapabilityId },
    #[error("duplicate execution-role contract {role_id:?}")]
    DuplicateRoleContract { role_id: ExecutionRoleId },
    #[error("execution-role contract {role_id:?} is invalid: {reason}")]
    InvalidRoleContract {
        role_id: ExecutionRoleId,
        reason: String,
    },
    #[error("execution-role provider {role_id:?} is not bound")]
    RoleProviderNotBound { role_id: ExecutionRoleId },
    #[error("execution-role provider {role_id:?} is unavailable on mount {mount_id:?}")]
    RoleProviderUnavailable {
        role_id: ExecutionRoleId,
        mount_id: PluginMountId,
    },
    #[error("execution-role member {capability_id:?} is not provided by {role_id:?}")]
    RoleProviderMemberUnavailable {
        role_id: ExecutionRoleId,
        capability_id: CapabilityId,
    },
    #[error("duplicate role provider for {role_id:?} on mount {mount_id:?}")]
    DuplicateRoleProvider {
        role_id: ExecutionRoleId,
        mount_id: PluginMountId,
    },
    #[error("role provider for {role_id:?} on mount {mount_id:?} is invalid: {reason}")]
    InvalidRoleProvider {
        role_id: ExecutionRoleId,
        mount_id: PluginMountId,
        reason: String,
    },
    #[error(
        "package {package_id:?} requires missing package {dependency_id:?}@{dependency_version:?}"
    )]
    MissingPackageDependency {
        package_id: PackageId,
        dependency_id: PackageId,
        dependency_version: VersionString,
    },
    #[error("package dependency graph contains a cycle")]
    PackageDependencyCycle,
    #[error(
        "capability {capability_id:?} requires missing capability {dependency_id:?}@{dependency_version:?}"
    )]
    MissingCapabilityDependency {
        capability_id: CapabilityId,
        dependency_id: CapabilityId,
        dependency_version: VersionString,
    },
    #[error("capability dependency graph contains a cycle")]
    CapabilityDependencyCycle,
    #[error("skill {skill_id:?} requires missing capability {capability_id:?}")]
    MissingSkillCapability {
        skill_id: SkillId,
        capability_id: CapabilityId,
    },
    #[error(
        "MCP mapping {server_id:?}/{tool_key:?} targets a missing capability {capability_id:?}"
    )]
    MissingMcpCapability {
        server_id: McpServerId,
        tool_key: McpToolKey,
        capability_id: CapabilityId,
    },
    #[error("duplicate service provider for {service_id:?}")]
    DuplicateServiceProvider { service_id: ServiceKeyId },
    #[error("mount {mount_id:?} requires missing service {service_id:?}@{version:?}")]
    MissingService {
        mount_id: PluginMountId,
        service_id: ServiceKeyId,
        version: VersionString,
    },
    #[error(
        "mount {mount_id:?} requires service {service_id:?}@{required:?}, but provider has {actual:?}"
    )]
    ServiceVersionMismatch {
        mount_id: PluginMountId,
        service_id: ServiceKeyId,
        required: VersionString,
        actual: VersionString,
    },
    #[error("service dependency graph contains a cycle")]
    ServiceDependencyCycle,
    #[error("runtime service {service_id:?} has an unexpected Rust type")]
    ServiceTypeMismatch { service_id: ServiceKeyId },
    #[error(
        "registration for mount {mount_id:?} did not export declared service {service_id:?}"
    )]
    MissingRuntimeServiceExport {
        mount_id: PluginMountId,
        service_id: ServiceKeyId,
    },
    #[error(
        "registration for mount {mount_id:?} exported undeclared service {service_id:?}"
    )]
    UndeclaredRuntimeServiceExport {
        mount_id: PluginMountId,
        service_id: ServiceKeyId,
    },
    #[error(
        "registration for mount {mount_id:?} has no handler for capability {capability_id:?}"
    )]
    MissingCapabilityHandler {
        mount_id: PluginMountId,
        capability_id: CapabilityId,
    },
    #[error(
        "registration for mount {mount_id:?} has an undeclared handler for {capability_id:?}"
    )]
    UndeclaredCapabilityHandler {
        mount_id: PluginMountId,
        capability_id: CapabilityId,
    },
    #[error("preset revision is invalid: {reason}")]
    InvalidPresetRevision { reason: String },
    #[error("preset surface {surface} is not declared")]
    SurfaceNotDeclared { surface: String },
    #[error("capability {capability_id:?}@{version:?} is not materialized")]
    CapabilityNotMaterialized {
        capability_id: CapabilityId,
        version: VersionString,
    },
    #[error("skill {skill_id:?}@{version:?} is not materialized")]
    SkillNotMaterialized {
        skill_id: SkillId,
        version: VersionString,
    },
    #[error("capability {capability_id:?} is unavailable on surface {surface}")]
    CapabilityUnavailableOnSurface {
        capability_id: CapabilityId,
        surface: String,
    },
    #[error("capability {capability_id:?} is unavailable on target {target}/{surface}")]
    CapabilityUnavailableOnPlatform {
        capability_id: CapabilityId,
        target: String,
        surface: String,
    },
    #[error("capability {capability_id:?} requires unavailable runtime feature {feature}")]
    RuntimeFeatureUnavailable {
        capability_id: CapabilityId,
        feature: String,
    },
    #[error("capability ceiling contains conflict between {left:?} and {right:?}")]
    CapabilityConflict {
        left: CapabilityId,
        right: CapabilityId,
    },
    #[error("capability action {action_id:?} is not declared by {capability_id:?}")]
    ActionNotDeclared {
        capability_id: CapabilityId,
        action_id: ActionId,
    },
    #[error("resource binding {binding_id:?} is missing")]
    ResourceBindingMissing {
        binding_id: ResourceBindingId,
    },
    #[error(
        "capability {capability_id:?} received undeclared resource binding {binding_id:?} of kind {resource_kind}"
    )]
    UnexpectedResourceBinding {
        capability_id: CapabilityId,
        binding_id: ResourceBindingId,
        resource_kind: String,
    },
    #[error("resource binding {binding_id:?} belongs to another principal")]
    ResourceOwnerMismatch {
        binding_id: ResourceBindingId,
    },
    #[error("capability {capability_id:?} is missing resource kind {resource_kind}")]
    CapabilityResourceNotBound {
        capability_id: CapabilityId,
        resource_kind: String,
    },
    #[error("skill {skill_id:?} requires direct capability selection {capability_id:?}")]
    SkillRequiresCapability {
        skill_id: SkillId,
        capability_id: CapabilityId,
    },
    #[error("snapshot digest construction failed: {reason}")]
    Digest { reason: String },
    #[error("snapshot contract validation failed: {reason}")]
    SnapshotValidation { reason: String },
    #[error("capability {capability_id:?} is not in the frozen snapshot ceiling")]
    CapabilityNotInPreset { capability_id: CapabilityId },
    #[error("capability {capability_id:?} is not active")]
    CapabilityNotActive { capability_id: CapabilityId },
    #[error("active generation conflict: expected {expected}, current {current}")]
    ActivationGenerationConflict { expected: u64, current: u64 },
    #[error("active generation counter is exhausted")]
    ActivationGenerationExhausted,
    #[error("capability handler failed: {reason}")]
    CapabilityExecution { reason: String },
    #[error("kernel registry lock is poisoned")]
    RegistryPoisoned,
    #[error(
        "compiled snapshot expects registry generation {expected_generation}/{expected_digest:?}, current is {actual_generation}/{actual_digest:?}"
    )]
    RegistryGenerationMismatch {
        expected_generation: u64,
        expected_digest: DigestHex,
        actual_generation: u64,
        actual_digest: DigestHex,
    },
}

impl KernelError {
    pub fn canonical_code(&self) -> CanonicalErrorCode {
        use nomifun_agent_contracts::{
            CAPABILITY_NOT_ACTIVE, CAPABILITY_NOT_IN_PRESET, CAPABILITY_NOT_MATERIALIZED,
            CAPABILITY_UNAVAILABLE_ON_PLATFORM, PRESET_RESOURCE_NOT_BOUND,
            PRESET_REVISION_DIGEST_MISMATCH, RESOURCE_OWNER_MISMATCH,
        };

        let code = match self {
            Self::CapabilityNotInPreset { .. } => CAPABILITY_NOT_IN_PRESET,
            Self::CapabilityNotActive { .. } | Self::ActivationGenerationConflict { .. } => {
                CAPABILITY_NOT_ACTIVE
            }
            Self::CapabilityUnavailableOnPlatform { .. }
            | Self::CapabilityUnavailableOnSurface { .. } => CAPABILITY_UNAVAILABLE_ON_PLATFORM,
            Self::ResourceOwnerMismatch { .. } => RESOURCE_OWNER_MISMATCH,
            Self::ResourceBindingMissing { .. }
            | Self::UnexpectedResourceBinding { .. }
            | Self::CapabilityResourceNotBound { .. } => PRESET_RESOURCE_NOT_BOUND,
            Self::InvalidPresetRevision { .. }
            | Self::Digest { .. }
            | Self::SnapshotValidation { .. } => PRESET_REVISION_DIGEST_MISMATCH,
            _ => CAPABILITY_NOT_MATERIALIZED,
        };
        CanonicalErrorCode::from(code)
    }
}
