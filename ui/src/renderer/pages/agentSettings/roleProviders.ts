import type {
  AgentCatalogResponse, AgentPresetDocument, ExactCatalogRef, RoleProviderSelection,
} from '@/common/types/agentPlatform';

const refKey = (ref: ExactCatalogRef<'capability'>) => JSON.stringify([ref.id, ref.version]);

export const providerSelectionKey = (selection: RoleProviderSelection): string => JSON.stringify([
  selection.role.key.role_id, selection.role.key.contract_version,
  selection.role.contract_digest, selection.provider_mount_id,
]);

/** Display-only reachability. Save/Preview, not this helper, resolves the actual plan. */
export function relevantRoleIds(document: AgentPresetDocument, catalog: AgentCatalogResponse): string[] {
  const visible = new Set(Object.keys(document.system_role_provider_overrides));
  const pending = document.enabled_capabilities.map(item => item.capability);
  for (const binding of document.skill_bindings) {
    const skill = catalog.skills.find(item => item.skill.id === binding.id && item.skill.version === binding.version);
    pending.push(...(skill?.required_capabilities ?? []));
  }
  const visited = new Set<string>();
  const byRef = new Map(catalog.capabilities.map(item => [refKey(item.capability), item]));
  while (pending.length) {
    const reference = pending.pop()!;
    const key = refKey(reference);
    if (visited.has(key)) continue;
    visited.add(key);
    pending.push(...(byRef.get(key)?.required_capabilities ?? []));
  }
  for (const item of catalog.roles) {
    if (item.capabilities.some(ref => visited.has(refKey(ref)))) visible.add(item.role.key.role_id);
  }
  return [...visible].sort();
}

export function selectRoleProvider(
  document: AgentPresetDocument, roleId: string, selection?: RoleProviderSelection,
): AgentPresetDocument {
  if (selection && selection.role.key.role_id !== roleId) throw new Error('Provider belongs to another role');
  const overrides = { ...document.system_role_provider_overrides };
  if (selection) overrides[roleId] = structuredClone(selection);
  else delete overrides[roleId];
  return { ...document, system_role_provider_overrides: overrides };
}
