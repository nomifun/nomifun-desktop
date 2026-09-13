import type { AgentResourceSelection } from '@/common/types/agentPlatform';

export const AUTOMATIC_AGENT_RESOURCE_IDS: Readonly<Record<string, string>> = {
  workspace: 'default-workspace',
  project_memory: 'default-project-memory',
  process_session: 'managed-process-session',
  terminal: 'managed-terminal',
  asset_library: 'creative-studio-assets',
};

export const USER_AGENT_RESOURCE_KINDS = [
  'companion',
  'customer',
  'knowledge_base',
  'channel',
  'robot',
  'mcp_server',
  'canvas',
  'generation_provider',
  'miniapp',
] as const;

export type UserAgentResourceKind = (typeof USER_AGENT_RESOURCE_KINDS)[number];
export type AgentResourceSelectionValue = Partial<Record<UserAgentResourceKind, string>>;

export type AgentResourceSelectionResolution = {
  selections: AgentResourceSelection[];
  missingKinds: string[];
};

export const pickerKindForResourceKind = (kind: string): UserAgentResourceKind | undefined => {
  const normalized = kind === 'companion_memory' ? 'companion' : kind;
  return (USER_AGENT_RESOURCE_KINDS as readonly string[]).includes(normalized)
    ? normalized as UserAgentResourceKind
    : undefined;
};

export const requiredAgentResourcePickerKinds = (
  requiredKinds: Iterable<string>
): UserAgentResourceKind[] => {
  const fields = new Set<UserAgentResourceKind>();
  for (const kind of requiredKinds) {
    if (AUTOMATIC_AGENT_RESOURCE_IDS[kind]) continue;
    const pickerKind = pickerKindForResourceKind(kind);
    if (pickerKind) fields.add(pickerKind);
  }
  return USER_AGENT_RESOURCE_KINDS.filter((kind) => fields.has(kind));
};

/**
 * Convert product choices into the intentionally narrow create-session wire
 * contract. Resource ownership, operations, paths, credentials and connection
 * data remain server-owned and are never accepted from this UI.
 */
export const resolveAgentResourceSelections = (
  requiredKinds: Iterable<string>,
  value: AgentResourceSelectionValue
): AgentResourceSelectionResolution => {
  const selections: AgentResourceSelection[] = [];
  const missingKinds: string[] = [];
  for (const resourceKind of [...new Set(requiredKinds)].sort()) {
    const automaticId = AUTOMATIC_AGENT_RESOURCE_IDS[resourceKind];
    const pickerKind = pickerKindForResourceKind(resourceKind);
    const resourceId = automaticId ?? (pickerKind ? value[pickerKind] : undefined);
    if (!resourceId) {
      missingKinds.push(resourceKind);
      continue;
    }
    selections.push({ resource_kind: resourceKind, resource_id: resourceId });
  }
  return { selections, missingKinds };
};

export const selectedCapabilityIds = (
  enabled: readonly { capability: { id: string } }[]
): Set<string> => new Set(enabled.map((entry) => entry.capability.id));
