use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nomifun_agent_contracts::{
    AgentPresetRevision, AgentPresetRevisionPayload, CanonicalErrorCode,
    CapabilityConsumer, ContributionLock, ContributionSourceKind, DigestHex,
    McpToolCapabilityMapping, OfficialPresetKey, OperationId, PluginMountId,
    PresetRevisionRef, PrincipalRef, ResolvedCapability, ResolvedSnapshotEnvelope,
    SkillRef, StableSourceIdentity, UserId, VersionString, digest_payload,
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
        let payload: AgentPresetRevisionPayload = wire_cast(&request.draft.document)?;
        let draft_digest = digest_payload(&payload)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        let clean = current_revision.is_some_and(|current| current.payload == payload);
        let contribution_locks = if clean {
            current_revision
                .map(|revision| revision.contribution_locks.clone())
                .unwrap_or_default()
        } else {
            contribution_locks_for_payload(&payload, catalog)?
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
            let (registry, mut environment) = self.canonical_inputs()?;
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
        if let Some(code) = catalog.unavailable_capabilities.get(&reference.id) {
            diagnostics.push(error_diagnostic(
                code.clone(),
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
) -> Result<Vec<ContributionLock>, ControlPlaneError> {
    let mut locks = Vec::new();
    let mut seen = BTreeSet::new();

    for selection in payload
        .initial_capabilities
        .iter()
        .chain(payload.on_demand_capabilities.iter())
    {
        let Some(capability) = catalog.find_capability(&selection.capability) else {
            continue;
        };
        if catalog.source_kind(&capability.package)
            == nomifun_agent_contracts::PluginSourceKind::TestFixture
        {
            // TestHost registrations may drive deterministic Kernel fixtures,
            // but never mint formal Catalog provenance or Revision locks.
            continue;
        }
        let entry = catalog
            .capability_catalog_entry(&selection.capability)?
            .ok_or_else(|| {
                ControlPlaneError::canonical(
                    "CAPABILITY_NOT_MATERIALIZED",
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    format!(
                        "capability {}@{} is not materialized",
                        selection.capability.id.as_ref(),
                        selection.capability.version.as_ref()
                    ),
                )
            })?;
        let operation_lock = entry
            .operation_lock(CapabilityConsumer::Agent)
            .map_err(|error| {
                ControlPlaneError::canonical(
                    error.code(),
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
        let Some(definition) = catalog.find_skill(skill) else {
            continue;
        };
        let contribution_id = format!("skill:{}", definition.id.as_ref());
        if !seen.insert(contribution_id.clone()) {
            continue;
        }
        let (source_kind, source_identity, mount_id) =
            contribution_source_for_package(catalog, &definition.package);
        locks.push(ContributionLock {
            source_kind,
            source_identity,
            mount_id,
            miniapp_id: None,
            mcp_binding_id: None,
            contribution_id: contribution_id.into(),
            contract_digest: definition.body_ref.digest.clone(),
        });
    }

    locks.sort();
    Ok(locks)
}

fn contribution_source_for_package(
    catalog: &CatalogSnapshot,
    package: &nomifun_agent_contracts::PackageRef,
) -> (ContributionSourceKind, StableSourceIdentity, Option<PluginMountId>) {
    let source_identity = StableSourceIdentity::from(format!(
        "{}@{}",
        package.id.as_ref(),
        package.version.as_ref()
    ));
    match catalog.source_kind(package) {
        nomifun_agent_contracts::PluginSourceKind::ManagedLocal => (
            ContributionSourceKind::PluginMount,
            source_identity,
            Some(PluginMountId::from(format!(
                "package:{}@{}",
                package.id.as_ref(),
                package.version.as_ref()
            ))),
        ),
        _ => (ContributionSourceKind::PlatformBuiltin, source_identity, None),
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
                .find(|capability| &capability.id == id)
        })
        .flat_map(|capability| capability.requires_runtime_features.iter())
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
        .filter(|capability| initial_ids.contains(&capability.id))
        .collect::<Vec<_>>();
    let required_resource_kinds = catalog
        .capabilities
        .iter()
        .filter(|capability| selected_ids.contains(&capability.id))
        .flat_map(|capability| capability.contributions.resource_kinds.iter())
        .collect::<BTreeSet<_>>();
    PreviewSummaryDto {
        initial_count: payload.initial_capabilities.len() as u32,
        on_demand_count: payload.on_demand_capabilities.len() as u32,
        active_at_start_count: initial_manifests.len() as u32,
        model_tool_count: initial_manifests
            .iter()
            .map(|manifest| manifest.contributions.actions.len() as u32)
            .sum(),
        context_contributor_count: initial_manifests
            .iter()
            .map(|manifest| manifest.contributions.context_schema_refs.len() as u32)
            .sum(),
        on_demand_index_count: snapshot
            .map(|snapshot| snapshot.content.compact_on_demand_index.len() as u32)
            .unwrap_or(payload.on_demand_capabilities.len() as u32),
        skill_count: payload.skill_bindings.len() as u32,
        mcp_count: catalog
            .mcp_tools
            .iter()
            .filter(|mapping| selected_ids.contains(&mapping.capability.id))
            .count() as u32,
        required_resource_kind_count: required_resource_kinds.len() as u32,
        provider_initialization_count: payload.model_route_refs.len() as u32,
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
                .map(|selection| ResolvedCapability {
                    capability: selection.capability.clone(),
                    source_package: catalog
                        .find_capability(&selection.capability)
                        .map(|capability| capability.package.clone())
                        .unwrap_or_else(|| {
                            nomifun_agent_contracts::PackageRef {
                                id: "unmaterialized".into(),
                                version: "0.0.0".into(),
                            }
                        }),
                    schema_digest: DigestHex::from("unmaterialized"),
                    dependency_path: vec![selection.capability.id.clone()],
                    required_runtime_features: BTreeSet::new(),
                })
                .collect()
        });
    let on_demand_refs = snapshot
        .map(|snapshot| snapshot.content.on_demand_capabilities.clone())
        .unwrap_or_else(|| {
            payload
                .on_demand_capabilities
                .iter()
                .map(|selection| ResolvedCapability {
                    capability: selection.capability.clone(),
                    source_package: catalog
                        .find_capability(&selection.capability)
                        .map(|capability| capability.package.clone())
                        .unwrap_or_else(|| {
                            nomifun_agent_contracts::PackageRef {
                                id: "unmaterialized".into(),
                                version: "0.0.0".into(),
                            }
                        }),
                    schema_digest: DigestHex::from("unmaterialized"),
                    dependency_path: vec![selection.capability.id.clone()],
                    required_runtime_features: BTreeSet::new(),
                })
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
        .filter(|mapping| selected_ids.contains(&mapping.capability.id))
        .map(mcp_mapping_api)
        .collect();
    let required_resource_kinds = catalog
        .capabilities
        .iter()
        .filter(|capability| selected_ids.contains(&capability.id))
        .flat_map(|capability| {
            capability
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

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{CapabilityId, RuntimeProfileKind};

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
