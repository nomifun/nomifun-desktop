/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  OFFICIAL_PRESET_KEYS,
  type AgentPresetSummary,
  type OfficialPresetKey,
} from '@/common/types/agentPlatform';
import type { AgentPresetId, ProviderId } from '@/common/types/ids';
import { configService } from '@/common/config/configService';
import type { ExecutableAgentPreset, GuidAgentSelection } from '../types';

export const DEFAULT_GUID_AGENT_SELECTION: GuidAgentSelection = {
  kind: 'template',
  templateKey: 'chat.minimal',
};

/** Normalize persisted/unknown selection state to a workbench catalog identity. */
export const normalizeGuidAgentSelection = (
  value: unknown
): GuidAgentSelection => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return DEFAULT_GUID_AGENT_SELECTION;
  }
  const candidate = value as Record<string, unknown>;
  if (candidate.kind === 'preset' && typeof candidate.presetId === 'string') {
    return {
      kind: 'preset',
      presetId: candidate.presetId as AgentPresetId,
    };
  }
  if (
    candidate.kind === 'template' &&
    typeof candidate.templateKey === 'string' &&
    OFFICIAL_PRESET_KEYS.includes(candidate.templateKey as OfficialPresetKey)
  ) {
    return {
      kind: 'template',
      templateKey: candidate.templateKey as OfficialPresetKey,
    };
  }
  return DEFAULT_GUID_AGENT_SELECTION;
};

/** Save default nomi provider/model so the Guid page restores it next session. */
export async function saveNomiDefaultModel(
  provider_id: ProviderId,
  use_model: string
): Promise<void> {
  try {
    await configService.set('nomi.defaultModel', { provider_id, model: use_model });
  } catch {
    /* silent */
  }
}

export const isExecutableAgentPreset = (
  preset: AgentPresetSummary
): preset is ExecutableAgentPreset => Boolean(preset.current_stable_revision);

export const getAgentPresetKey = (
  preset: Pick<AgentPresetSummary, 'preset_id'>
): string => preset.preset_id;
