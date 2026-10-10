//! Current-generation Session binding derivation and forward contract evolution.
//!
//! A bundled publisher may replace its implementation without replacing the
//! Session's authority. This is a new execution binding, not a claim that old
//! schema inputs or native checkpoints can execute against the new artifact.
//! Closed history remains immutable data across the canonical transition.

use crate::{
    AgentPresetRevision, CanonicalErrorCode, CapabilityId, CHAT_MODEL_TASK_AGENT_CHAT,
    ContributionSourceKind, PluginSourceKind, PresetContractViolation, ResolvedCapability,
    ResolvedRoleProviderLock, ResolvedSnapshotEnvelope,
};
use std::collections::{BTreeMap, BTreeSet};

pub const SESSION_CONTRACT_EVOLUTION_REJECTED: &str = "AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED";

fn rejected(message: impl Into<String>) -> PresetContractViolation {
    PresetContractViolation {
        code: CanonicalErrorCode::from(SESSION_CONTRACT_EVOLUTION_REJECTED),
        message: message.into(),
    }
}

fn validate_artifacts(
    revision: &AgentPresetRevision,
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<(), PresetContractViolation> {
    revision.validate()?;
    snapshot.validate()?;
    if snapshot.content.preset_revision_ref != revision.reference
        || snapshot.content.model_route_refs != revision.payload.model_route_refs
        || snapshot.content.chat_route_identity != revision.chat_route_identity()?
    {
        return Err(rejected("Revision and Snapshot do not identify the same frozen binding"));
    }
    Ok(())
}

/// Validate the input to the dedicated model derivation. Only the canonical
/// Chat route can change; capability resolution and installation defaults are
/// deliberately absent from this operation.
pub fn validate_model_only_revision_derivation(
    source: &AgentPresetRevision,
    snapshot: &ResolvedSnapshotEnvelope,
    target: &AgentPresetRevision,
) -> Result<(), PresetContractViolation> {
    validate_artifacts(source, snapshot)?;
    target.validate()?;
    if !source.payload.model_route_refs.contains_key(CHAT_MODEL_TASK_AGENT_CHAT)
        || !target.payload.model_route_refs.contains_key(CHAT_MODEL_TASK_AGENT_CHAT)
    {
        return Err(rejected("model derivation requires an existing canonical Chat route"));
    }
    let mut payload = target.payload.clone();
    payload.model_route_refs.remove(CHAT_MODEL_TASK_AGENT_CHAT);
    payload.chat_route_records.remove(CHAT_MODEL_TASK_AGENT_CHAT);
    let mut original = source.payload.clone();
    original.model_route_refs.remove(CHAT_MODEL_TASK_AGENT_CHAT);
    original.chat_route_records.remove(CHAT_MODEL_TASK_AGENT_CHAT);
    let mut locks = target.contribution_locks.clone();
    let mut original_locks = source.contribution_locks.clone();
    locks.sort();
    original_locks.sort();
    if payload != original || locks != original_locks || source.created_by != target.created_by {
        return Err(rejected("model derivation must preserve every non-model authorial field and contribution lock"));
    }
    Ok(())
}

/// Admit a fresh forward execution contract from the same bundled publishers.
/// Exact schemas and artifacts may change, but grants, effects, resources,
/// dependency graph, provider mounts and runtime requirements may not. MCP,
/// managed packages and Skill content must remain exact.
pub fn validate_bundled_contract_evolution(
    source_revision: &AgentPresetRevision,
    source: &ResolvedSnapshotEnvelope,
    target_revision: &AgentPresetRevision,
    target: &ResolvedSnapshotEnvelope,
) -> Result<(), PresetContractViolation> {
    validate_artifacts(source_revision, source)?;
    validate_artifacts(target_revision, target)?;
    if target_revision.payload != source_revision.payload || source_revision.created_by != target_revision.created_by {
        return Err(rejected("contract evolution must preserve the Session's authorial configuration"));
    }
    let left = &source.content;
    let right = &target.content;
    if left.schema_version != right.schema_version
        || left.resolver_version != right.resolver_version
        || left.required_runtime_protocol_version != right.required_runtime_protocol_version
        || left.required_runtime_profile != right.required_runtime_profile
        || left.runtime_feature_inventory_digest != right.runtime_feature_inventory_digest
        || left.required_runtime_features != right.required_runtime_features
        || left.canonical_schema_manifest_digest != right.canonical_schema_manifest_digest
        || left.context_order != right.context_order
        || left.middleware_order != right.middleware_order
        || left.required_resource_kinds != right.required_resource_kinds
        || left.capability_allowlist != right.capability_allowlist
        || left.skill_locks != right.skill_locks
        || left.mcp_tool_locks != right.mcp_tool_locks
        || source.actor != target.actor || source.scene != target.scene
        || source.surface != target.surface || source.audience != target.audience
    {
        return Err(rejected("contract evolution changed generation, authority, runtime requirements, Skill or MCP identity"));
    }
    let mut normalized = right.clone();
    for capability in &mut normalized.enabled_capabilities {
        let original = left.enabled_capabilities.iter()
            .find(|value| value.capability == capability.capability)
            .ok_or_else(|| rejected("contract evolution introduced a capability"))?;
        validate_capability_evolution(original, capability)?;
        *capability = original.clone();
    }
    if normalized.enabled_capabilities != left.enabled_capabilities {
        return Err(rejected("contract evolution changed the selected dependency graph"));
    }
    for (role, provider) in &mut normalized.resolved_role_providers {
        let original = left.resolved_role_providers.get(role)
            .ok_or_else(|| rejected("contract evolution introduced a role provider"))?;
        validate_provider_evolution(original, provider)?;
        *provider = original.clone();
    }
    if normalized.resolved_role_providers != left.resolved_role_providers {
        return Err(rejected("contract evolution removed a role provider"));
    }
    let mut target_locks = target_revision.contribution_locks.clone();
    let mut source_locks = source_revision.contribution_locks.clone();
    for lock in &mut target_locks {
        if let Some(capability) = right.enabled_capabilities.iter()
            .find(|capability| &capability.contribution_lock == lock)
            && let Some(original) = left.enabled_capabilities.iter()
                .find(|original| original.capability == capability.capability)
        {
            *lock = original.contribution_lock.clone();
        }
    }
    target_locks.sort();
    source_locks.sort();
    if target_locks != source_locks {
        return Err(rejected("contract evolution changed an unproven contribution lock"));
    }
    // Only fresh compilation identities may differ globally. Comparing the
    // complete content keeps any future field frozen unless this contract
    // explicitly grants it forward-evolution authority.
    normalized.preset_revision_ref = left.preset_revision_ref.clone();
    if let (Some(candidate), Some(original)) =
        (&mut normalized.chat_route_identity, &left.chat_route_identity)
    {
        candidate.preset_revision_id = original.preset_revision_id.clone();
    }
    normalized.compiled_runtime_profile_digest = left.compiled_runtime_profile_digest.clone();
    normalized.target_contribution_manifest_digest = left.target_contribution_manifest_digest.clone();
    if &normalized != left {
        return Err(rejected("contract evolution changed an unproven global execution requirement"));
    }
    Ok(())
}

/// Prove one explicit Session extension selection against its final compiled
/// plan. The owner supplies the authorized direct roots; an extension's name
/// alone never grants authority to replace a core dependency. Retained host
/// infrastructure can be protected even when it belongs to the mutable roots.
///
/// Skill selection and captured content are part of this explicit operation.
/// Every other authorial field and global execution requirement stays frozen.
/// The complete closure of each protected root keeps its authority, resources,
/// effects, provider and graph. Only those same bundled publishers may refresh
/// schemas/artifacts; managed packages and MCP dependencies stay exact.
pub fn validate_session_extension_derivation(
    source_revision: &AgentPresetRevision,
    source: &ResolvedSnapshotEnvelope,
    target_revision: &AgentPresetRevision,
    target: &ResolvedSnapshotEnvelope,
    mutable_roots: &BTreeSet<CapabilityId>,
    protected_retained_roots: &BTreeSet<CapabilityId>,
) -> Result<(), PresetContractViolation> {
    validate_artifacts(source_revision, source)?;
    validate_artifacts(target_revision, target)?;
    let source_roots = direct_roots(source_revision, source)?;
    let target_roots = direct_roots(target_revision, target)?;
    let available_roots = source_roots.union(&target_roots).cloned().collect::<BTreeSet<_>>();
    if !mutable_roots.is_subset(&available_roots)
        || !protected_retained_roots.is_subset(mutable_roots)
        || !protected_retained_roots.is_subset(&source_roots)
        || !protected_retained_roots.is_subset(&target_roots)
    {
        return Err(rejected("extension derivation intent does not identify selected direct roots"));
    }
    let mut original_payload = source_revision.payload.clone();
    let mut candidate_payload = target_revision.payload.clone();
    for payload in [&mut original_payload, &mut candidate_payload] {
        payload.enabled_capabilities.retain(|selection| !mutable_roots.contains(&selection.capability.id));
        payload.enabled_capabilities.sort_by(|left, right| left.capability.cmp(&right.capability));
        payload.skill_bindings.clear();
    }
    if original_payload != candidate_payload || source_revision.created_by != target_revision.created_by {
        return Err(rejected("extension selection changed a frozen authorial field or core grant"));
    }

    let protected_roots = source_roots.difference(mutable_roots)
        .chain(protected_retained_roots.iter()).cloned().collect::<BTreeSet<_>>();
    let original_core = dependency_closure(source, &protected_roots)?;
    let candidate_core = dependency_closure(target, &protected_roots)?;
    if original_core != candidate_core {
        return Err(rejected("extension selection changed a protected dependency closure"));
    }
    let original_nodes = capability_nodes(source);
    let candidate_nodes = capability_nodes(target);
    for id in &original_core {
        let original = original_nodes[id];
        let mut candidate = (*candidate_nodes[id]).clone();
        // The compiler's diagnostic path uses the first selected root. An
        // explicit root deletion may recompute it, while the proven closure
        // and exact direct edges retain the same execution authority.
        candidate.dependency_path = original.dependency_path.clone();
        validate_capability_evolution(original, &candidate)?;
    }
    for (revision, snapshot, roots) in [
        (source_revision, source, &source_roots), (target_revision, target, &target_roots),
    ] {
        if dependency_closure(snapshot, roots)?.len() != snapshot.content.enabled_capabilities.len() {
            return Err(rejected("extension selection contains capabilities outside its authorized roots"));
        }
        validate_derived_requirements(snapshot)?;
        validate_contribution_lock_coverage(revision, snapshot)?;
    }

    let original_roles = source.content.resolved_role_providers.iter()
        .filter(|(_, provider)| !provider.supported_members.is_disjoint(&original_core))
        .map(|(role, _)| role.clone()).collect::<BTreeSet<_>>();
    let candidate_roles = target.content.resolved_role_providers.iter()
        .filter(|(_, provider)| !provider.supported_members.is_disjoint(&candidate_core))
        .map(|(role, _)| role.clone()).collect::<BTreeSet<_>>();
    if original_roles != candidate_roles {
        return Err(rejected("extension selection changed a protected role requirement"));
    }
    for role in &original_roles {
        validate_provider_evolution(&source.content.resolved_role_providers[role],
            &target.content.resolved_role_providers[role])?;
    }
    for snapshot in [source, target] {
        if snapshot.content.resolved_role_providers.values().any(|provider|
            provider.supported_members.is_disjoint(&snapshot.content.capability_allowlist))
        {
            return Err(rejected("extension selection introduced a provider outside its execution graph"));
        }
    }
    let core_mcp_locks = |snapshot: &ResolvedSnapshotEnvelope| {
        snapshot.content.mcp_tool_locks.iter()
            .filter(|lock| original_core.contains(&lock.capability_id)).cloned().collect::<Vec<_>>()
    };
    if core_mcp_locks(source) != core_mcp_locks(target) {
        return Err(rejected("extension selection changed a protected MCP materialization"));
    }

    // Normalize only fields proved above or explicitly owned by the extension
    // operation. Comparing the complete content freezes future global fields
    // by default, instead of silently broadening this authorization boundary.
    let left = &source.content;
    let mut normalized = target.content.clone();
    normalized.enabled_capabilities = left.enabled_capabilities.clone();
    normalized.required_runtime_features = left.required_runtime_features.clone();
    normalized.required_resource_kinds = left.required_resource_kinds.clone();
    normalized.capability_allowlist = left.capability_allowlist.clone();
    normalized.skill_locks = left.skill_locks.clone();
    normalized.mcp_tool_locks = left.mcp_tool_locks.clone();
    normalized.resolved_role_providers = left.resolved_role_providers.clone();
    normalized.preset_revision_ref = left.preset_revision_ref.clone();
    if let (Some(candidate), Some(original)) = (&mut normalized.chat_route_identity, &left.chat_route_identity) {
        candidate.preset_revision_id = original.preset_revision_id.clone();
    }
    normalized.compiled_runtime_profile_digest = left.compiled_runtime_profile_digest.clone();
    normalized.target_contribution_manifest_digest = left.target_contribution_manifest_digest.clone();
    if &normalized != left || source.actor != target.actor || source.scene != target.scene
        || source.surface != target.surface || source.audience != target.audience
    {
        return Err(rejected("extension selection changed a frozen global execution requirement"));
    }
    Ok(())
}

fn capability_nodes(snapshot: &ResolvedSnapshotEnvelope) -> BTreeMap<CapabilityId, &ResolvedCapability> {
    snapshot.content.enabled_capabilities.iter().map(|capability|
        (capability.capability.id.clone(), capability)).collect()
}

fn direct_roots(
    revision: &AgentPresetRevision,
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<BTreeSet<CapabilityId>, PresetContractViolation> {
    let roots = revision.payload.enabled_capabilities.iter()
        .map(|selection| selection.capability.id.clone()).collect::<BTreeSet<_>>();
    let compiled_roots = snapshot.content.contributions()
        .map(|capability| capability.capability.id.clone()).collect::<BTreeSet<_>>();
    if roots != compiled_roots {
        return Err(rejected("Revision roots do not identify the compiled contribution graph"));
    }
    Ok(roots)
}

fn dependency_closure(
    snapshot: &ResolvedSnapshotEnvelope,
    roots: &BTreeSet<CapabilityId>,
) -> Result<BTreeSet<CapabilityId>, PresetContractViolation> {
    let nodes = capability_nodes(snapshot);
    let mut reachable = BTreeSet::new();
    let mut work = roots.iter().cloned().collect::<Vec<_>>();
    while let Some(id) = work.pop() {
        if reachable.insert(id.clone()) {
            let capability = nodes.get(&id).ok_or_else(|| rejected("selected root or dependency is missing from the frozen graph"))?;
            work.extend(capability.dependency_refs.iter().map(|edge| edge.id.clone()));
        }
    }
    Ok(reachable)
}

fn validate_derived_requirements(snapshot: &ResolvedSnapshotEnvelope) -> Result<(), PresetContractViolation> {
    let features = snapshot.content.enabled_capabilities.iter()
        .flat_map(|capability| capability.required_runtime_features.iter().cloned()).collect::<BTreeSet<_>>();
    let resources = snapshot.content.enabled_capabilities.iter()
        .flat_map(|capability| capability.required_resource_kinds.iter().cloned()).collect::<BTreeSet<_>>();
    if features != snapshot.content.required_runtime_features || resources != snapshot.content.required_resource_kinds {
        return Err(rejected("extension selection contains unattributed runtime or resource requirements"));
    }
    Ok(())
}

fn validate_contribution_lock_coverage(
    revision: &AgentPresetRevision,
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<(), PresetContractViolation> {
    if snapshot.content.contributions().any(|capability|
        capability.resolved_source.source_kind != PluginSourceKind::TestFixture
            && !revision.contribution_locks.contains(&capability.contribution_lock))
        || snapshot.content.skill_locks.iter().filter_map(|skill| skill.package_lock()).any(|skill|
            skill.resolved_source.source_kind != PluginSourceKind::TestFixture
                && !revision.contribution_locks.contains(&skill.contribution_lock))
    {
        return Err(rejected("extension selection omitted a selected contribution lock"));
    }
    if revision.contribution_locks.iter().any(|lock|
        !snapshot.content.contributions().any(|capability| &capability.contribution_lock == lock)
            && !snapshot.content.skill_locks.iter().filter_map(|skill| skill.package_lock())
                .any(|skill| &skill.contribution_lock == lock))
    {
        return Err(rejected("extension selection changed an unproven contribution lock"));
    }
    if revision.payload.skill_bindings.len() != snapshot.content.skill_locks.len()
        || revision.payload.skill_bindings.iter().any(|binding| !snapshot.content.skill_locks.iter().any(|lock|
            match (binding, lock) {
                (crate::AgentSkillBinding::Package(selected), crate::ResolvedSkillLock::Package(resolved)) => selected == &resolved.skill,
                (crate::AgentSkillBinding::Library { skill: selected, selected: selected_flag, .. },
                    crate::ResolvedSkillLock::Library { skill: resolved, selected: resolved_flag, .. }) =>
                        selected == resolved && selected_flag == resolved_flag,
                _ => false,
            }))
    {
        return Err(rejected("extension selection did not compile its explicit Skill intent"));
    }
    Ok(())
}

fn validate_provider_evolution(
    source: &ResolvedRoleProviderLock,
    target: &ResolvedRoleProviderLock,
) -> Result<(), PresetContractViolation> {
    if source == target { return Ok(()); }
    if source.source.source_kind != PluginSourceKind::Bundled || target.source.source_kind != PluginSourceKind::Bundled {
        return Err(rejected("only bundled publishers can evolve a Session role provider"));
    }
    let mut normalized = target.clone();
    normalized.provider.role.contract_digest = source.provider.role.contract_digest.clone();
    normalized.provider.contribution_digest = source.provider.contribution_digest.clone();
    normalized.source.source_digest = source.source.source_digest.clone();
    if &normalized != source {
        return Err(rejected("contract evolution changed the role provider authority or provenance"));
    }
    Ok(())
}

fn validate_capability_evolution(
    source: &ResolvedCapability,
    target: &ResolvedCapability,
) -> Result<(), PresetContractViolation> {
    if source == target { return Ok(()); }
    if source.resolved_source.source_kind != PluginSourceKind::Bundled
        || target.resolved_source.source_kind != PluginSourceKind::Bundled
        || source.contribution_lock.source_kind == ContributionSourceKind::McpBinding
        || target.contribution_lock.source_kind == ContributionSourceKind::McpBinding
    {
        return Err(rejected("only bundled publishers can evolve a Session contract"));
    }
    let mut normalized = target.clone();
    normalized.schema_digest = source.schema_digest.clone();
    normalized.target_artifact_digest = source.target_artifact_digest.clone();
    normalized.contribution_lock.contract_digest = source.contribution_lock.contract_digest.clone();
    normalized.resolved_source.source_digest = source.resolved_source.source_digest.clone();
    normalized.display_name = source.display_name.clone();
    normalized.description = source.description.clone();
    for action in &mut normalized.actions {
        let original = source.actions.iter().find(|value| value.action_id == action.action_id)
            .ok_or_else(|| rejected("contract evolution introduced an action"))?;
        action.input_schema = original.input_schema.clone();
        action.output_schema = original.output_schema.clone();
    }
    if &normalized != source {
        return Err(rejected(format!("capability {} changed authority, effects, resources, dependencies or provenance", source.capability.id.as_ref())));
    }
    Ok(())
}
