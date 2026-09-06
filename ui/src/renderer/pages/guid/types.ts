/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { GuidAgentSelectionPreference } from '@/common/config/configKeys';
import type { AgentPresetSummary } from '@/common/types/agentPlatform';

/** The quick-start surface either uses plain Nomi or one frozen AgentPreset. */
export type GuidAgentSelection = GuidAgentSelectionPreference;

/** A saved AgentPreset that can be launched because it has a stable revision. */
export type ExecutableAgentPreset = AgentPresetSummary & {
  current_stable_revision: NonNullable<AgentPresetSummary['current_stable_revision']>;
};

/** Computed option for the Guid @ mention dropdown. */
export type MentionOption = {
  key: string;
  label: string;
  tokens: Set<string>;
  selection: GuidAgentSelection;
  avatarEmoji: string | undefined;
  avatarImage: string | undefined;
  logo: string | undefined;
};
