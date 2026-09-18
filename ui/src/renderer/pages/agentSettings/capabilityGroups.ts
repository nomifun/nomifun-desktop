import type {
  AgentCatalogResponse,
  AgentPresetDocument,
  CapabilityCatalogItem,
  CapabilityModuleCatalogItem,
  ExactCatalogRef,
} from '@/common/types/agentPlatform';
import { capabilityReferenceKey } from './model';

export const MODULE_CATEGORIES = [
  'knowledge',
  'development',
  'web',
  'collaboration',
  'creation',
  'automation',
  'devices',
  'integrations',
] as const;

export type ModuleCategory = (typeof MODULE_CATEGORIES)[number];
export type ModuleReference = ExactCatalogRef<'capability'>;

export function moduleCategory(reference: ModuleReference): ModuleCategory {
  const id = String(reference.id);
  if (id === 'knowledge' || id.endsWith('.memory')) return 'knowledge';
  if (id.startsWith('workspace.') || id === 'ssh' || id === 'plugin.development') {
    return 'development';
  }
  if (id === 'web.research' || id === 'browser') return 'web';
  if (
    id === 'agent.collaboration' ||
    id === 'channel.messaging' ||
    id === 'companion' ||
    id === 'customer.service'
  ) {
    return 'collaboration';
  }
  if (id === 'creation.media' || id === 'creative.workshop' || id === 'office') {
    return 'creation';
  }
  if (id === 'requirements' || id === 'automation.schedule') return 'automation';
  if (id === 'computer' || id === 'robot') return 'devices';
  return 'integrations';
}

export const selectedModuleReferences = (document: AgentPresetDocument): ModuleReference[] =>
  [...new Map(
    document.enabled_capabilities.map(({ capability }) => [
      capabilityReferenceKey(capability),
      capability,
    ])
  ).values()];

export const isBuiltinModule = (
  module: CapabilityModuleCatalogItem,
  capabilityCatalog: readonly CapabilityCatalogItem[]
): boolean => moduleAvailability(module, capabilityCatalog)?.source_kind === 'bundled';

export function moduleAvailability(
  module: CapabilityModuleCatalogItem | undefined,
  capabilityCatalog: readonly CapabilityCatalogItem[]
): CapabilityCatalogItem | undefined {
  if (!module) return undefined;
  const key = capabilityReferenceKey(module.module);
  return capabilityCatalog.find((item) => capabilityReferenceKey(item.capability) === key);
}

export function moduleIsAvailable(
  module: CapabilityModuleCatalogItem | undefined,
  capabilityCatalog: readonly CapabilityCatalogItem[]
): boolean {
  return Boolean(module) &&
    moduleAvailability(module, capabilityCatalog)?.materialization_state === 'materialized';
}

export const moduleIsSelectable = (
  module: CapabilityModuleCatalogItem | undefined,
  capabilityCatalog: readonly CapabilityCatalogItem[]
): boolean => module?.authoring_policy === 'direct' && moduleIsAvailable(module, capabilityCatalog);

export function unavailableModuleReferences(
  document: AgentPresetDocument,
  catalog: Pick<AgentCatalogResponse, 'modules' | 'capabilities'>
): ModuleReference[] {
  const byKey = new Map(catalog.modules.map((module) => [capabilityReferenceKey(module.module), module]));
  return document.enabled_capabilities.flatMap((selection) => {
    const module = byKey.get(capabilityReferenceKey(selection.capability));
    const knownActions = new Set(module?.actions.map((action) => action.action_id) ?? []);
    const actions = selection.action_allowlist ?? [];
    const hasMissingAction = actions.some((action) => !knownActions.has(action));
    const hasNoActionGrant = Boolean(module?.actions.length) && actions.length === 0;
    return !moduleIsAvailable(module, catalog.capabilities) || hasMissingAction || hasNoActionGrant
      ? [selection.capability]
      : [];
  });
}
