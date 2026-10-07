/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import { conversationSshHostId } from '../../utils/conversationSshBinding';

type ConversationListItem = Pick<TChatConversation, 'execution_step_id' | 'extra' | 'agent_snapshot'>;

/** Attempt transcripts, companion-owned sessions, SSH-bound sessions have dedicated surfaces; they never re-enter the ordinary
 * work-conversation list. */
export const isOrdinaryWorkConversation = (conversation: ConversationListItem): boolean => {
  const extra = conversation.extra;
  const isCompanionConversation =
    !!extra?.companion_session ||
    !!extra?.companion_id ||
    !!extra?.channel_platform ||
    conversation.agent_snapshot?.preset_name === 'companion.default' ||
    conversation.agent_snapshot?.enabled_capabilities.includes('companion') === true;
  const isSshHostConversation = conversationSshHostId(conversation) != null;
  const isExecutionAttemptTranscript = Boolean(conversation.execution_step_id);
  return (
    !isCompanionConversation &&
    !isSshHostConversation &&
    !isExecutionAttemptTranscript
  );
};
