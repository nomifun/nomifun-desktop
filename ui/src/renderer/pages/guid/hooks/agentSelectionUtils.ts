/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentPresetSummary } from '@/common/types/agentPlatform';
import type { ProviderId } from '@/common/types/ids';
import { configService } from '@/common/config/configService';
import type { ExecutableAgentPreset } from '../types';

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
