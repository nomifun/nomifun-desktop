import type { AgentPresetDocument, CapabilityCatalogItem } from '@/common/types/agentPlatform';
import { capabilityReferenceKey, placeCapability } from './model';
import { capabilityIsAvailable, type CapabilityReference } from './capabilityGroups';

/** Resolve the whole edit before applying it, including dependencies and conflicts. */
export function planCapabilityChange(document: AgentPresetDocument, catalog: readonly CapabilityCatalogItem[], requested: readonly CapabilityReference[], enable: boolean) {
  const keyOf = capabilityReferenceKey;
  const byKey = new Map(catalog.map(item => [keyOf(item.capability), item]));
  const selected = new Map(document.enabled_capabilities.map(item => [keyOf(item.capability), item.capability]));
  const requestedKeys = new Set(requested.map(keyOf));
  const affected = new Map<string, CapabilityReference>();
  const blocked = new Map<string, CapabilityReference>();
  const visiting = new Set<string>();
  const add = (reference: CapabilityReference) => {
    const key = keyOf(reference);
    if (selected.has(key) || affected.has(key)) return;
    if (visiting.has(key)) { blocked.set(key, reference); return; }
    const item = byKey.get(key);
    if (!capabilityIsAvailable(item) || [...selected.values(), ...affected.values()].some(current => current.id === reference.id && keyOf(current) !== key)) { blocked.set(key, reference); return; }
    visiting.add(key);
    item!.required_capabilities.forEach(add);
    visiting.delete(key);
    affected.set(key, reference);
  };
  if (enable) {
    requested.forEach(add);
    const result = new Set([...selected.keys(), ...affected.keys()]);
    for (const key of result) {
      for (const conflict of byKey.get(key)?.conflicting_capabilities ?? []) {
        if (result.has(keyOf(conflict))) blocked.set(keyOf(conflict), conflict);
      }
    }
  } else {
    requested.filter(reference => selected.has(keyOf(reference))).forEach(reference => affected.set(keyOf(reference), reference));
    let changed = true;
    while (changed) {
      changed = false;
      for (const [key, reference] of selected) {
        if (!affected.has(key) && byKey.get(key)?.required_capabilities.some(dependency => affected.has(keyOf(dependency)))) { affected.set(key, reference); changed = true; }
      }
    }
  }
  const changes = [...affected.values()];
  return {
    document: blocked.size ? document : changes.reduce((next, reference) => placeCapability(next, reference, enable ? 'enabled' : 'none'), document),
    affected: changes,
    additional: changes.filter(reference => !requestedKeys.has(keyOf(reference))),
    blocked: [...blocked.values()],
  };
}
