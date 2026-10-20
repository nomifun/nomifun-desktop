import type { AgentPresetSummary, OfficialPresetKey } from '@/common/types/agentPlatform';
import type { AgentPresetId } from '@/common/types/ids';

/** Workbench identity shared by creation, conversations, and scheduled tasks. */
export type AgentSelection =
  | { kind: 'template'; templateKey: OfficialPresetKey }
  | { kind: 'preset'; presetId: AgentPresetId };

export type ExecutableAgentPreset = AgentPresetSummary & {
  current_stable_revision: NonNullable<AgentPresetSummary['current_stable_revision']>;
};

export const DEFAULT_AGENT_SELECTION: AgentSelection = { kind: 'template', templateKey: 'assistant.general' };
