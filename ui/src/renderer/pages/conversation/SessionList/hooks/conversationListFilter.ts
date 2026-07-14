/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';

type ConversationListItem = Pick<TChatConversation, 'execution_step_id' | 'extra'>;

/** Attempt transcripts and companion-owned sessions have dedicated surfaces;
 * they never re-enter the ordinary work-conversation list. */
export const isOrdinaryWorkConversation = (conversation: ConversationListItem): boolean => {
  const extra = conversation.extra as
    | {
        is_health_check?: boolean;
        companionSession?: boolean;
        companionId?: string;
        channelPlatform?: string;
      }
    | undefined;
  const isCompanionConversation =
    !!extra?.companionSession || !!extra?.companionId || !!extra?.channelPlatform;
  const isExecutionAttemptTranscript = Boolean(conversation.execution_step_id);
  return extra?.is_health_check !== true && !isCompanionConversation && !isExecutionAttemptTranscript;
};
