import type { IMcpServer } from '@/common/config/storage';
import type { ConnectorId } from '@/common/connectors';
import type { SkillInfo } from '@/common/types/skill';
import type { AgentSessionCapabilitySelection } from '@/common/types/agentPlatform';

export type SessionSkillOption = Omit<SkillInfo, 'source'> & {
  source: SkillInfo['source'] | 'extension';
  auto: boolean;
};

export type SessionCapabilityCatalog = {
  skills: SessionSkillOption[];
  autoSkillNames: ReadonlySet<string>;
  mcpServers: SessionMcpOption[];
  appSourceError?: string;
  /** Connector readiness gates the skills those connectors provide. */
  connectorsReady?: Partial<Record<ConnectorId, boolean>>;
};

export type SessionMcpOption = IMcpServer & { appSource?: boolean };

export type SessionCapabilityDraft = {
  skillNames: string[];
  mcpServerIds: string[];
};

export const toSessionCapabilitySelection = (draft: SessionCapabilityDraft): AgentSessionCapabilitySelection => ({
  skill_names: [...new Set(draft.skillNames)].sort(),
  mcp_server_ids: [...new Set(draft.mcpServerIds)].sort(),
});

export const fromSessionCapabilitySelection = (selection: AgentSessionCapabilitySelection): SessionCapabilityDraft => ({
  skillNames: [...selection.skill_names],
  mcpServerIds: [...selection.mcp_server_ids],
});

export const defaultSessionCapabilityDraft = (catalog: SessionCapabilityCatalog): SessionCapabilityDraft => ({
  skillNames: catalog.skills.filter((skill) => skill.auto && skill.session_available !== false).map((skill) => skill.name),
  mcpServerIds: [],
});

/** Keep missing identities removable; a known disabled source is no longer a choice. */
export const withoutDisabledMcpSelections = (
  draft: SessionCapabilityDraft,
  servers: readonly IMcpServer[],
  lockedServerIds?: ReadonlySet<string>
): SessionCapabilityDraft => {
  const disabledIds = new Set<string>(servers.filter((server) => !server.enabled).map((server) => server.mcp_server_id));
  const mcpServerIds = draft.mcpServerIds.filter((id) => !disabledIds.has(id) || lockedServerIds?.has(id));
  return mcpServerIds.length === draft.mcpServerIds.length ? draft : { ...draft, mcpServerIds };
};
