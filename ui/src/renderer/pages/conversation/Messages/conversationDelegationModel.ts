/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ConversationId, ExecutionId, MessageId } from '@/common/types/ids';
import type { TAgentExecutionDetail } from '@/common/types/agentExecution/agentExecutionTypes';
import type { TurnDisclosureInputItem, TurnDisclosureProcessState } from './turnDisclosureModel';
import { isTerminalExecutionStatus } from '../execution/executionStatusMeta';

export interface ConversationDelegation {
  executionId: ExecutionId;
  turnId: MessageId;
  detail: TAgentExecutionDetail | null;
  unfinished: boolean;
}

/** UUIDv7 carries message wall time when createdAt is only a history cursor. */
const identityTime = (id: string | undefined): number | undefined => {
  if (!id || !/^[\da-f]{8}-[\da-f]{4}-7[\da-f]{3}-[\da-f]{4}-[\da-f]{12}$/i.test(id)) return undefined;
  return Number.parseInt(id.replaceAll('-', '').slice(0, 12), 16);
};

/** Bind collaboration to its initiating request, never to the newest unrelated turn. */
export function resolveConversationDelegation(
  conversationId: ConversationId | undefined,
  execution: { conversationId: ConversationId; executionId: ExecutionId | null; detail: TAgentExecutionDetail | null } | null,
  items: TurnDisclosureInputItem[],
): ConversationDelegation | undefined {
  if (!conversationId || !execution?.executionId || execution.conversationId !== conversationId) return undefined;
  const detail = execution.detail?.execution.execution_id === execution.executionId ? execution.detail : null;
  if (detail?.execution.lead_conversation_id && detail.execution.lead_conversation_id !== conversationId) return undefined;
  const createdAt = detail?.execution.created_at ?? identityTime(execution.executionId);
  if (createdAt === undefined) return undefined;
  const starts = new Map(items.flatMap(item => item.turnId && item.turnStartedAt !== undefined
    ? [[item.turnId, item.turnStartedAt] as const] : []));
  let turnId: MessageId | undefined;
  let latestStart = -Infinity;
  for (const item of items) {
    if (item.role !== 'user' || !item.turnId) continue;
    const start = item.displayAt ?? starts.get(item.turnId) ?? identityTime(item.sourceMessageIds?.[0]) ?? item.createdAt;
    if (start <= createdAt && start >= latestStart) { turnId = item.turnId; latestStart = start; }
  }
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
    endAt: running ? turn.endAt : Math.max(turn.endAt, delegation.detail?.execution.updated_at ?? turn.endAt) };
}
