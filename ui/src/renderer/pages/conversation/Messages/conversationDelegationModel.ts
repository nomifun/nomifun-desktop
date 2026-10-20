/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ConversationId, ExecutionId, MessageId } from '@/common/types/ids';
import type { DelegationRecord } from '@/common/chat/delegationRecords';
import type { TAgentExecutionDetail } from '@/common/types/agentExecution/agentExecutionTypes';
import type { TurnDisclosureInputItem, TurnDisclosureProcessState } from './turnDisclosureModel';
import { isTerminalExecutionStatus } from '../execution/executionStatusMeta';

export interface ConversationDelegation {
  executionId: ExecutionId;
  turnId: MessageId;
  detail: TAgentExecutionDetail | null;
  unfinished: boolean;
}

/** Bind collaboration to its initiating request, never to the newest unrelated turn. */
export function resolveConversationDelegation(
  conversationId: ConversationId | undefined,
  execution: { conversationId: ConversationId; executionId: ExecutionId | null; detail: TAgentExecutionDetail | null } | null,
  items: TurnDisclosureInputItem[],
  records: DelegationRecord[],
  activeSource?: { turnId?: MessageId; requestMessageId?: MessageId },
): ConversationDelegation | undefined {
  if (!conversationId || !execution?.executionId || execution.conversationId !== conversationId) return undefined;
  const detail = execution.detail?.execution.execution_id === execution.executionId ? execution.detail : null;
  if (detail?.execution.lead_conversation_id && detail.execution.lead_conversation_id !== conversationId) return undefined;
  const sources = new Set(records.filter(record => record.execution_id === execution.executionId
    && record.source.agent_session_id === conversationId).map(record => record.source.source_message_id));
  // An incomplete/unloaded source cannot authorize attaching current work to a different request.
  if (sources.size !== 1) return undefined;
  const source = [...sources][0];
  const owner = items.find(item => item.role === 'user' && item.turnId
    && (item.sourceMessageIds?.includes(source) || item.turnId === source));
  const turnId = owner?.turnId ?? (activeSource?.requestMessageId === source ? activeSource.turnId : undefined);
  if (!turnId) return undefined;
  return { executionId: execution.executionId, turnId, detail,
    unfinished: !detail || !isTerminalExecutionStatus(detail.execution.status) };
}

/** Presentation only: root Turn lifecycle and composer ownership remain unchanged. */
export function delegatedTurnPresentation<T extends {
  running: boolean; state: TurnDisclosureProcessState; defaultCollapsed: boolean; endAt: number;
}>(turn: T, delegation: ConversationDelegation | undefined): T {
  if (!delegation) return turn;
  const running = turn.running || delegation.unfinished;
  const status = delegation.detail?.execution.status;
  const state: TurnDisclosureProcessState = running ? 'running'
    : status === 'cancelled' ? 'canceled'
    : status === 'failed' || status === 'completed_with_failures' ? 'failed' : turn.state;
  return { ...turn, running, state, defaultCollapsed: !running,
    endAt: running ? turn.endAt : Number.isFinite(turn.endAt)
      ? Math.max(turn.endAt, delegation.detail?.execution.updated_at ?? turn.endAt)
      : delegation.detail?.execution.updated_at ?? turn.endAt };
}
