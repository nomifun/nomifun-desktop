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

/// Remove a deleted catalog selection without selecting a replacement. A
/// primary match clears the default; failover matches only prune that queue.
pub fn catalog_model_detached_payload(
    source: &crate::AgentPresetRevisionPayload,
    provider_id: &str,
    model: Option<&str>,
) -> Option<crate::AgentPresetRevisionPayload> {
    let mut payload = source.clone();
    let route = payload.chat_route_records.get_mut(CHAT_MODEL_TASK_AGENT_CHAT)?;
    let matches = |candidate: &crate::ChatRouteCandidate| candidate.provider_id == provider_id
        && model.is_none_or(|model| candidate.model == model);
    if matches(&route.primary) {
        payload.chat_route_records.remove(CHAT_MODEL_TASK_AGENT_CHAT);
        payload.model_route_refs.remove(CHAT_MODEL_TASK_AGENT_CHAT);
    } else {
        let before = route.failovers.len();
        route.failovers.retain(|candidate| !matches(candidate));
        if route.failovers.len() == before { return None; }
    }
    Some(payload)
}

/// Author a new current configuration after catalog deletion. This grants no
/// Session transition and never changes the source Revision or Snapshot.
pub fn validate_catalog_model_detachment(
    source: &AgentPresetRevision,
    snapshot: &ResolvedSnapshotEnvelope,
    target: &AgentPresetRevision,
    provider_id: &str,
    model: Option<&str>,
) -> Result<(), PresetContractViolation> {
    validate_artifacts(source, snapshot)?;
    target.validate()?;
    let expected = catalog_model_detached_payload(&source.payload, provider_id, model)
        .ok_or_else(|| rejected("catalog detachment has no matching model selection"))?;
    let mut locks = source.contribution_locks.clone(); locks.sort();
    let mut next_locks = target.contribution_locks.clone(); next_locks.sort();
    if target.payload != expected || locks != next_locks || target.created_by != source.created_by
        || target.reference.preset_id != source.reference.preset_id
        || source.reference.revision.checked_add(1) != Some(target.reference.revision) {
        return Err(rejected("catalog detachment must only clear the deleted model in the next configuration revision"));
    }
    Ok(())
}

/// The sole current-generation process-launch transition. Replace only an
/// explicitly granted retired synchronous launch; never infer control grants.
/// Its publisher/effect/resource equivalence is proved against both Snapshots
/// by `validate_bundled_contract_evolution` before any binding can be committed.
pub fn bounded_process_launch_payload(source: &crate::AgentPresetRevisionPayload) -> crate::AgentPresetRevisionPayload {
    let mut payload = source.clone();
    for selection in &mut payload.enabled_capabilities {
        if selection.capability.id.as_ref() == "workspace.process"
            && selection.action_allowlist.remove(&crate::ActionId::from("workspace.process/exec")) {
            selection.action_allowlist.insert("workspace.process/start".into());
        }
    }
    payload
}

/// Admit a fresh forward execution contract from the same bundled publishers.
/// Exact schemas and artifacts may change, but grants, effects, resources,
/// dependency graph, provider mounts and runtime requirements may not. MCP,
/// managed packages and Skill content must remain exact.
/// Previously absent Action resource metadata may be completed only by
/// retaining the entire former Module gate and constraining its operations.
pub fn validate_bundled_contract_evolution(
    source_revision: &AgentPresetRevision,
    source: &ResolvedSnapshotEnvelope,
    target_revision: &AgentPresetRevision,
    target: &ResolvedSnapshotEnvelope,
) -> Result<(), PresetContractViolation> {
    validate_artifacts(source_revision, source)?;
    validate_artifacts(target_revision, target)?;
    if target_revision.payload != bounded_process_launch_payload(&source_revision.payload) || source_revision.created_by != target_revision.created_by {
        return Err(rejected("contract evolution must preserve the Session's authorial configuration"));
    }
    let left = &source.content;
    let right = &target.content;
    if !schema_manifest_matches_or_evolves(left, right) {
        return Err(rejected("contract evolution changed an unsupported canonical_schema_manifest_digest"));
    }
    if left.schema_version != right.schema_version
        || left.resolver_version != right.resolver_version
        || left.required_runtime_protocol_version != right.required_runtime_protocol_version
        || left.required_runtime_profile != right.required_runtime_profile
        || left.runtime_feature_inventory_digest != right.runtime_feature_inventory_digest
        || left.required_runtime_features != right.required_runtime_features
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
    normalized.canonical_schema_manifest_digest = left.canonical_schema_manifest_digest.clone();
    if &normalized != left {
        return Err(rejected("contract evolution changed an unproven global execution requirement"));
    }
    Ok(())
}

/// Retire the host-owned default personal Agent's former exact-empty surface.
/// The Session owner separately proves that this is that official default,
/// then commits a normal idle binding transition. The only new roots are the
/// published local file/process tools; every other authored field stays exact.
pub fn validate_default_local_os_evolution(
    source_revision: &AgentPresetRevision,
    source: &ResolvedSnapshotEnvelope,
    target_revision: &AgentPresetRevision,
    target: &ResolvedSnapshotEnvelope,
) -> Result<(), PresetContractViolation> {
    if !source_revision.payload.enabled_capabilities.is_empty()
        || !source_revision.payload.skill_bindings.is_empty()
    {
        return Err(rejected("default local OS evolution requires the former exact-empty contract"));
    }
    let mut expected = source_revision.payload.clone();
    expected.enabled_capabilities = crate::default_local_os_capability_selections();
    if target_revision.payload != expected {
        return Err(rejected("default local OS evolution may only add the complete published file/process tools"));
    }
    let roots = BTreeSet::from([CapabilityId::from("workspace.files"), CapabilityId::from("workspace.process")]);
    validate_session_extension_derivation(source_revision, source, target_revision, target, &roots, &BTreeSet::new())
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
    if !schema_manifest_matches_or_evolves(&source.content, &target.content) {
        return Err(rejected("extension selection changed an unsupported canonical_schema_manifest_digest"));
    }
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
    normalized.canonical_schema_manifest_digest = left.canonical_schema_manifest_digest.clone();
    if &normalized != left || source.actor != target.actor || source.scene != target.scene
        || source.surface != target.surface || source.audience != target.audience
    {
        return Err(rejected("extension selection changed a frozen global execution requirement"));
    }
    Ok(())
}

fn schema_manifest_matches_or_evolves(
    source: &crate::ResolvedSnapshotContent,
    target: &crate::ResolvedSnapshotContent,
) -> bool {
    source.canonical_schema_manifest_digest == target.canonical_schema_manifest_digest
        || crate::agent_store_schema_manifest_evolution_compatible(
            &source.canonical_schema_manifest_digest, &target.canonical_schema_manifest_digest,
        )
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
    let bounded_source = bounded_process_predecessor(source, target)?;
    let source = bounded_source.as_ref().unwrap_or(source);
    let mut normalized = target.clone();
    normalized.schema_digest = source.schema_digest.clone();
    normalized.target_artifact_digest = source.target_artifact_digest.clone();
    normalized.contribution_lock.contract_digest = source.contribution_lock.contract_digest.clone();
    normalized.resolved_source.source_digest = source.resolved_source.source_digest.clone();
    normalized.display_name = source.display_name.clone();
    normalized.description = source.description.clone();
    // A newly published descriptor does not grant an existing Session that
    // Action. Keep its exact allowlist and omit only new, ungranted metadata
    // from the authority comparison. Selected Actions remain strictly frozen.
    for action in &normalized.actions {
        if !source.actions.iter().any(|original| original.action_id == action.action_id)
            && target.action_allowlist.contains(&action.action_id)
        {
            return Err(rejected("contract evolution introduced a granted action"));
        }
    }
    normalized.actions.retain(|action| source.actions.iter()
        .any(|original| original.action_id == action.action_id));
    for action in &mut normalized.actions {
        let original = source.actions.iter().find(|value| value.action_id == action.action_id)
            .ok_or_else(|| rejected("contract evolution introduced an action"))?;
        action.input_schema = original.input_schema.clone();
        action.output_schema = original.output_schema.clone();
        // Older current-generation publications omitted the exact Action
        // resource contract. Their fallback was the complete Module gate. An
        // explicit contract can complete that metadata only when it retains
        // every former resource slot. Its operation checks then add constraints
        // on the same bindings, never authorize new resources or operations.
        // Dropping a slot (including an empty map on a resource-backed Module)
        // would bypass the former gate. Existing exact maps and their removal
        // remain frozen, even when a different map appears more restrictive.
        if source.required_resource_kinds == target.required_resource_kinds
            && original.required_resource_operations.is_none()
            && action.required_resource_operations.as_ref().is_some_and(|requirements|
                requirements.keys().eq(source.required_resource_kinds.iter()))
        {
            action.required_resource_operations = None;
        }
    }
    if &normalized != source {
        return Err(rejected(format!("capability {} changed authority, effects, resources, dependencies or provenance", source.capability.id.as_ref())));
    }
    Ok(())
}

/// Compare a retired descriptor as immutable source data, not an executable
/// alias. Both existing launch grants collapse to the same ExecuteLocal gate;
/// poll/input/resize/cancel selections remain exact in the normal comparison.
fn bounded_process_predecessor(source: &ResolvedCapability, target: &ResolvedCapability)
    -> Result<Option<ResolvedCapability>, PresetContractViolation> {
    let retired = crate::ActionId::from("workspace.process/exec");
    if source.capability.id.as_ref() != "workspace.process"
        || !source.actions.iter().any(|action| action.action_id == retired) { return Ok(None); }
    if source.source_package.id.as_ref() != "nomifun.workspace-execution"
        || source.resolved_source.source_identity != "nomifun.workspace-execution"
        || source.contribution_lock.source_kind != ContributionSourceKind::PlatformBuiltin
        || source.contribution_lock.source_identity.as_ref() != "nomifun.workspace-execution"
        || source.required_resource_kinds != BTreeSet::from(["process_session".into()])
        || target.actions.iter().any(|action| action.action_id == retired) {
        return Err(rejected("bounded process launch requires the exact retired bundled publisher and resource contract"));
    }
    let retired_descriptor = source.actions.iter().find(|action| action.action_id == retired).expect("retired descriptor checked");
    let start = crate::ActionId::from("workspace.process/start");
    let original_start = source.actions.iter().find(|action| action.action_id == start);
    let next_start = target.actions.iter().find(|action| action.action_id == start)
        .ok_or_else(|| rejected("bounded process launch has no replacement descriptor"))?;
    let execute_gate = BTreeMap::from([("process_session".into(),BTreeSet::from(["execute".to_owned()]))]);
    for action in [Some(retired_descriptor), original_start, Some(next_start)].into_iter().flatten() {
        if action.effect_class != crate::EffectClass::ExecuteLocal
            || action.presentation != crate::ToolPresentationKind::FunctionTool
            || action.required_resource_operations.as_ref().is_some_and(|operations| operations != &execute_gate) {
            return Err(rejected("bounded launch changed execution effect or its exact resource operation"));
        }
    }
    let mut equivalent = source.clone();
    if original_start.is_some() {
        equivalent.actions.retain(|action| action.action_id != retired);
    } else {
        equivalent.actions.iter_mut().find(|action|action.action_id==retired).expect("retired descriptor checked").action_id = start.clone();
    }
    if equivalent.action_allowlist.remove(&retired) { equivalent.action_allowlist.insert(start); }
    Ok(Some(equivalent))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionId, CapabilityActionDescriptor, CapabilityRef, ContributionId, ContributionLock,
        DigestHex, EffectClass, PackageRef, PluginSourceMetadata, ToolPresentationKind};

    fn descriptor(id: &str) -> CapabilityActionDescriptor {
        CapabilityActionDescriptor {
            action_id: id.into(), input_schema: format!("schema://{id}/input").into(),
            output_schema: format!("schema://{id}/output").into(), effect_class: EffectClass::WriteReversible,
            presentation: ToolPresentationKind::FunctionTool, required_resource_operations: None,
        }
    }

    fn source() -> ResolvedCapability {
        ResolvedCapability {
            consumption: Default::default(), dependency_refs: Vec::new(),
            capability: CapabilityRef { id: "workspace.files".into() },
            source_package: PackageRef { id: "nomifun.workspace-execution".into(), version: "1.0.0".into() },
            contribution_id: ContributionId::from("capability:workspace.files"),
            contribution_lock: ContributionLock {
                source_kind: ContributionSourceKind::PlatformBuiltin,
                source_identity: "nomifun.workspace-execution".into(), mount_id: None, mcp_binding_id: None,
                contribution_id: "capability:workspace.files".into(), contract_digest: "a".repeat(64).into(),
            },
            resolved_mount_id: None,
            resolved_source: PluginSourceMetadata {
                source_kind: PluginSourceKind::Bundled, source_identity: "nomifun.workspace-execution".into(),
                source_digest: Some("b".repeat(64).into()),
            },
            target_artifact_digest: "b".repeat(64).into(), schema_digest: DigestHex::from("a".repeat(64)),
            dependency_path: vec!["workspace.files".into()], required_runtime_features: BTreeSet::new(),
            display_name: None, description: None,
            actions: vec![descriptor("workspace.files/write")], required_resource_kinds: BTreeSet::new(),
            action_allowlist: BTreeSet::from([ActionId::from("workspace.files/write")]),
        }
    }

    #[test]
    fn bundled_new_action_metadata_does_not_change_existing_session_grants() {
        let source = source();
        let mut target = source.clone();
        target.actions.push(descriptor("workspace.files/rename"));
        assert!(validate_capability_evolution(&source, &target).is_ok(),
            "new ungranted metadata must not block existing file Sessions");
        target.action_allowlist.insert("workspace.files/rename".into());
        assert!(validate_capability_evolution(&source, &target).is_err(),
            "a new descriptor cannot silently authorize a new Action");
        target.action_allowlist.remove(&ActionId::from("workspace.files/rename"));
        target.actions[0].effect_class = EffectClass::Destructive;
        assert!(validate_capability_evolution(&source, &target).is_err(),
            "existing Action authority remains exact");
    }

    fn process_source(grants: &[&str]) -> ResolvedCapability {
        let mut process = source();
        process.capability.id = "workspace.process".into();
        process.contribution_id = "capability:workspace.process".into();
        process.contribution_lock.contribution_id = process.contribution_id.clone();
        process.dependency_path = vec!["workspace.process".into()];
        process.required_resource_kinds = BTreeSet::from(["process_session".into()]);
        process.action_allowlist = grants.iter().map(|id|(*id).into()).collect();
        process.actions = ["workspace.process/exec","workspace.process/start","workspace.process/poll","workspace.process/input"]
            .into_iter().map(|id| {
                let mut action = descriptor(id);
                action.effect_class = if id.ends_with("/poll") {EffectClass::ReadLocal} else {EffectClass::ExecuteLocal};
                action.required_resource_operations = Some(BTreeMap::from([
                    ("process_session".into(),BTreeSet::from(["execute".to_owned()]))
                ]));
                action
            }).collect();
        process
    }

    #[test]
    fn bounded_launch_replaces_only_the_explicit_launch_grant_and_retires_ungranted_descriptor() {
        for grants in [vec!["workspace.process/exec"],vec!["workspace.process/exec","workspace.process/start"],
            vec!["workspace.process/start"], vec!["workspace.process/exec","workspace.process/poll"]] {
            let original = process_source(&grants);
            let original_bytes = serde_json::to_vec(&original).unwrap();
            let mut target = original.clone();
            target.actions.retain(|action|action.action_id.as_ref()!="workspace.process/exec");
            if target.action_allowlist.remove(&"workspace.process/exec".into()) {target.action_allowlist.insert("workspace.process/start".into());}
            validate_capability_evolution(&original,&target).unwrap();
            assert_eq!(serde_json::to_vec(&original).unwrap(),original_bytes,"source history stays immutable");
            assert!(!target.action_allowlist.contains(&"workspace.process/input".into()),"no stdin authority can be inferred");
            for forbidden in ["workspace.process/poll","workspace.process/input","workspace.process/cancel"] {
                if original.action_allowlist.contains(&forbidden.into()) {continue;}
                let mut widened = target.clone();
                widened.action_allowlist.insert(forbidden.into());
                assert!(validate_capability_evolution(&original,&widened).is_err(),"must not add {forbidden}");
            }
            for change in 0..4 {
                let mut invalid = target.clone();
                match change {
                    0 => invalid.resolved_source.source_identity = "another-publisher".into(),
                    1 => invalid.actions.iter_mut().find(|action|action.action_id.as_ref()=="workspace.process/start").unwrap().effect_class = EffectClass::Destructive,
                    2 => invalid.actions.iter_mut().find(|action|action.action_id.as_ref()=="workspace.process/start").unwrap().required_resource_operations = Some(BTreeMap::new()),
                    3 => { invalid.required_resource_kinds.insert("another-resource".into()); },
                    _ => unreachable!(),
                }
                assert!(validate_capability_evolution(&original,&invalid).is_err(),"case {change}");
            }
        }
    }

    fn manifest_digests() -> (DigestHex, DigestHex) {
        let current = crate::agent_store_schema_manifest_payload();
        let current_digest = crate::digest_payload(&current).unwrap();
        let mut previous = current;
        previous.tables.retain(|table| !matches!(table.table_name.as_str(),
            "mcp_oauth_clients" | "mcp_oauth_grants"
            | "feishu_connections" | "feishu_operations" | "feishu_operation_items" | "feishu_operation_receipts"));
        (crate::digest_payload(&previous).unwrap(), current_digest)
    }

    fn binding_artifacts(
        id: &str,
        capabilities: Vec<ResolvedCapability>,
        schema_digest: DigestHex,
    ) -> (AgentPresetRevision, ResolvedSnapshotEnvelope) {
        let mut revision = AgentPresetRevision {
            reference: crate::PresetRevisionRef { preset_id: id.into(), revision: 1, revision_digest: "pending".into() },
            payload: crate::AgentPresetRevisionPayload {
                context_order: Vec::new(), middleware_order: Vec::new(), schema_version: "1.0.0".into(),
                model_route_refs: BTreeMap::new(), chat_route_records: BTreeMap::new(),
                enabled_capabilities: capabilities.iter().map(|capability| crate::CapabilitySelection {
                    capability: capability.capability.clone(), action_allowlist: capability.action_allowlist.clone(),
                }).collect(),
                skill_bindings: Vec::new(), system_role_provider_overrides: BTreeMap::new(),
                persona: "Retain my persona".into(), instructions: "Retain my instructions".into(),
                starter_prompts: Vec::new(), runtime_policy: Default::default(),
            },
            contribution_locks: capabilities.iter().map(|capability| capability.contribution_lock.clone()).collect(),
            created_by: "owner".into(), created_at_ms: 1, reason: None,
        };
        revision.reference.revision_digest = revision.revision_digest().unwrap();
        let content = crate::ResolvedSnapshotContent {
            context_order: Vec::new(), middleware_order: Vec::new(), schema_version: "1.0.0".into(),
            resolver_version: "1.0.0".into(), preset_revision_ref: revision.reference.clone(),
            required_runtime_protocol_version: "1.0.0".into(), required_runtime_profile: crate::RuntimeProfileKind::ManagedMinimal,
            runtime_feature_inventory_digest: "c".repeat(64).into(), required_runtime_features: BTreeSet::new(),
            compiled_runtime_profile_digest: "d".repeat(64).into(), model_route_refs: BTreeMap::new(), chat_route_identity: None,
            required_resource_kinds: capabilities.iter().flat_map(|capability| capability.required_resource_kinds.iter().cloned()).collect(),
            capability_allowlist: capabilities.iter().map(|capability| capability.capability.id.clone()).collect(),
            enabled_capabilities: capabilities, skill_locks: Vec::new(), mcp_tool_locks: Vec::new(),
            resolved_role_providers: BTreeMap::new(), canonical_schema_manifest_digest: schema_digest,
            target_contribution_manifest_digest: "e".repeat(64).into(),
        };
        let snapshot = ResolvedSnapshotEnvelope {
            snapshot_ref: crate::ResolvedSnapshotRef { snapshot_id: format!("snapshot:{id}").into(), snapshot_digest: crate::digest_payload(&content).unwrap() },
            content, actor: crate::PrincipalRef { principal_kind: "user".into(), principal_id: "owner".into() },
            scene: "agent_settings".into(), surface: "desktop".into(), audience: "owner".into(),
            created_at_ms: 1, resolver_run_id: format!("resolve:{id}").into(), availability_evidence_revision: "1".into(),
        };
        validate_artifacts(&revision, &snapshot).unwrap();
        (revision, snapshot)
    }

    fn refresh_snapshot_digest(snapshot: &mut ResolvedSnapshotEnvelope) {
        snapshot.snapshot_ref.snapshot_digest = crate::digest_payload(&snapshot.content).unwrap();
    }

    #[test]
    fn bundled_evolution_accepts_the_preserved_schema_transition_without_relaxing_authority() {
        let (previous, current) = manifest_digests();
        let original = process_source(&["workspace.process/exec", "workspace.process/start", "workspace.process/poll"]);
        let mut replacement = original.clone();
        replacement.actions.retain(|action| action.action_id.as_ref() != "workspace.process/exec");
        replacement.action_allowlist.remove(&"workspace.process/exec".into());
        let (source_revision, source_snapshot) = binding_artifacts("old-general", vec![original], previous);
        let (target_revision, target_snapshot) = binding_artifacts("current-general", vec![replacement], current);
        let original_bytes = serde_json::to_vec(&source_snapshot).unwrap();
        validate_bundled_contract_evolution(&source_revision, &source_snapshot, &target_revision, &target_snapshot).unwrap();
        assert_eq!(serde_json::to_vec(&source_snapshot).unwrap(), original_bytes);

        for change in 0..6 {
            let mut invalid = target_snapshot.clone();
            match change {
                0 => { invalid.content.enabled_capabilities[0].action_allowlist.insert("workspace.process/input".into()); },
                1 => invalid.content.enabled_capabilities[0].actions[0].effect_class = EffectClass::Destructive,
                2 => { invalid.content.required_resource_kinds.insert("another-resource".into()); },
                3 => invalid.content.runtime_feature_inventory_digest = "f".repeat(64).into(),
                4 => invalid.content.schema_version = "2.0.0".into(),
                _ => invalid.content.enabled_capabilities[0].resolved_source.source_identity = "another-publisher".into(),
            }
            refresh_snapshot_digest(&mut invalid);
            assert!(validate_bundled_contract_evolution(&source_revision, &source_snapshot, &target_revision, &invalid).is_err(), "case {change}");
        }
    }

    #[test]
    fn binding_evolution_rejects_unknown_reverse_and_other_generation_schema_transitions() {
        let (previous, current) = manifest_digests();
        let mut future = crate::agent_store_schema_manifest_payload();
        future.data_generation += 1;
        let future = crate::digest_payload(&future).unwrap();
        for (source_digest, target_digest) in [
            ("f".repeat(64).into(), current.clone()), (current.clone(), previous),
            (future.clone(), current.clone()), (current, future),
        ] {
            let (source_revision, source_snapshot) = binding_artifacts("source", vec![source()], source_digest);
            let (target_revision, target_snapshot) = binding_artifacts("target", vec![source()], target_digest);
            assert!(validate_bundled_contract_evolution(&source_revision, &source_snapshot, &target_revision, &target_snapshot).is_err());
            assert!(validate_session_extension_derivation(&source_revision, &source_snapshot, &target_revision, &target_snapshot,
                &BTreeSet::new(), &BTreeSet::new()).is_err());
        }
    }

    #[test]
    fn extension_and_default_os_derivations_accept_only_the_proved_schema_change() {
        let (previous, current) = manifest_digests();
        let mut extension = source();
        extension.capability.id = "extension".into();
        extension.dependency_path = vec!["extension".into()];
        extension.contribution_id = "capability:extension".into();
        extension.contribution_lock.contribution_id = extension.contribution_id.clone();
        extension.actions = vec![descriptor("extension/read")];
        extension.action_allowlist = BTreeSet::from(["extension/read".into()]);
        let (source_revision, source_snapshot) = binding_artifacts("source", vec![source()], previous.clone());
        let (target_revision, target_snapshot) = binding_artifacts("target", vec![source(), extension], current.clone());
        let roots = BTreeSet::from(["extension".into()]);
        validate_session_extension_derivation(&source_revision, &source_snapshot, &target_revision, &target_snapshot,
            &roots, &BTreeSet::new()).unwrap();
        let mut invalid = target_snapshot.clone();
        invalid.content.enabled_capabilities[0].actions[0].effect_class = EffectClass::Destructive;
        refresh_snapshot_digest(&mut invalid);
        assert!(validate_session_extension_derivation(&source_revision, &source_snapshot, &target_revision, &invalid,
            &roots, &BTreeSet::new()).is_err(), "an explicit extension cannot change retained core authority");

        let (empty_revision, empty_snapshot) = binding_artifacts("old-empty-default", Vec::new(), previous);
        let defaults = crate::default_local_os_capability_selections().into_iter().map(|selection| {
            let mut capability = source();
            capability.capability = selection.capability;
            capability.dependency_path = vec![capability.capability.id.clone()];
            capability.contribution_id = format!("capability:{}", capability.capability.id.as_ref()).into();
            capability.contribution_lock.contribution_id = capability.contribution_id.clone();
            capability.actions = selection.action_allowlist.iter().map(|action| descriptor(action.as_ref())).collect();
            capability.action_allowlist = selection.action_allowlist;
            capability
        }).collect();
        let (default_revision, default_snapshot) = binding_artifacts("current-default", defaults, current);
        validate_default_local_os_evolution(&empty_revision, &empty_snapshot, &default_revision, &default_snapshot).unwrap();
        let mut invalid = default_snapshot;
        invalid.content.canonical_schema_manifest_digest = "f".repeat(64).into();
        refresh_snapshot_digest(&mut invalid);
        assert!(validate_default_local_os_evolution(&empty_revision, &empty_snapshot, &default_revision, &invalid).is_err());
    }
}
