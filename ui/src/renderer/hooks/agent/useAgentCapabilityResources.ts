/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CapabilitySelection,
  CapabilityModuleCatalogItem,
} from '@/common/types/agentPlatform';

export const requiredResourceKindsForCapabilitySelections = (
  selections: readonly CapabilitySelection[],
  catalog: readonly CapabilityModuleCatalogItem[]
): Set<string> => {
  const modules = new Map(catalog.map((module) => [String(module.module.id), module]));
  const kinds = new Set<string>();
  for (const selection of selections) {
    const module = modules.get(String(selection.capability.id));
    if (!module) continue;
    const declaredActionKinds = new Set(module.actions.flatMap((action) =>
      Object.keys(action.required_resource_operations ?? {})
    ));
    // Context and Event consumers still use the complete Module resource gate.
    const keepModuleGate = module.context_schema_refs.length || module.event_schema_refs.length;
    module.required_resource_kinds
      .filter((kind) => keepModuleGate || !declaredActionKinds.has(kind))
      .forEach((kind) => kinds.add(kind));
    for (const actionId of selection.action_allowlist ?? []) {
      const action = module.actions.find((candidate) => candidate.action_id === actionId);
      const requirements = action?.required_resource_operations;
      const actionKinds = requirements == null
        ? module.required_resource_kinds
        : Object.keys(requirements);
      actionKinds.forEach((kind) => kinds.add(kind));
    }
  }
  return kinds;
};
