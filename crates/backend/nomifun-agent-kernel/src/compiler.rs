use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    ActionId, AgentPresetRevision, CapabilityId, CapabilityRef, CapabilitySelection,
    CanonicalSchemaRef, CompactOnDemandCapabilityEntry, DigestHex, ExecutionRoleId,
    InstallationRoleBinding, ModelRouteId, OperationId, PlatformConstraint,
    PrecomputedActivationPlan, PrincipalRef, ResolvedCapability, ResolvedMcpToolLock,
    ResolvedRoleProviderLock, ResolvedSkillLock, ResolvedSnapshotContent,
    ResolvedSnapshotEnvelope, ResolvedSnapshotId, ResolvedSnapshotRef, ResourceBindingId,
    ResourceKind, RoleProviderSelection, RuntimeFeatureId,
    RuntimeProfileKind, RuntimeTarget, SkillId, TypedResourceBinding, VersionString,
    digest_payload,
};
use serde::Serialize;

use crate::{KernelError, MaterializedCapability, MaterializedRegistry};

const COMPACT_DESCRIPTION_CHARS: usize = 160;
const COMPACT_SEARCH_TERM_CHARS: usize = 48;
const COMPACT_SEARCH_TERM_COUNT: usize = 12;

#[derive(Clone, Debug)]
pub struct CompilerEnvironment {
    pub resolver_version: VersionString,
    pub required_runtime_protocol_version: VersionString,
    pub required_runtime_profile: RuntimeProfileKind,
    pub runtime_feature_inventory_digest: DigestHex,
    pub available_runtime_features: BTreeSet<RuntimeFeatureId>,
    pub installation_role_bindings:
        BTreeMap<ExecutionRoleId, InstallationRoleBinding>,
    pub canonical_schema_manifest_digest: DigestHex,
    pub target_contribution_manifest_digest: DigestHex,
    pub host_target: RuntimeTarget,
    pub host_surface: String,
    pub availability_evidence_revision: String,
}

#[derive(Clone, Debug)]
pub struct CompileRequest {
    pub revision: AgentPresetRevision,
    pub principal: PrincipalRef,
    pub scene: String,
    pub surface: String,
    pub audience: String,
    pub created_at_ms: i64,
    pub resolver_run_id: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CompiledCapabilityPolicy {
    pub allowed_actions: BTreeSet<ActionId>,
    pub resource_binding_ids: BTreeSet<ResourceBindingId>,
    pub required_resource_kinds: BTreeSet<ResourceKind>,
}

#[derive(Clone, Debug)]
pub struct CompiledSnapshot {
    pub envelope: ResolvedSnapshotEnvelope,
    pub authority_policies: BTreeMap<CapabilityId, CompiledCapabilityPolicy>,
    pub registry_generation: u64,
    pub registry_digest: DigestHex,
}

impl CompiledSnapshot {
    pub fn snapshot_ref(&self) -> &ResolvedSnapshotRef {
        &self.envelope.snapshot_ref
    }

    pub fn content(&self) -> &ResolvedSnapshotContent {
        &self.envelope.content
    }

    pub fn policy(
        &self,
        capability_id: &CapabilityId,
    ) -> Option<&CompiledCapabilityPolicy> {
        self.authority_policies.get(capability_id)
    }

    pub fn binding(
        &self,
        binding_id: &ResourceBindingId,
    ) -> Option<&TypedResourceBinding> {
        self.envelope
            .content
            .typed_resource_bindings
            .iter()
            .find(|binding| &binding.binding_id == binding_id)
    }

    pub fn role_provider(
        &self,
        role_id: &ExecutionRoleId,
    ) -> Option<&ResolvedRoleProviderLock> {
        self.envelope.content.resolved_role_providers.get(role_id)
    }
}

#[derive(Serialize)]
struct CompiledRuntimeProfileDigestInput {
    profile_kind: RuntimeProfileKind,
    required_runtime_features: BTreeSet<RuntimeFeatureId>,
    registry_digest: DigestHex,
    initial_capabilities: Vec<CapabilityId>,
    on_demand_capabilities: Vec<CapabilityId>,
    on_demand_activation_plans: BTreeMap<CapabilityId, PrecomputedActivationPlan>,
    authority_policies: BTreeMap<CapabilityId, CompiledCapabilityPolicy>,
    skill_ids: Vec<SkillId>,
    model_route_refs: BTreeMap<String, ModelRouteId>,
    resolved_role_providers: BTreeMap<ExecutionRoleId, ResolvedRoleProviderLock>,
}

pub struct AgentPresetCompiler;

impl AgentPresetCompiler {
    pub fn compile(
        registry: &MaterializedRegistry,
        environment: &CompilerEnvironment,
        request: CompileRequest,
    ) -> Result<CompiledSnapshot, KernelError> {
        request
            .revision
            .validate()
            .map_err(|error| KernelError::InvalidPresetRevision {
                reason: error.message,
            })?;
        if !request.revision.payload.surfaces.contains(&request.surface) {
            return Err(KernelError::SurfaceNotDeclared {
                surface: request.surface,
            });
        }

        let bindings = validate_resource_bindings(
            &request.revision.payload.resource_bindings,
            &request.principal,
        )?;
        let initial_direct = direct_selection_map(
            &request.revision.payload.initial_capabilities,
        );
        let on_demand_direct = direct_selection_map(
            &request.revision.payload.on_demand_capabilities,
        );
        let direct_ids = initial_direct
            .keys()
            .chain(on_demand_direct.keys())
            .cloned()
            .collect::<BTreeSet<_>>();

        validate_direct_selections(registry, &initial_direct)?;
        validate_direct_selections(registry, &on_demand_direct)?;

        let mut paths = BTreeMap::<CapabilityId, Vec<CapabilityId>>::new();
        let mut initial_ids = BTreeSet::new();
        for root in initial_direct.keys() {
            let bundle = dependency_bundle(registry, root)?;
            record_dependency_paths(registry, root, &mut paths)?;
            initial_ids.extend(bundle);
        }
        if let Some(overlap) = on_demand_direct
            .keys()
            .find(|capability_id| initial_ids.contains(*capability_id))
        {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "on-demand capability {} is required by the initial closure",
                    overlap.as_ref()
                ),
            });
        }

        let mut on_demand_bundles = BTreeMap::new();
        let mut on_demand_ids = BTreeSet::new();
        for root in on_demand_direct.keys() {
            let mut bundle = dependency_bundle(registry, root)?;
            record_dependency_paths(registry, root, &mut paths)?;
            bundle.retain(|capability_id| !initial_ids.contains(capability_id));
            on_demand_ids.extend(bundle.iter().cloned());
            on_demand_bundles.insert(root.clone(), bundle);
        }
        let ceiling = initial_ids
            .union(&on_demand_ids)
            .cloned()
            .collect::<BTreeSet<_>>();

        validate_capability_ceiling(registry, environment, &request.surface, &ceiling)?;
        validate_conflicts(registry, &ceiling)?;

        let authority_policies = compile_authority_policies(
            registry,
            &initial_direct,
            &on_demand_direct,
            &initial_ids,
            &on_demand_bundles,
            &bindings,
        )?;
        let initial_capabilities =
            resolved_capabilities(registry, &initial_ids, &paths)?;
        let on_demand_capabilities =
            resolved_capabilities(registry, &on_demand_ids, &paths)?;
        let activation_plans = compile_activation_plans(
            registry,
            &on_demand_bundles,
            &authority_policies,
            &request.revision.payload.model_route_refs,
        )?;
        let compact_on_demand_index = compile_compact_index(
            registry,
            &request.revision.payload.on_demand_capabilities,
            &activation_plans,
        )?;
        let skill_locks = compile_skill_locks(
            registry,
            &request.revision.payload.skill_bindings,
            &direct_ids,
        )?;
        let mcp_tool_locks = compile_mcp_locks(registry, &ceiling);
        let resolved_role_providers = compile_role_provider_locks(
            registry,
            &request.revision.payload.system_role_provider_overrides,
            &environment.installation_role_bindings,
            &ceiling,
            &bindings,
            environment,
        )?;
        let capability_runtime_features = ceiling
            .iter()
            .flat_map(|capability_id| {
                registry.capabilities[capability_id]
                    .manifest
                    .requires_runtime_features
                    .iter()
                    .map(|feature| feature.id.clone())
            })
            .collect::<BTreeSet<_>>();
        let required_runtime_features = if environment.required_runtime_profile
            == RuntimeProfileKind::CodingNative
        {
            environment.available_runtime_features.clone()
        } else {
            capability_runtime_features
        };
        let mut typed_resource_bindings = bindings.into_values().collect::<Vec<_>>();
        typed_resource_bindings.sort_by(|left, right| {
            left.binding_id.cmp(&right.binding_id)
        });

        let compiled_runtime_profile_digest =
            digest_payload(&CompiledRuntimeProfileDigestInput {
                profile_kind: environment.required_runtime_profile,
                required_runtime_features: required_runtime_features.clone(),
                registry_digest: registry.registry_digest.clone(),
                initial_capabilities: initial_ids.iter().cloned().collect(),
                on_demand_capabilities: on_demand_ids.iter().cloned().collect(),
                on_demand_activation_plans: activation_plans.clone(),
                authority_policies: authority_policies.clone(),
                skill_ids: skill_locks
                    .iter()
                    .map(|lock| lock.skill.id.clone())
                    .collect(),
                model_route_refs: request.revision.payload.model_route_refs.clone(),
                resolved_role_providers: resolved_role_providers.clone(),
            })
            .map_err(|error| KernelError::Digest {
                reason: error.to_string(),
            })?;

        let chat_route_identity = request
            .revision
            .chat_route_identity()
            .map_err(|error| KernelError::InvalidPresetRevision {
                reason: error.message,
            })?;
        let content = ResolvedSnapshotContent {
            schema_version: VersionString::from("1.0.0"),
            resolver_version: environment.resolver_version.clone(),
            preset_revision_ref: request.revision.reference,
            required_runtime_protocol_version: environment
                .required_runtime_protocol_version
                .clone(),
            required_runtime_profile: environment.required_runtime_profile,
            runtime_feature_inventory_digest: environment
                .runtime_feature_inventory_digest
                .clone(),
            required_runtime_features,
            compiled_runtime_profile_digest,
            model_route_refs: request.revision.payload.model_route_refs,
            chat_route_identity,
            initial_capabilities,
            on_demand_capabilities,
            on_demand_activation_plans: activation_plans,
            compact_on_demand_index,
            capability_allowlist: ceiling,
            skill_locks,
            mcp_tool_locks,
            resolved_role_providers,
            typed_resource_bindings,
            canonical_schema_manifest_digest: environment
                .canonical_schema_manifest_digest
                .clone(),
            target_contribution_manifest_digest: environment
                .target_contribution_manifest_digest
                .clone(),
        };
        let snapshot_digest =
            digest_payload(&content).map_err(|error| KernelError::Digest {
                reason: error.to_string(),
            })?;
        let envelope = ResolvedSnapshotEnvelope {
            snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from(format!(
                    "resolved:{}",
                    snapshot_digest.as_ref()
                )),
                snapshot_digest,
            },
            content,
            actor: request.principal,
            scene: request.scene,
            surface: request.surface,
            audience: request.audience,
            created_at_ms: request.created_at_ms,
            resolver_run_id: request.resolver_run_id,
            availability_evidence_revision: environment
                .availability_evidence_revision
                .clone(),
        };
        envelope
            .validate()
            .map_err(|error| KernelError::SnapshotValidation {
                reason: error.message,
            })?;
        Ok(CompiledSnapshot {
            envelope,
            authority_policies,
            registry_generation: registry.generation,
            registry_digest: registry.registry_digest.clone(),
        })
    }
}

fn direct_selection_map(
    selections: &[CapabilitySelection],
) -> BTreeMap<CapabilityId, &CapabilitySelection> {
    selections
        .iter()
        .map(|selection| (selection.capability.id.clone(), selection))
        .collect()
}

fn validate_direct_selections(
    registry: &MaterializedRegistry,
    selections: &BTreeMap<CapabilityId, &CapabilitySelection>,
) -> Result<(), KernelError> {
    for selection in selections.values() {
        let Some(capability) = registry.capability(&selection.capability.id) else {
            return Err(KernelError::CapabilityNotMaterialized {
                capability_id: selection.capability.id.clone(),
                version: selection.capability.version.clone(),
            });
        };
        if capability.manifest.version != selection.capability.version {
            return Err(KernelError::CapabilityNotMaterialized {
                capability_id: selection.capability.id.clone(),
                version: selection.capability.version.clone(),
            });
        }
        let declared_actions = capability
            .manifest
            .contributions
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect::<BTreeSet<_>>();
        if let Some(action_id) = selection
            .action_allowlist
            .iter()
            .find(|action_id| !declared_actions.contains(*action_id))
        {
            return Err(KernelError::ActionNotDeclared {
                capability_id: capability.manifest.id.clone(),
                action_id: action_id.clone(),
            });
        }
    }
    Ok(())
}

fn validate_resource_bindings(
    bindings: &[TypedResourceBinding],
    principal: &PrincipalRef,
) -> Result<BTreeMap<ResourceBindingId, TypedResourceBinding>, KernelError> {
    let mut resolved = BTreeMap::new();
    for binding in bindings {
        if binding.owner_id != principal.principal_id {
            return Err(KernelError::ResourceOwnerMismatch {
                binding_id: binding.binding_id.clone(),
            });
        }
        if resolved
            .insert(binding.binding_id.clone(), binding.clone())
            .is_some()
        {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "duplicate resource binding {}",
                    binding.binding_id.as_ref()
                ),
            });
        }
    }
    Ok(resolved)
}

fn dependency_bundle(
    registry: &MaterializedRegistry,
    root: &CapabilityId,
) -> Result<Vec<CapabilityId>, KernelError> {
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut order = Vec::new();
    visit_dependency(
        registry,
        root,
        &mut visiting,
        &mut visited,
        &mut order,
    )?;
    Ok(order)
}

fn visit_dependency(
    registry: &MaterializedRegistry,
    capability_id: &CapabilityId,
    visiting: &mut BTreeSet<CapabilityId>,
    visited: &mut BTreeSet<CapabilityId>,
    order: &mut Vec<CapabilityId>,
) -> Result<(), KernelError> {
    if visited.contains(capability_id) {
        return Ok(());
    }
    if !visiting.insert(capability_id.clone()) {
        return Err(KernelError::CapabilityDependencyCycle);
    }
    let capability = registry.capability(capability_id).ok_or_else(|| {
        KernelError::CapabilityNotMaterialized {
            capability_id: capability_id.clone(),
            version: VersionString::from("unknown"),
        }
    })?;
    let mut dependencies = capability.manifest.requires.clone();
    dependencies.sort_by(|left, right| left.id.cmp(&right.id));
    for dependency in dependencies {
        visit_dependency(registry, &dependency.id, visiting, visited, order)?;
    }
    visiting.remove(capability_id);
    visited.insert(capability_id.clone());
    order.push(capability_id.clone());
    Ok(())
}

fn record_dependency_paths(
    registry: &MaterializedRegistry,
    root: &CapabilityId,
    paths: &mut BTreeMap<CapabilityId, Vec<CapabilityId>>,
) -> Result<(), KernelError> {
    record_path(registry, root, vec![root.clone()], paths)
}

fn record_path(
    registry: &MaterializedRegistry,
    current: &CapabilityId,
    path: Vec<CapabilityId>,
    paths: &mut BTreeMap<CapabilityId, Vec<CapabilityId>>,
) -> Result<(), KernelError> {
    let replace = paths
        .get(current)
        .is_none_or(|existing| &path < existing);
    if replace {
        paths.insert(current.clone(), path.clone());
    }
    let capability = registry.capability(current).ok_or_else(|| {
        KernelError::CapabilityNotMaterialized {
            capability_id: current.clone(),
            version: VersionString::from("unknown"),
        }
    })?;
    let mut dependencies = capability.manifest.requires.clone();
    dependencies.sort_by(|left, right| left.id.cmp(&right.id));
    for dependency in dependencies {
        if path.contains(&dependency.id) {
            return Err(KernelError::CapabilityDependencyCycle);
        }
        let mut next_path = path.clone();
        next_path.push(dependency.id.clone());
        record_path(registry, &dependency.id, next_path, paths)?;
    }
    Ok(())
}

fn validate_capability_ceiling(
    registry: &MaterializedRegistry,
    environment: &CompilerEnvironment,
    surface: &str,
    ceiling: &BTreeSet<CapabilityId>,
) -> Result<(), KernelError> {
    for capability_id in ceiling {
        let capability = &registry.capabilities[capability_id].manifest;
        if !capability.supported_surfaces.is_empty()
            && !capability.supported_surfaces.contains(surface)
        {
            return Err(KernelError::CapabilityUnavailableOnSurface {
                capability_id: capability_id.clone(),
                surface: surface.to_owned(),
            });
        }
        if !platform_supported(
            capability,
            &environment.host_target,
            &environment.host_surface,
        ) {
            return Err(KernelError::CapabilityUnavailableOnPlatform {
                capability_id: capability_id.clone(),
                target: environment.host_target.as_ref().to_owned(),
                surface: environment.host_surface.clone(),
            });
        }
        for feature in &capability.requires_runtime_features {
            if !environment.available_runtime_features.contains(&feature.id) {
                return Err(KernelError::RuntimeFeatureUnavailable {
                    capability_id: capability_id.clone(),
                    feature: feature.id.as_ref().to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn platform_supported(
    capability: &nomifun_agent_contracts::CapabilityManifest,
    target: &RuntimeTarget,
    surface: &str,
) -> bool {
    capability.supported_platforms.is_empty()
        || capability
            .supported_platforms
            .iter()
            .any(|constraint| match constraint {
                PlatformConstraint::Any => true,
                PlatformConstraint::Targets {
                    host_targets,
                    host_surfaces,
                } => {
                    host_targets.contains(target)
                        && (host_surfaces.is_empty()
                            || host_surfaces.contains(surface))
                }
            })
}

fn validate_conflicts(
    registry: &MaterializedRegistry,
    ceiling: &BTreeSet<CapabilityId>,
) -> Result<(), KernelError> {
    for capability_id in ceiling {
        let capability = &registry.capabilities[capability_id].manifest;
        if let Some(conflict) = capability
            .conflicts
            .iter()
            .find(|conflict| ceiling.contains(&conflict.capability.id))
        {
            return Err(KernelError::CapabilityConflict {
                left: capability_id.clone(),
                right: conflict.capability.id.clone(),
            });
        }
    }
    Ok(())
}

fn compile_authority_policies(
    registry: &MaterializedRegistry,
    initial_direct: &BTreeMap<CapabilityId, &CapabilitySelection>,
    on_demand_direct: &BTreeMap<CapabilityId, &CapabilitySelection>,
    initial_ids: &BTreeSet<CapabilityId>,
    on_demand_bundles: &BTreeMap<CapabilityId, Vec<CapabilityId>>,
    bindings: &BTreeMap<ResourceBindingId, TypedResourceBinding>,
) -> Result<BTreeMap<CapabilityId, CompiledCapabilityPolicy>, KernelError> {
    let mut policies = BTreeMap::<CapabilityId, CompiledCapabilityPolicy>::new();
    for (root, selection) in initial_direct.iter().chain(on_demand_direct.iter()) {
        let bundle = if initial_ids.contains(root) {
            dependency_bundle(registry, root)?
        } else {
            on_demand_bundles
                .get(root)
                .cloned()
                .unwrap_or_default()
        };
        let selected_bindings = selection
            .resource_binding_refs
            .iter()
            .map(|binding_id| {
                bindings
                    .get(binding_id)
                    .ok_or_else(|| KernelError::ResourceBindingMissing {
                        binding_id: binding_id.clone(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for capability_id in bundle {
            let capability = &registry.capabilities[&capability_id].manifest;
            let required_resource_kinds =
                capability.contributions.resource_kinds.clone();
            for resource_kind in &required_resource_kinds {
                if !selected_bindings
                    .iter()
                    .any(|binding| &binding.resource_kind == resource_kind)
                {
                    return Err(KernelError::CapabilityResourceNotBound {
                        capability_id: capability_id.clone(),
                        resource_kind: resource_kind.as_ref().to_owned(),
                    });
                }
            }
            let resource_binding_ids = selected_bindings
                .iter()
                .filter(|binding| {
                    required_resource_kinds.is_empty()
                        || required_resource_kinds.contains(&binding.resource_kind)
                })
                .map(|binding| binding.binding_id.clone())
                .collect::<BTreeSet<_>>();
            let declared_actions = capability
                .contributions
                .actions
                .iter()
                .map(|action| action.action_id.clone())
                .collect::<BTreeSet<_>>();
            let allowed_actions = initial_direct
                .get(&capability_id)
                .or_else(|| on_demand_direct.get(&capability_id))
                .filter(|direct| !direct.action_allowlist.is_empty())
                .map(|direct| direct.action_allowlist.clone())
                .unwrap_or(declared_actions);
            policies
                .entry(capability_id)
                .and_modify(|policy| {
                    policy.allowed_actions.extend(allowed_actions.clone());
                    policy
                        .resource_binding_ids
                        .extend(resource_binding_ids.clone());
                    policy
                        .required_resource_kinds
                        .extend(required_resource_kinds.clone());
                })
                .or_insert(CompiledCapabilityPolicy {
                    allowed_actions,
                    resource_binding_ids,
                    required_resource_kinds,
                });
        }
    }
    Ok(policies)
}

fn resolved_capabilities(
    registry: &MaterializedRegistry,
    capability_ids: &BTreeSet<CapabilityId>,
    paths: &BTreeMap<CapabilityId, Vec<CapabilityId>>,
) -> Result<Vec<ResolvedCapability>, KernelError> {
    capability_ids
        .iter()
        .map(|capability_id| {
            let capability = &registry.capabilities[capability_id];
            Ok(ResolvedCapability {
                capability: CapabilityRef {
                    id: capability_id.clone(),
                    version: capability.manifest.version.clone(),
                },
                source_package: capability.manifest.package.clone(),
                schema_digest: capability.schema_digest.clone(),
                dependency_path: paths
                    .get(capability_id)
                    .cloned()
                    .unwrap_or_else(|| vec![capability_id.clone()]),
                required_runtime_features: capability
                    .manifest
                    .requires_runtime_features
                    .iter()
                    .map(|feature| feature.id.clone())
                    .collect(),
            })
        })
        .collect()
}

fn compile_activation_plans(
    registry: &MaterializedRegistry,
    bundles: &BTreeMap<CapabilityId, Vec<CapabilityId>>,
    policies: &BTreeMap<CapabilityId, CompiledCapabilityPolicy>,
    model_routes: &BTreeMap<String, ModelRouteId>,
) -> Result<BTreeMap<CapabilityId, PrecomputedActivationPlan>, KernelError> {
    let model_route_refs = model_routes
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    bundles
        .iter()
        .map(|(root, bundle)| {
            let mut tool_schema_refs = BTreeSet::<CanonicalSchemaRef>::new();
            let mut context_schema_refs = BTreeSet::<CanonicalSchemaRef>::new();
            let mut resource_binding_refs = BTreeSet::<ResourceBindingId>::new();
            for capability_id in bundle {
                let capability = &registry.capabilities[capability_id].manifest;
                for action in &capability.contributions.actions {
                    tool_schema_refs.insert(action.input_schema.clone());
                    tool_schema_refs.insert(action.output_schema.clone());
                }
                context_schema_refs.extend(
                    capability
                        .contributions
                        .context_schema_refs
                        .iter()
                        .cloned(),
                );
                if let Some(policy) = policies.get(capability_id) {
                    resource_binding_refs
                        .extend(policy.resource_binding_ids.iter().cloned());
                }
            }
            Ok((
                root.clone(),
                PrecomputedActivationPlan {
                    root_capability_id: root.clone(),
                    capability_bundle: bundle.clone(),
                    tool_schema_refs: tool_schema_refs.into_iter().collect(),
                    context_schema_refs: context_schema_refs.into_iter().collect(),
                    resource_binding_refs: resource_binding_refs.into_iter().collect(),
                    model_route_refs: model_route_refs.clone(),
                },
            ))
        })
        .collect()
}

fn compile_compact_index(
    registry: &MaterializedRegistry,
    selections: &[CapabilitySelection],
    plans: &BTreeMap<CapabilityId, PrecomputedActivationPlan>,
) -> Result<Vec<CompactOnDemandCapabilityEntry>, KernelError> {
    let mut entries = Vec::with_capacity(selections.len());
    for selection in selections {
        let capability = &registry.capabilities[&selection.capability.id];
        let plan = &plans[&selection.capability.id];
        entries.push(CompactOnDemandCapabilityEntry {
            capability_id: selection.capability.id.clone(),
            display_name: capability.manifest.display.name.clone(),
            short_description: truncate_chars(
                &capability.manifest.display.description,
                COMPACT_DESCRIPTION_CHARS,
            ),
            search_terms: compact_search_terms(capability),
            activation_plan_digest: digest_payload(plan).map_err(|error| {
                KernelError::Digest {
                    reason: error.to_string(),
                }
            })?,
        });
    }
    entries.sort_by(|left, right| left.capability_id.cmp(&right.capability_id));
    Ok(entries)
}

fn compact_search_terms(capability: &MaterializedCapability) -> Vec<String> {
    let mut terms = BTreeSet::new();
    for value in [
        capability.manifest.id.as_ref(),
        capability.manifest.display.name.as_str(),
        capability.manifest.display.description.as_str(),
    ] {
        for term in value
            .split(|character: char| {
                character.is_whitespace()
                    || matches!(character, '.' | '-' | '_' | '/' | ':')
            })
            .map(str::trim)
            .filter(|term| !term.is_empty())
        {
            terms.insert(
                truncate_chars(&term.to_ascii_lowercase(), COMPACT_SEARCH_TERM_CHARS),
            );
            if terms.len() >= COMPACT_SEARCH_TERM_COUNT {
                break;
            }
        }
        if terms.len() >= COMPACT_SEARCH_TERM_COUNT {
            break;
        }
    }
    terms.into_iter().collect()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn compile_skill_locks(
    registry: &MaterializedRegistry,
    skill_refs: &[nomifun_agent_contracts::SkillRef],
    direct_capability_ids: &BTreeSet<CapabilityId>,
) -> Result<Vec<ResolvedSkillLock>, KernelError> {
    let mut locks = Vec::with_capacity(skill_refs.len());
    for skill_ref in skill_refs {
        let Some(skill) = registry.skill(&skill_ref.id) else {
            return Err(KernelError::SkillNotMaterialized {
                skill_id: skill_ref.id.clone(),
                version: skill_ref.version.clone(),
            });
        };
        if skill.definition.version != skill_ref.version {
            return Err(KernelError::SkillNotMaterialized {
                skill_id: skill_ref.id.clone(),
                version: skill_ref.version.clone(),
            });
        }
        for requirement in &skill.definition.requires_capabilities {
            if !direct_capability_ids.contains(&requirement.id) {
                return Err(KernelError::SkillRequiresCapability {
                    skill_id: skill_ref.id.clone(),
                    capability_id: requirement.id.clone(),
                });
            }
        }
        locks.push(ResolvedSkillLock {
            skill: skill_ref.clone(),
            body_digest: skill.definition.body_ref.digest.clone(),
            required_capabilities: skill
                .definition
                .requires_capabilities
                .iter()
                .map(|capability| capability.id.clone())
                .collect(),
        });
    }
    locks.sort_by(|left, right| left.skill.id.cmp(&right.skill.id));
    Ok(locks)
}

fn compile_mcp_locks(
    registry: &MaterializedRegistry,
    ceiling: &BTreeSet<CapabilityId>,
) -> Vec<ResolvedMcpToolLock> {
    let mut locks = ceiling
        .iter()
        .filter_map(|capability_id| registry.mcp_for_capability(capability_id))
        .map(|mcp| ResolvedMcpToolLock {
            server_id: mcp.mapping.server_id.clone(),
            canonical_tool_key: mcp.mapping.canonical_tool_key.clone(),
            capability_id: mcp.mapping.capability.id.clone(),
            schema_digest: mcp.mapping.schema_digest.clone(),
            materialization_revision: registry.generation,
        })
        .collect::<Vec<_>>();
    locks.sort_by(|left, right| {
        (&left.server_id, &left.canonical_tool_key)
            .cmp(&(&right.server_id, &right.canonical_tool_key))
    });
    locks
}

fn compile_role_provider_locks(
    registry: &MaterializedRegistry,
    overrides: &BTreeMap<ExecutionRoleId, RoleProviderSelection>,
    installation_bindings: &BTreeMap<ExecutionRoleId, InstallationRoleBinding>,
    ceiling: &BTreeSet<CapabilityId>,
    bindings: &BTreeMap<ResourceBindingId, TypedResourceBinding>,
    environment: &CompilerEnvironment,
) -> Result<
    BTreeMap<ExecutionRoleId, ResolvedRoleProviderLock>,
    KernelError,
> {
    let required_roles = ceiling
        .iter()
        .filter_map(|capability_id| registry.role_for_capability(capability_id).cloned())
        .collect::<BTreeSet<_>>();
    let mut locks = BTreeMap::new();
    for role_id in required_roles {
        let selection = overrides
            .get(&role_id)
            .or_else(|| {
                installation_bindings
                    .get(&role_id)
                    .map(|binding| &binding.selection)
            })
            .ok_or_else(|| KernelError::RoleProviderNotBound {
                role_id: role_id.clone(),
            })?;
        let selected_members = ceiling
            .iter()
            .filter(|capability_id| {
                registry.role_for_capability(capability_id) == Some(&role_id)
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        locks.insert(
            role_id.clone(),
            resolve_exact_role_provider_lock(
                registry,
                &role_id,
                selection,
                &selected_members,
                bindings,
                environment,
            )?,
        );
    }
    Ok(locks)
}

/// Resolve one exact Role Provider using the same rules as Agent compilation.
///
/// Non-Agent application operations call this at admission and persist/pass the
/// returned lock with their typed resource set. Execution must not call this
/// again or consult a newer installation default.
pub fn resolve_exact_role_provider_lock(
    registry: &MaterializedRegistry,
    role_id: &ExecutionRoleId,
    selection: &RoleProviderSelection,
    selected_members: &BTreeSet<CapabilityId>,
    bindings: &BTreeMap<ResourceBindingId, TypedResourceBinding>,
    environment: &CompilerEnvironment,
) -> Result<ResolvedRoleProviderLock, KernelError> {
    let contract = registry
        .role_contract(role_id)
        .ok_or_else(|| KernelError::RoleProviderNotBound {
            role_id: role_id.clone(),
        })?;
    if selection.role.key != contract.manifest.key
        || selection.role.contract_digest != contract.contract_digest
    {
        return Err(KernelError::RoleProviderUnavailable {
            role_id: role_id.clone(),
            mount_id: selection.provider_mount_id.clone(),
        });
    }
    let provider = registry
        .role_provider(role_id, &selection.provider_mount_id)
        .ok_or_else(|| KernelError::RoleProviderUnavailable {
            role_id: role_id.clone(),
            mount_id: selection.provider_mount_id.clone(),
        })?;
    let contract_members = contract
        .manifest
        .members
        .iter()
        .map(|member| member.capability.id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(capability_id) = selected_members
        .difference(&contract_members)
        .next()
    {
        return Err(KernelError::RoleProviderMemberUnavailable {
            role_id: role_id.clone(),
            capability_id: capability_id.clone(),
        });
    }

    let mut resource_binding_refs = BTreeSet::new();
    for capability_id in selected_members {
        let member = provider
            .contribution
            .members
            .get(capability_id)
            .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                role_id: role_id.clone(),
                capability_id: capability_id.clone(),
            })?;
        if !member.supported_platforms.is_empty()
            && !member.supported_platforms.iter().any(|constraint| {
                provider_platform_supported(
                    constraint,
                    &environment.host_target,
                    &environment.host_surface,
                )
            })
        {
            return Err(KernelError::CapabilityUnavailableOnPlatform {
                capability_id: capability_id.clone(),
                target: environment.host_target.as_ref().to_owned(),
                surface: environment.host_surface.clone(),
            });
        }
        for resource_kind in &member.required_resource_kinds {
            let matching = bindings
                .values()
                .filter(|binding| &binding.resource_kind == resource_kind)
                .collect::<Vec<_>>();
            if matching.is_empty() {
                return Err(KernelError::CapabilityResourceNotBound {
                    capability_id: capability_id.clone(),
                    resource_kind: resource_kind.as_ref().to_owned(),
                });
            }
            if matching.len() > 1 {
                return Err(KernelError::InvalidPresetRevision {
                    reason: format!(
                        "role {} has multiple bindings for resource kind {}",
                        role_id.as_ref(),
                        resource_kind.as_ref()
                    ),
                });
            }
            resource_binding_refs.insert(matching[0].binding_id.clone());
        }
    }
    Ok(ResolvedRoleProviderLock {
        provider: provider.provider.clone(),
        source: provider.source.clone(),
        supported_members: provider.contribution.members.keys().cloned().collect(),
        resource_binding_refs: resource_binding_refs.into_iter().collect(),
    })
}

fn provider_platform_supported(
    constraint: &PlatformConstraint,
    target: &RuntimeTarget,
    surface: &str,
) -> bool {
    match constraint {
        PlatformConstraint::Any => true,
        PlatformConstraint::Targets {
            host_targets,
            host_surfaces,
        } => {
            host_targets.contains(target)
                && (host_surfaces.is_empty() || host_surfaces.contains(surface))
        }
    }
}
