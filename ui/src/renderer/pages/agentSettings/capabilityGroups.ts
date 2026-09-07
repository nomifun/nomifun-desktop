import type { AgentPresetDocument, CapabilityCatalogItem, ExactCatalogRef } from '@/common/types/agentPlatform';
import { capabilityReferenceKey } from './model';

export const CAPABILITY_CATEGORIES = [
  'knowledge', 'development', 'web', 'collaboration', 'creation', 'automation', 'models', 'integrations',
] as const;
export type CapabilityCategory = (typeof CAPABILITY_CATEGORIES)[number];
export type CapabilityReference = ExactCatalogRef<'capability'>;

export function capabilityCategory(reference: CapabilityReference): CapabilityCategory {
  const family = reference.id.split('.')[0];
  if (['knowledge', 'memory', 'session'].includes(family)) return 'knowledge';
  if (['fs', 'vcs', 'process', 'terminal', 'workspace', 'ssh'].includes(family)) return 'development';
  if (['web', 'browser', 'computer', 'a11y', 'citation'].includes(family)) return 'web';
  if (['agent', 'companion', 'channel', 'customer_service', 'robot'].includes(family)) return 'collaboration';
  if (['creation', 'workshop', 'office', 'miniapp'].includes(family)) return 'creation';
  if (['requirements', 'autowork', 'schedule', 'idmm', 'notification', 'remote', 'ingress'].includes(family)) return 'automation';
  if (family === 'llm') return 'models';
  return 'integrations';
}

export const selectedCapabilityReferences = (document: AgentPresetDocument): CapabilityReference[] =>
  [...new Map([...document.initial_capabilities, ...document.on_demand_capabilities]
    .map(({ capability }) => [capabilityReferenceKey(capability), capability])).values()];

export const isBuiltinCapability = (item: CapabilityCatalogItem): boolean =>
  ['bundled', 'first_party', 'platform_builtin'].includes(item.source_kind);

export const capabilityIsAvailable = (item?: CapabilityCatalogItem): boolean =>
  item?.materialization_state === 'materialized';

export function unavailableCapabilityReferences(
  document: AgentPresetDocument,
  catalog: readonly CapabilityCatalogItem[],
): CapabilityReference[] {
  const available = new Set(catalog.filter(capabilityIsAvailable).map((item) => capabilityReferenceKey(item.capability)));
  return selectedCapabilityReferences(document).filter((reference) => !available.has(capabilityReferenceKey(reference)));
}
