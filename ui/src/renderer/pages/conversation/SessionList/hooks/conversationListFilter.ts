/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import type { CompanionId, SshHostId } from '@/common/types/ids';

type ConversationListItem = Pick<TChatConversation, 'execution_step_id' | 'extra'>;

/** Attempt transcripts, companion-owned sessions, and SSH-bound sessions have
 * dedicated surfaces; they never re-enter the ordinary work-conversation list. */
export const isOrdinaryWorkConversation = (conversation: ConversationListItem): boolean => {
  const extra = conversation.extra as
    | {
        is_health_check?: boolean;
        companion_session?: boolean;
        companion_id?: CompanionId;
        channel_platform?: string;
        ssh_host_id?: SshHostId;
      }
    | undefined;
  const isCompanionConversation =
    !!extra?.companion_session || !!extra?.companion_id || !!extra?.channel_platform;
  const isSshHostConversation = !!extra?.ssh_host_id;
  const isExecutionAttemptTranscript = Boolean(conversation.execution_step_id);
  return (
    extra?.is_health_check !== true &&
    !isCompanionConversation &&
    !isSshHostConversation &&
    !isExecutionAttemptTranscript
  );
};
