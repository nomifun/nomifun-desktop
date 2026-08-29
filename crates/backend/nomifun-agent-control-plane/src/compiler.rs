use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use nomifun_agent_contracts::{
    AgentPresetRevision, AgentPresetRevisionPayload, CanonicalErrorCode, CapabilityId,
    CapabilityManifest, CapabilityRef, CompactOnDemandCapabilityEntry, DigestHex,
    McpToolCapabilityMapping, OfficialPresetKey, OperationId, PrecomputedActivationPlan,
    PresetRevisionRef, PrincipalRef, ResolvedCapability, ResolvedMcpToolLock, ResolvedSkillLock,
    ResolvedSnapshotContent, ResolvedSnapshotEnvelope, ResolvedSnapshotId, ResolvedSnapshotRef,
    RuntimeFeatureId, RuntimeProfileKind, SkillRef, UserId, VersionString, digest_payload,
};
use nomifun_api_types::{
    AgentPresetRevisionDto, McpToolCatalogItemDto, PreviewCapabilityDto, PreviewDiagnosticDto,
    PreviewDiagnosticSeverityDto, PreviewStatusDto, PreviewSummaryDto, ResolveAgentPresetPreviewRequest,
    ResolveAgentPresetPreviewResponse, RevisionDiffDto, SnapshotInspectorDto,
};
use serde_json::json;
use uuid::Uuid;

use crate::catalog::{CatalogSnapshot, OfficialTemplateCatalog};
use crate::error::ControlPlaneError;
use crate::wire::{wire_cast, wire_name};

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
struct ResolvedNode {
    manifest: CapabilityManifest,
    dependency_path: Vec<CapabilityId>,
}

#[derive(Clone, Debug)]
pub struct PreviewCompilation {
    pub response: ResolveAgentPresetPreviewResponse,
    pub payload: AgentPresetRevisionPayload,
    pub candidate_revision_ref: PresetRevisionRef,
    pub snapshot: Option<ResolvedSnapshotEnvelope>,
}

#[derive(Clone)]
pub struct PresetPreviewCompiler {
    release: CompilerReleaseInputs,
    official_templates: OfficialTemplateCatalog,
}

impl PresetPreviewCompiler {
    pub fn new(
        release: CompilerReleaseInputs,
        official_templates: OfficialTemplateCatalog,
    ) -> Self {
        Self {
            release,
            official_templates,
        }
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
        let clean = current_revision
            .is_some_and(|current| current.payload == payload);
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
                revision_digest: draft_digest.clone(),
            }
        };

        let mut diagnostics = Vec::new();
        let candidate_revision = AgentPresetRevision {
            reference: candidate_revision_ref.clone(),
            payload: payload.clone(),
            created_by: owner.clone(),
            created_at_ms: now_ms(),
            reason: None,
        };
        if let Err(violation) = candidate_revision.validate() {
            diagnostics.push(error_diagnostic(
                violation.code,
                violation.message,
                None,
            ));
        }

        validate_resource_bindings(owner, &payload, &mut diagnostics);
        let directly_selected = payload
            .initial_capabilities
            .iter()
            .chain(&payload.on_demand_capabilities)
            .map(|selection| selection.capability.id.clone())
            .collect::<BTreeSet<_>>();

        let initial_root_nodes = resolve_roots(
            catalog,
            payload
                .initial_capabilities
                .iter()
                .map(|selection| &selection.capability),
            &directly_selected,
            &payload,
            &mut diagnostics,
        );
        let initial_nodes = merge_root_nodes(&initial_root_nodes, &mut diagnostics);
        let on_demand_nodes = resolve_roots(
            catalog,
            payload
                .on_demand_capabilities
                .iter()
                .map(|selection| &selection.capability),
            &directly_selected,
            &payload,
            &mut diagnostics,
        );
        validate_skills(catalog, &payload, &directly_selected, &mut diagnostics);
        validate_coding_baseline(
            transient_template_key,
            &self.official_templates,
            &directly_selected,
            &initial_nodes,
            &on_demand_nodes,
            &mut diagnostics,
        );
        if clean && current_snapshot.is_none() {
            diagnostics.push(error_diagnostic(
                CanonicalErrorCode::from("CAPABILITY_NOT_MATERIALIZED"),
                "the saved Revision has no persisted ResolvedSnapshotRef",
                Some(candidate_revision_ref.preset_id.as_ref().to_owned()),
            ));
        }

        let has_errors = diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == PreviewDiagnosticSeverityDto::Error);
        let revision_diff = revision_diff(current_revision, &payload);
        let snapshot = if has_errors {
            None
        } else if clean {
            current_snapshot.cloned()
        } else {
            Some(self.build_snapshot(
                owner,
                request,
                &candidate_revision_ref,
                &payload,
                catalog,
                &initial_nodes,
                &on_demand_nodes,
            )?)
        };

        let summary = preview_summary(&payload, catalog, &initial_nodes, &snapshot);
        let inspector = preview_inspector(
            &self.release,
            &candidate_revision_ref,
            &payload,
            catalog,
            &initial_nodes,
            &on_demand_nodes,
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
        let ready = snapshot.is_some() && !has_errors;
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
            candidate_revision_ref,
            snapshot,
        })
    }

    fn build_snapshot(
        &self,
        owner: &UserId,
        request: &ResolveAgentPresetPreviewRequest,
        candidate_revision_ref: &PresetRevisionRef,
        payload: &AgentPresetRevisionPayload,
        catalog: &CatalogSnapshot,
        initial_nodes: &BTreeMap<CapabilityId, ResolvedNode>,
        on_demand_nodes: &BTreeMap<CapabilityId, Vec<ResolvedNode>>,
    ) -> Result<ResolvedSnapshotEnvelope, ControlPlaneError> {
        let initial_capabilities = initial_nodes
            .values()
            .map(resolved_capability)
            .collect::<Result<Vec<_>, _>>()?;
        let on_demand_capabilities = payload
            .on_demand_capabilities
            .iter()
            .filter_map(|selection| {
                catalog
                    .find_capability(&selection.capability)
                    .map(|manifest| ResolvedNode {
                        manifest: manifest.clone(),
                        dependency_path: vec![manifest.id.clone()],
                    })
            })
            .map(|node| resolved_capability(&node))
            .collect::<Result<Vec<_>, _>>()?;

        let mut activation_plans = BTreeMap::new();
        let mut compact_index = Vec::new();
        for selection in &payload.on_demand_capabilities {
            let Some(nodes) = on_demand_nodes.get(&selection.capability.id) else {
                continue;
            };
            let plan = activation_plan(nodes, payload);
            let plan_digest = digest_payload(&plan)
                .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
            let display = catalog
                .find_capability(&selection.capability)
                .expect("validated on-demand capability exists")
                .display
                .clone();
            let terms = search_terms(
                selection.capability.id.as_ref(),
                &display.name,
                &display.description,
            );
            compact_index.push(CompactOnDemandCapabilityEntry {
                capability_id: selection.capability.id.clone(),
                display_name: display.name.clone(),
                short_description: display.description.clone(),
                search_terms: terms,
                activation_plan_digest: plan_digest,
            });
            activation_plans.insert(selection.capability.id.clone(), plan);
        }

        let capability_allowlist = initial_nodes
            .keys()
            .cloned()
            .chain(
                on_demand_nodes
                    .values()
                    .flat_map(|nodes| nodes.iter().map(|node| node.manifest.id.clone())),
            )
            .collect::<BTreeSet<_>>();
        let required_runtime_features = initial_nodes
            .values()
            .chain(on_demand_nodes.values().flatten())
            .flat_map(|node| {
                node.manifest
                    .requires_runtime_features
                    .iter()
                    .map(|feature| feature.id.clone())
            })
            .collect::<BTreeSet<_>>();
        let required_runtime_profile = runtime_profile(&required_runtime_features);
        let compiled_runtime_profile_digest = digest_payload(&json!({
            "profile": &required_runtime_profile,
            "initial": initial_nodes.keys().collect::<Vec<_>>(),
            "on_demand": &payload.on_demand_capabilities,
            "persona": &payload.persona,
            "instructions": &payload.instructions,
            "context_policy": &payload.context_policy,
            "execution_constraints": &payload.execution_constraints,
            "runtime_budget": &payload.runtime_budget,
        }))
        .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        let skill_locks = payload
            .skill_bindings
            .iter()
            .filter_map(|reference| catalog.find_skill(reference))
            .map(|skill| ResolvedSkillLock {
                skill: SkillRef {
                    id: skill.id.clone(),
                    version: skill.version.clone(),
                },
                body_digest: skill.body_ref.digest.clone(),
                required_capabilities: skill
                    .requires_capabilities
                    .iter()
                    .map(|reference| reference.id.clone())
                    .collect(),
            })
            .collect();
        let mcp_tool_locks = catalog
            .mcp_tools
            .iter()
            .filter(|mapping| capability_allowlist.contains(&mapping.capability.id))
            .map(|mapping| ResolvedMcpToolLock {
                server_id: mapping.server_id.clone(),
                canonical_tool_key: mapping.canonical_tool_key.clone(),
                capability_id: mapping.capability.id.clone(),
                schema_digest: mapping.schema_digest.clone(),
                materialization_revision: materialization_revision(mapping),
            })
            .collect();
        let content = ResolvedSnapshotContent {
            schema_version: VersionString::from("1.0.0"),
            resolver_version: self.release.resolver_version.clone(),
            preset_revision_ref: candidate_revision_ref.clone(),
            required_runtime_protocol_version: self.release.runtime_protocol_version.clone(),
            required_runtime_profile,
            runtime_feature_inventory_digest: self
                .release
                .runtime_feature_inventory_digest
                .clone(),
            required_runtime_features,
            compiled_runtime_profile_digest,
            model_route_refs: payload.model_route_refs.clone(),
            initial_capabilities,
            on_demand_capabilities,
            on_demand_activation_plans: activation_plans,
            compact_on_demand_index: compact_index,
            capability_allowlist,
            skill_locks,
            mcp_tool_locks,
            typed_resource_bindings: payload.resource_bindings.clone(),
            canonical_schema_manifest_digest: self
                .release
                .canonical_schema_manifest_digest
                .clone(),
            target_contribution_manifest_digest: self
                .release
                .target_contribution_manifest_digest
                .clone(),
        };
        let snapshot_ref = ResolvedSnapshotRef {
            snapshot_id: ResolvedSnapshotId::from(Uuid::now_v7().to_string()),
            snapshot_digest: digest_payload(&content)
                .map_err(|error| ControlPlaneError::Wire(error.to_string()))?,
        };
        let snapshot = ResolvedSnapshotEnvelope {
            snapshot_ref,
            content,
            actor: PrincipalRef {
                principal_kind: "user".into(),
                principal_id: owner.as_ref().to_owned(),
            },
            scene: request.scene.clone(),
            surface: request.surface.clone(),
            audience: request.audience.clone(),
            created_at_ms: now_ms(),
            resolver_run_id: OperationId::from(Uuid::now_v7().to_string()),
            availability_evidence_revision: self
                .release
                .availability_evidence_revision
                .clone(),
        };
        snapshot.validate().map_err(|violation| {
            ControlPlaneError::canonical(
                violation.code,
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                violation.message,
            )
        })?;
        Ok(snapshot)
    }
}

fn resolve_roots<'a>(
    catalog: &CatalogSnapshot,
    roots: impl Iterator<Item = &'a CapabilityRef>,
    directly_selected: &BTreeSet<CapabilityId>,
    payload: &AgentPresetRevisionPayload,
    diagnostics: &mut Vec<PreviewDiagnosticDto>,
) -> BTreeMap<CapabilityId, Vec<ResolvedNode>> {
    let mut resolved = BTreeMap::new();
    for root in roots {
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut nodes = Vec::new();
        resolve_node(
            catalog,
            root,
            vec![root.id.clone()],
            &mut visiting,
            &mut visited,
            &mut nodes,
            diagnostics,
        );
        validate_capability_contracts(
            catalog,
            &nodes,
            directly_selected,
            payload,
            diagnostics,
        );
        resolved.insert(root.id.clone(), nodes);
    }
    resolved
}

fn merge_root_nodes(
    roots: &BTreeMap<CapabilityId, Vec<ResolvedNode>>,
    diagnostics: &mut Vec<PreviewDiagnosticDto>,
) -> BTreeMap<CapabilityId, ResolvedNode> {
    let mut merged: BTreeMap<CapabilityId, ResolvedNode> = BTreeMap::new();
    for node in roots.values().flatten() {
        if let Some(existing) = merged.get(&node.manifest.id) {
            if existing.manifest.version != node.manifest.version {
                diagnostics.push(error_diagnostic(
                    CanonicalErrorCode::from("CAPABILITY_NOT_MATERIALIZED"),
                    "dependency closure selected two exact versions for one capability id",
                    Some(node.manifest.id.as_ref().to_owned()),
                ));
            }
            continue;
        }
        merged.insert(node.manifest.id.clone(), node.clone());
    }
    merged
}

#[allow(clippy::too_many_arguments)]
fn resolve_node(
    catalog: &CatalogSnapshot,
    reference: &CapabilityRef,
    path: Vec<CapabilityId>,
    visiting: &mut BTreeSet<CapabilityId>,
    visited: &mut BTreeSet<CapabilityId>,
    output: &mut Vec<ResolvedNode>,
    diagnostics: &mut Vec<PreviewDiagnosticDto>,
) {
    if visited.contains(&reference.id) {
        return;
    }
    if !visiting.insert(reference.id.clone()) {
        diagnostics.push(error_diagnostic(
            CanonicalErrorCode::from("CAPABILITY_NOT_MATERIALIZED"),
            "capability dependency cycle detected",
            Some(reference.id.as_ref().to_owned()),
        ));
        return;
    }
    let Some(manifest) = catalog.find_capability(reference) else {
        diagnostics.push(error_diagnostic(
            CanonicalErrorCode::from("CAPABILITY_NOT_MATERIALIZED"),
            format!(
                "capability {}@{} is not materialized",
                reference.id.as_ref(),
                reference.version.as_ref()
            ),
            Some(reference.id.as_ref().to_owned()),
        ));
        visiting.remove(&reference.id);
        return;
    };
    if let Some(code) = catalog.unavailable_capabilities.get(&manifest.id) {
        diagnostics.push(error_diagnostic(
            code.clone(),
            format!("capability {} is unavailable on this host", manifest.id.as_ref()),
            Some(manifest.id.as_ref().to_owned()),
        ));
    }
    for dependency in &manifest.requires {
        let mut dependency_path = path.clone();
        dependency_path.push(dependency.id.clone());
        resolve_node(
            catalog,
            dependency,
            dependency_path,
            visiting,
            visited,
            output,
            diagnostics,
        );
    }
    visiting.remove(&reference.id);
    visited.insert(reference.id.clone());
    output.push(ResolvedNode {
        manifest: manifest.clone(),
        dependency_path: path,
    });
}

fn validate_capability_contracts(
    catalog: &CatalogSnapshot,
    nodes: &[ResolvedNode],
    directly_selected: &BTreeSet<CapabilityId>,
    payload: &AgentPresetRevisionPayload,
    diagnostics: &mut Vec<PreviewDiagnosticDto>,
) {
    let resource_bindings = payload
        .resource_bindings
        .iter()
        .map(|binding| binding.resource_kind.clone())
        .collect::<BTreeSet<_>>();
    for node in nodes {
        for conflict in &node.manifest.conflicts {
            if directly_selected.contains(&conflict.capability.id) {
                diagnostics.push(error_diagnostic(
                    CanonicalErrorCode::from("PRESET_CAPABILITY_SET_OVERLAP"),
                    conflict.reason.clone(),
                    Some(node.manifest.id.as_ref().to_owned()),
                ));
            }
        }
        for resource_kind in &node.manifest.contributions.resource_kinds {
            if !resource_bindings.contains(resource_kind) {
                diagnostics.push(error_diagnostic(
                    CanonicalErrorCode::from("PRESET_RESOURCE_NOT_BOUND"),
                    format!(
                        "capability {} requires a {} resource binding",
                        node.manifest.id.as_ref(),
                        resource_kind.as_ref()
                    ),
                    Some(node.manifest.id.as_ref().to_owned()),
                ));
            }
        }
        if catalog
            .unavailable_capabilities
            .contains_key(&node.manifest.id)
        {
            continue;
        }
    }
}

fn validate_resource_bindings(
    owner: &UserId,
    payload: &AgentPresetRevisionPayload,
    diagnostics: &mut Vec<PreviewDiagnosticDto>,
) {
    let mut binding_ids = BTreeSet::new();
    for binding in &payload.resource_bindings {
        if !binding_ids.insert(binding.binding_id.clone()) {
            diagnostics.push(error_diagnostic(
                CanonicalErrorCode::from("PRESET_RESOURCE_NOT_BOUND"),
                "typed resource binding ids must be unique",
                Some(binding.binding_id.as_ref().to_owned()),
            ));
        }
        if binding.owner_id != owner.as_ref() {
            diagnostics.push(error_diagnostic(
                CanonicalErrorCode::from("RESOURCE_OWNER_MISMATCH"),
                "typed resource binding owner does not match the authenticated owner",
                Some(binding.binding_id.as_ref().to_owned()),
            ));
        }
    }
    for selection in payload
        .initial_capabilities
        .iter()
        .chain(&payload.on_demand_capabilities)
    {
        for binding_ref in &selection.resource_binding_refs {
            if !binding_ids.contains(binding_ref) {
                diagnostics.push(error_diagnostic(
                    CanonicalErrorCode::from("PRESET_RESOURCE_NOT_BOUND"),
                    "capability references a resource binding that is not present",
                    Some(binding_ref.as_ref().to_owned()),
                ));
            }
        }
    }
}

fn validate_skills(
    catalog: &CatalogSnapshot,
    payload: &AgentPresetRevisionPayload,
    directly_selected: &BTreeSet<CapabilityId>,
    diagnostics: &mut Vec<PreviewDiagnosticDto>,
) {
    for reference in &payload.skill_bindings {
        let Some(skill) = catalog.find_skill(reference) else {
            diagnostics.push(error_diagnostic(
                CanonicalErrorCode::from("CAPABILITY_NOT_MATERIALIZED"),
                format!(
                    "skill {}@{} is not materialized",
                    reference.id.as_ref(),
                    reference.version.as_ref()
                ),
                Some(reference.id.as_ref().to_owned()),
            ));
            continue;
        };
        for capability in &skill.requires_capabilities {
            if !directly_selected.contains(&capability.id) {
                diagnostics.push(error_diagnostic(
                    CanonicalErrorCode::from("SKILL_REQUIRES_CAPABILITY"),
                    format!(
                        "skill {} requires directly selected capability {}",
                        skill.id.as_ref(),
                        capability.id.as_ref()
                    ),
                    Some(skill.id.as_ref().to_owned()),
                ));
            }
        }
    }
}

fn validate_coding_baseline(
    transient_template_key: Option<OfficialPresetKey>,
    official_templates: &OfficialTemplateCatalog,
    directly_selected: &BTreeSet<CapabilityId>,
    initial_nodes: &BTreeMap<CapabilityId, ResolvedNode>,
    on_demand_nodes: &BTreeMap<CapabilityId, Vec<ResolvedNode>>,
    diagnostics: &mut Vec<PreviewDiagnosticDto>,
) {
    if transient_template_key != Some(OfficialPresetKey::CodingCodex) {
        return;
    }
    let required_ids = official_templates
        .required_capability_ids(OfficialPresetKey::CodingCodex)
        .unwrap_or_default();
    let missing_ids = required_ids
        .difference(directly_selected)
        .map(|id| id.as_ref().to_owned())
        .collect::<Vec<_>>();
    let actual_features = initial_nodes
        .values()
        .chain(on_demand_nodes.values().flatten())
        .flat_map(|node| {
            node.manifest
                .requires_runtime_features
                .iter()
                .map(|feature| feature.id.clone())
        })
        .collect::<BTreeSet<_>>();
    let required_features = official_templates
        .required_runtime_features(OfficialPresetKey::CodingCodex)
        .unwrap_or_default();
    let missing_features = required_features
        .difference(&actual_features)
        .map(|feature| feature.as_ref().to_owned())
        .collect::<Vec<_>>();
    if !missing_ids.is_empty() || !missing_features.is_empty() {
        diagnostics.push(PreviewDiagnosticDto {
            severity: PreviewDiagnosticSeverityDto::Error,
            code: "CODING_CODEX_NATIVE_INCOMPLETE".into(),
            message: "coding.codex must retain the complete frozen Coding capability and runtime-feature baseline".into(),
            subject: Some("coding.codex".into()),
            details: Some(json!({
                "missing_capability_ids": missing_ids,
                "missing_runtime_features": missing_features,
            })),
        });
    }
}

fn resolved_capability(node: &ResolvedNode) -> Result<ResolvedCapability, ControlPlaneError> {
    Ok(ResolvedCapability {
        capability: CapabilityRef {
            id: node.manifest.id.clone(),
            version: node.manifest.version.clone(),
        },
        source_package: node.manifest.package.clone(),
        schema_digest: digest_payload(&node.manifest)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?,
        dependency_path: node.dependency_path.clone(),
        required_runtime_features: node
            .manifest
            .requires_runtime_features
            .iter()
            .map(|feature| feature.id.clone())
            .collect(),
    })
}

fn activation_plan(
    nodes: &[ResolvedNode],
    payload: &AgentPresetRevisionPayload,
) -> PrecomputedActivationPlan {
    let resource_kinds = nodes
        .iter()
        .flat_map(|node| node.manifest.contributions.resource_kinds.iter().cloned())
        .collect::<BTreeSet<_>>();
    PrecomputedActivationPlan {
        root_capability_id: nodes
            .last()
            .expect("validated activation plan has a root")
            .manifest
            .id
            .clone(),
        capability_bundle: nodes
            .iter()
            .map(|node| node.manifest.id.clone())
            .collect(),
        tool_schema_refs: nodes
            .iter()
            .flat_map(|node| {
                node.manifest
                    .contributions
                    .actions
                    .iter()
                    .flat_map(|action| [action.input_schema.clone(), action.output_schema.clone()])
            })
            .collect(),
        context_schema_refs: nodes
            .iter()
            .flat_map(|node| {
                node.manifest
                    .contributions
                    .context_schema_refs
                    .iter()
                    .cloned()
            })
            .collect(),
        resource_binding_refs: payload
            .resource_bindings
            .iter()
            .filter(|binding| resource_kinds.contains(&binding.resource_kind))
            .map(|binding| binding.binding_id.clone())
            .collect(),
        model_route_refs: payload.model_route_refs.values().cloned().collect(),
    }
}

fn runtime_profile(features: &BTreeSet<RuntimeFeatureId>) -> RuntimeProfileKind {
    if features
        .iter()
        .any(|feature| matches!(feature.as_ref(), "code_mode" | "review.workflow"))
    {
        RuntimeProfileKind::CodingNative
    } else {
        RuntimeProfileKind::ManagedMinimal
    }
}

fn preview_summary(
    payload: &AgentPresetRevisionPayload,
    catalog: &CatalogSnapshot,
    initial_nodes: &BTreeMap<CapabilityId, ResolvedNode>,
    _snapshot: &Option<ResolvedSnapshotEnvelope>,
) -> PreviewSummaryDto {
    let initial_manifests = initial_nodes
        .values()
        .map(|node| &node.manifest)
        .collect::<Vec<_>>();
    let selected_ids = payload
        .initial_capabilities
        .iter()
        .chain(&payload.on_demand_capabilities)
        .map(|selection| selection.capability.id.clone())
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
        on_demand_index_count: payload.on_demand_capabilities.len() as u32,
        skill_count: payload.skill_bindings.len() as u32,
        mcp_count: catalog
            .mcp_tools
            .iter()
            .filter(|mapping| selected_ids.contains(&mapping.capability.id))
            .count() as u32,
        resource_binding_count: payload.resource_bindings.len() as u32,
        provider_initialization_count: (payload.model_route_refs.len()
            + payload
                .resource_bindings
                .iter()
                .filter(|binding| binding.connection_config_ref.is_some())
                .count()) as u32,
    }
}

fn preview_inspector(
    release: &CompilerReleaseInputs,
    candidate_revision_ref: &PresetRevisionRef,
    payload: &AgentPresetRevisionPayload,
    catalog: &CatalogSnapshot,
    initial_nodes: &BTreeMap<CapabilityId, ResolvedNode>,
    on_demand_nodes: &BTreeMap<CapabilityId, Vec<ResolvedNode>>,
    snapshot: Option<&ResolvedSnapshotEnvelope>,
) -> Result<SnapshotInspectorDto, ControlPlaneError> {
    let initial = initial_nodes
        .values()
        .map(preview_capability)
        .collect();
    let on_demand = on_demand_nodes
        .values()
        .filter_map(|nodes| nodes.last())
        .map(preview_capability)
        .collect();
    let selected_ids = payload
        .initial_capabilities
        .iter()
        .chain(&payload.on_demand_capabilities)
        .map(|selection| selection.capability.id.clone())
        .collect::<BTreeSet<_>>();
    let mcp_materializations = catalog
        .mcp_tools
        .iter()
        .filter(|mapping| selected_ids.contains(&mapping.capability.id))
        .map(mcp_mapping_api)
        .collect();
    let tool_schema_refs = initial_nodes
        .values()
        .chain(on_demand_nodes.values().flatten())
        .flat_map(|node| {
            node.manifest
                .contributions
                .actions
                .iter()
                .flat_map(|action| {
                    [
                        action.input_schema.as_ref().to_owned(),
                        action.output_schema.as_ref().to_owned(),
                    ]
                })
        })
        .collect();
    let context_schema_refs = initial_nodes
        .values()
        .chain(on_demand_nodes.values().flatten())
        .flat_map(|node| {
            node.manifest
                .contributions
                .context_schema_refs
                .iter()
                .map(|reference| reference.as_ref().to_owned())
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
        compact_on_demand_index: payload
            .on_demand_capabilities
            .iter()
            .map(|selection| selection.capability.id.as_ref().to_owned())
            .collect(),
        tool_schema_refs,
        context_schema_refs,
        mcp_materializations,
        typed_resource_bindings: wire_cast(&payload.resource_bindings)?,
        service_key_diagnostics: catalog.service_key_diagnostics.clone(),
    })
}

fn preview_capability(node: &ResolvedNode) -> PreviewCapabilityDto {
    PreviewCapabilityDto {
        capability: nomifun_api_types::ExactCatalogRefDto {
            id: node.manifest.id.as_ref().to_owned(),
            version: node.manifest.version.as_ref().to_owned(),
        },
        display_name: node.manifest.display.name.clone(),
        source_package: nomifun_api_types::ExactCatalogRefDto {
            id: node.manifest.package.id.as_ref().to_owned(),
            version: node.manifest.package.version.as_ref().to_owned(),
        },
        dependency_path: node
            .dependency_path
            .iter()
            .map(|id| id.as_ref().to_owned())
            .collect(),
        required_runtime_features: node
            .manifest
            .requires_runtime_features
            .iter()
            .map(|feature| feature.id.as_ref().to_owned())
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
        resource_bindings_changed: current
            .is_none_or(|revision| revision.payload.resource_bindings != payload.resource_bindings),
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

fn materialization_revision(mapping: &McpToolCapabilityMapping) -> u64 {
    mapping
        .materialization_version
        .as_ref()
        .split('.')
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

fn search_terms(id: &str, name: &str, description: &str) -> Vec<String> {
    let mut terms = BTreeSet::from([id.to_lowercase(), name.to_lowercase()]);
    terms.extend(
        description
            .split(|character: char| !character.is_alphanumeric())
            .filter(|term| term.len() >= 3)
            .map(str::to_lowercase),
    );
    terms.into_iter().collect()
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
        created_by: revision.created_by.as_ref().to_owned(),
        created_at_ms: revision.created_at_ms,
        reason: revision.reason.clone(),
    })
}
