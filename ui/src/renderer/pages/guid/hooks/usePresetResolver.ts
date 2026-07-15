/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { Preset, PresetReference } from '@/common/types/agent/presetTypes';
import { useCallback } from 'react';

type UsePresetResolverOptions = {
  /**
   * Backend-merged preset catalog (`GET /api/presets`). The frontend uses this
   * catalog only for presentation. Runtime resolution
   * happens atomically in `POST /api/presets/{id}/resolve` during create.
   */
  presets: Preset[];
};

type UsePresetResolverResult = {
  resolvePresetAgentType: (
    agentInfo: { agent_type: string; backend?: string; preset_id?: PresetReference } | undefined
  ) => string;
};

/**
 * Compatibility facade for existing selection UI. It intentionally never
 * reads prompt/skill files or materializes a runtime configuration.
 */
export const usePresetResolver = ({
  presets: _presets,
}: UsePresetResolverOptions): UsePresetResolverResult => {
  const resolvePresetAgentType = useCallback(
    (agentInfo: { agent_type: string; backend?: string; preset_id?: PresetReference } | undefined): string => {
      if (!agentInfo) return 'gemini';
      return agentInfo.backend || agentInfo.agent_type;
    },
    []
  );

  return {
    resolvePresetAgentType,
  };
};
