/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentPresetSummary, OfficialPresetKey } from '@/common/types/agentPlatform';
import { OFFICIAL_PRESET_KEYS } from '@/common/types/agentPlatform';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';
import type { TFunction } from 'i18next';

/** Presentation only: a saved official customization always remains a Preset choice. */
export function agentPresetDisplayName(preset: AgentPresetSummary, t: TFunction): string {
  if (preset.source !== 'official') return preset.display_name;
  const name = OFFICIAL_PRESET_KEYS.includes(preset.display_name as OfficialPresetKey)
    ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[preset.display_name as OfficialPresetKey]}.name`)
    : preset.display_name;
  return t('guid.agentEntries.savedCustomization', { name });
}
