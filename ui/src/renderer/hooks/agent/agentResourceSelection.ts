import type { AgentResourceSelection } from '@/common/types/agentPlatform';

export const AUTOMATIC_AGENT_RESOURCE_IDS: Readonly<Record<string, string>> = {
  workspace: 'default-workspace',
  project_memory: 'default-project-memory',
  process_session: 'managed-process-session',
  terminal: 'managed-terminal',
  asset_library: 'creative-studio-assets',
  browser: 'managed-browser',
  computer: 'local-desktop',
  scheduler: 'installation-scheduler',
};

export const USER_AGENT_RESOURCE_KINDS = [
  'companion',
  'customer',
  'knowledge_base',
  'channel',
  'robot',
  'mcp_server',
  'canvas',
  'plugin',
] as const;

/**
 * Enhancement resources may be absent from a Session. Their capability grant
 * remains frozen, but resource-backed Actions are materialized only when the
 * concrete target is selected. Computer is intentionally excluded: once the
 * local desktop is bound its OS permissions are a hard launch prerequisite.
 */
export const OPTIONAL_UNBOUND_AGENT_RESOURCE_KINDS = [
  'knowledge_base',
  'channel',
  'robot',
  'canvas',
  'plugin',
  'ssh_host',
] as const;

export const agentResourceKindMayRemainUnbound = (kind: string): boolean =>
  (OPTIONAL_UNBOUND_AGENT_RESOURCE_KINDS as readonly string[]).includes(kind);

export type UserAgentResourceKind = (typeof USER_AGENT_RESOURCE_KINDS)[number];
export type AgentResourceSelectionValue = Partial<Record<UserAgentResourceKind, string>> & {
  /** Frozen tool/resource servers. The singular field remains legacy-compatible. */
  mcp_servers?: string[];
};

export const selectedMcpResourceIds = (value: AgentResourceSelectionValue): string[] =>
  [...new Set((value.mcp_servers ?? (value.mcp_server ? [value.mcp_server] : [])).filter(Boolean))];

export const hasFrozenMcpTools = (capabilities: Iterable<string>): boolean =>
  [...capabilities].some((id) => /^nomi\.mcp\.v1\.[0-9a-f]{64}$/.test(id));

/** Other unmapped consumers still require one exact server on the backend. */
export const allowsMultipleMcpServers = (capabilities: Iterable<string>): boolean => {
  return hasFrozenMcpTools(capabilities);
};

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
    if (resourceKind === 'mcp_server') {
      const servers = selectedMcpResourceIds(value);
      if (!servers.length) missingKinds.push(resourceKind);
      for (const resourceId of servers) selections.push({ resource_kind: resourceKind, resource_id: resourceId });
      continue;
    }
    const automaticId = AUTOMATIC_AGENT_RESOURCE_IDS[resourceKind];
    const pickerKind = pickerKindForResourceKind(resourceKind);
    const resourceId = automaticId ?? (pickerKind ? value[pickerKind] : undefined);
    if (!resourceId) {
      if (!agentResourceKindMayRemainUnbound(resourceKind)) missingKinds.push(resourceKind);
      continue;
    }
    selections.push({ resource_kind: resourceKind, resource_id: resourceId });
  }
  return { selections, missingKinds };
};

export const selectedCapabilityIds = (
  enabled: readonly { capability: { id: string } }[]
): Set<string> => new Set(enabled.map((entry) => entry.capability.id));
