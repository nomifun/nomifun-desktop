use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nomifun_agent_contracts::{
    AgentPresetRevision, AgentPresetRevisionPayload, CanonicalErrorCode,
    CapabilityCatalogPublication, CapabilityConsumer, CapabilityRef,
    ContributionLock, ContributionSourceKind, ResolvedCapability,
    PluginProductCapabilityCatalogPublication, OfficialPresetKey, OperationId,
    PresetRevisionRef, PrincipalRef, PluginSourceMetadata, PluginSourceKind,
    ResolvedSnapshotEnvelope, UserId, digest_payload,
};
use nomifun_agent_kernel::{
    AgentPresetCompiler as KernelAgentPresetCompiler, CompileRequest, CompilerEnvironment,
    KernelError, KernelRegistry, MaterializedRegistry,
};
use nomifun_api_types::{AgentPresetDraftDto, AgentPresetRevisionDto};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::catalog::{availability_code, CatalogSnapshot, OfficialTemplateCatalog};
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
    official_templates: OfficialTemplateCatalog,
    canonical_registry: Option<Arc<dyn CanonicalRegistryProvider>>,
    canonical_environment: Option<CompilerEnvironment>,
    runtime_validator: Option<Arc<dyn Fn(&AgentPresetRevisionPayload, &ResolvedSnapshotEnvelope) -> Result<(), String> + Send + Sync>>,
}

impl PresetRevisionCompiler {
    pub fn new(official_templates: OfficialTemplateCatalog) -> Self {
        Self {
            official_templates,
            canonical_registry: None,
            canonical_environment: None,
            runtime_validator: None,
        }
    }

    /// Bind revision saves to the exact registry and environment used by
    /// Session Open. The provider is evaluated for every changed draft.
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

    /// The embedding host owns the open runtime catalog and compatibility checks.
    /// Validate engine compatibility before persisting a workbench revision.
    pub fn with_runtime_validator(
        mut self,
        validator: impl Fn(&AgentPresetRevisionPayload, &ResolvedSnapshotEnvelope) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.runtime_validator = Some(Arc::new(validator));
        self
    }

    pub(crate) fn compile(
        &self,
        owner: &UserId,
        draft: &AgentPresetDraftDto,
        current_revision: Option<&AgentPresetRevision>,
        current_snapshot: Option<&ResolvedSnapshotEnvelope>,
        transient_template_key: Option<OfficialPresetKey>,
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
            current_snapshot,
            current_canonical_inputs
                .as_ref()
                .map(|(registry, _)| registry.as_ref()),
        ) {
            (Some(snapshot), Some(registry)) => snapshot_matches_registry(snapshot, registry)?,
            _ => true,
        };
        let clean = payload_unchanged && materialization_unchanged;
        let canonical_inputs = if clean {
            None
        } else if let Some(inputs) = current_canonical_inputs {
            Some(inputs)
        } else {
            Some(self.canonical_inputs()?)
        };
        let plugin_product_capabilities = if clean {
            Vec::new()
        } else {
            resolved_plugin_product_capabilities_for_payload(&payload, catalog)?
        };
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
        validate_template_baseline(
            transient_template_key,
            &self.official_templates,
            &payload,
            self.canonical_environment
                .as_ref()
                .map(|environment| &environment.available_runtime_features),
            &mut diagnostics,
        );
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
            let (registry, mut environment) = canonical_inputs
                .expect("dirty compilation has canonical inputs");
            let selected_capabilities = payload
                .enabled_capabilities
                .iter()
                .map(|selection| selection.capability.id.clone())
                .collect::<BTreeSet<_>>();
            environment.required_runtime_profile = runtime_profile_for_compile(
                environment.required_runtime_profile,
                transient_template_key,
                current_snapshot.map(|snapshot| snapshot.content.required_runtime_profile),
                &selected_capabilities,
                &self
                    .official_templates
                    .required_capability_ids(OfficialPresetKey::CodingCodex)
                    .unwrap_or_default(),
            );
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
                plugin_product_capabilities,
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
        if let (Some(validator), Some(candidate)) = (&self.runtime_validator, &snapshot)
            && let Err(message) = validator(&payload, candidate)
        {
            diagnostics.push(error_diagnostic(
                CanonicalErrorCode::from("AGENT_RUNTIME_ENGINE_UNAVAILABLE"), message, None,
            ));
            snapshot = None;
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
        if resolved.contribution_lock.source_kind
            == ContributionSourceKind::PluginProductActiveRelease
        {
            continue;
        }
        let Some(current) = registry.capability(&resolved.capability.id) else {
            return Ok(false);
        };
        let manifest_digest = digest_payload(&current.manifest)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        if current.manifest.id != resolved.capability.id
            || current.manifest.version != resolved.capability.version
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

fn runtime_profile_for_compile(
    default_profile: nomifun_agent_contracts::RuntimeProfileKind,
    template_key: Option<OfficialPresetKey>,
    current_profile: Option<nomifun_agent_contracts::RuntimeProfileKind>,
    selected_capabilities: &BTreeSet<nomifun_agent_contracts::CapabilityId>,
    coding_required_capabilities: &BTreeSet<nomifun_agent_contracts::CapabilityId>,
) -> nomifun_agent_contracts::RuntimeProfileKind {
    if template_key == Some(OfficialPresetKey::CodingCodex)
        || current_profile == Some(nomifun_agent_contracts::RuntimeProfileKind::CodingNative)
        || (!coding_required_capabilities.is_empty()
            && coding_required_capabilities.is_subset(selected_capabilities))
    {
        nomifun_agent_contracts::RuntimeProfileKind::CodingNative
    } else {
        default_profile
    }
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
                    "capability {}@{} is not materialized",
                    reference.id.as_ref(),
                    reference.version.as_ref()
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

fn resolved_plugin_product_capabilities_for_payload(
    payload: &AgentPresetRevisionPayload,
    catalog: &CatalogSnapshot,
) -> Result<Vec<ResolvedCapability>, ControlPlaneError> {
    let mut capabilities = Vec::new();
    for selection in payload
        .enabled_capabilities
        .iter()
    {
        if let Some(capability) =
            resolved_plugin_product_capability_for_selection(selection, catalog)?
        {
            capabilities.push(capability);
        }
    }
    capabilities.sort_by(|left, right| left.capability.cmp(&right.capability));
    Ok(capabilities)
}

fn resolved_plugin_product_capability_for_selection(
    selection: &nomifun_agent_contracts::CapabilitySelection,
    catalog: &CatalogSnapshot,
) -> Result<Option<ResolvedCapability>, ControlPlaneError> {
    let Some((publication, capability)) =
        plugin_product_publication_for(catalog, &selection.capability)?
    else {
        return Ok(None);
    };
    let operation_lock = plugin_product_catalog_operation_lock(publication, capability)?;
    let manifest = &capability.manifest;
    let required_resource_kinds = capability.entry.typed_resource_kinds.clone();
    if required_resource_kinds != manifest.contributions.resource_kinds {
        return Err(plugin_product_catalog_error(format!(
            "Plugin Product capability {}@{} has inconsistent typed resource requirements",
            selection.capability.id.as_ref(),
            selection.capability.version.as_ref()
        )));
    }

    let resolved = ResolvedCapability {
        capability: capability.entry.capability.clone(),
        source_package: manifest.package.clone(),
        contribution_id: capability.entry.contribution_id.clone(),
        contribution_lock: operation_lock.contribution,
        resolved_mount_id: None,
        resolved_source: PluginSourceMetadata {
            source_kind: PluginSourceKind::ManagedLocal,
            source_identity: capability.entry.provenance.source_identity.as_ref().to_owned(),
            source_digest: Some(publication.active_release.release_digest.clone()),
        },
        target_artifact_digest: publication.active_release.release_digest.clone(),
        schema_digest: capability.entry.contract_digest.clone(),
        dependency_path: vec![capability.entry.capability.id.clone()],
        required_runtime_features: capability.entry.required_runtime_features.clone(),
        plugin_product_id: Some(publication.plugin_product_id.clone()),
        active_release: Some(publication.active_release.clone()),
        active_release_epoch: Some(publication.active_release_epoch),
        catalog_digest: Some(publication.catalog_digest.clone()),
        display_name: Some(manifest.display.name.clone()),
        description: Some(manifest.display.description.clone()),
        actions: manifest.contributions.actions.clone(),
        required_resource_kinds,
        action_allowlist: selection.action_allowlist.clone(),
    };
    resolved
        .validate()
        .map_err(|error| plugin_product_catalog_error(error.message))?;
    Ok(Some(resolved))
}

fn plugin_product_publication_for<'a>(
    catalog: &'a CatalogSnapshot,
    reference: &CapabilityRef,
) -> Result<
    Option<(
        &'a PluginProductCapabilityCatalogPublication,
        &'a CapabilityCatalogPublication,
    )>,
    ControlPlaneError,
> {
    let has_kernel_materialization =
        catalog.materialized_capability(reference).is_some();
    let mut match_value = None;
    for publication in catalog.plugin_product_publications.values() {
        for capability in &publication.capabilities {
            if capability.entry.capability != *reference {
                continue;
            }
            if match_value.is_some() {
                return Err(plugin_product_catalog_error(format!(
                    "Plugin Product capability {}@{} appears in multiple publications",
                    reference.id.as_ref(),
                    reference.version.as_ref()
                )));
            }
            match_value = Some((publication, capability));
        }
    }
    if has_kernel_materialization && match_value.is_some() {
        return Err(plugin_product_catalog_error(format!(
            "capability {}@{} is present in both the Kernel registry and a Plugin Product publication",
            reference.id.as_ref(),
            reference.version.as_ref()
        )));
    }
    Ok(match_value)
}

fn plugin_product_catalog_operation_lock(
    publication: &PluginProductCapabilityCatalogPublication,
    capability: &CapabilityCatalogPublication,
) -> Result<nomifun_agent_contracts::CapabilityOperationLock, ControlPlaneError> {
    publication
        .validate()
        .map_err(|error| plugin_product_catalog_error(error.to_string()))?;
    capability
        .validate()
        .map_err(|error| plugin_product_catalog_error(error.to_string()))?;
    let operation_lock = capability
        .entry
        .operation_lock(CapabilityConsumer::Agent)
        .map_err(|error| {
            ControlPlaneError::canonical(
                error.code(),
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                error.to_string(),
            )
        })?;
    operation_lock
        .validate()
        .map_err(|error| plugin_product_catalog_error(error.to_string()))?;
    let contribution = &operation_lock.contribution;
    if operation_lock.capability != capability.entry.capability
        || contribution.source_kind != ContributionSourceKind::PluginProductActiveRelease
        || contribution.plugin_product_id.as_ref() != Some(&publication.plugin_product_id)
        || contribution.mount_id.is_some()
        || contribution.mcp_binding_id.is_some()
        || contribution.contribution_id != capability.entry.contribution_id
        || contribution.contract_digest != capability.entry.contract_digest
        || operation_lock.target_artifact_digest.as_ref()
            != Some(&publication.active_release.release_digest)
    {
        return Err(plugin_product_catalog_error(format!(
            "Catalog operation lock for Plugin Product capability {}@{} does not bind the exact Active Release",
            capability.entry.capability.id.as_ref(),
            capability.entry.capability.version.as_ref()
        )));
    }
    Ok(operation_lock)
}

fn plugin_product_catalog_error(message: impl Into<String>) -> ControlPlaneError {
    ControlPlaneError::canonical(
        "CAPABILITY_CATALOG_INVALID",
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        message,
    )
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
        if let Some(plugin_product_capability) =
            resolved_plugin_product_capability_for_selection(selection, catalog)?
        {
            let contribution_id = plugin_product_capability.contribution_id.clone();
            if seen.insert(contribution_id.as_ref().to_owned()) {
                locks.push(plugin_product_capability.contribution_lock);
            }
            continue;
        }
        let Some(catalog_capability) =
            catalog.materialized_capability(&selection.capability)
        else {
            continue;
        };
        if catalog_capability.source.source_kind
            == nomifun_agent_contracts::PluginSourceKind::TestFixture
        {
            // TestHost registrations may drive deterministic Kernel fixtures,
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
                        "capability {}@{} has no formal Catalog entry",
                        selection.capability.id.as_ref(),
                        selection.capability.version.as_ref()
                    ),
                )
            })?;
        let kernel_capability = registry
            .capability(&selection.capability.id)
            .filter(|materialized| {
                materialized.manifest.version == selection.capability.version
            })
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "CAPABILITY_NOT_MATERIALIZED",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    format!(
                        "capability {}@{} is not present in the canonical Kernel registry",
                        selection.capability.id.as_ref(),
                        selection.capability.version.as_ref()
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
                    "capability {}@{} differs between the Catalog and canonical Kernel registry",
                    selection.capability.id.as_ref(),
                    selection.capability.version.as_ref()
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

fn validate_template_baseline(
    template_key: Option<OfficialPresetKey>,
    templates: &OfficialTemplateCatalog,
    payload: &AgentPresetRevisionPayload,
    available_runtime_features: Option<
        &BTreeSet<nomifun_agent_contracts::RuntimeFeatureId>,
    >,
    diagnostics: &mut Vec<CompilationDiagnostic>,
) {
    if template_key != Some(OfficialPresetKey::CodingCodex) {
        return;
    }
    let selected = payload
        .enabled_capabilities
        .iter()
        .map(|selection| selection.capability.id.clone())
        .collect::<BTreeSet<_>>();
    let missing_capabilities = templates
        .required_capability_ids(OfficialPresetKey::CodingCodex)
        .unwrap_or_default()
        .difference(&selected)
        .map(|id| id.as_ref().to_owned())
        .collect::<Vec<_>>();
    // Runtime availability belongs to the validated CompilerEnvironment.
    // A capability manifest declares what that capability requires; aggregating
    // those declarations here inverted the relationship and made a complete
    // host look empty whenever the selected capabilities had no dependencies.
    let available_features = available_runtime_features.cloned().unwrap_or_default();
    let missing_features = templates
        .required_runtime_features(OfficialPresetKey::CodingCodex)
        .unwrap_or_default()
        .difference(&available_features)
        .map(|feature| feature.as_ref().to_owned())
        .collect::<Vec<_>>();
    if !missing_capabilities.is_empty() || !missing_features.is_empty() {
        diagnostics.push(CompilationDiagnostic {
            code: "CODING_CODEX_NATIVE_INCOMPLETE".into(),
            message: "coding.codex must retain the complete frozen Coding capability and runtime-feature baseline".into(),
            subject: Some("coding.codex".into()),
            details: Some(json!({
                "missing_capability_ids": missing_capabilities,
                "missing_runtime_features": missing_features,
            })),
        });
    }
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
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        ActionId, ArtifactId, CapabilityActionDescriptor,
        CapabilityCatalogMaterialization, CapabilityCatalogMaterializer,
        CapabilityContributions, CapabilityId, CapabilityKind,
        CapabilityManifest, CapabilityOwner, CapabilityProvenance,
        CapabilityRef, CapabilityReleaseState, CapabilitySelection,
        CanonicalSchemaRef, CatalogAvailability, ContributionId, DigestHex, EffectClass,
        LocalizedMetadata,
        LogicalArtifactRef, McpBindingId, McpServerId, McpToolCapabilityMapping,
        McpToolKey, PluginProductId, PluginReleaseId, PluginReleaseRef, PackageId,
        PackageRef, PlatformConstraint, PluginMountId, PluginSourceKind,
        PluginSourceMetadata, ResourceKind, RuntimeProfileKind, SkillDefinition, SkillId,
        SkillRef, StableSourceIdentity, StrictJsonValue, ToolPresentationKind, VersionString,
        capability_surface_declarations,
    };
    use nomifun_agent_kernel::{
        MaterializedCapability, MaterializedMcpTool, MaterializedSkill,
    };
    use std::collections::BTreeMap;

    #[test]
    fn coding_template_baseline_uses_the_validated_runtime_environment_inventory() {
        let templates = OfficialTemplateCatalog::load().unwrap();
        let seed = templates.seed(OfficialPresetKey::CodingCodex).unwrap();
        let payload = AgentPresetRevisionPayload {
            runtime_engine: None,
            schema_version: VersionString::from("1.0.0"),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: seed
                .enabled_capabilities
                .iter()
                .cloned()
                .map(|capability| CapabilitySelection {
                    capability,
                    action_allowlist: BTreeSet::new(),
                })
                .collect(),
            skill_bindings: seed.skill_bindings.clone(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: String::new(),
            instructions: String::new(),
            starter_prompts: Vec::new(),
        };
        let available = templates
            .required_runtime_features(OfficialPresetKey::CodingCodex)
            .unwrap();
        let mut diagnostics = Vec::new();
        validate_template_baseline(
            Some(OfficialPresetKey::CodingCodex),
            &templates,
            &payload,
            Some(&available),
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty());

        validate_template_baseline(
            Some(OfficialPresetKey::CodingCodex),
            &templates,
            &payload,
            Some(&BTreeSet::new()),
            &mut diagnostics,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "CODING_CODEX_NATIVE_INCOMPLETE");
        assert_eq!(
            diagnostics[0].details.as_ref().unwrap()["missing_runtime_features"]
                .as_array()
                .unwrap()
                .len(),
            available.len()
        );
    }

    #[test]
    fn coding_profile_survives_template_provenance_and_saved_snapshot_reloads() {
        let coding_required =
            BTreeSet::from([CapabilityId::from("fs.read"), CapabilityId::from("process.exec")]);

        assert_eq!(
            runtime_profile_for_compile(
                RuntimeProfileKind::ManagedMinimal,
                Some(OfficialPresetKey::CodingCodex),
                None,
                &BTreeSet::new(),
                &coding_required,
            ),
            RuntimeProfileKind::CodingNative
        );
        assert_eq!(
            runtime_profile_for_compile(
                RuntimeProfileKind::ManagedMinimal,
                None,
                Some(RuntimeProfileKind::CodingNative),
                &BTreeSet::new(),
                &coding_required,
            ),
            RuntimeProfileKind::CodingNative
        );
        assert_eq!(
            runtime_profile_for_compile(
                RuntimeProfileKind::ManagedMinimal,
                None,
                None,
                &coding_required,
                &coding_required,
            ),
            RuntimeProfileKind::CodingNative
        );
        assert_eq!(
            runtime_profile_for_compile(
                RuntimeProfileKind::ManagedMinimal,
                None,
                None,
                &BTreeSet::from([CapabilityId::from("fs.read")]),
                &coding_required,
            ),
            RuntimeProfileKind::ManagedMinimal
        );
    }

    #[test]
    fn contribution_locks_preserve_exact_managed_skill_and_mcp_facts() {
        let package = PackageRef {
            id: PackageId::from("managed.example"),
            version: VersionString::from("1.0.0"),
        };
        let mount_id = PluginMountId::from("managed-example-mount");
        let artifact_digest = DigestHex::from("a".repeat(64));
        let source = PluginSourceMetadata {
            source_kind: PluginSourceKind::ManagedLocal,
            source_identity: mount_id.as_ref().to_owned(),
            source_digest: Some(artifact_digest.clone()),
        };
        let capability_ref = CapabilityRef {
            id: CapabilityId::from("managed.example.run"),
            version: VersionString::from("1.0.0"),
        };
        let capability_manifest = CapabilityManifest {
            id: capability_ref.id.clone(),
            contribution_id: ContributionId::from(
                "capability:managed.example.run",
            ),
            version: capability_ref.version.clone(),
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
            plugin_product_id: None,
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
                        plugin_product_id: None,
                        mcp_binding_id: Some(binding_id.clone()),
                        artifact_digest: Some(artifact_digest.clone()),
                    },
                    release_state:
                        CapabilityReleaseState::PublishedActive,
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
            source_kind: ContributionSourceKind::PluginMount,
            source_identity: StableSourceIdentity::from(
                source.source_identity.clone(),
            ),
            mount_id: Some(mount_id.clone()),
            plugin_product_id: None,
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
            formal_capability_entries: BTreeMap::from([(
                catalog_entry.capability.clone(),
                catalog_entry,
            )]),
            plugin_product_publications: BTreeMap::new(),
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
            runtime_engine: None,
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
            .mount_id = Some(PluginMountId::from("drifted-mount"));
        let error =
            contribution_locks_for_payload(&payload, &catalog, &registry)
                .unwrap_err();
        assert_eq!(error.code().as_ref(), "CAPABILITY_CATALOG_INVALID");
    }

    #[test]
    fn plugin_product_contribution_lock_uses_catalog_without_kernel_materialization() {
        let package = PackageRef {
            id: PackageId::from("plugin.example"),
            version: VersionString::from("1.0.0"),
        };
        let plugin_product_id = PluginProductId::from("plugin-example");
        let capability_ref = CapabilityRef {
            id: CapabilityId::from("plugin.example.search"),
            version: VersionString::from("1.0.0"),
        };
        let action = CapabilityActionDescriptor {
            action_id: ActionId::from("plugin.example.search.invoke"),
            input_schema: CanonicalSchemaRef::from("schema://plugin.example/search-input@1"),
            output_schema: CanonicalSchemaRef::from(
                "schema://plugin.example/search-output@1",
            ),
            effect_class: EffectClass::ReadSensitive,
            presentation: ToolPresentationKind::FunctionTool,
        };
        let manifest = CapabilityManifest {
            id: capability_ref.id.clone(),
            contribution_id: ContributionId::from("capability:plugin.example.search"),
            version: capability_ref.version.clone(),
            kind: CapabilityKind::Tool,
            package: package.clone(),
            display: LocalizedMetadata {
                name: "Plugin Search".to_owned(),
                description: "Search through the selected resource.".to_owned(),
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
            supported_platforms: vec![PlatformConstraint::Any],
            config_schema: StrictJsonValue(json!({"type": "object"})),
            contributions: CapabilityContributions {
                actions: vec![action.clone()],
                resource_kinds: BTreeSet::from([ResourceKind::from("knowledge.base")]),
                ..Default::default()
            },
        };
        let active_release = PluginReleaseRef {
            release_id: PluginReleaseId::from("release-1"),
            artifact_id: ArtifactId::from("artifact-1"),
            release_digest: DigestHex::from("a".repeat(64)),
            manifest_digest: DigestHex::from("b".repeat(64)),
        };
        let entry = CapabilityCatalogMaterializer::materialize(
            CapabilityCatalogMaterialization {
                manifest: manifest.clone(),
                provenance: CapabilityProvenance {
                    owner: CapabilityOwner::Package {
                        package: package.clone(),
                    },
                    source_kind: ContributionSourceKind::PluginProductActiveRelease,
                    source_identity: StableSourceIdentity::from(
                        "plugin-product:plugin-example",
                    ),
                    mount_id: None,
                    plugin_product_id: Some(plugin_product_id.clone()),
                    mcp_binding_id: None,
                    artifact_digest: Some(active_release.release_digest.clone()),
                },
                release_state: CapabilityReleaseState::PublishedActive,
                availability: BTreeMap::from([(
                    CapabilityConsumer::Agent,
                    CatalogAvailability::Active,
                )]),
            },
        )
        .unwrap();
        let mut publication = PluginProductCapabilityCatalogPublication {
            plugin_product_id: plugin_product_id.clone(),
            active_release: active_release.clone(),
            active_release_epoch: 7,
            catalog_digest: DigestHex::from("0".repeat(64)),
            capabilities: vec![CapabilityCatalogPublication {
                manifest,
                entry: entry.clone(),
            }],
        };
        publication.catalog_digest = publication.computed_catalog_digest().unwrap();
        publication.validate().unwrap();

        let catalog = CatalogSnapshot {
            capabilities: Vec::new(),
            formal_capability_entries: BTreeMap::from([(
                entry.capability.clone(),
                entry.clone(),
            )]),
            plugin_product_publications: BTreeMap::from([(
                plugin_product_id.clone(),
                publication.clone(),
            )]),
            skills: Vec::new(),
            mcp_tools: Vec::new(),
            unavailable_capabilities: BTreeMap::new(),
            service_key_diagnostics: Vec::new(),
        };
        catalog.validate().unwrap();

        let action_allowlist = BTreeSet::from([action.action_id.clone()]);
        let payload = AgentPresetRevisionPayload {
            runtime_engine: None,
            schema_version: VersionString::from("1.0.0"),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![CapabilitySelection {
                capability: capability_ref.clone(),
                action_allowlist: action_allowlist.clone(),
            }],

            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "Plugin fixture".to_owned(),
            instructions: "Use the exact Catalog lock.".to_owned(),
            starter_prompts: Vec::new(),
        };

        let registry = MaterializedRegistry::empty();
        let locks = contribution_locks_for_payload(&payload, &catalog, &registry).unwrap();
        assert_eq!(locks, vec![ContributionLock {
            source_kind: ContributionSourceKind::PluginProductActiveRelease,
            source_identity: StableSourceIdentity::from(
                "plugin-product:plugin-example",
            ),
            mount_id: None,
            plugin_product_id: Some(plugin_product_id.clone()),
            mcp_binding_id: None,
            contribution_id: entry.contribution_id.clone(),
            contract_digest: entry.contract_digest.clone(),
        }]);

        let resolved =
            resolved_plugin_product_capability_for_selection(&payload.enabled_capabilities[0], &catalog)
                .unwrap()
                .unwrap();
        assert_eq!(resolved.capability, capability_ref);
        assert_eq!(resolved.plugin_product_id, Some(plugin_product_id));
        assert_eq!(resolved.active_release, Some(active_release));
        assert_eq!(resolved.active_release_epoch, Some(7));
        assert_eq!(resolved.catalog_digest, Some(publication.catalog_digest));
        assert_eq!(resolved.source_package, package);
        assert_eq!(resolved.actions, vec![action]);
        assert_eq!(resolved.required_resource_kinds, BTreeSet::from([
            ResourceKind::from("knowledge.base"),
        ]));
        assert_eq!(resolved.action_allowlist, action_allowlist);
    }
}
