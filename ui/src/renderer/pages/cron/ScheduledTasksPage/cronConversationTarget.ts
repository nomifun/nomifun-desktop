/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ConversationId } from '@/common/types/ids';

export type ConversationExecutionMode = 'new_conversation' | 'existing' | 'specified';
export type BackendExecutionMode = Exclude<ConversationExecutionMode, 'specified'>;

export type CronConversationTarget =
  | {
      kind: 'unbound';
      executionMode: BackendExecutionMode;
    }
  | {
      kind: 'specified';
      executionMode: 'existing';
      conversationId: ConversationId;
    };

export type CronConversationRequestFields = {
  execution_mode: BackendExecutionMode;
  conversation_id?: ConversationId;
};

/**
 * Resolve the UI execution mode into the backend conversation target.
 *
 * New-conversation and continuing-conversation tasks intentionally start
 * without a conversation ID. The backend creates a conversation on the first
 * run and, for continuing mode, reuses that lazily-bound conversation later.
 * Only the explicit "specified conversation" mode may carry a pre-existing ID.
 */
export function resolveCronConversationTarget(
  executionMode: ConversationExecutionMode,
  specifiedConversationId?: ConversationId,
): CronConversationTarget | null {
  if (executionMode === 'specified') {
    return specifiedConversationId
      ? {
          kind: 'specified',
          executionMode: 'existing',
          conversationId: specifiedConversationId,
        }
      : null;
  }

  return {
    kind: 'unbound',
    executionMode,
  };
}

/** Build only the target-related create fields, omitting IDs for unbound modes. */
export function buildCronConversationRequestFields(
  target: CronConversationTarget,
): CronConversationRequestFields {
  return target.kind === 'specified'
    ? { execution_mode: target.executionMode, conversation_id: target.conversationId }
    : { execution_mode: target.executionMode };
}
