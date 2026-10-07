/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { OfficialPresetKey } from '@/common/types/agentPlatform';
import { isConversationAgentTemplateKey } from '@/renderer/components/agent/conversationAgentCatalog';

/** Only the host-verified projection may identify an internal official template. */
export const officialConversationTemplateKey = (extra: unknown): OfficialPresetKey | null => {
  if (!extra || typeof extra !== 'object' || Array.isArray(extra)) return null;
  const key = (extra as Record<string, unknown>).official_template_key;
  return typeof key === 'string' && isConversationAgentTemplateKey(key) ? key : null;
};
