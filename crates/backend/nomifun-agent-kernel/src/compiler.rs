use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    ActionId, AgentPresetRevision, CapabilityConsumer, CapabilityId,
    CapabilityOperationLock, CapabilityRef, CapabilitySelection,
    DigestHex, ExecutionRoleId, InstallationRoleBinding,
    ModelRouteId, OperationId, PlatformConstraint,
    PrincipalRef, ResolvedCapability, ResolvedMcpToolLock, ResolvedRoleProviderLock,
    ResolvedMiniAppCapability, ResolvedSkillLock, ResolvedSnapshotContent,
    ResolvedSnapshotEnvelope, ResolvedSnapshotId, ResolvedSnapshotRef,
    ResourceBindingId, ResourceKind, RoleProviderSelection, RuntimeFeatureId,
    RuntimeProfileKind, RuntimeTarget, SkillId, TypedResourceBinding, VersionString,
    digest_payload,
};
use serde::Serialize;

use crate::{KernelError, MaterializedRegistry};


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
    /// Exact MiniApp Active Release projections resolved by the owning
    /// application service. MiniApp capabilities are deliberately not
    /// materialized in the Kernel Plugin Registry.
    pub miniapp_capabilities: Vec<ResolvedMiniAppCapability>,
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
    pub target_resource_bindings: Vec<TypedResourceBinding>,
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
        self.target_resource_bindings
            .iter()
            .find(|binding| &binding.binding_id == binding_id)
    }

    pub fn resource_bindings(&self) -> &[TypedResourceBinding] {
        &self.target_resource_bindings
    }

    pub fn resolved_capability(
        &self,
        capability_id: &CapabilityId,
    ) -> Option<&ResolvedCapability> {
        self.envelope
            .content
            .enabled_capabilities
            .iter()
            .find(|capability| &capability.capability.id == capability_id)
    }

    pub fn resolved_miniapp_capability(
        &self,
        capability_id: &CapabilityId,
    ) -> Option<&ResolvedMiniAppCapability> {
        self.envelope
            .content
            .enabled_miniapp_capabilities
            .iter()
            .find(|capability| &capability.capability.id == capability_id)
    }

    /// Attach one target's concrete resources without changing the immutable
    /// Preset Snapshot identity. Capabilities continue to own the required
    /// resource kinds; this step only resolves those slots for a Session,
    /// companion, automation target, or other consumer binding.
    pub fn with_target_resource_bindings(
        mut self,
        principal: &PrincipalRef,
        bindings: Vec<TypedResourceBinding>,
    ) -> Result<Self, KernelError> {
        let mut by_id = BTreeMap::new();
        let mut by_kind = BTreeMap::<ResourceKind, Vec<ResourceBindingId>>::new();
        for binding in bindings {
            if binding.owner_id != principal.principal_id {
                return Err(KernelError::ResourceOwnerMismatch {
                    binding_id: binding.binding_id,
                });
            }
            if by_id
                .insert(binding.binding_id.clone(), binding.clone())
                .is_some()
            {
                return Err(KernelError::InvalidPresetRevision {
                    reason: format!(
                        "duplicate target resource binding {}",
                        binding.binding_id.as_ref()
                    ),
                });
            }
            by_kind
                .entry(binding.resource_kind.clone())
                .or_default()
                .push(binding.binding_id);
        }
        for ids in by_kind.values_mut() {
            ids.sort();
        }
        for policy in self.authority_policies.values_mut() {
            policy.resource_binding_ids.clear();
            for resource_kind in &policy.required_resource_kinds {
                let matches = by_kind.get(resource_kind).cloned().unwrap_or_default();
                if matches.len() > 1 {
                    return Err(KernelError::InvalidPresetRevision {
                        reason: format!(
                            "target has multiple bindings for resource kind {}",
                            resource_kind.as_ref()
                        ),
                    });
                }
                policy.resource_binding_ids.extend(matches);
            }
        }
        self.target_resource_bindings = by_id.into_values().collect();
        Ok(self)
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
    capability_operation_locks: Vec<CapabilityOperationLock>,
    enabled_capabilities: Vec<CapabilityId>,
    authority_policies: BTreeMap<CapabilityId, CompiledCapabilityPolicy>,
    skill_ids: Vec<SkillId>,
    model_route_refs: BTreeMap<String, ModelRouteId>,
    resolved_role_providers: BTreeMap<ExecutionRoleId, ResolvedRoleProviderLock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    miniapp: Option<MiniAppRuntimeProfileDigestInput>,
}

#[derive(Serialize)]
struct MiniAppRuntimeProfileDigestInput {
    enabled_capabilities: Vec<ResolvedMiniAppCapability>,
    required_resource_kinds: BTreeSet<ResourceKind>,
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
        let initial_direct = direct_selection_map(
            &request.revision.payload.enabled_capabilities,
        );
        let miniapp_by_id = validate_miniapp_inputs(
            registry,
            &request.revision,
            &request.miniapp_capabilities,
        )?;
        let enabled_miniapp_capabilities = request.miniapp_capabilities.clone();
        let initial_plugin_direct = initial_direct
            .iter()
            .filter(|(capability_id, _)| !miniapp_by_id.contains_key(*capability_id))
            .map(|(capability_id, selection)| {
                (capability_id.clone(), *selection)
            })
            .collect::<BTreeMap<_, _>>();
        let direct_ids = initial_direct
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();

        validate_direct_selections(registry, &initial_plugin_direct)?;
        validate_revision_contribution_locks(
            registry,
            &request.revision,
            &miniapp_by_id,
        )?;

        let mut paths = BTreeMap::<CapabilityId, Vec<CapabilityId>>::new();
        let mut initial_ids = BTreeSet::new();
        for root in initial_plugin_direct.keys() {
            let bundle = dependency_bundle(registry, root)?;
            record_dependency_paths(registry, root, &mut paths)?;
            initial_ids.extend(bundle);
        }
        let ceiling = initial_ids.clone();

        validate_capability_ceiling(registry, environment, &request.surface, &ceiling)?;
        validate_conflicts(registry, &ceiling)?;

        let authority_policies = compile_authority_policies(
            registry,
            &initial_plugin_direct,
        )?;
        let enabled_capabilities = resolved_capabilities(
            registry,
            &initial_ids,
            &paths,
        )?;
        let mut authority_policies = authority_policies;
        merge_miniapp_authority_policies(
            &mut authority_policies,
            &enabled_miniapp_capabilities,
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
            environment,
        )?;
        let miniapp_ids = enabled_miniapp_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect::<BTreeSet<_>>();
        let capability_allowlist = ceiling
            .union(&miniapp_ids)
            .cloned()
            .collect::<BTreeSet<_>>();
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
        let compiled_runtime_profile_digest =
            digest_payload(&CompiledRuntimeProfileDigestInput {
                profile_kind: environment.required_runtime_profile,
                required_runtime_features: required_runtime_features.clone(),
                capability_operation_locks: enabled_capabilities
                    .iter()
                    .map(resolved_capability_operation_lock)
                    .chain(
                        enabled_miniapp_capabilities
                            .iter()
                            .map(resolved_miniapp_capability_operation_lock),
                    )
                    .collect(),
                enabled_capabilities: initial_ids.iter().cloned().collect(),

                authority_policies: authority_policies.clone(),
                skill_ids: skill_locks
                    .iter()
                    .map(|lock| lock.skill.id.clone())
                    .collect(),
                model_route_refs: request.revision.payload.model_route_refs.clone(),
                resolved_role_providers: resolved_role_providers.clone(),
                miniapp: (!enabled_miniapp_capabilities.is_empty())
                .then(|| MiniAppRuntimeProfileDigestInput {
                    enabled_capabilities: enabled_miniapp_capabilities.clone(),
                    required_resource_kinds: enabled_miniapp_capabilities
                        .iter()
                        .flat_map(|capability| {
                            capability.required_resource_kinds.iter().cloned()
                        })
                        .collect(),
                }),
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
        let required_resource_kinds = authority_policies
            .values()
            .flat_map(|policy| policy.required_resource_kinds.iter().cloned())
            .collect();
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
            enabled_capabilities,
            enabled_miniapp_capabilities,
            required_resource_kinds,

            capability_allowlist,
            skill_locks,
            mcp_tool_locks,
            resolved_role_providers,
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
            target_resource_bindings: Vec::new(),
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

fn validate_miniapp_inputs<'a>(
    registry: &MaterializedRegistry,
    revision: &AgentPresetRevision,
    capabilities: &'a [ResolvedMiniAppCapability],
) -> Result<BTreeMap<CapabilityId, &'a ResolvedMiniAppCapability>, KernelError> {
    let mut by_id = BTreeMap::new();
    let mut contribution_ids = BTreeSet::new();
    let mut publication_facts = BTreeMap::new();

    for capability in capabilities {
        capability
            .validate()
            .map_err(|error| KernelError::InvalidPresetRevision {
                reason: error.message,
            })?;
        if capability.capability.id.as_ref().trim().is_empty()
            || capability.capability.version.as_ref().trim().is_empty()
        {
            return Err(KernelError::InvalidPresetRevision {
                reason: "MiniApp capability reference must be non-empty".to_owned(),
            });
        }
        if registry.capability(&capability.capability.id).is_some() {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "MiniApp capability {} must not be present in the Kernel Plugin Registry",
                    capability.capability.id.as_ref()
                ),
            });
        }

        let mut action_ids = BTreeSet::new();
        for action in &capability.actions {
            if action.action_id.as_ref().trim().is_empty() {
                return Err(KernelError::InvalidPresetRevision {
                    reason: format!(
                        "MiniApp capability {} contains an empty action ID",
                        capability.capability.id.as_ref()
                    ),
                });
            }
            if !action_ids.insert(action.action_id.clone()) {
                return Err(KernelError::InvalidPresetRevision {
                    reason: format!(
                        "MiniApp capability {} declares duplicate action {}",
                        capability.capability.id.as_ref(),
                        action.action_id.as_ref()
                    ),
                });
            }
        }
        if let Some(action_id) = capability
            .action_allowlist
            .iter()
            .find(|action_id| !action_ids.contains(*action_id))
        {
            return Err(KernelError::ActionNotDeclared {
                capability_id: capability.capability.id.clone(),
                action_id: action_id.clone(),
            });
        }
        if capability
            .required_resource_kinds
            .iter()
            .any(|kind| kind.as_ref().trim().is_empty())
        {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "MiniApp capability {} contains an empty resource kind",
                    capability.capability.id.as_ref()
                ),
            });
        }
        if by_id
            .insert(capability.capability.id.clone(), capability)
            .is_some()
        {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "MiniApp capability {} is supplied more than once",
                    capability.capability.id.as_ref()
                ),
            });
        }
        if !contribution_ids.insert(capability.contribution_id.clone()) {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "MiniApp contribution {} is supplied more than once",
                    capability.contribution_id.as_ref()
                ),
            });
        }

        let facts = (
            capability.active_release.clone(),
            capability.active_release_epoch,
            capability.catalog_digest.clone(),
            capability.source_package.clone(),
        );
        if let Some(existing) = publication_facts
            .insert(capability.miniapp_id.clone(), facts.clone())
        {
            if existing != facts {
                return Err(KernelError::InvalidPresetRevision {
                    reason: format!(
                        "MiniApp {} has inconsistent Active Release or Catalog facts",
                        capability.miniapp_id.as_ref()
                    ),
                });
            }
        }
    }

    for capability in capabilities {
        let mut matched_selection = None;
        for selection in revision
            .payload
            .enabled_capabilities
            .iter()
        {
            if selection.capability.id != capability.capability.id {
                continue;
            }
            if selection.capability != capability.capability {
                return Err(KernelError::CapabilityNotMaterialized {
                    capability_id: selection.capability.id.clone(),
                    version: selection.capability.version.clone(),
                });
            }
            matched_selection = Some(selection);
            break;
        }
        let Some(selection) = matched_selection else {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "MiniApp capability {} is not selected by the Revision",
                    capability.capability.id.as_ref()
                ),
            });
        };
        if let Some(action_id) = selection
            .action_allowlist
            .iter()
            .find(|action_id| {
                !capability
                    .actions
                    .iter()
                    .any(|action| &action.action_id == *action_id)
            })
        {
            return Err(KernelError::ActionNotDeclared {
                capability_id: capability.capability.id.clone(),
                action_id: action_id.clone(),
            });
        }
        if selection.action_allowlist != capability.action_allowlist {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "MiniApp capability {} action allowlist differs from its Revision selection",
                    capability.capability.id.as_ref()
                ),
            });
        }
    }

    Ok(by_id)
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

fn validate_revision_contribution_locks(
    registry: &MaterializedRegistry,
    revision: &AgentPresetRevision,
    miniapp_capabilities: &BTreeMap<CapabilityId, &ResolvedMiniAppCapability>,
) -> Result<(), KernelError> {
    for selection in revision
        .payload
        .enabled_capabilities
        .iter()
    {
        if let Some(miniapp) = miniapp_capabilities.get(&selection.capability.id) {
            if miniapp.capability != selection.capability {
                return Err(KernelError::CapabilityNotMaterialized {
                    capability_id: selection.capability.id.clone(),
                    version: selection.capability.version.clone(),
                });
            }
            let frozen = revision
                .contribution_locks
                .iter()
                .find(|lock| lock.contribution_id == miniapp.contribution_id)
                .ok_or_else(|| KernelError::CapabilityProvenanceDrift {
                    capability_id: selection.capability.id.clone(),
                    reason: format!(
                        "Revision is missing MiniApp contribution lock {}",
                        miniapp.contribution_id.as_ref()
                    ),
                })?;
            if frozen != &miniapp.contribution_lock {
                return Err(KernelError::CapabilityProvenanceDrift {
                    capability_id: selection.capability.id.clone(),
                    reason:
                        "Revision MiniApp contribution lock does not match the exact projection"
                            .to_owned(),
                });
            }
            continue;
        }
        let capability = registry
            .capability(&selection.capability.id)
            .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                capability_id: selection.capability.id.clone(),
                version: selection.capability.version.clone(),
            })?;
        if capability.source.source_kind
            == nomifun_agent_contracts::PluginSourceKind::TestFixture
        {
            continue;
        }
        let frozen = revision
            .contribution_locks
            .iter()
            .find(|lock| lock.contribution_id == capability.contribution_id)
            .ok_or_else(|| KernelError::CapabilityProvenanceDrift {
                capability_id: selection.capability.id.clone(),
                reason: format!(
                    "Revision is missing contribution lock {}",
                    capability.contribution_id.as_ref()
                ),
            })?;
        if frozen != &capability.contribution_lock {
            return Err(KernelError::CapabilityProvenanceDrift {
                capability_id: selection.capability.id.clone(),
                reason: "Revision contribution lock does not match the materialized target"
                    .to_owned(),
            });
        }
    }
    for lock in &revision.contribution_locks {
        if lock.source_kind == nomifun_agent_contracts::ContributionSourceKind::MiniAppActiveRelease
            && !miniapp_capabilities
                .values()
                .any(|capability| capability.contribution_lock == *lock)
        {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "Revision contains an unselected MiniApp contribution lock {}",
                    lock.contribution_id.as_ref()
                ),
            });
        }
    }
    for reference in &revision.payload.skill_bindings {
        let skill = registry
            .skill(&reference.id)
            .filter(|skill| skill.definition.version == reference.version)
            .ok_or_else(|| KernelError::SkillNotMaterialized {
                skill_id: reference.id.clone(),
                version: reference.version.clone(),
            })?;
        if skill.source.source_kind
            == nomifun_agent_contracts::PluginSourceKind::TestFixture
        {
            continue;
        }
        let frozen = revision
            .contribution_locks
            .iter()
            .find(|lock| lock.contribution_id == skill.contribution_id)
            .ok_or_else(|| KernelError::SkillProvenanceDrift {
                skill_id: reference.id.clone(),
                reason: format!(
                    "Revision is missing contribution lock {}",
                    skill.contribution_id.as_ref()
                ),
            })?;
        if frozen != &skill.contribution_lock {
            return Err(KernelError::SkillProvenanceDrift {
                skill_id: reference.id.clone(),
                reason:
                    "Revision contribution lock does not match the materialized Skill target"
                        .to_owned(),
            });
        }
    }
    Ok(())
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
) -> Result<BTreeMap<CapabilityId, CompiledCapabilityPolicy>, KernelError> {
    let mut policies = BTreeMap::<CapabilityId, CompiledCapabilityPolicy>::new();
    for root in initial_direct.keys() {
        let bundle = dependency_bundle(registry, root)?;
        for capability_id in bundle {
            let capability = &registry.capabilities[&capability_id].manifest;
            let required_resource_kinds =
                capability.contributions.resource_kinds.clone();
            let declared_actions = capability
                .contributions
                .actions
                .iter()
                .map(|action| action.action_id.clone())
                .collect::<BTreeSet<_>>();
            let allowed_actions = initial_direct
                .get(&capability_id)
                .filter(|direct| !direct.action_allowlist.is_empty())
                .map(|direct| direct.action_allowlist.clone())
                .unwrap_or(declared_actions);
            policies
                .entry(capability_id)
                .and_modify(|policy| {
                    policy.allowed_actions.extend(allowed_actions.clone());
                    policy
                        .required_resource_kinds
                        .extend(required_resource_kinds.clone());
                })
                .or_insert(CompiledCapabilityPolicy {
                    allowed_actions,
                    resource_binding_ids: BTreeSet::new(),
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
                contribution_id: capability.contribution_id.clone(),
                contribution_lock: capability.contribution_lock.clone(),
                resolved_mount_id: capability.mount_id.clone(),
                resolved_source: capability.source.clone(),
                target_artifact_digest: capability.target_artifact_digest.clone(),
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

fn resolved_capability_operation_lock(
    capability: &ResolvedCapability,
) -> CapabilityOperationLock {
    CapabilityOperationLock {
        capability: capability.capability.clone(),
        consumer: CapabilityConsumer::Agent,
        contribution: capability.contribution_lock.clone(),
        target_artifact_digest: Some(capability.target_artifact_digest.clone()),
    }
}

fn resolved_miniapp_capability_operation_lock(
    capability: &ResolvedMiniAppCapability,
) -> CapabilityOperationLock {
    CapabilityOperationLock {
        capability: capability.capability.clone(),
        consumer: CapabilityConsumer::Agent,
        contribution: capability.contribution_lock.clone(),
        target_artifact_digest: Some(capability.active_release.release_digest.clone()),
    }
}

fn merge_miniapp_authority_policies(
    policies: &mut BTreeMap<CapabilityId, CompiledCapabilityPolicy>,
    initial: &[ResolvedMiniAppCapability],
) -> Result<(), KernelError> {
    for capability in initial.iter() {
        let declared_actions = capability
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect::<BTreeSet<_>>();
        let allowed_actions = if capability.action_allowlist.is_empty() {
            declared_actions
        } else {
            capability.action_allowlist.clone()
        };
        if policies
            .insert(
                capability.capability.id.clone(),
                CompiledCapabilityPolicy {
                    allowed_actions,
                    resource_binding_ids: BTreeSet::new(),
                    required_resource_kinds: capability.required_resource_kinds.clone(),
                },
            )
            .is_some()
        {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "MiniApp capability {} collides with a Plugin capability policy",
                    capability.capability.id.as_ref()
                ),
            });
        }
    }
    Ok(())
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
            materialization_revision: mcp_materialization_revision(
                &mcp.mapping.materialization_version,
            ),
        })
        .collect::<Vec<_>>();
    locks.sort_by(|left, right| {
        (&left.server_id, &left.canonical_tool_key)
            .cmp(&(&right.server_id, &right.canonical_tool_key))
    });
    locks
}

/// The Snapshot lock identifies the materialized mapping version, not the
/// process-local Registry generation. Registry generations can change when an
/// unrelated package is republished, while the persisted v4 materialization
/// row uses the mapping's semantic version major as its bounded revision.
fn mcp_materialization_revision(version: &VersionString) -> u64 {
    version
        .as_ref()
        .split('.')
        .next()
        .and_then(|major| major.parse::<u64>().ok())
        .filter(|revision| *revision >= 1)
        .unwrap_or(1)
}

fn compile_role_provider_locks(
    registry: &MaterializedRegistry,
    overrides: &BTreeMap<ExecutionRoleId, RoleProviderSelection>,
    installation_bindings: &BTreeMap<ExecutionRoleId, InstallationRoleBinding>,
    ceiling: &BTreeSet<CapabilityId>,
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
            resolve_role_provider_lock(
                registry,
                &role_id,
                selection,
                &selected_members,
                None,
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
    resolve_role_provider_lock(
        registry,
        role_id,
        selection,
        selected_members,
        Some(bindings),
        environment,
    )
}

fn resolve_role_provider_lock(
    registry: &MaterializedRegistry,
    role_id: &ExecutionRoleId,
    selection: &RoleProviderSelection,
    selected_members: &BTreeSet<CapabilityId>,
    bindings: Option<&BTreeMap<ResourceBindingId, TypedResourceBinding>>,
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
        if let Some(bindings) = bindings {
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
            }
        }
    }
    Ok(ResolvedRoleProviderLock {
        provider: provider.provider.clone(),
        source: provider.source.clone(),
        supported_members: provider.contribution.members.keys().cloned().collect(),
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_agent_contracts::{
        ActionId, AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload,
        CapabilityActionDescriptor, CapabilityRef, ContributionId,
        ContributionLock, ContributionSourceKind, DigestHex, EffectClass, MiniAppId,
        MiniAppReleaseId, MiniAppReleaseRef, PackageId, PackageRef, PresetRevisionRef,
        PrincipalRef, ResolvedMiniAppCapability, ToolPresentationKind, UserId,
    };

    use super::*;

    const VERSION: &str = "1.0.0";
    const CAPABILITY_ID: &str = "miniapp.fixture.echo";
    const ACTION_ID: &str = "miniapp.fixture.echo.invoke";
    const CONTRIBUTION_ID: &str = "capability:miniapp.fixture.echo";
    const MINIAPP_ID: &str = "miniapp-fixture";

    fn digest(fill: char) -> DigestHex {
        DigestHex::from(fill.to_string().repeat(64))
    }

    fn miniapp_capability(action_allowlist: BTreeSet<ActionId>) -> ResolvedMiniAppCapability {
        let contribution_lock = ContributionLock {
            source_kind: ContributionSourceKind::MiniAppActiveRelease,
            source_identity: format!("miniapp:{MINIAPP_ID}").into(),
            mount_id: None,
            miniapp_id: Some(MiniAppId::from(MINIAPP_ID)),
            mcp_binding_id: None,
            contribution_id: ContributionId::from(CONTRIBUTION_ID),
            contract_digest: digest('a'),
        };
        ResolvedMiniAppCapability {
            capability: CapabilityRef {
                id: CAPABILITY_ID.into(),
                version: VERSION.into(),
            },
            source_package: PackageRef {
                id: PackageId::from("miniapp.fixture"),
                version: VERSION.into(),
            },
            contribution_id: ContributionId::from(CONTRIBUTION_ID),
            contribution_lock,
            miniapp_id: MiniAppId::from(MINIAPP_ID),
            active_release: MiniAppReleaseRef {
                release_id: MiniAppReleaseId::from("release-fixture"),
                artifact_id: "artifact-fixture".into(),
                release_digest: digest('b'),
                manifest_digest: digest('c'),
            },
            active_release_epoch: 7,
            catalog_digest: digest('d'),
            display_name: "Fixture Echo".to_owned(),
            description: "Echo from a MiniApp Active Release".to_owned(),
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from(ACTION_ID),
                input_schema: "schema://miniapp.fixture.echo/input".into(),
                output_schema: "schema://miniapp.fixture.echo/output".into(),
                effect_class: EffectClass::Pure,
                presentation: ToolPresentationKind::FunctionTool,
            }],
            required_resource_kinds: BTreeSet::from(["workspace".into()]),
            action_allowlist,
        }
    }

    fn revision(
        action_allowlist: BTreeSet<ActionId>,
        lock: ContributionLock,
    ) -> AgentPresetRevision {
        let selection = CapabilitySelection {
            capability: CapabilityRef {
                id: CAPABILITY_ID.into(),
                version: VERSION.into(),
            },
            action_allowlist,
        };
        let payload = AgentPresetRevisionPayload {
            schema_version: VERSION.into(),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![selection],
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "fixture".to_owned(),
            instructions: "fixture".to_owned(),
            starter_prompts: Vec::new(),
        };
        let mut revision = AgentPresetRevision {
            reference: PresetRevisionRef {
                preset_id: AgentPresetId::from("fixture.preset"),
                revision: 1,
                revision_digest: digest('0'),
            },
            payload,
            contribution_locks: vec![lock],
            created_by: UserId::from("fixture-user"),
            created_at_ms: 1,
            reason: None,
        };
        revision.reference.revision_digest = revision.revision_digest().unwrap();
        revision
    }

    fn environment() -> CompilerEnvironment {
        CompilerEnvironment {
            resolver_version: VERSION.into(),
            required_runtime_protocol_version: VERSION.into(),
            required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: digest('e'),
            available_runtime_features: BTreeSet::new(),
            installation_role_bindings: BTreeMap::new(),
            canonical_schema_manifest_digest: digest('f'),
            target_contribution_manifest_digest: digest('1'),
            host_target: RuntimeTarget::from("windows-desktop-x64"),
            host_surface: "desktop".to_owned(),
            availability_evidence_revision: "compiler-test".to_owned(),
        }
    }

    fn compile_request(
        revision: AgentPresetRevision,
        capability: ResolvedMiniAppCapability,
    ) -> CompileRequest {
        CompileRequest {
            revision,
            miniapp_capabilities: vec![capability],
            principal: PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: "fixture-user".to_owned(),
            },
            scene: "test".to_owned(),
            surface: "desktop".to_owned(),
            audience: "test".to_owned(),
            created_at_ms: 2,
            resolver_run_id: OperationId::from("compiler-test"),
        }
    }

    #[test]
    fn compiler_keeps_miniapp_out_of_registry_closure_and_freezes_initial_policy() {
        let action_allowlist = BTreeSet::from([ActionId::from(ACTION_ID)]);
        let capability = miniapp_capability(action_allowlist.clone());
        let saved_revision = revision(
            action_allowlist,
            capability.contribution_lock.clone(),
        );
        let compiled = AgentPresetCompiler::compile(
            &MaterializedRegistry::empty(),
            &environment(),
            compile_request(saved_revision, capability),
        )
        .expect("MiniApp capability should compile without Kernel registry materialization");

        assert!(compiled.content().enabled_capabilities.is_empty());
        assert_eq!(compiled.content().enabled_miniapp_capabilities.len(), 1);
        assert!(
            compiled
                .content()
                .capability_allowlist
                .contains(&CapabilityId::from(CAPABILITY_ID))
        );
        assert_eq!(
            compiled
                .policy(&CapabilityId::from(CAPABILITY_ID))
                .expect("MiniApp authority policy")
                .required_resource_kinds,
            BTreeSet::from(["workspace".into()])
        );
    }

    #[test]
    fn compiler_freezes_enabled_miniapp_and_rejects_lock_drift() {
        let action_allowlist = BTreeSet::new();
        let capability = miniapp_capability(action_allowlist.clone());
        let saved_revision = revision(
            action_allowlist,
            capability.contribution_lock.clone(),
        );
        let compiled = AgentPresetCompiler::compile(
            &MaterializedRegistry::empty(),
            &environment(),
            compile_request(saved_revision.clone(), capability.clone()),
        )
        .expect("enabled MiniApp capability should compile");
        assert_eq!(
            compiled
                .content()
                .enabled_miniapp_capabilities
                .first()
                .expect("enabled MiniApp projection")
                .active_release_epoch,
            7
        );

        let mut drifted_lock = capability.contribution_lock.clone();
        drifted_lock.source_identity = "miniapp:other".into();
        let drifted = revision(
            BTreeSet::new(),
            drifted_lock,
        );
        let error = AgentPresetCompiler::compile(
            &MaterializedRegistry::empty(),
            &environment(),
            compile_request(drifted, capability),
        )
        .expect_err("drifted MiniApp revision lock must fail closed");
        assert!(matches!(
            error,
            KernelError::CapabilityProvenanceDrift { .. }
        ));
    }
}
