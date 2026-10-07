import type { IMcpServer } from '@/common/config/storage';
import type { SkillInfo } from '@/common/types/skill';
import type { AgentSessionCapabilitySelection } from '@/common/types/agentPlatform';

export type SessionSkillOption = Omit<SkillInfo, 'source'> & {
  source: SkillInfo['source'] | 'extension';
  auto: boolean;
};

export type SessionCapabilityCatalog = {
  skills: SessionSkillOption[];
  autoSkillNames: ReadonlySet<string>;
  mcpServers: IMcpServer[];
};

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
  mcpServerIds: catalog.mcpServers.filter((server) => server.enabled && server.last_test_status === 'connected' && Boolean(server.tools?.length)).map((server) => server.mcp_server_id),
});
