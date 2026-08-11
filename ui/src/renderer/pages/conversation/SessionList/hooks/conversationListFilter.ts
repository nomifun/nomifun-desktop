/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import type { CompanionId, SshHostId } from '@/common/types/ids';

type ConversationListItem = Pick<TChatConversation, 'execution_step_id' | 'extra'>;

/** Attempt transcripts, companion-owned sessions, SSH-bound sessions and robot
 * threads have dedicated surfaces; they never re-enter the ordinary
 * work-conversation list. */
export const isOrdinaryWorkConversation = (conversation: ConversationListItem): boolean => {
  const extra = conversation.extra as
    | {
        is_health_check?: boolean;
        companion_session?: boolean;
        companion_id?: CompanionId;
        channel_platform?: string;
        ssh_host_id?: SshHostId;
        robot_session?: boolean;
        robot_id?: string;
      }
    | undefined;
  const isCompanionConversation =
    !!extra?.companion_session || !!extra?.companion_id || !!extra?.channel_platform;
  const isSshHostConversation = !!extra?.ssh_host_id;
  // Named explicitly rather than left to `companion_id`: a robot thread is a
  // device's long-lived conversation, and the companion marker it happens to
  // carry is the companion GROUP's key — leaning on it would put robot threads
  // back in this list the day that marker changes.
  const isRobotConversation = !!extra?.robot_session || !!extra?.robot_id;
  const isExecutionAttemptTranscript = Boolean(conversation.execution_step_id);
  return (
    extra?.is_health_check !== true &&
    !isCompanionConversation &&
    !isSshHostConversation &&
    !isRobotConversation &&
    !isExecutionAttemptTranscript
  );
};
