use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nomifun_agent_contracts::{
    AgentPresetRevision, AgentPresetRevisionPayload, CanonicalErrorCode,
    CapabilityConsumer, ContributionLock, OperationId,
    PresetRevisionRef, PrincipalRef,
    ResolvedSnapshotEnvelope, UserId, digest_payload,
};
use nomifun_agent_kernel::{
    AgentPresetCompiler as KernelAgentPresetCompiler, CompileRequest, CompilerEnvironment,
    KernelError, KernelRegistry, MaterializedRegistry,
};
use nomifun_api_types::{AgentPresetDraftDto, AgentPresetRevisionDto};
use serde::Serialize;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use uuid::Uuid;

use crate::catalog::{availability_code, CatalogSnapshot};
use crate::error::ControlPlaneError;
use crate::wire::wire_cast;

/// Supplies the exact materialized registry used by the Kernel execution path.
///
/// The provider is intentionally lazy because the platform constructs the
/// Control Plane before publishing its initial plugin registrations.
pub trait CanonicalRegistryProvider: Send + Sync {
    fn snapshot(&self) -> Result<Arc<MaterializedRegistry>, ControlPlaneError>;
}

impl<F> CanonicalRegistryProvider for F
where
    F: Fn() -> Result<Arc<MaterializedRegistry>, ControlPlaneError> + Send + Sync,
{
    fn snapshot(&self) -> Result<Arc<MaterializedRegistry>, ControlPlaneError> {
        self()
    }
}

impl CanonicalRegistryProvider for KernelRegistry {
    fn snapshot(&self) -> Result<Arc<MaterializedRegistry>, ControlPlaneError> {
        KernelRegistry::snapshot(self).map_err(|error| ControlPlaneError::Wire(error.to_string()))
    }
}

struct StaticCanonicalRegistryProvider {
    registry: Arc<MaterializedRegistry>,
}

impl CanonicalRegistryProvider for StaticCanonicalRegistryProvider {
    fn snapshot(&self) -> Result<Arc<MaterializedRegistry>, ControlPlaneError> {
        Ok(Arc::clone(&self.registry))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PresetCompilation {
    pub(crate) payload: AgentPresetRevisionPayload,
    pub(crate) contribution_locks: Vec<ContributionLock>,
    pub(crate) candidate_revision_ref: PresetRevisionRef,
    pub(crate) snapshot: Option<ResolvedSnapshotEnvelope>,
    pub(crate) diagnostics: Vec<CompilationDiagnostic>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CompilationDiagnostic {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) subject: Option<String>,
    pub(crate) details: Option<Value>,
}

#[derive(Clone)]
pub struct PresetRevisionCompiler {
    canonical_registry: Option<Arc<dyn CanonicalRegistryProvider>>,
    canonical_environment: Option<CompilerEnvironment>,
    consumer_validator: Option<Arc<ConsumerValidator>>,
}

type ConsumerValidator = dyn Fn(&MaterializedRegistry, &ResolvedSnapshotEnvelope) -> Result<(), ControlPlaneError> + Send + Sync;

impl PresetRevisionCompiler {
    pub(crate) fn with_current_role_bindings(
        mut self,
        bindings: std::collections::BTreeMap<nomifun_agent_contracts::ExecutionRoleId, nomifun_agent_contracts::InstallationRoleBinding>,
    ) -> Result<Self, ControlPlaneError> {
        let environment = self.canonical_environment.as_mut().ok_or_else(||
            ControlPlaneError::Wire("canonical compiler environment is not configured".into()))?;
        environment.installation_role_bindings = bindings;
        Ok(self)
    }

    pub(crate) fn validate_role_default(
        &self,
        selection: &nomifun_agent_contracts::RoleProviderSelection,
    ) -> Result<(), ControlPlaneError> {
        let (registry, environment) = self.canonical_inputs()?;
        KernelAgentPresetCompiler::validate_role_default(&registry, &environment, selection)
            .map_err(|error| ControlPlaneError::canonical(
                error.canonical_code(), axum::http::StatusCode::UNPROCESSABLE_ENTITY, error.to_string(),
            ))
    }

    pub fn new() -> Self {
        Self {
            canonical_registry: None,
            canonical_environment: None,
            consumer_validator: None,
        }
    }

    /// Validate a host consumer against the very registry/plan used by the
    /// canonical compiler. This may reject, never rewrite or re-resolve it.
    /// Runs for compilation and unchanged saved-plan reuse alike, including
    /// authoring saves and product selection checks that invoke the compiler.
    pub fn with_consumer_validator<F>(mut self, validate: F) -> Self
    where F: Fn(&MaterializedRegistry, &ResolvedSnapshotEnvelope) -> Result<(), ControlPlaneError> + Send + Sync + 'static {
        self.consumer_validator = Some(Arc::new(validate));
        self
    }

    /// Bind revision saves to the exact registry and environment used by
    /// Session Open. The provider is also evaluated before reusing a saved draft.
    pub fn with_canonical_registry<P>(
        mut self,
        provider: Arc<P>,
        environment: CompilerEnvironment,
    ) -> Self
    where
        P: CanonicalRegistryProvider + 'static,
    {
        self.canonical_registry = Some(provider);
        self.canonical_environment = Some(environment);
        self
    }

    pub fn with_materialized_registry(
        self,
        registry: Arc<MaterializedRegistry>,
        environment: CompilerEnvironment,
    ) -> Self {
        self.with_canonical_registry(
            Arc::new(StaticCanonicalRegistryProvider { registry }),
            environment,
        )
    }

    pub(crate) fn compile(
        &self,
        owner: &UserId,
        draft: &AgentPresetDraftDto,
        current_revision: Option<&AgentPresetRevision>,
        current_snapshot: Option<&ResolvedSnapshotEnvelope>,
        catalog: &CatalogSnapshot,
    ) -> Result<PresetCompilation, ControlPlaneError> {
        catalog.validate()?;
        let payload: AgentPresetRevisionPayload = wire_cast(&draft.document)?;
        let payload_unchanged = current_revision.is_some_and(|current| current.payload == payload);
        let current_canonical_inputs = if payload_unchanged && current_snapshot.is_some() {
            self.canonical_inputs_if_configured()?
        } else {
            None
        };
        let materialization_unchanged = match (
            current_revision,
            current_snapshot,
            current_canonical_inputs.as_ref(),
        ) {
            (Some(revision), Some(snapshot), Some((registry, environment))) => {
                snapshot_matches_registry(snapshot, registry)?
                    && snapshot.content.context_order == revision.payload.context_order
                    && snapshot.content.middleware_order == revision.payload.middleware_order
                    && KernelAgentPresetCompiler::skills_unchanged(registry, revision, snapshot)
                    && KernelAgentPresetCompiler::role_providers_unchanged(
                        registry, environment, revision, snapshot,
                    )
            }
            _ => true,
        };
        let clean = payload_unchanged && materialization_unchanged;
        let consumer_registry = current_canonical_inputs.as_ref().map(|(registry, _)| registry.clone());
        let canonical_inputs = if clean {
            None
        } else if let Some(inputs) = current_canonical_inputs {
            Some(inputs)
        } else {
            Some(self.canonical_inputs()?)
        };
        let consumer_registry = canonical_inputs.as_ref().map(|(registry, _)| registry.clone()).or(consumer_registry);
        let contribution_locks = if clean {
            current_revision
                .map(|revision| revision.contribution_locks.clone())
                .unwrap_or_default()
        } else {
            contribution_locks_for_payload(
                &payload,
                catalog,
                canonical_inputs
                    .as_ref()
                    .expect("dirty compilation has canonical inputs")
                    .0
                    .as_ref(),
            )?
        };
        let revision_digest = digest_payload(&nomifun_agent_contracts::AgentPresetRevisionDigestInput {
            payload: payload.clone(),
            contribution_locks: {
                let mut locks = contribution_locks.clone();
                locks.sort();
                locks
            },
        })
        .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        let candidate_revision_ref = if clean {
            current_revision
                .expect("clean draft has a current revision")
                .reference
                .clone()
        } else {
            PresetRevisionRef {
                preset_id: draft.preset_id.clone().into(),
                revision: current_revision
                    .map(|revision| revision.reference.revision + 1)
                    .unwrap_or(1),
                revision_digest,
            }
        };

        let candidate_revision = AgentPresetRevision {
            reference: candidate_revision_ref.clone(),
            payload: payload.clone(),
            contribution_locks: contribution_locks.clone(),
            created_by: owner.clone(),
            created_at_ms: now_ms(),
            reason: None,
        };
        let mut diagnostics = Vec::new();
        if let Err(violation) = candidate_revision.validate() {
            diagnostics.push(error_diagnostic(
                violation.code,
                violation.message,
                None,
            ));
        }
        validate_direct_catalog_availability(&payload, catalog, &mut diagnostics);
        if clean && current_snapshot.is_none() {
            diagnostics.push(error_diagnostic(
                CanonicalErrorCode::from("CAPABILITY_NOT_MATERIALIZED"),
                "the saved Revision has no persisted ResolvedSnapshotRef",
                Some(candidate_revision_ref.preset_id.as_ref().to_owned()),
            ));
        }

        let compiled = if clean || has_errors(&diagnostics) {
            None
        } else {
            let (registry, environment) = canonical_inputs
                .expect("dirty compilation has canonical inputs");
            let request = CompileRequest {
                revision: candidate_revision,
                principal: PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: owner.as_ref().to_owned(),
                },
                scene: "agent_settings".to_owned(),
                surface: "desktop".to_owned(),
                audience: "owner".to_owned(),
                created_at_ms: now_ms(),
                resolver_run_id: OperationId::from(Uuid::now_v7().to_string()),
            };
            match KernelAgentPresetCompiler::compile(&registry, &environment, request) {
                Ok(snapshot) => Some(snapshot),
                Err(error) => {
                    diagnostics.push(kernel_error_diagnostic(&error));
                    None
                }
            }
        };

        let mut snapshot = if has_errors(&diagnostics) {
            None
        } else if clean {
            current_snapshot.cloned()
        } else {
            compiled.as_ref().map(|compiled| compiled.envelope.clone())
        };
        if let (Some(validate), Some(candidate)) = (&self.consumer_validator, &snapshot) {
            let registry = consumer_registry.as_deref().ok_or_else(||
                ControlPlaneError::Wire("consumer validation requires the canonical registry".into()))?;
            if let Err(error) = validate(registry, candidate) {
                diagnostics.push(CompilationDiagnostic {
                    code: error.code().as_ref().to_owned(), message: error.to_string(),
                    subject: None, details: error.details(),
                });
                snapshot = None;
            }
        }
        Ok(PresetCompilation {
            payload,
            contribution_locks,
            candidate_revision_ref,
            snapshot,
            diagnostics,
        })
    }

    fn canonical_inputs(
        &self,
    ) -> Result<(Arc<MaterializedRegistry>, CompilerEnvironment), ControlPlaneError> {
        self.canonical_inputs_if_configured()?.ok_or_else(|| {
            ControlPlaneError::Wire(
                "canonical compiler registry and environment are not configured".to_owned(),
            )
        })
    }

    fn canonical_inputs_if_configured(
        &self,
    ) -> Result<Option<(Arc<MaterializedRegistry>, CompilerEnvironment)>, ControlPlaneError> {
        match (&self.canonical_registry, &self.canonical_environment) {
            (Some(provider), Some(environment)) => {
                Ok(Some((provider.snapshot()?, environment.clone())))
            }
            (None, None) => Ok(None),
            _ => Err(ControlPlaneError::Wire(
                "canonical compiler registry and environment must be configured together"
                    .to_owned(),
            )),
        }
    }
}

fn snapshot_matches_registry(
    snapshot: &ResolvedSnapshotEnvelope,
    registry: &MaterializedRegistry,
) -> Result<bool, ControlPlaneError> {
    for resolved in snapshot
        .content
        .enabled_capabilities
        .iter()
    {
        let Some(current) = registry.capability(&resolved.capability.id) else {
            return Ok(false);
        };
        let manifest_digest = digest_payload(&current.manifest)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        if current.manifest.id != resolved.capability.id
            || current.manifest.package != resolved.source_package
            || current.contribution_id != resolved.contribution_id
            || current.contribution_lock != resolved.contribution_lock
            || resolved.resolved_mount_id.as_ref() != Some(&current.mount_id)
            || current.source != resolved.resolved_source
            || current.target_artifact_digest != resolved.target_artifact_digest
            || current.schema_digest != resolved.schema_digest
            || manifest_digest != resolved.schema_digest
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn has_errors(diagnostics: &[CompilationDiagnostic]) -> bool {
    !diagnostics.is_empty()
}

pub(crate) fn validate_direct_catalog_availability(
    payload: &AgentPresetRevisionPayload,
    catalog: &CatalogSnapshot,
    diagnostics: &mut Vec<CompilationDiagnostic>,
) {
    let mut seen = BTreeSet::new();
    for selection in payload
        .enabled_capabilities
        .iter()
    {
        let reference = &selection.capability;
        if !seen.insert(reference.id.clone()) {
            continue;
        }
        if catalog.find_capability(reference).is_none() {
            diagnostics.push(error_diagnostic(
                CanonicalErrorCode::from("CAPABILITY_NOT_MATERIALIZED"),
                format!(
                    "capability {} is not materialized",
                    reference.id.as_ref()
                ),
                Some(reference.id.as_ref().to_owned()),
            ));
            continue;
        }
        let unavailable_code = catalog
            .formal_capability_entries
            .get(reference)
            .and_then(|entry| {
                entry
                    .availability_for(CapabilityConsumer::Agent)
                    .and_then(availability_code)
            });
        if let Some(code) = unavailable_code {
            diagnostics.push(error_diagnostic(
                code,
                format!("capability {} is unavailable on this host", reference.id.as_ref()),
                Some(reference.id.as_ref().to_owned()),
            ));
        }
    }

    for reference in &payload.skill_bindings {
        if catalog.find_skill(reference).is_none() {
            diagnostics.push(error_diagnostic(
                CanonicalErrorCode::from("CAPABILITY_NOT_MATERIALIZED"),
                format!(
                    "skill {}@{} is not materialized",
                    reference.id.as_ref(),
                    reference.version.as_ref()
                ),
                Some(reference.id.as_ref().to_owned()),
            ));
        }
    }
}

fn contribution_locks_for_payload(
    payload: &AgentPresetRevisionPayload,
    catalog: &CatalogSnapshot,
    registry: &MaterializedRegistry,
) -> Result<Vec<ContributionLock>, ControlPlaneError> {
    let mut locks = Vec::new();
    let mut seen = BTreeSet::new();

    for selection in payload
        .enabled_capabilities
        .iter()
    {
        let Some(catalog_capability) =
            catalog.materialized_capability(&selection.capability)
        else {
            continue;
        };
        if catalog_capability.source.source_kind
            == nomifun_agent_contracts::PluginSourceKind::TestFixture
        {
            // Test-only registrations may drive deterministic Kernel fixtures,
            // but never mint formal Catalog provenance or Revision locks.
            continue;
        }
        catalog
            .capability_catalog_entry(&selection.capability)?
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "CAPABILITY_NOT_MATERIALIZED",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    format!(
                        "capability {} has no formal Catalog entry",
                        selection.capability.id.as_ref()
                    ),
                )
            })?;
        let kernel_capability = registry
            .capability(&selection.capability.id)
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "CAPABILITY_NOT_MATERIALIZED",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    format!(
                        "capability {} is not present in the canonical Kernel registry",
                        selection.capability.id.as_ref()
                    ),
                )
            })?;
        if catalog_capability.contribution_lock
            != kernel_capability.contribution_lock
            || catalog_capability.target_artifact_digest
                != kernel_capability.target_artifact_digest
            || catalog_capability.schema_digest
                != kernel_capability.schema_digest
        {
            return Err(ControlPlaneError::canonical(
                "CAPABILITY_CATALOG_INVALID",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "capability {} differs between the Catalog and canonical Kernel registry",
                    selection.capability.id.as_ref()
                ),
            ));
        }
        let operation_lock =
            kernel_capability.operation_lock(CapabilityConsumer::Agent);
        operation_lock.validate().map_err(|error| {
            ControlPlaneError::canonical(
                "CAPABILITY_CATALOG_INVALID",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                error.to_string(),
            )
        })?;
        let contribution_id = operation_lock.contribution.contribution_id.clone();
        if seen.insert(contribution_id.as_ref().to_owned()) {
            locks.push(operation_lock.contribution);
        }
    }

    for skill in &payload.skill_bindings {
        let Some(catalog_skill) = catalog.materialized_skill(skill) else {
            continue;
        };
        if catalog_skill.source.source_kind
            == nomifun_agent_contracts::PluginSourceKind::TestFixture
        {
            continue;
        }
        let kernel_skill = registry
            .skill(&skill.id)
            .filter(|materialized| {
                materialized.definition.version == skill.version
            })
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "CAPABILITY_NOT_MATERIALIZED",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    format!(
                        "skill {}@{} is not present in the canonical Kernel registry",
                        skill.id.as_ref(),
                        skill.version.as_ref()
                    ),
                )
            })?;
        if catalog_skill.contribution_lock != kernel_skill.contribution_lock
            || catalog_skill.target_artifact_digest
                != kernel_skill.target_artifact_digest
            || catalog_skill.contract_digest != kernel_skill.contract_digest
        {
            return Err(ControlPlaneError::canonical(
                "CAPABILITY_CATALOG_INVALID",
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "skill {}@{} differs between the Catalog and canonical Kernel registry",
                    skill.id.as_ref(),
                    skill.version.as_ref()
                ),
            ));
        }
        kernel_skill
            .contribution_lock
            .validate()
            .map_err(|error| {
                ControlPlaneError::canonical(
                    "CAPABILITY_CATALOG_INVALID",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    error.message,
                )
            })?;
        let contribution_id =
            kernel_skill.contribution_lock.contribution_id.clone();
        if seen.insert(contribution_id.as_ref().to_owned()) {
            locks.push(kernel_skill.contribution_lock.clone());
        }
    }

    locks.sort();
    Ok(locks)
}

fn kernel_error_diagnostic(error: &KernelError) -> CompilationDiagnostic {
    error_diagnostic(
        error.canonical_code(),
        error.to_string(),
        None,
    )
}

fn error_diagnostic(
    code: CanonicalErrorCode,
    message: impl Into<String>,
    subject: Option<String>,
) -> CompilationDiagnostic {
    CompilationDiagnostic {
        code: code.as_ref().to_owned(),
        message: message.into(),
        subject,
        details: None,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

pub fn revision_api(
    revision: &AgentPresetRevision,
) -> Result<AgentPresetRevisionDto, ControlPlaneError> {
    Ok(AgentPresetRevisionDto {
        reference: wire_cast(&revision.reference)?,
        document: wire_cast(&revision.payload)?,
        contribution_locks: wire_cast(&revision.contribution_locks)?,
        created_by: revision.created_by.as_ref().to_owned(),
        created_at_ms: revision.created_at_ms,
        reason: revision.reason.clone(),
    })
}

#[cfg(test)]
#[path = "compiler_role_tests.rs"]
mod role_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        CapabilityCatalogMaterialization, CapabilityCatalogMaterializer,
        CapabilityContributions, CapabilityId, CapabilityKind,
        CapabilityManifest, CapabilityOwner, CapabilityProvenance,
        CapabilityPublicationState, CapabilityRef, CapabilitySelection, ContributionSourceKind,
        CatalogAvailability, ContributionId, DigestHex,
        LocalizedMetadata,
        LogicalArtifactRef, McpBindingId, McpServerId, McpToolCapabilityMapping,
        McpToolKey, PackageId,
        PackageRef, AgentModuleId, PluginSourceKind,
        PluginSourceMetadata, SkillDefinition, SkillId,
        SkillRef, StableSourceIdentity, StrictJsonValue, VersionString,
        capability_surface_declarations,
    };
    use nomifun_agent_kernel::{
        MaterializedCapability, MaterializedMcpTool, MaterializedSkill,
    };
    use std::collections::BTreeMap;

    #[test]
    fn contribution_locks_preserve_exact_managed_skill_and_mcp_facts() {
        let package = PackageRef {
            id: PackageId::from("managed.example"),
            version: VersionString::from("1.0.0"),
        };
        let mount_id = AgentModuleId::from("managed-example-mount");
        let artifact_digest = DigestHex::from("a".repeat(64));
        let source = PluginSourceMetadata {
            source_kind: PluginSourceKind::ManagedLocal,
            source_identity: mount_id.as_ref().to_owned(),
            source_digest: Some(artifact_digest.clone()),
        };
        let capability_ref = CapabilityRef {
            id: CapabilityId::from("managed.example.run"),
        };
        let capability_manifest = CapabilityManifest {
            id: capability_ref.id.clone(),
            contribution_id: ContributionId::from(
                "capability:managed.example.run",
            ),
            kind: CapabilityKind::Tool,
            package: package.clone(),
            display: LocalizedMetadata {
                name: "Managed example".to_owned(),
                description: "Managed MCP-backed fixture".to_owned(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces: capability_surface_declarations(
                ["desktop"],
                [CapabilityConsumer::Agent],
            ),
            requires_runtime_features: Vec::new(),
            supported_platforms: Vec::new(),
            config_schema: StrictJsonValue(json!({"type": "object"})),
            contributions: CapabilityContributions::default(),
        };
        let capability_digest =
            digest_payload(&capability_manifest).unwrap();
        let binding_id =
            McpBindingId::from("managed.server:managed.server.run");
        let capability_lock = ContributionLock {
            source_kind: ContributionSourceKind::McpBinding,
            source_identity: StableSourceIdentity::from(
                "mcp:managed.server",
            ),
            mount_id: Some(mount_id.clone()),
            mcp_binding_id: Some(binding_id.clone()),
            contribution_id: capability_manifest.contribution_id.clone(),
            contract_digest: capability_digest.clone(),
        };
        let materialized_capability = MaterializedCapability {
            manifest: capability_manifest.clone(),
            schema_digest: capability_digest.clone(),
            contribution_id: capability_manifest.contribution_id.clone(),
            contribution_lock: capability_lock.clone(),
            target_artifact_digest: artifact_digest.clone(),
            mount_id: mount_id.clone(),
            source: source.clone(),
        };
        let catalog_entry =
            CapabilityCatalogMaterializer::materialize(
                CapabilityCatalogMaterialization {
                    manifest: capability_manifest,
                    provenance: CapabilityProvenance {
                        owner: CapabilityOwner::Package {
                            package: package.clone(),
                        },
                        source_kind: capability_lock.source_kind,
                        source_identity: capability_lock
                            .source_identity
                            .clone(),
                        mount_id: capability_lock.mount_id.clone(),
                        mcp_binding_id: Some(binding_id.clone()),
                        artifact_digest: Some(artifact_digest.clone()),
                    },
                    publication_state: CapabilityPublicationState::Active,
                    availability: BTreeMap::from([(
                        CapabilityConsumer::Agent,
                        CatalogAvailability::Active,
                    )]),
                },
            )
            .unwrap();
        let skill_definition = SkillDefinition {
            id: SkillId::from("managed.example.skill"),
            version: VersionString::from("1.0.0"),
            package: package.clone(),
            display: LocalizedMetadata {
                name: "Managed skill".to_owned(),
                description: "Managed skill fixture".to_owned(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            body_ref: LogicalArtifactRef {
                artifact_id: "managed.example.skill.body".into(),
                normalized_relative_path: "skills/example/SKILL.md"
                    .to_owned(),
                digest: DigestHex::from("b".repeat(64)),
            },
            resources: Vec::new(),
            requires_capabilities: vec![capability_ref.clone()],
            supported_surfaces: BTreeSet::from(["desktop".to_owned()]),
        };
        let skill_digest = digest_payload(&skill_definition).unwrap();
        let skill_lock = ContributionLock {
            source_kind: ContributionSourceKind::AgentModule,
            source_identity: StableSourceIdentity::from(
                source.source_identity.clone(),
            ),
            mount_id: Some(mount_id.clone()),
            mcp_binding_id: None,
            contribution_id: ContributionId::from(
                "skill:managed.example.skill",
            ),
            contract_digest: skill_digest.clone(),
        };
        let materialized_skill = MaterializedSkill {
            definition: skill_definition.clone(),
            contribution_id: skill_lock.contribution_id.clone(),
            contract_digest: skill_digest,
            contribution_lock: skill_lock.clone(),
            target_artifact_digest: artifact_digest.clone(),
            mount_id: mount_id.clone(),
            source: source.clone(),
        };
        let mapping = McpToolCapabilityMapping {
            package,
            server_id: McpServerId::from("managed.server"),
            canonical_tool_key: McpToolKey::from("managed.server.run"),
            schema_digest: DigestHex::from("c".repeat(64)),
            capability: capability_ref.clone(),
            materialization_version: VersionString::from("1.0.0"),
        };
        let materialized_mcp = MaterializedMcpTool {
            mapping: mapping.clone(),
            binding_id,
            contribution_lock: capability_lock.clone(),
            target_artifact_digest: artifact_digest,
            mount_id,
            source,
        };
        let catalog = CatalogSnapshot {
            capabilities: vec![materialized_capability.clone()],
            role_contracts: Vec::new(),
            role_providers: Vec::new(),
            formal_capability_entries: BTreeMap::from([(
                catalog_entry.capability.clone(),
                catalog_entry,
            )]),
            skills: vec![materialized_skill.clone()],
            mcp_tools: vec![materialized_mcp.clone()],
            unavailable_capabilities: BTreeMap::new(),
            service_key_diagnostics: Vec::new(),
        };
        let mut registry = MaterializedRegistry::empty();
        registry.capabilities.insert(
            capability_ref.id.clone(),
            materialized_capability,
        );
        registry.skills.insert(
            skill_definition.id.clone(),
            materialized_skill,
        );
        let mcp_key = (
            mapping.server_id.clone(),
            mapping.canonical_tool_key.clone(),
        );
        registry
            .mcp_by_capability
            .insert(capability_ref.id.clone(), mcp_key.clone());
        registry.mcp_tools.insert(mcp_key, materialized_mcp);
        let payload = AgentPresetRevisionPayload {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: VersionString::from("1.0.0"),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![CapabilitySelection {
                capability: capability_ref,
                action_allowlist: BTreeSet::new(),
            }],

            skill_bindings: vec![SkillRef {
                id: skill_definition.id,
                version: skill_definition.version,
            }],
            system_role_provider_overrides: BTreeMap::new(),
            persona: "Managed fixture".to_owned(),
            instructions: "Use exact managed contributions.".to_owned(),
            starter_prompts: Vec::new(),
            runtime_policy: Default::default(),
        };

        catalog.validate().unwrap();
        let locks =
            contribution_locks_for_payload(&payload, &catalog, &registry)
                .unwrap();
        assert_eq!(locks.len(), 2);
        assert!(locks.contains(&capability_lock));
        assert!(locks.contains(&skill_lock));
        let api = revision_api(&AgentPresetRevision {
            reference: PresetRevisionRef { preset_id: "preset".into(), revision: 1, revision_digest: "a".repeat(64).into() },
            payload: payload.clone(), contribution_locks: locks.clone(),
            created_by: "owner".into(), created_at_ms: 1, reason: None,
        }).unwrap();
        assert_eq!(serde_json::to_value(api.contribution_locks).unwrap(), serde_json::to_value(&locks).unwrap());
        let catalog_api = catalog.as_api().unwrap();
        assert_eq!(catalog_api.mcp_tools[0].canonical_tool_key, mapping.canonical_tool_key.as_ref());
        assert_eq!(catalog_api.mcp_tools[0].source_package.id, mapping.package.id.as_ref());

        registry
            .skills
            .get_mut(&SkillId::from("managed.example.skill"))
            .unwrap()
            .contribution_lock
            .mount_id = Some(AgentModuleId::from("drifted-mount"));
        let error =
            contribution_locks_for_payload(&payload, &catalog, &registry)
                .unwrap_err();
        assert_eq!(error.code().as_ref(), "CAPABILITY_CATALOG_INVALID");
    }

}
