/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentPresetSummary } from '@/common/types/agentPlatform';
import { useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';
import { useAgents } from '@/renderer/hooks/agent/useAgents';

export type UseConversationAgentsResult = {
  /** Detected ordinary execution Agents. */
  cliAgents: ReturnType<typeof useAgents>['agents'];
  /** User-owned AgentPreset summaries. They are launch identities, not runtime rows. */
  agentPresets: AgentPresetSummary[];
  isLoading: boolean;
  refresh: () => Promise<void>;
};

/**
 * Shared selector data for Conversation-adjacent surfaces such as Cron.
 * Runtime Agent rows and saved AgentPreset summaries remain separate types.
 */
export const useConversationAgents = (): UseConversationAgentsResult => {
  const { agents, isLoading: agentsLoading, refreshCustomAgents } = useAgents();
  const { presets: agentPresets, isLoading: presetsLoading, refresh: refreshPresets } = useAgentPresets();

  return {
    cliAgents: agents,
    agentPresets,
    isLoading: agentsLoading || presetsLoading,
    refresh: async () => {
      await Promise.all([refreshCustomAgents(), refreshPresets()]);
    },
  };
};
