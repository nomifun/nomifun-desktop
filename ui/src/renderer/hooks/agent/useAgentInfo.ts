/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import type {
  AgentPresetId,
  AgentResolvedSnapshot,
} from '@/common/types/agentPlatform';
import { useMemo } from 'react';

export interface AgentInfo {
  preset_id: AgentPresetId;
  name: string;
  logo?: string;
  isEmoji: boolean;
  revision?: number;
}

export function resolveAgentPresetId(conversation: TChatConversation): AgentPresetId | null {
  return conversation.preset_id ?? null;
}

export function resolveAgentSnapshot(conversation: TChatConversation): AgentResolvedSnapshot | null {
  const value = conversation.agent_snapshot;
  if (!value || typeof value !== 'object') return null;
  const candidate = value as Partial<AgentResolvedSnapshot>;
  return typeof candidate.preset_id === 'string' && typeof candidate.preset_name === 'string'
    ? (candidate as AgentResolvedSnapshot)
    : null;
}

export function resolveAgentDisplayName(
  conversation: TChatConversation,
  snapshot: AgentResolvedSnapshot | null,
): string {
  return (
    snapshot?.preset_name?.trim() ||
    conversation.name?.trim() ||
    'Agent'
  );
}

/**
 * Historical conversation identity is snapshot-only. There is intentionally
 * no live AgentPreset lookup: editing an Agent must never rewrite history.
 */
export function useAgentInfo(conversation: TChatConversation | undefined): {
  info: AgentInfo | null;
  isLoading: false;
} {
  return useMemo(() => {
    if (!conversation) return { info: null, isLoading: false as const };
    const presetId = resolveAgentPresetId(conversation);
    const snapshot = resolveAgentSnapshot(conversation);
    if (!presetId || !snapshot) return { info: null, isLoading: false as const };
    return {
      info: {
        preset_id: presetId,
        name: resolveAgentDisplayName(conversation, snapshot),
        isEmoji: false,
        revision: snapshot.preset_revision,
      },
      isLoading: false as const,
    };
  }, [conversation]);
}
