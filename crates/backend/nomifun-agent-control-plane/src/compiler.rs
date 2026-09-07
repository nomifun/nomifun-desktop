use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nomifun_agent_contracts::{
    AgentPresetRevision, AgentPresetRevisionPayload, CanonicalErrorCode,
    CapabilityConsumer, ContributionId, ContributionLock, ContributionSourceKind, DigestHex,
    McpToolCapabilityMapping, OfficialPresetKey, OperationId, PluginMountId,
    PresetRevisionRef, PrincipalRef, ResolvedCapability, ResolvedSnapshotEnvelope,
    PluginSourceKind, PluginSourceMetadata, SkillRef,
    StableSourceIdentity, UserId, VersionString, digest_payload,
};
use nomifun_agent_kernel::{
    AgentPresetCompiler as KernelAgentPresetCompiler, CompileRequest, CompilerEnvironment,
    KernelError, KernelRegistry, MaterializedRegistry,
};
use nomifun_api_types::{
    AgentPresetRevisionDto, ContributionLockDto, McpToolCatalogItemDto, PreviewCapabilityDto, PreviewDiagnosticDto,
    PreviewDiagnosticSeverityDto, PreviewStatusDto, PreviewSummaryDto,
    ResolveAgentPresetPreviewRequest, ResolveAgentPresetPreviewResponse, RevisionDiffDto,
    SnapshotInspectorDto,
};
use serde_json::json;
use uuid::Uuid;

use crate::catalog::{CatalogSnapshot, OfficialTemplateCatalog};
use crate::error::ControlPlaneError;
use crate::wire::{wire_cast, wire_name};

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
pub struct CompilerReleaseInputs {
    pub resolver_version: VersionString,
    pub runtime_protocol_version: VersionString,
    pub runtime_feature_inventory_digest: DigestHex,
    pub canonical_schema_manifest_digest: DigestHex,
    pub target_contribution_manifest_digest: DigestHex,
    pub availability_evidence_revision: String,
}

#[derive(Clone, Debug)]
pub struct PreviewCompilation {
    pub response: ResolveAgentPresetPreviewResponse,
    pub payload: AgentPresetRevisionPayload,
    pub contribution_locks: Vec<ContributionLock>,
    pub candidate_revision_ref: PresetRevisionRef,
    pub snapshot: Option<ResolvedSnapshotEnvelope>,
}

#[derive(Clone)]
pub struct PresetPreviewCompiler {
    release: CompilerReleaseInputs,
    official_templates: OfficialTemplateCatalog,
    canonical_registry: Option<Arc<dyn CanonicalRegistryProvider>>,
    canonical_environment: Option<CompilerEnvironment>,
}

impl PresetPreviewCompiler {
    pub fn new(
        release: CompilerReleaseInputs,
        official_templates: OfficialTemplateCatalog,
    ) -> Self {
        Self {
            release,
            official_templates,
            canonical_registry: None,
            canonical_environment: None,
        }
    }

    /// Bind Preview/Save/Test to the exact registry and environment used by
    /// Session Open. The provider is evaluated for every dirty compile.
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

    pub fn compile(
        &self,
        owner: &UserId,
        request: &ResolveAgentPresetPreviewRequest,
        current_revision: Option<&AgentPresetRevision>,
        current_snapshot: Option<&ResolvedSnapshotEnvelope>,
        transient_template_key: Option<OfficialPresetKey>,
        catalog: &CatalogSnapshot,
    ) -> Result<PreviewCompilation, ControlPlaneError> {
        catalog.validate()?;
        let payload: AgentPresetRevisionPayload = wire_cast(&request.draft.document)?;
        let draft_digest = digest_payload(&payload)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        let clean = current_revision.is_some_and(|current| current.payload == payload);
        let canonical_inputs = if clean {
            None
        } else {
            Some(self.canonical_inputs()?)
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
                preset_id: request.draft.preset_id.clone().into(),
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
            catalog,
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
                .initial_capabilities
                .iter()
                .chain(&payload.on_demand_capabilities)
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
                scene: request.scene.clone(),
                surface: request.surface.clone(),
                audience: request.audience.clone(),
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

        let snapshot = if has_errors(&diagnostics) {
            None
        } else if clean {
            current_snapshot.cloned()
        } else {
            compiled.as_ref().map(|compiled| compiled.envelope.clone())
        };
        let revision_diff = revision_diff(current_revision, &payload);
        let summary = preview_summary(&payload, catalog, snapshot.as_ref());
        let inspector = preview_inspector(
            &self.release,
            &candidate_revision_ref,
            &payload,
            catalog,
            snapshot.as_ref(),
        )?;
        let resolved_snapshot_ref = snapshot
            .as_ref()
            .map(|snapshot| wire_cast(&snapshot.snapshot_ref))
            .transpose()?;
        let preview_digest = digest_payload(&json!({
            "draft_digest": &draft_digest,
            "candidate_revision_ref": &candidate_revision_ref,
            "resolved_snapshot_ref": snapshot.as_ref().map(|value| &value.snapshot_ref),
            "diagnostics": &diagnostics,
        }))
        .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        let ready = snapshot.is_some() && !has_errors(&diagnostics);
        let response = ResolveAgentPresetPreviewResponse {
            status: if ready {
                PreviewStatusDto::Ready
            } else {
                PreviewStatusDto::Blocked
            },
            draft_digest: draft_digest.as_ref().to_owned(),
            preview_digest: preview_digest.as_ref().to_owned(),
            candidate_revision_ref: wire_cast(&candidate_revision_ref)?,
            resolved_snapshot_ref,
            summary,
            diagnostics,
            revision_diff,
            inspector,
            can_save_revision: ready,
            can_create_session: ready,
        };

        Ok(PreviewCompilation {
            response,
            payload,
            contribution_locks,
            candidate_revision_ref,
            snapshot,
        })
    }

    fn canonical_inputs(
        &self,
    ) -> Result<(Arc<MaterializedRegistry>, CompilerEnvironment), ControlPlaneError> {
        match (&self.canonical_registry, &self.canonical_environment) {
            (Some(provider), Some(environment)) => Ok((provider.snapshot()?, environment.clone())),
            (None, None) => Err(ControlPlaneError::Wire(
                "canonical compiler registry and environment are not configured".to_owned(),
            )),
            _ => Err(ControlPlaneError::Wire(
                "canonical compiler registry and environment must be configured together"
                    .to_owned(),
            )),
        }
    }
}

fn has_errors(diagnostics: &[PreviewDiagnosticDto]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == PreviewDiagnosticSeverityDto::Error)
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

fn validate_direct_catalog_availability(
    payload: &AgentPresetRevisionPayload,
    catalog: &CatalogSnapshot,
    diagnostics: &mut Vec<PreviewDiagnosticDto>,
) {
    let mut seen = BTreeSet::new();
    for selection in payload
        .initial_capabilities
        .iter()
        .chain(&payload.on_demand_capabilities)
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
                    .and_then(catalog_unavailable_code)
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
        .initial_capabilities
        .iter()
        .chain(payload.on_demand_capabilities.iter())
    {
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

fn catalog_unavailable_code(
    availability: &nomifun_agent_contracts::CatalogAvailability,
) -> Option<CanonicalErrorCode> {
    match availability {
        nomifun_agent_contracts::CatalogAvailability::Active => None,
        nomifun_agent_contracts::CatalogAvailability::Unavailable { reason }
        | nomifun_agent_contracts::CatalogAvailability::Disabled { reason } => {
            Some(CanonicalErrorCode::from(reason.clone()))
        }
        nomifun_agent_contracts::CatalogAvailability::NeedsRuntime { .. } => {
            Some(CanonicalErrorCode::from("CAPABILITY_NEEDS_RUNTIME"))
        }
        nomifun_agent_contracts::CatalogAvailability::ContractMismatch { .. } => {
            Some(CanonicalErrorCode::from(
                "CAPABILITY_CONTRACT_MISMATCH",
            ))
        }
    }
}

fn validate_template_baseline(
    template_key: Option<OfficialPresetKey>,
    templates: &OfficialTemplateCatalog,
    payload: &AgentPresetRevisionPayload,
    catalog: &CatalogSnapshot,
    diagnostics: &mut Vec<PreviewDiagnosticDto>,
) {
    if template_key != Some(OfficialPresetKey::CodingCodex) {
        return;
    }
    let selected = payload
        .initial_capabilities
        .iter()
        .chain(&payload.on_demand_capabilities)
        .map(|selection| selection.capability.id.clone())
        .collect::<BTreeSet<_>>();
    let missing_capabilities = templates
        .required_capability_ids(OfficialPresetKey::CodingCodex)
        .unwrap_or_default()
        .difference(&selected)
        .map(|id| id.as_ref().to_owned())
        .collect::<Vec<_>>();
    let available_features = selected
        .iter()
        .filter_map(|id| {
            catalog
                .capabilities
                .iter()
                .find(|capability| &capability.manifest.id == id)
        })
        .flat_map(|capability| {
            capability.manifest.requires_runtime_features.iter()
        })
        .map(|feature| feature.id.clone())
        .collect::<BTreeSet<_>>();
    let missing_features = templates
        .required_runtime_features(OfficialPresetKey::CodingCodex)
        .unwrap_or_default()
        .difference(&available_features)
        .map(|feature| feature.as_ref().to_owned())
        .collect::<Vec<_>>();
    if !missing_capabilities.is_empty() || !missing_features.is_empty() {
        diagnostics.push(PreviewDiagnosticDto {
            severity: PreviewDiagnosticSeverityDto::Error,
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

fn kernel_error_diagnostic(error: &KernelError) -> PreviewDiagnosticDto {
    error_diagnostic(
        error.canonical_code(),
        error.to_string(),
        None,
    )
}

fn preview_summary(
    payload: &AgentPresetRevisionPayload,
    catalog: &CatalogSnapshot,
    snapshot: Option<&ResolvedSnapshotEnvelope>,
) -> PreviewSummaryDto {
    let initial_ids = snapshot
        .map(|snapshot| {
            snapshot
                .content
                .initial_capabilities
                .iter()
                .map(|capability| capability.capability.id.clone())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_else(|| {
            payload
                .initial_capabilities
                .iter()
                .map(|selection| selection.capability.id.clone())
                .collect()
        });
    let selected_ids = snapshot
        .map(|snapshot| snapshot.content.capability_allowlist.clone())
        .unwrap_or_else(|| {
            payload
                .initial_capabilities
                .iter()
                .chain(&payload.on_demand_capabilities)
                .map(|selection| selection.capability.id.clone())
                .collect()
        });
    let initial_manifests = catalog
        .capabilities
        .iter()
        .filter(|capability| {
            initial_ids.contains(&capability.manifest.id)
        })
        .collect::<Vec<_>>();
    let required_resource_kinds = catalog
        .capabilities
        .iter()
        .filter(|capability| {
            selected_ids.contains(&capability.manifest.id)
        })
        .flat_map(|capability| {
            capability
                .manifest
                .contributions
                .resource_kinds
                .iter()
        })
        .collect::<BTreeSet<_>>();
    PreviewSummaryDto {
        initial_count: payload.initial_capabilities.len() as u32,
        on_demand_count: payload.on_demand_capabilities.len() as u32,
        active_at_start_count: initial_manifests.len() as u32,
        model_tool_count: initial_manifests
            .iter()
            .map(|capability| {
                capability.manifest.contributions.actions.len() as u32
            })
            .sum(),
        context_contributor_count: initial_manifests
            .iter()
            .map(|capability| {
                capability
                    .manifest
                    .contributions
                    .context_schema_refs
                    .len() as u32
            })
            .sum(),
        on_demand_index_count: snapshot
            .map(|snapshot| snapshot.content.compact_on_demand_index.len() as u32)
            .unwrap_or(payload.on_demand_capabilities.len() as u32),
        skill_count: payload.skill_bindings.len() as u32,
        mcp_count: catalog
            .mcp_tools
            .iter()
            .filter(|mcp| {
                selected_ids.contains(&mcp.mapping.capability.id)
            })
            .count() as u32,
        required_resource_kind_count: required_resource_kinds.len() as u32,
        provider_initialization_count: payload.model_route_refs.len() as u32,
    }
}

fn preview_resolved_capability(
    selection: &nomifun_agent_contracts::CapabilitySelection,
    catalog: &CatalogSnapshot,
) -> ResolvedCapability {
    let manifest = catalog.find_capability(&selection.capability);
    let source_package = manifest
        .map(|capability| capability.package.clone())
        .unwrap_or_else(|| nomifun_agent_contracts::PackageRef {
            id: "unmaterialized".into(),
            version: "0.0.0".into(),
        });
    let contribution_id = manifest
        .map(|capability| capability.contribution_id.clone())
        .unwrap_or_else(|| {
            ContributionId::from(format!(
                "unmaterialized:{}",
                selection.capability.id.as_ref()
            ))
        });
    ResolvedCapability {
        capability: selection.capability.clone(),
        source_package,
        contribution_id: contribution_id.clone(),
        contribution_lock: ContributionLock {
            source_kind: ContributionSourceKind::PlatformBuiltin,
            source_identity: StableSourceIdentity::from("unmaterialized"),
            mount_id: None,
            miniapp_id: None,
            mcp_binding_id: None,
            contribution_id,
            contract_digest: DigestHex::from("0".repeat(64)),
        },
        resolved_mount_id: PluginMountId::from("unmaterialized"),
        resolved_source: PluginSourceMetadata {
            source_kind: PluginSourceKind::Bundled,
            source_identity: "unmaterialized".to_owned(),
            source_digest: None,
        },
        target_artifact_digest: DigestHex::from("0".repeat(64)),
        schema_digest: DigestHex::from("0".repeat(64)),
        dependency_path: vec![selection.capability.id.clone()],
        required_runtime_features: BTreeSet::new(),
    }
}

fn preview_inspector(
    release: &CompilerReleaseInputs,
    candidate_revision_ref: &PresetRevisionRef,
    payload: &AgentPresetRevisionPayload,
    catalog: &CatalogSnapshot,
    snapshot: Option<&ResolvedSnapshotEnvelope>,
) -> Result<SnapshotInspectorDto, ControlPlaneError> {
    let initial_refs = snapshot
        .map(|snapshot| snapshot.content.initial_capabilities.clone())
        .unwrap_or_else(|| {
            payload
                .initial_capabilities
                .iter()
                .map(|selection| preview_resolved_capability(selection, catalog))
                .collect()
        });
    let on_demand_refs = snapshot
        .map(|snapshot| snapshot.content.on_demand_capabilities.clone())
        .unwrap_or_else(|| {
            payload
                .on_demand_capabilities
                .iter()
                .map(|selection| preview_resolved_capability(selection, catalog))
                .collect()
        });
    let selected_ids = snapshot
        .map(|snapshot| snapshot.content.capability_allowlist.clone())
        .unwrap_or_else(|| {
            payload
                .initial_capabilities
                .iter()
                .chain(&payload.on_demand_capabilities)
                .map(|selection| selection.capability.id.clone())
                .collect()
        });
    let mut tool_schema_refs = BTreeSet::new();
    let mut context_schema_refs = BTreeSet::new();
    for reference in initial_refs.iter().chain(on_demand_refs.iter()) {
        if let Some(capability) = catalog.find_capability(&reference.capability) {
            for action in &capability.contributions.actions {
                tool_schema_refs.insert(action.input_schema.as_ref().to_owned());
                tool_schema_refs.insert(action.output_schema.as_ref().to_owned());
            }
            context_schema_refs.extend(
                capability
                    .contributions
                    .context_schema_refs
                    .iter()
                    .map(|reference| reference.as_ref().to_owned()),
            );
        }
    }
    let initial = initial_refs
        .iter()
        .map(|reference| preview_capability(reference, catalog))
        .collect();
    let on_demand = on_demand_refs
        .iter()
        .map(|reference| preview_capability(reference, catalog))
        .collect();
    let mcp_materializations = catalog
        .mcp_tools
        .iter()
        .filter(|mcp| {
            selected_ids.contains(&mcp.mapping.capability.id)
        })
        .map(|mcp| mcp_mapping_api(&mcp.mapping))
        .collect();
    let required_resource_kinds = catalog
        .capabilities
        .iter()
        .filter(|capability| {
            selected_ids.contains(&capability.manifest.id)
        })
        .flat_map(|capability| {
            capability
                .manifest
                .contributions
                .resource_kinds
                .iter()
                .map(|kind| kind.as_ref().to_owned())
        })
        .collect();
    Ok(SnapshotInspectorDto {
        snapshot_ref: snapshot
            .map(|snapshot| wire_cast(&snapshot.snapshot_ref))
            .transpose()?,
        preset_revision_ref: Some(wire_cast(candidate_revision_ref)?),
        runtime_profile: snapshot
            .map(|snapshot| wire_name(&snapshot.content.required_runtime_profile))
            .transpose()?,
        required_runtime_protocol_version: release.runtime_protocol_version.as_ref().to_owned(),
        required_runtime_features: snapshot
            .map(|snapshot| {
                snapshot
                    .content
                    .required_runtime_features
                    .iter()
                    .map(|feature| feature.as_ref().to_owned())
                    .collect()
            })
            .unwrap_or_default(),
        initial_capabilities: initial,
        on_demand_capabilities: on_demand,
        compact_on_demand_index: snapshot
            .map(|snapshot| {
                snapshot
                    .content
                    .compact_on_demand_index
                    .iter()
                    .map(|entry| entry.capability_id.as_ref().to_owned())
                    .collect()
            })
            .unwrap_or_else(|| {
                payload
                    .on_demand_capabilities
                    .iter()
                    .map(|selection| selection.capability.id.as_ref().to_owned())
                    .collect()
            }),
        tool_schema_refs: tool_schema_refs.into_iter().collect(),
        context_schema_refs: context_schema_refs.into_iter().collect(),
        mcp_materializations,
        required_resource_kinds,
        service_key_diagnostics: catalog.service_key_diagnostics.clone(),
    })
}

fn preview_capability(
    reference: &ResolvedCapability,
    catalog: &CatalogSnapshot,
) -> PreviewCapabilityDto {
    let (display_name, source_package) = catalog
        .find_capability(&reference.capability)
        .map(|capability| {
            (
                capability.display.name.clone(),
                capability.package.clone(),
            )
        })
        .unwrap_or_else(|| {
            (
                reference.capability.id.as_ref().to_owned(),
                reference.source_package.clone(),
            )
        });
    PreviewCapabilityDto {
        capability: nomifun_api_types::ExactCatalogRefDto {
            id: reference.capability.id.as_ref().to_owned(),
            version: reference.capability.version.as_ref().to_owned(),
        },
        display_name,
        source_package: nomifun_api_types::ExactCatalogRefDto {
            id: source_package.id.as_ref().to_owned(),
            version: source_package.version.as_ref().to_owned(),
        },
        dependency_path: reference
            .dependency_path
            .iter()
            .map(|id| id.as_ref().to_owned())
            .collect(),
        required_runtime_features: reference
            .required_runtime_features
            .iter()
            .map(|feature| feature.as_ref().to_owned())
            .collect(),
    }
}

fn mcp_mapping_api(mapping: &McpToolCapabilityMapping) -> McpToolCatalogItemDto {
    McpToolCatalogItemDto {
        server_id: mapping.server_id.as_ref().to_owned(),
        canonical_tool_key: mapping.canonical_tool_key.as_ref().to_owned(),
        capability: nomifun_api_types::ExactCatalogRefDto {
            id: mapping.capability.id.as_ref().to_owned(),
            version: mapping.capability.version.as_ref().to_owned(),
        },
        source_package: nomifun_api_types::ExactCatalogRefDto {
            id: mapping.package.id.as_ref().to_owned(),
            version: mapping.package.version.as_ref().to_owned(),
        },
        schema_digest: mapping.schema_digest.as_ref().to_owned(),
        materialization_version: mapping.materialization_version.as_ref().to_owned(),
    }
}

fn revision_diff(
    current: Option<&AgentPresetRevision>,
    payload: &AgentPresetRevisionPayload,
) -> RevisionDiffDto {
    let before_initial = current
        .map(|revision| capability_ids(&revision.payload.initial_capabilities))
        .unwrap_or_default();
    let before_on_demand = current
        .map(|revision| capability_ids(&revision.payload.on_demand_capabilities))
        .unwrap_or_default();
    let before_skills = current
        .map(|revision| skill_ids(&revision.payload.skill_bindings))
        .unwrap_or_default();
    let after_initial = capability_ids(&payload.initial_capabilities);
    let after_on_demand = capability_ids(&payload.on_demand_capabilities);
    let after_skills = skill_ids(&payload.skill_bindings);
    RevisionDiffDto {
        added_initial: after_initial.difference(&before_initial).cloned().collect(),
        removed_initial: before_initial.difference(&after_initial).cloned().collect(),
        added_on_demand: after_on_demand
            .difference(&before_on_demand)
            .cloned()
            .collect(),
        removed_on_demand: before_on_demand
            .difference(&after_on_demand)
            .cloned()
            .collect(),
        added_skills: after_skills.difference(&before_skills).cloned().collect(),
        removed_skills: before_skills.difference(&after_skills).cloned().collect(),
        model_routes_changed: current
            .is_none_or(|revision| revision.payload.model_route_refs != payload.model_route_refs),
        instructions_changed: current.is_none_or(|revision| {
            revision.payload.persona != payload.persona
                || revision.payload.instructions != payload.instructions
        }),
    }
}

fn capability_ids(
    capabilities: &[nomifun_agent_contracts::CapabilitySelection],
) -> BTreeSet<String> {
    capabilities
        .iter()
        .map(|selection| selection.capability.id.as_ref().to_owned())
        .collect()
}

fn skill_ids(skills: &[SkillRef]) -> BTreeSet<String> {
    skills
        .iter()
        .map(|skill| skill.id.as_ref().to_owned())
        .collect()
}

fn error_diagnostic(
    code: CanonicalErrorCode,
    message: impl Into<String>,
    subject: Option<String>,
) -> PreviewDiagnosticDto {
    PreviewDiagnosticDto {
        severity: PreviewDiagnosticSeverityDto::Error,
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
        contribution_locks: revision
            .contribution_locks
            .iter()
            .map(|lock| {
                Ok(ContributionLockDto {
                    source_kind: wire_name(&lock.source_kind)?,
                    source_identity: lock.source_identity.as_ref().to_owned(),
                    mount_id: lock.mount_id.as_ref().map(|value| value.as_ref().to_owned()),
                    miniapp_id: lock
                        .miniapp_id
                        .as_ref()
                        .map(|value| value.as_ref().to_owned()),
                    mcp_binding_id: lock
                        .mcp_binding_id
                        .as_ref()
                        .map(|value| value.as_ref().to_owned()),
                    contribution_id: lock.contribution_id.as_ref().to_owned(),
                    contract_digest: lock.contract_digest.as_ref().to_owned(),
                })
            })
            .collect::<Result<Vec<_>, ControlPlaneError>>()?,
        created_by: revision.created_by.as_ref().to_owned(),
        created_at_ms: revision.created_at_ms,
        reason: revision.reason.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        CapabilityCatalogMaterialization, CapabilityCatalogMaterializer,
        CapabilityContributions, CapabilityId, CapabilityKind,
        CapabilityManifest, CapabilityOwner, CapabilityProvenance,
        CapabilityRef, CapabilityReleaseState, CapabilitySelection,
        CatalogAvailability, LocalizedMetadata, LogicalArtifactRef,
        McpBindingId, McpServerId, McpToolCapabilityMapping, McpToolKey,
        PackageId, PackageRef, PluginSourceMetadata, RuntimeProfileKind,
        SkillDefinition, SkillId, StrictJsonValue,
        capability_surface_declarations,
    };
    use nomifun_agent_kernel::{
        MaterializedCapability, MaterializedMcpTool, MaterializedSkill,
    };
    use std::collections::BTreeMap;

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
            miniapp_id: None,
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
                        miniapp_id: None,
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
            miniapp_id: None,
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
            schema_version: VersionString::from("1.0.0"),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            initial_capabilities: vec![CapabilitySelection {
                capability: capability_ref,
                action_allowlist: BTreeSet::new(),
            }],
            on_demand_capabilities: Vec::new(),
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
}
