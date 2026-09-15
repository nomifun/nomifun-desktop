//! Selected execution graph inside the canonical compiler. This is transient
//! compilation state; exact edges and consumption are frozen on the existing
//! ResolvedCapability records, never in a language-specific plan.
use super::*;

#[derive(Default)]
pub(super) struct DependencyGraph {
    pub edges: BTreeMap<CapabilityId, Vec<CapabilityRef>>,
    pub paths: BTreeMap<CapabilityId, Vec<CapabilityId>>,
    pub roles: BTreeMap<ExecutionRoleId, ResolvedRoleProviderLock>,
}

pub(super) fn resolve(
    registry: &MaterializedRegistry,
    environment: &CompilerEnvironment,
    revision: &AgentPresetRevision,
    roots: &BTreeSet<CapabilityId>,
) -> Result<DependencyGraph, KernelError> {
    let mut graph = DependencyGraph::default();
    let mut visiting = BTreeSet::new();
    // Iterative DFS avoids growing the Rust call stack with plugin-supplied
    // chains. Sorted roots/edges retain deterministic diagnostic paths.
    let mut work = roots
        .iter()
        .rev()
        .map(|id| (id.clone(), vec![id.clone()], false))
        .collect::<Vec<_>>();
    while let Some((id, path, finish)) = work.pop() {
        if finish {
            visiting.remove(&id);
            continue;
        }
        if visiting.contains(&id) {
            return Err(KernelError::CapabilityDependencyCycle);
        }
        if graph.edges.contains_key(&id) {
            continue;
        }
        visiting.insert(id.clone());
        let capability =
            registry
                .capability(&id)
                .ok_or_else(|| KernelError::CapabilityNotMaterialized {
                    capability_id: id.clone(),
                    version: "unknown".into(),
                })?;
        let mut refs = capability.manifest.requires.clone();
        if let Some(role_id) = registry.role_for_capability(&id) {
            // Resolve only the member actually reached, using the original
            // resolver. Unselected Provider candidates never enter this graph.
            let locks = compile_role_provider_locks(
                registry,
                &revision.payload.system_role_provider_overrides,
                &environment.installation_role_bindings,
                &BTreeSet::from([id.clone()]),
                environment,
            )?;
            let lock = &locks[role_id];
            let provider = registry
                .role_provider(role_id, &lock.provider.mount_id)
                .ok_or(KernelError::RegistryPoisoned)?;
            let member = provider
                .contribution
                .members
                .get(&id)
                .ok_or(KernelError::RegistryPoisoned)?;
            for (member_id, member) in
                std::iter::once((&id, member)).chain(registry.role_resource_members(provider, &id))
            {
                // Implicit resource exports already have exact identity in
                // the Provider lock. Include their requirements without
                // exposing those exports as additional public contributions.
                if member_id != &id {
                    refs.extend(
                        registry
                            .capability(member_id)
                            .ok_or(KernelError::RegistryPoisoned)?
                            .manifest
                            .requires
                            .clone(),
                    );
                }
                if let Some(implementation) = &member.implementation {
                    refs.extend(
                        registry
                            .capability(&implementation.id)
                            .ok_or(KernelError::RegistryPoisoned)?
                            .manifest
                            .requires
                            .clone(),
                    );
                }
            }
            graph.roles.extend(locks);
        }
        refs.sort();
        refs.dedup();
        for dependency in &refs {
            if !registry
                .capability(&dependency.id)
                .is_some_and(|value| value.manifest.version == dependency.version)
            {
                return Err(KernelError::MissingCapabilityDependency {
                    capability_id: id.clone(),
                    dependency_id: dependency.id.clone(),
                    dependency_version: dependency.version.clone(),
                });
            }
        }
        work.push((id.clone(), path.clone(), true));
        for dependency in refs.iter().rev() {
            let mut next = path.clone();
            next.push(dependency.id.clone());
            work.push((dependency.id.clone(), next, false));
        }
        graph.paths.insert(id.clone(), path);
        graph.edges.insert(id, refs);
    }
    Ok(graph)
}
