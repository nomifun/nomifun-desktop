/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import useSWR from 'swr';
import { ipcBridge } from '@/common';
import type { Preset } from '@/common/types/agent/presetTypes';
import { DETECTED_AGENTS_SWR_KEY, fetchDetectedAgents } from '@/renderer/utils/model/agentTypes';
import type { AgentMetadata } from '@/renderer/utils/model/agentTypes';

export type UseConversationAgentsResult = {
  /** Detected execution engines (acp, extension, remote, nomi, gemini, etc.) */
  cliAgents: AgentMetadata[];
  /** Reusable configurations from `/api/presets`, kept separate from execution Agents. */
  presets: Preset[];
  /** Loading state */
  isLoading: boolean;
  /** Refresh data */
  refresh: () => Promise<void>;
};

/**
 * Hook to fetch available CLI agents and presets for launch selectors.
 *
 * Two independent data sources:
 *   - Execution engines — from AgentRegistry via IPC (agents.detected)
 *   - Presets — from backend `/api/presets` (merged builtin + user + extension)
 */
export const useConversationAgents = (): UseConversationAgentsResult => {
  // Execution engines from AgentRegistry (shared cache with useDetectedAgents / useGuidAgentSelection)
  const {
    data: cliAgents,
    isLoading: isLoadingAgents,
    mutate,
  } = useSWR<AgentMetadata[]>(DETECTED_AGENTS_SWR_KEY, fetchDetectedAgents);

  const { data: presets, isLoading: isLoadingPresets } = useSWR('presets.list', async () => {
    try {
      const list = await ipcBridge.presets.list.invoke();
      return list.filter((preset) => preset.enabled !== false);
    } catch (error) {
      console.error('Failed to load presets for conversation selector:', error);
      return [] as Preset[];
    }
  });

  const refresh = async () => {
    await mutate();
  };

  return {
    cliAgents: cliAgents || [],
    presets: presets || [],
    isLoading: isLoadingAgents || isLoadingPresets,
    refresh,
  };
};
