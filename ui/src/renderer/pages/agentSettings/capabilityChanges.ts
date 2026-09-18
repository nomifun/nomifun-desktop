import type {
  AgentCatalogResponse,
  AgentPresetDocument,
  CapabilityModuleCatalogItem,
} from '@/common/types/agentPlatform';
import { capabilityReferenceKey, placeCapability } from './model';
import { moduleIsAvailable, type ModuleReference } from './capabilityGroups';

const defaultActions = (module: CapabilityModuleCatalogItem): string[] =>
  module.actions
    .filter((action) => ['pure', 'read_local', 'read_sensitive'].includes(action.effect_class))
    .map((action) => action.action_id)
    .sort((left, right) => left.localeCompare(right));

export function setModuleActions(
  document: AgentPresetDocument,
  reference: ModuleReference,
  actionIds: readonly string[]
): AgentPresetDocument {
  const key = capabilityReferenceKey(reference);
  return {
    ...document,
    enabled_capabilities: document.enabled_capabilities.map((selection) =>
      capabilityReferenceKey(selection.capability) === key
        ? { ...selection, action_allowlist: [...new Set(actionIds)].sort((left, right) => left.localeCompare(right)) }
        : selection
    ),
  };
}

/** Resolve one complete Module edit before applying it, including exact dependencies and conflicts. */
export function planModuleChange(
  document: AgentPresetDocument,
  catalog: Pick<AgentCatalogResponse, 'modules' | 'capabilities'>,
  requested: readonly ModuleReference[],
  enable: boolean
) {
  const keyOf = capabilityReferenceKey;
  const byKey = new Map(catalog.modules.map((module) => [keyOf(module.module), module]));
  const selected = new Map(
    document.enabled_capabilities.map((selection) => [keyOf(selection.capability), selection.capability])
  );
  const affected = new Map<string, ModuleReference>();
  const blocked = new Map<string, ModuleReference>();
  const visiting = new Set<string>();

  const validateDependency = (reference: ModuleReference) => {
    const key = keyOf(reference);
    if (visiting.has(key)) {
      blocked.set(key, reference);
      return;
    }
    const module = byKey.get(key);
    if (
      !module ||
      !moduleIsAvailable(module, catalog.capabilities) ||
      [...selected.values()].some(
        (current) => current.id === reference.id && keyOf(current) !== key
      )
    ) {
      blocked.set(key, reference);
      return;
    }
    visiting.add(key);
    module.required_modules.forEach(validateDependency);
    visiting.delete(key);
  };

  const add = (reference: ModuleReference) => {
    const key = keyOf(reference);
    if (selected.has(key) || affected.has(key)) return;
    const module = byKey.get(key);
    if (!module || module.authoring_policy !== 'direct' || !moduleIsAvailable(module, catalog.capabilities)) {
      blocked.set(key, reference);
      return;
    }
    module.required_modules.forEach(validateDependency);
    if (blocked.size > 0) return;
    affected.set(key, reference);
  };

  if (enable) {
    requested.forEach((reference) => add(reference));
    const result = new Set([...selected.keys(), ...affected.keys()]);
    for (const key of result) {
      for (const conflict of byKey.get(key)?.conflicting_modules ?? []) {
        if (result.has(keyOf(conflict))) blocked.set(keyOf(conflict), conflict);
      }
    }
  } else {
    requested
      .filter((reference) => selected.has(keyOf(reference)))
      .forEach((reference) => affected.set(keyOf(reference), reference));
    let changed = true;
    while (changed) {
      changed = false;
      for (const [key, reference] of selected) {
        if (
          !affected.has(key) &&
          byKey.get(key)?.required_modules.some((dependency) => affected.has(keyOf(dependency)))
        ) {
          affected.set(key, reference);
          changed = true;
        }
      }
    }
  }

  const changes = [...affected.values()];
  let next = document;
  if (blocked.size === 0) {
    for (const reference of changes) {
      next = placeCapability(next, reference, enable ? 'enabled' : 'none');
      if (enable) {
        const module = byKey.get(keyOf(reference));
        if (module) next = setModuleActions(next, reference, defaultActions(module));
      }
    }
  }

  return {
    document: blocked.size ? document : next,
    affected: changes,
    additional: [],
    blocked: [...blocked.values()],
  };
}
