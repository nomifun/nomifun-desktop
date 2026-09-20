use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    ActionId, AgentPresetRevision, CapabilityAuthoringPolicy, CapabilityConsumer, CapabilityId,
    CapabilityOperationLock, CapabilityRef, CapabilitySelection,
    ContributionSourceKind, DigestHex, ExecutionRoleId, InstallationRoleBinding,
    ModelRouteId, OperationId, PlatformConstraint,
    PrincipalRef, ResolvedCapability, ResolvedMcpToolLock, ResolvedRoleProviderLock,
    ResolvedSkillLock, ResolvedSnapshotContent,
    ResolvedSnapshotEnvelope, ResolvedSnapshotId, ResolvedSnapshotRef,
    ResourceBindingId, ResourceKind, RoleProviderSelection, RuntimeFeatureId,
    RuntimeProfileKind, RuntimeTarget, SkillId, TypedResourceBinding, VersionString,
    digest_payload,
};
use serde::Serialize;

use crate::{KernelError, MaterializedRegistry};

#[path = "compiler_dependencies.rs"]
mod dependencies;


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
    /// Exact Plugin Product Active Release projections resolved by the owning
    /// application service. Plugin Product capabilities are deliberately not
    /// materialized in the Kernel Plugin Registry.
    pub plugin_product_capabilities: Vec<ResolvedCapability>,
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

    /// Whether every resource kind required by a selected capability has a
    /// concrete target binding for this Session. A false result is not a
    /// compilation error: enhancement capabilities remain frozen but their
    /// Actions and lifecycle contributions must not be materialized.
    pub fn capability_resources_bound(
        &self,
        capability_id: &CapabilityId,
    ) -> Result<bool, KernelError> {
        let policy = self.policy(capability_id).ok_or_else(|| {
            KernelError::CapabilityNotInPreset {
                capability_id: capability_id.clone(),
            }
        })?;
        let mut bound_kinds = BTreeSet::new();
        for binding_id in &policy.resource_binding_ids {
            let binding = self.binding(binding_id).ok_or_else(|| {
                KernelError::InvalidPresetRevision {
                    reason: format!(
                        "capability {} references missing target resource binding {}",
                        capability_id.as_ref(),
                        binding_id.as_ref(),
                    ),
                }
            })?;
            bound_kinds.insert(binding.resource_kind.clone());
        }
        Ok(policy.required_resource_kinds.is_subset(&bound_kinds))
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

    pub(crate) fn require_contribution(&self, capability_id: &CapabilityId) -> Result<(), KernelError> {
        if self.resolved_capability(capability_id)
            .is_some_and(|value| value.consumption.is_contribution()) {
            Ok(())
        } else {
            Err(KernelError::CapabilityNotInPreset { capability_id: capability_id.clone() })
        }
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
        for (capability_id, policy) in &mut self.authority_policies {
            policy.resource_binding_ids.clear();
            for resource_kind in &policy.required_resource_kinds {
                let mut matches = by_kind.get(resource_kind).cloned().unwrap_or_default();
                // MCP mappings already freeze the server identity. A tool
                // receives only that server, never the Session's entire set.
                // The bundled resource provider explicitly selects one member
                // at dispatch. Its connect/OAuth dependencies share that set;
                // other unmapped/native consumers remain cardinality one.
                if resource_kind.as_ref() == "mcp_server"
                    && let Some(lock) = self.envelope.content.mcp_tool_locks.iter()
                        .find(|lock| &lock.capability_id == capability_id)
                {
                    matches.retain(|id| by_id.get(id).is_some_and(|binding|
                        binding.resource_id.as_ref() == lock.server_id.as_ref()));
                    if matches.len() != 1 {
                        return Err(KernelError::InvalidPresetRevision {
                            reason: format!("MCP capability {} requires one exact frozen server binding", capability_id.as_ref()),
                        });
                    }
                }
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    context_order: Vec<CapabilityId>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    middleware_order: Vec<CapabilityId>,
    profile_kind: RuntimeProfileKind,
    required_runtime_features: BTreeSet<RuntimeFeatureId>,
    capability_operation_locks: Vec<CapabilityOperationLock>,
    enabled_capabilities: Vec<ResolvedCapability>,
    required_resource_kinds: BTreeSet<ResourceKind>,
    authority_policies: BTreeMap<CapabilityId, CompiledCapabilityPolicy>,
    skill_ids: Vec<SkillId>,
    model_route_refs: BTreeMap<String, ModelRouteId>,
    resolved_role_providers: BTreeMap<ExecutionRoleId, ResolvedRoleProviderLock>,
}

pub struct AgentPresetCompiler;

impl AgentPresetCompiler {
    /// Authoring reuse uses the same Skill resolver as compilation. Artifact
    /// replacement must invalidate reuse even when the body itself is unchanged.
    pub fn skills_unchanged(
        registry: &MaterializedRegistry,
        revision: &AgentPresetRevision,
        snapshot: &ResolvedSnapshotEnvelope,
    ) -> bool {
        let direct = revision.payload.enabled_capabilities.iter()
            .map(|value| value.capability.id.clone()).collect();
        compile_skill_locks(registry, &revision.payload.skill_bindings, &direct, &snapshot.surface)
            .is_ok_and(|locks| locks == snapshot.content.skill_locks)
    }

    /// Validate an installation default using the same Role resolver as saves.
    /// Only required members are checked here; optional members and actual
    /// resource instances remain the responsibility of each consumer's admission.
    pub fn validate_role_default(
        registry: &MaterializedRegistry,
        environment: &CompilerEnvironment,
        selection: &RoleProviderSelection,
    ) -> Result<(), KernelError> {
        let role_id = &selection.role.key.role_id;
        let contract = registry.role_contract(role_id)
            .ok_or_else(|| KernelError::RoleProviderNotBound { role_id: role_id.clone() })?;
        let required = contract.manifest.members.iter()
            .filter(|member| member.requirement == nomifun_agent_contracts::RoleMemberRequirement::Required)
            .map(|member| member.capability.id.clone()).collect();
        resolve_role_provider_lock(registry, role_id, selection, &required, None, environment).map(|_| ())
    }

    /// Check the Role portion of an unchanged draft using the canonical resolver.
    ///
    /// This is an authoring-time reuse check, not execution admission: current
    /// defaults apply to a newly saved Snapshot, never to an existing Session.
    /// Resolution errors invalidate reuse so the normal compile can report them.
    /// Recompute the selected closure and consumption facts too: matching Role
    /// locks alone cannot prove that a saved dependency plan is still current.
    pub fn role_providers_unchanged(
        registry: &MaterializedRegistry,
        environment: &CompilerEnvironment,
        revision: &AgentPresetRevision,
        snapshot: &ResolvedSnapshotEnvelope,
    ) -> bool {
        let external: BTreeSet<_> = snapshot.content.enabled_capabilities.iter()
            .filter(|capability| capability.plugin_product_id.is_some())
            .map(|capability| capability.capability.id.clone()).collect();
        let roots = revision.payload.enabled_capabilities.iter()
            .map(|selection| selection.capability.id.clone())
            .filter(|id| !external.contains(id)).collect();
        let Ok(graph) = dependencies::resolve(registry, environment, revision, &roots)
            else { return false; };
        if graph.roles != snapshot.content.resolved_role_providers {
            return false;
        }
        let ceiling = graph.edges.keys().cloned().collect();
        if validate_conflicts(registry, &ceiling, &graph.roles, &external).is_err() {
            return false;
        }
        let direct = direct_selection_map(&revision.payload.enabled_capabilities)
            .into_iter()
            .filter(|(id, _)| registry.capability(id).is_some())
            .collect::<BTreeMap<_, _>>();
        let Ok(policies) = compile_authority_policies(registry, &direct, &ceiling)
            else { return false; };
        let Ok(mut capabilities) = resolved_capabilities(registry, &ceiling, &graph.paths, &policies)
            else { return false; };
        for capability in &mut capabilities {
            capability.consumption = if roots.contains(&capability.capability.id) {
                nomifun_agent_contracts::CapabilityConsumption::Contribution
            } else {
                nomifun_agent_contracts::CapabilityConsumption::Dependency
            };
            capability.dependency_refs = graph.edges[&capability.capability.id].clone();
        }
        apply_role_requirements(registry, &graph.roles, &mut capabilities).is_ok()
            && capabilities.iter().eq(snapshot.content.enabled_capabilities.iter()
                .filter(|capability| !external.contains(&capability.capability.id)))
    }

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
        let plugin_product_by_id = validate_plugin_product_inputs(
            registry,
            &request.revision,
            &request.plugin_product_capabilities,
        )?;
        let enabled_plugin_product_capabilities = request.plugin_product_capabilities.clone();
        let initial_plugin_direct = initial_direct
            .iter()
            .filter(|(capability_id, _)| {
                !plugin_product_by_id.contains_key(*capability_id)
            })
            .map(|(capability_id, selection)| {
                (capability_id.clone(), *selection)
            })
            .collect::<BTreeMap<_, _>>();
        let direct_ids = initial_direct
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();

        validate_direct_selections(registry, &initial_plugin_direct)?;
        for id in &request.revision.payload.context_order {
            let valid = registry.capability(id).is_some_and(|value| {
                value.manifest.contributes_context()
                    && value.manifest.supports_consumer(CapabilityConsumer::Agent)
            });
            if !valid {
                return Err(KernelError::InvalidPresetRevision {
                    reason: format!("context_order capability {} must publish Agent Context", id.as_ref()),
                });
            }
        }
        for id in &request.revision.payload.middleware_order {
            // Ordering follows the actual frozen middleware Action contract;
            // CapabilityKind is presentation metadata, not execution support.
            // Plugin Product actions may target a different source-integrated
            // consumer. The generic compiler freezes their order and identity;
            // the installed consumer validator decides phase support.
            let valid = plugin_product_by_id.contains_key(id)
                || registry.capability(id).is_some_and(|value| {
                    nomifun_agent_contracts::tool_middleware::phase_for_actions(
                        &value.manifest.contributions.actions,
                    )
                    .is_some()
                        && value.manifest.supports_consumer(CapabilityConsumer::Agent)
                });
            if !valid {
                return Err(KernelError::InvalidPresetRevision {
                    reason: format!("middleware_order capability {} must freeze a supported Agent middleware Action contribution", id.as_ref()),
                });
            }
        }
        validate_revision_contribution_locks(
            registry,
            &request.revision,
            &plugin_product_by_id,
        )?;
        for capability in plugin_product_by_id.values() {
            for feature in &capability.required_runtime_features {
                if !environment.available_runtime_features.contains(feature) {
                    return Err(KernelError::RuntimeFeatureUnavailable {
                        capability_id: capability.capability.id.clone(),
                        feature: feature.as_ref().to_owned(),
                    });
                }
            }
        }

        let graph = dependencies::resolve(registry, environment, &request.revision,
            &initial_plugin_direct.keys().cloned().collect())?;
        let ceiling = graph.edges.keys().cloned().collect::<BTreeSet<_>>();

        validate_capability_ceiling(registry, environment, &request.surface, &ceiling)?;

        let authority_policies = compile_authority_policies(
            registry,
            &initial_plugin_direct,
            &ceiling,
        )?;
        let mut enabled_capabilities = resolved_capabilities(
            registry,
            &ceiling,
            &graph.paths,
            &authority_policies,
        )?;
        for resolved in &mut enabled_capabilities {
            resolved.consumption = if direct_ids.contains(&resolved.capability.id) {
                nomifun_agent_contracts::CapabilityConsumption::Contribution
            } else {
                nomifun_agent_contracts::CapabilityConsumption::Dependency
            };
            resolved.dependency_refs = graph.edges[&resolved.capability.id].clone();
        }
        let mut authority_policies = authority_policies;
        merge_plugin_product_authority_policies(
            &mut authority_policies,
            &enabled_plugin_product_capabilities,
        )?;
        enabled_capabilities.extend(enabled_plugin_product_capabilities);
        enabled_capabilities.sort_by(|left, right| left.capability.cmp(&right.capability));
        let skill_locks = compile_skill_locks(
            registry,
            &request.revision.payload.skill_bindings,
            &direct_ids,
            &request.surface,
        )?;
        let mcp_tool_locks = compile_mcp_locks(registry, &direct_ids);
        let resolved_role_providers = graph.roles;
        validate_conflicts(
            registry,
            &ceiling,
            &resolved_role_providers,
            &plugin_product_by_id.keys().cloned().collect(),
        )?;
        apply_role_requirements(
            registry,
            &resolved_role_providers,
            &mut enabled_capabilities,
        )?;
        for capability in &enabled_capabilities {
            let policy = authority_policies.get_mut(&capability.capability.id)
                .ok_or_else(|| KernelError::CapabilityNotInPreset {
                    capability_id: capability.capability.id.clone(),
                })?;
            policy.required_resource_kinds = capability.required_resource_kinds.clone();
        }
        let capability_allowlist = enabled_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect::<BTreeSet<_>>();
        let capability_runtime_features = enabled_capabilities
            .iter()
            .flat_map(|capability| {
                capability.required_runtime_features.iter().cloned()
            })
            .collect::<BTreeSet<_>>();
        let required_runtime_features = capability_runtime_features;
        let required_resource_kinds = authority_policies
            .values()
            .flat_map(|policy| policy.required_resource_kinds.iter().cloned())
            .collect::<BTreeSet<_>>();
        let compiled_runtime_profile_digest =
            digest_payload(&CompiledRuntimeProfileDigestInput {
                context_order: request.revision.payload.context_order.clone(),
                middleware_order: request.revision.payload.middleware_order.clone(),
                profile_kind: environment.required_runtime_profile,
                required_runtime_features: required_runtime_features.clone(),
                capability_operation_locks: enabled_capabilities
                    .iter()
                    .map(resolved_capability_operation_lock)
                    .collect(),
                enabled_capabilities: enabled_capabilities.clone(),
                required_resource_kinds: required_resource_kinds.clone(),
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
            context_order: request.revision.payload.context_order,
            middleware_order: request.revision.payload.middleware_order,
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

fn validate_plugin_product_inputs<'a>(
    registry: &MaterializedRegistry,
    revision: &AgentPresetRevision,
    capabilities: &'a [ResolvedCapability],
) -> Result<BTreeMap<CapabilityId, &'a ResolvedCapability>, KernelError> {
    let mut by_id = BTreeMap::new();
    let mut contribution_ids = BTreeSet::new();
    let mut publication_facts = BTreeMap::new();

    for capability in capabilities {
        capability
            .validate()
            .map_err(|error| KernelError::InvalidPresetRevision {
                reason: error.message,
            })?;
        if !capability.consumption.is_contribution() || !capability.dependency_refs.is_empty() {
            return Err(KernelError::InvalidPresetRevision {
                reason: "Plugin Product projections cannot supply compiler-owned dependency graph fields".into(),
            });
        }
        if capability.contribution_lock.source_kind
            != ContributionSourceKind::PluginProductActiveRelease
        {
            return Err(KernelError::InvalidPresetRevision {
                reason: "application-supplied capabilities must bind a Plugin Product Active Release"
                    .to_owned(),
            });
        }
        let product_id = capability.plugin_product_id.as_ref().ok_or_else(|| {
            KernelError::InvalidPresetRevision {
                reason: "Plugin Product capability is missing plugin_product_id".to_owned(),
            }
        })?;
        let release = capability.active_release.as_ref().ok_or_else(|| {
            KernelError::InvalidPresetRevision {
                reason: "Plugin Product capability is missing active_release".to_owned(),
            }
        })?;
        if capability.target_artifact_digest != release.release_digest {
            return Err(KernelError::InvalidPresetRevision {
                reason:
                    "Plugin Product capability target Artifact differs from its Active Release"
                        .to_owned(),
            });
        }
        if capability.capability.id.as_ref().trim().is_empty()
            || capability.capability.version.as_ref().trim().is_empty()
        {
            return Err(KernelError::InvalidPresetRevision {
                reason: "Plugin Product capability reference must be non-empty".to_owned(),
            });
        }
        if registry.capability(&capability.capability.id).is_some() {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "Plugin Product capability {} must not be present in the Kernel Plugin Registry",
                    capability.capability.id.as_ref()
                ),
            });
        }

        let mut action_ids = BTreeSet::new();
        for action in &capability.actions {
            if action.action_id.as_ref().trim().is_empty() {
                return Err(KernelError::InvalidPresetRevision {
                    reason: format!(
                        "Plugin Product capability {} contains an empty action ID",
                        capability.capability.id.as_ref()
                    ),
                });
            }
            if !action_ids.insert(action.action_id.clone()) {
                return Err(KernelError::InvalidPresetRevision {
                    reason: format!(
                        "Plugin Product capability {} declares duplicate action {}",
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
                    "Plugin Product capability {} contains an empty resource kind",
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
                    "Plugin Product capability {} is supplied more than once",
                    capability.capability.id.as_ref()
                ),
            });
        }
        if !contribution_ids.insert(capability.contribution_id.clone()) {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "Plugin Product contribution {} is supplied more than once",
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
            .insert(product_id.clone(), facts.clone())
        {
            if existing != facts {
                return Err(KernelError::InvalidPresetRevision {
                    reason: format!(
                        "Plugin Product {} has inconsistent Active Release or Catalog facts",
                        product_id.as_ref()
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
                    "Plugin Product capability {} is not selected by the Revision",
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
                    "Plugin Product capability {} action allowlist differs from its Revision selection",
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
        let authoring = capability
            .manifest
            .authoring_policy()
            .map_err(|reason| KernelError::InvalidPresetRevision { reason })?;
        if authoring != CapabilityAuthoringPolicy::Direct {
            return Err(KernelError::CapabilityNotAuthorable {
                capability_id: capability.manifest.id.clone(),
                policy: authoring.as_str().to_owned(),
            });
        }
        let declared_actions = capability
            .manifest
            .contributions
            .actions
            .iter()
            .map(|action| action.action_id.clone())
            .collect::<BTreeSet<_>>();
        if !declared_actions.is_empty() && selection.action_allowlist.is_empty() {
            return Err(KernelError::ActionGrantRequired {
                capability_id: capability.manifest.id.clone(),
            });
        }
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
    plugin_product_capabilities: &BTreeMap<CapabilityId, &ResolvedCapability>,
) -> Result<(), KernelError> {
    for selection in revision
        .payload
        .enabled_capabilities
        .iter()
    {
        if let Some(plugin_product) =
            plugin_product_capabilities.get(&selection.capability.id)
        {
            if plugin_product.capability != selection.capability {
                return Err(KernelError::CapabilityNotMaterialized {
                    capability_id: selection.capability.id.clone(),
                    version: selection.capability.version.clone(),
                });
            }
            let frozen = revision
                .contribution_locks
                .iter()
                .find(|lock| lock.contribution_id == plugin_product.contribution_id)
                .ok_or_else(|| KernelError::CapabilityProvenanceDrift {
                    capability_id: selection.capability.id.clone(),
                    reason: format!(
                        "Revision is missing Plugin Product contribution lock {}",
                        plugin_product.contribution_id.as_ref()
                    ),
                })?;
            if frozen != &plugin_product.contribution_lock {
                return Err(KernelError::CapabilityProvenanceDrift {
                    capability_id: selection.capability.id.clone(),
                    reason: "Revision Plugin Product contribution lock does not match the exact projection"
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
        if lock.source_kind == ContributionSourceKind::PluginProductActiveRelease
            && !plugin_product_capabilities
                .values()
                .any(|capability| capability.contribution_lock == *lock)
        {
            return Err(KernelError::InvalidPresetRevision {
                reason: format!(
                    "Revision contains an unselected Plugin Product contribution lock {}",
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
    locks: &BTreeMap<ExecutionRoleId, ResolvedRoleProviderLock>,
    external: &BTreeSet<CapabilityId>,
) -> Result<(), KernelError> {
    // A conflict concerns actual consumption, not Catalog membership or public
    // Tool visibility. Keep the facade's contract constraints and additionally
    // check the selected implementation and the resource factories it uses.
    // This set never becomes an allowlist or a source of authority policies.
    let mut consumed = ceiling.clone();
    for (role_id, lock) in locks {
        let provider = registry.role_provider(role_id, &lock.provider.mount_id)
            .ok_or_else(|| KernelError::RoleProviderUnavailable {
                role_id: role_id.clone(),
                mount_id: lock.provider.mount_id.clone(),
            })?;
        for id in ceiling.iter().filter(|id| registry.role_for_capability(id) == Some(role_id)) {
            let member = provider.contribution.members.get(id)
                .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                    role_id: role_id.clone(), capability_id: id.clone(),
                })?;
            for (member_id, implementation) in std::iter::once((id, member))
                .chain(registry.role_resource_members(provider, id))
            {
                consumed.insert(member_id.clone());
                if let Some(implementation) = &implementation.implementation {
                    consumed.insert(implementation.id.clone());
                }
            }
        }
    }
    for capability_id in &consumed {
        let capability = &registry.capability(capability_id)
            .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                capability_id: capability_id.clone(), version: "unknown".into(),
            })?.manifest;
        if let Some(conflict) = capability
            .conflicts
            .iter()
            .find(|conflict| consumed.contains(&conflict.capability.id)
                || external.contains(&conflict.capability.id))
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
    ceiling: &BTreeSet<CapabilityId>,
) -> Result<BTreeMap<CapabilityId, CompiledCapabilityPolicy>, KernelError> {
    let mut policies = BTreeMap::<CapabilityId, CompiledCapabilityPolicy>::new();
    for capability_id in ceiling {
        let capability = &registry.capabilities[capability_id].manifest;
        let declared_actions = capability.contributions.actions.iter()
            .map(|action| action.action_id.clone()).collect::<BTreeSet<_>>();
        // Direct grants are exact. Dependency-only modules receive their
        // declared actions solely for scoped dependency calls; they never
        // become direct Snapshot contributions.
        let allowed_actions = initial_direct
            .get(capability_id)
            .map(|direct| direct.action_allowlist.clone())
            .unwrap_or(declared_actions);
        policies.insert(capability_id.clone(), CompiledCapabilityPolicy {
            allowed_actions,
            resource_binding_ids: BTreeSet::new(),
            required_resource_kinds: capability.contributions.resource_kinds.clone(),
        });
    }
    Ok(policies)
}

fn resolved_capabilities(
    registry: &MaterializedRegistry,
    capability_ids: &BTreeSet<CapabilityId>,
    paths: &BTreeMap<CapabilityId, Vec<CapabilityId>>,
    policies: &BTreeMap<CapabilityId, CompiledCapabilityPolicy>,
) -> Result<Vec<ResolvedCapability>, KernelError> {
    capability_ids
        .iter()
        .map(|capability_id| {
            let capability = &registry.capabilities[capability_id];
            let policy = policies.get(capability_id).ok_or_else(|| {
                KernelError::CapabilityNotInPreset {
                    capability_id: capability_id.clone(),
                }
            })?;
            Ok(ResolvedCapability {
                consumption: Default::default(),
                dependency_refs: Vec::new(),
                capability: CapabilityRef {
                    id: capability_id.clone(),
                    version: capability.manifest.version.clone(),
                },
                source_package: capability.manifest.package.clone(),
                contribution_id: capability.contribution_id.clone(),
                contribution_lock: capability.contribution_lock.clone(),
                resolved_mount_id: Some(capability.mount_id.clone()),
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
                plugin_product_id: None,
                active_release: None,
                active_release_epoch: None,
                catalog_digest: None,
                display_name: None,
                description: None,
                actions: capability.manifest.contributions.actions.clone(),
                required_resource_kinds: capability.manifest.contributions.resource_kinds.clone(),
                action_allowlist: policy.allowed_actions.clone(),
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

fn merge_plugin_product_authority_policies(
    policies: &mut BTreeMap<CapabilityId, CompiledCapabilityPolicy>,
    initial: &[ResolvedCapability],
) -> Result<(), KernelError> {
    for capability in initial.iter() {
        let allowed_actions = capability.action_allowlist.clone();
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
                    "Plugin Product capability {} collides with a Plugin capability policy",
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
    surface: &str,
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
        let surfaces = &skill.definition.supported_surfaces;
        let has_consumers = surfaces.iter().any(|value| value.starts_with("consumer:"));
        let has_hosts = surfaces.iter().any(|value| !value.starts_with("consumer:"));
        if (has_hosts && !surfaces.contains(surface))
            || (has_consumers && !surfaces.contains("consumer:agent"))
        {
            return Err(KernelError::SkillUnavailableOnSurface { skill_id: skill_ref.id.clone(), surface: surface.into() });
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
            contribution_lock: skill.contribution_lock.clone(),
            resolved_mount_id: skill.mount_id.clone(),
            resolved_source: skill.source.clone(),
            target_artifact_digest: skill.target_artifact_digest.clone(),
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

/// Project selected implementation requirements into the existing policy and
/// profile. No implementation capability is added to the public Tool set and
/// no action or resource instance is granted by selecting a Provider.
fn apply_role_requirements(
    registry: &MaterializedRegistry,
    locks: &BTreeMap<ExecutionRoleId, ResolvedRoleProviderLock>,
    capabilities: &mut [ResolvedCapability],
) -> Result<(), KernelError> {
    for resolved in capabilities {
        let id = &resolved.capability.id;
        let Some(role) = registry.role_for_capability(id) else {
            continue;
        };
        let lock = locks.get(role).ok_or_else(|| KernelError::RoleProviderNotBound {
            role_id: role.clone(),
        })?;
        let provider = registry.role_provider(role, &lock.provider.mount_id)
            .ok_or_else(|| KernelError::RoleProviderUnavailable {
                role_id: role.clone(),
                mount_id: lock.provider.mount_id.clone(),
            })?;
        let member = provider.contribution.members.get(id)
            .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                role_id: role.clone(),
                capability_id: id.clone(),
            })?;
        // These are execution requirements of the selected implementation,
        // not another copy of the facade manifest. Aggregates are derived from
        // these records; provenance/schema locks keep their canonical identity.
        resolved.required_resource_kinds = member.required_resource_kinds.clone();
        resolved.required_runtime_features.clear();
        for (member_id, requirement) in std::iter::once((id, member))
            .chain(registry.role_resource_members(provider, id))
        {
            for capability_id in std::iter::once(member_id)
                .chain(requirement.implementation.as_ref().map(|value| &value.id))
            {
                let capability = registry.capability(capability_id)
                    .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                        capability_id: capability_id.clone(),
                        version: "unknown".into(),
                    })?;
                resolved.required_runtime_features.extend(capability.manifest.requires_runtime_features
                    .iter().map(|value| value.id.clone()));
            }
        }
    }
    Ok(())
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
    let lock = resolve_role_provider_lock(
        registry,
        role_id,
        selection,
        selected_members,
        Some(bindings),
        environment,
    )?;
    // Non-Agent admission has an actual selected member set too. A default
    // binding alone is not a plan to consume every required member together.
    validate_conflicts(
        registry,
        selected_members,
        &BTreeMap::from([(role_id.clone(), lock.clone())]),
        &BTreeSet::new(),
    )?;
    Ok(lock)
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

    // Resource factories used internally by a selected Tool/Context must pass
    // the same admission as explicitly selected exports, without making them
    // public Tools or activating their unrelated Role members.
    let mut consumed_members = selected_members.clone();
    for id in selected_members {
        consumed_members.extend(registry.role_resource_members(provider, id)
            .into_iter().map(|(id, _)| id.clone()));
    }
    validate_capability_ceiling(
        registry, environment, &environment.host_surface, &consumed_members,
    )?;
    for capability_id in &consumed_members {
        let member = provider
            .contribution
            .members
            .get(capability_id)
            .ok_or_else(|| KernelError::RoleProviderMemberUnavailable {
                role_id: role_id.clone(),
                capability_id: capability_id.clone(),
            })?;
        if let Some(implementation) = &member.implementation {
            // The selected facade's availability cannot stand in for the
            // implementation's platform/surface/features. Validation does not
            // add the implementation to model tools or grant any authority.
            validate_capability_ceiling(
                registry,
                environment,
                &environment.host_surface,
                &BTreeSet::from([implementation.id.clone()]),
            )?;
        }
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
        ContributionLock, ContributionSourceKind, DigestHex, EffectClass, PluginProductId,
        PluginReleaseId, PluginReleaseRef, PluginSourceKind, PluginSourceMetadata,
        PackageId, PackageRef, PresetRevisionRef, PrincipalRef, ResolvedCapability,
        ToolPresentationKind, UserId,
    };

    use super::*;

    const VERSION: &str = "1.0.0";
    const CAPABILITY_ID: &str = "plugin.fixture.echo";
    const ACTION_ID: &str = "plugin.fixture.echo.invoke";
    const CONTRIBUTION_ID: &str = "capability:plugin.fixture.echo";
    const PLUGIN_PRODUCT_ID: &str = "plugin-fixture";

    fn digest(fill: char) -> DigestHex {
        DigestHex::from(fill.to_string().repeat(64))
    }

    fn plugin_product_capability(action_allowlist: BTreeSet<ActionId>) -> ResolvedCapability {
        let contribution_lock = ContributionLock {
            source_kind: ContributionSourceKind::PluginProductActiveRelease,
            source_identity: format!("plugin-product:{PLUGIN_PRODUCT_ID}").into(),
            mount_id: None,
            plugin_product_id: Some(PluginProductId::from(PLUGIN_PRODUCT_ID)),
            mcp_binding_id: None,
            contribution_id: ContributionId::from(CONTRIBUTION_ID),
            contract_digest: digest('a'),
        };
        ResolvedCapability {
            consumption: Default::default(),
            dependency_refs: Vec::new(),
            capability: CapabilityRef {
                id: CAPABILITY_ID.into(),
                version: VERSION.into(),
            },
            source_package: PackageRef {
                id: PackageId::from("plugin.fixture"),
                version: VERSION.into(),
            },
            contribution_id: ContributionId::from(CONTRIBUTION_ID),
            contribution_lock,
            resolved_mount_id: None,
            resolved_source: PluginSourceMetadata {
                source_kind: PluginSourceKind::ManagedLocal,
                source_identity: format!("plugin-product:{PLUGIN_PRODUCT_ID}"),
                source_digest: Some(digest('b')),
            },
            target_artifact_digest: digest('b'),
            schema_digest: digest('a'),
            dependency_path: vec![CapabilityId::from(CAPABILITY_ID)],
            required_runtime_features: BTreeSet::new(),
            plugin_product_id: Some(PluginProductId::from(PLUGIN_PRODUCT_ID)),
            active_release: Some(PluginReleaseRef {
                release_id: PluginReleaseId::from("release-fixture"),
                artifact_id: "artifact-fixture".into(),
                release_digest: digest('b'),
                manifest_digest: digest('c'),
            }),
            active_release_epoch: Some(7),
            catalog_digest: Some(digest('d')),
            display_name: Some("Fixture Echo".to_owned()),
            description: Some("Echo from a Plugin Product Active Release".to_owned()),
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from(ACTION_ID),
                input_schema: "schema://plugin.fixture.echo/input".into(),
                output_schema: "schema://plugin.fixture.echo/output".into(),
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
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: VERSION.into(),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: vec![selection],
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: "fixture".to_owned(),
            instructions: "fixture".to_owned(),
            starter_prompts: Vec::new(),
            runtime_policy: Default::default(),
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
        capability: ResolvedCapability,
    ) -> CompileRequest {
        CompileRequest {
            revision,
            plugin_product_capabilities: vec![capability],
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
    fn middleware_order_accepts_alternative_product_contract_and_freezes_it() {
        let capability = plugin_product_capability(BTreeSet::from([ACTION_ID.into()]));
        let mut saved_revision = revision(
            capability.action_allowlist.clone(),
            capability.contribution_lock.clone(),
        );
        let unordered = AgentPresetCompiler::compile(
            &MaterializedRegistry::empty(),
            &environment(),
            compile_request(saved_revision.clone(), capability.clone()),
        ).unwrap();
        saved_revision.payload.middleware_order = vec![CAPABILITY_ID.into()];
        saved_revision.reference.revision_digest = saved_revision.revision_digest().unwrap();
        let compiled = AgentPresetCompiler::compile(
            &MaterializedRegistry::empty(),
            &environment(),
            compile_request(saved_revision, capability.clone()),
        ).expect("consumer support is not a generic compiler concern");

        assert_eq!(compiled.content().middleware_order, vec![CapabilityId::from(CAPABILITY_ID)]);
        assert_eq!(compiled.content().enabled_capabilities, vec![capability.clone()]);
        assert_eq!(compiled.policy(&CAPABILITY_ID.into()), unordered.policy(&CAPABILITY_ID.into()));
        assert_ne!(compiled.snapshot_ref(), unordered.snapshot_ref());
        assert_ne!(compiled.content().compiled_runtime_profile_digest,
            unordered.content().compiled_runtime_profile_digest);
    }

    #[test]
    fn middleware_order_still_rejects_duplicate_and_unselected_product_entries() {
        let granted = BTreeSet::from([ActionId::from(ACTION_ID)]);
        let capability = plugin_product_capability(granted.clone());
        for order in [vec![CAPABILITY_ID, CAPABILITY_ID], vec!["plugin.unselected"]] {
            let mut saved_revision = revision(granted.clone(), capability.contribution_lock.clone());
            saved_revision.payload.middleware_order = order.into_iter().map(Into::into).collect();
            saved_revision.reference.revision_digest = saved_revision.revision_digest().unwrap();
            let error = AgentPresetCompiler::compile(
                &MaterializedRegistry::empty(),
                &environment(),
                compile_request(saved_revision, capability.clone()),
            ).expect_err("middleware order must be unique and directly selected");
            assert!(matches!(error, KernelError::InvalidPresetRevision { reason }
                if reason.contains("middleware_order contains duplicate or unselected")));
        }
    }

    #[test]
    fn middleware_order_still_rejects_product_identity_drift() {
        let granted = BTreeSet::from([ActionId::from(ACTION_ID)]);
        let capability = plugin_product_capability(granted.clone());
        let mut saved_revision = revision(granted, capability.contribution_lock.clone());
        saved_revision.payload.middleware_order = vec![CAPABILITY_ID.into()];
        saved_revision.payload.enabled_capabilities[0].capability.version = "2.0.0".into();
        saved_revision.reference.revision_digest = saved_revision.revision_digest().unwrap();
        assert!(matches!(AgentPresetCompiler::compile(
            &MaterializedRegistry::empty(), &environment(),
            compile_request(saved_revision.clone(), capability.clone()),
        ), Err(KernelError::CapabilityNotMaterialized { .. })));

        saved_revision.payload.enabled_capabilities[0].capability = capability.capability.clone();
        saved_revision.contribution_locks[0].contract_digest = digest('e');
        saved_revision.reference.revision_digest = saved_revision.revision_digest().unwrap();
        assert!(matches!(AgentPresetCompiler::compile(
            &MaterializedRegistry::empty(), &environment(),
            compile_request(saved_revision, capability),
        ), Err(KernelError::CapabilityProvenanceDrift { .. })));
    }

    #[test]
    fn compiler_keeps_plugin_product_out_of_registry_closure_and_freezes_initial_policy() {
        let action_allowlist = BTreeSet::from([ActionId::from(ACTION_ID)]);
        let capability = plugin_product_capability(action_allowlist.clone());
        let saved_revision = revision(
            action_allowlist.clone(),
            capability.contribution_lock.clone(),
        );
        let compiled = AgentPresetCompiler::compile(
            &MaterializedRegistry::empty(),
            &environment(),
            compile_request(saved_revision, capability),
        )
        .expect("Plugin Product capability should compile without Kernel registry materialization");

        assert_eq!(compiled.content().enabled_capabilities.len(), 1);
        assert!(
            compiled
                .content()
                .capability_allowlist
                .contains(&CapabilityId::from(CAPABILITY_ID))
        );
        assert_eq!(
            compiled
                .policy(&CapabilityId::from(CAPABILITY_ID))
                .expect("Plugin Product authority policy")
                .required_resource_kinds,
            BTreeSet::from(["workspace".into()])
        );
    }

    #[test]
    fn unbound_enhancement_resources_are_detected_without_invalidating_the_snapshot() {
        let action_allowlist = BTreeSet::from([ActionId::from(ACTION_ID)]);
        let capability = plugin_product_capability(action_allowlist.clone());
        let saved_revision = revision(
            action_allowlist,
            capability.contribution_lock.clone(),
        );
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "fixture-user".to_owned(),
        };
        let compiled = AgentPresetCompiler::compile(
            &MaterializedRegistry::empty(),
            &environment(),
            compile_request(saved_revision, capability),
        )
        .unwrap();
        let capability_id = CapabilityId::from(CAPABILITY_ID);
        assert!(!compiled.capability_resources_bound(&capability_id).unwrap());

        let bound = compiled
            .with_target_resource_bindings(
                &principal,
                vec![TypedResourceBinding {
                    binding_id: ResourceBindingId::from("workspace:fixture"),
                    resource_kind: ResourceKind::from("workspace"),
                    resource_id: "fixture".into(),
                    owner_id: principal.principal_id.clone(),
                    operations: BTreeSet::from(["read".to_owned()]),
                    connection_config_ref: None,
                    typed_parameters: BTreeMap::new(),
                }],
            )
            .unwrap();
        assert!(bound.capability_resources_bound(&capability_id).unwrap());
    }

    #[test]
    fn compiler_freezes_enabled_plugin_product_and_rejects_lock_drift() {
        let action_allowlist = BTreeSet::from([ActionId::from(ACTION_ID)]);
        let capability = plugin_product_capability(action_allowlist.clone());
        let saved_revision = revision(
            action_allowlist.clone(),
            capability.contribution_lock.clone(),
        );
        let compiled = AgentPresetCompiler::compile(
            &MaterializedRegistry::empty(),
            &environment(),
            compile_request(saved_revision.clone(), capability.clone()),
        )
        .expect("enabled Plugin Product capability should compile");
        assert_eq!(
            compiled
                .content()
                .enabled_capabilities
                .first()
                .expect("enabled Plugin Product projection")
                .active_release_epoch,
            Some(7)
        );

        let mut drifted_lock = capability.contribution_lock.clone();
        drifted_lock.source_identity = "plugin-product:other".into();
        let drifted = revision(
            action_allowlist,
            drifted_lock,
        );
        let error = AgentPresetCompiler::compile(
            &MaterializedRegistry::empty(),
            &environment(),
            compile_request(drifted, capability),
        )
        .expect_err("drifted Plugin Product revision lock must fail closed");
        assert!(matches!(
            error,
            KernelError::CapabilityProvenanceDrift { .. }
        ));
    }
}
