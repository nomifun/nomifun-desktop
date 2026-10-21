/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { agentPlatform } from '@/common/adapter/ipcBridge';
import type {
  AgentPresetLibraryResponse,
  AgentPresetSummary,
} from '@/common/types/agentPlatform';
import useSWR from 'swr';
import { savedAgentPresets } from '@/renderer/components/agent/conversationAgentCatalog';

/** One shared cache for every product surface that selects an AgentPreset. */
export const AGENT_PRESET_LIBRARY_SWR_KEY = 'agent-presets.library';

export const fetchAgentPresetLibrary = async (): Promise<AgentPresetLibraryResponse> =>
  agentPlatform.library.invoke();

/**
 * The only renderer-side AgentPreset catalog hook.
 *
 * Templates are current published definitions. Saved owner configurations are
 * explicit Preset choices; their immutable revisions remain independently selectable.
 */
export const useAgentPresets = (): {
  library: AgentPresetLibraryResponse | undefined;
  presets: AgentPresetSummary[];
  isLoading: boolean;
  error: Error | undefined;
  refresh: () => Promise<void>;
} => {
  const { data, error, isLoading, mutate } = useSWR<
    AgentPresetLibraryResponse,
    Error
  >(
    AGENT_PRESET_LIBRARY_SWR_KEY,
    fetchAgentPresetLibrary,
  );

  return {
    library: data,
    presets: data ? savedAgentPresets(data) : [],
    isLoading,
    error,
    refresh: async () => {
      await mutate();
    },
  };
};
