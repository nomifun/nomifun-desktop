/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentPresetSummary } from '@/common/types/agentPlatform';

/** A saved AgentPreset that can be launched because it has a stable revision. */
export type ExecutableAgentPreset = AgentPresetSummary & {
  current_stable_revision: NonNullable<AgentPresetSummary['current_stable_revision']>;
};

/** Computed option for the Guid @ mention dropdown. */
export type MentionOption = {
  key: string;
  label: string;
  tokens: Set<string>;
  avatarEmoji: string | undefined;
  avatarImage: string | undefined;
  logo: string | undefined;
};
