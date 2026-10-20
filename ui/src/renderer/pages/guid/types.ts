/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentSelection } from '@/renderer/components/agent/types';
export type { ExecutableAgentPreset } from '@/renderer/components/agent/types';
export type GuidAgentSelection = AgentSelection;

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
