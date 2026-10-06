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
import { getAgentLogo } from '@/renderer/utils/model/agentLogo';
import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { officialConversationTemplateKey } from '@/renderer/pages/conversation/components/conversationAgentIdentity';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';

export interface AgentInfo {
  preset_id: AgentPresetId;
  name: string;
  logo?: string;
  isEmoji: boolean;
  revision?: number;
}

function resolveAgentPresetId(conversation: TChatConversation): AgentPresetId | null {
  return conversation.preset_id ?? null;
}

function resolveAgentSnapshot(conversation: TChatConversation): AgentResolvedSnapshot | null {
  const value = conversation.agent_snapshot;
  if (!value || typeof value !== 'object') return null;
  const candidate = value as Partial<AgentResolvedSnapshot>;
  return typeof candidate.preset_id === 'string' && typeof candidate.preset_name === 'string'
    ? (candidate as AgentResolvedSnapshot)
    : null;
}

function resolveAgentDisplayName(
  conversation: TChatConversation,
  snapshot: AgentResolvedSnapshot | null,
): string {
  return (
    (conversation.extra as { agent_name?: string } | undefined)?.agent_name?.trim() ||
    snapshot?.preset_name?.trim() ||
    conversation.name?.trim() ||
    'Agent'
  );
}

/**
 * Personal identity is snapshot-only. Official product names use the stable
 * host-verified template identity and current locale, without a catalog lookup.
 */
export function useAgentInfo(conversation: TChatConversation | undefined): {
  info: AgentInfo | null;
  isLoading: false;
} {
  const { t } = useTranslation();
  return useMemo(() => {
    if (!conversation) return { info: null, isLoading: false as const };
    const presetId = resolveAgentPresetId(conversation);
    const snapshot = resolveAgentSnapshot(conversation);
    if (!presetId || !snapshot) return { info: null, isLoading: false as const };
    const templateKey = officialConversationTemplateKey(conversation.extra);
    return {
      info: {
        preset_id: presetId,
        name: templateKey
          ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[templateKey]}.name`)
          : resolveAgentDisplayName(conversation, snapshot),
        logo:
          getAgentLogo(snapshot.resolved_agent_backend || conversation.type) ??
          undefined,
        isEmoji: false,
        revision: snapshot.preset_revision,
      },
      isLoading: false as const,
    };
  }, [conversation, t]);
}
