import type { AgentSessionCapabilitySelection } from '@/common/types/agentPlatform';
import type { IMcpServer } from '@/common/config/storage';
import type { SkillInfo } from '@/common/types/skill';

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

export const EMPTY_SESSION_CAPABILITY_DRAFT: SessionCapabilityDraft = {
  skillNames: [],
  mcpServerIds: [],
};

const uniqueSorted = (values: Iterable<string>): string[] =>
  Array.from(new Set(values)).sort((left, right) => left.localeCompare(right));

export const buildSessionCapabilitySelection = (
  draft: SessionCapabilityDraft,
  autoSkillNames: ReadonlySet<string>
): AgentSessionCapabilitySelection => {
  const selected = new Set(draft.skillNames);
  return {
    enabled_skills: uniqueSorted(draft.skillNames.filter((name) => !autoSkillNames.has(name))),
    excluded_auto_skills: uniqueSorted(
      Array.from(autoSkillNames).filter((name) => !selected.has(name))
    ),
    mcp_server_ids: Array.from(new Set(draft.mcpServerIds)),
  };
};

export const defaultSessionCapabilityDraft = (
  catalog: SessionCapabilityCatalog,
  presetSkillNames: Iterable<string> = []
): SessionCapabilityDraft => {
  const availableSkillNames = new Set(catalog.skills.map((skill) => skill.name));
  return {
    skillNames: uniqueSorted([
      ...catalog.autoSkillNames,
      ...Array.from(presetSkillNames).filter((name) => availableSkillNames.has(name)),
    ]),
    mcpServerIds: catalog.mcpServers
      .filter((server) => !server.builtin && server.enabled)
      .map((server) => server.mcp_server_id),
  };
};

export const draftFromSessionCapabilitySelection = (
  selection: AgentSessionCapabilitySelection,
  autoSkillNames: ReadonlySet<string>
): SessionCapabilityDraft => {
  const excluded = new Set(selection.excluded_auto_skills);
  return {
    skillNames: uniqueSorted([
      ...Array.from(autoSkillNames).filter((name) => !excluded.has(name)),
      ...selection.enabled_skills,
    ]),
    mcpServerIds: Array.from(new Set(selection.mcp_server_ids)),
  };
};

export const sessionCapabilitySelectionKey = (
  selection: AgentSessionCapabilitySelection
): string => JSON.stringify(selection);

export const toggleCapabilityValue = (
  values: readonly string[],
  value: string,
  checked: boolean
): string[] =>
  checked
    ? Array.from(new Set([...values, value]))
    : values.filter((candidate) => candidate !== value);
