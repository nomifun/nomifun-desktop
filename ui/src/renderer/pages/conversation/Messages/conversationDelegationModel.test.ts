import { describe, expect, test } from 'bun:test';
import { parseExecutionId, parseMessageId } from '@/common/types/ids';
import { childConversationId, executionId, leadConversationId, makeDetail, requestId } from '../../../../../test/fixtures/conversationDelegation';
import { delegatedTurnPresentation, resolveConversationDelegation } from './conversationDelegationModel';
import { assignTurnIdsFromUserRequests, type TurnDisclosureInputItem } from './turnDisclosureModel';

const nextRequestId = parseMessageId('00000000-0fa0-7000-8000-000000000003');
const items: TurnDisclosureInputItem[] = [
  { id: 'request', turnId: requestId, role: 'user', createdAt: 1, displayAt: 1000 },
  { id: 'root-answer', turnId: requestId, role: 'assistant', createdAt: 2, displayAt: 3000 },
  { id: 'later-request', turnId: nextRequestId, role: 'user', createdAt: 3, displayAt: 4000 },
];
const context = (detail = makeDetail()) => ({ conversationId: leadConversationId, executionId, detail });

describe('conversation delegation lifecycle', () => {
  test('binds to the initiating request rather than a newer unrelated turn or a history cursor', () => {
    const result = resolveConversationDelegation(leadConversationId, context(), items);
    expect(result?.turnId).toBe(requestId);
    expect(result?.unfinished).toBe(true);
  });

  test('correlates a distinct backend root id and uses its authoritative start time', () => {
    const rootId = parseMessageId('00000000-04b0-7000-8000-000000000006');
    const input = assignTurnIdsFromUserRequests([
      { id: 'request', turnId: requestId, role: 'user', createdAt: 1 },
      { id: 'root', turnId: rootId, role: 'metadata', createdAt: 2, turnStartedAt: 1200 },
      items[2],
    ]);
    expect(resolveConversationDelegation(leadConversationId, context(), input)?.turnId).toBe(rootId);
  });

  test('UUID wall time also binds reloaded and not-yet-synchronized execution snapshots', () => {
    const input = items.filter(item => item.role === 'user').map(item => ({ ...item, displayAt: undefined, sourceMessageIds: [item.turnId!] }));
    expect(resolveConversationDelegation(leadConversationId, context(), input)?.turnId).toBe(requestId);
    const result = resolveConversationDelegation(leadConversationId, { ...context(), detail: null }, input);
    expect(result?.turnId).toBe(requestId);
    expect(result?.unfinished).toBe(true);
    expect(result?.detail).toBeNull();
  });

  test('never leaks lead execution progress into another conversation or projected child transcript', () => {
    expect(resolveConversationDelegation(childConversationId, context(), items)).toBeUndefined();
    const detail = makeDetail();
    detail.execution.lead_conversation_id = childConversationId;
    expect(resolveConversationDelegation(leadConversationId, context(detail), items)).toBeUndefined();
    expect(resolveConversationDelegation(leadConversationId, null, items)).toBeUndefined();
    expect(resolveConversationDelegation(leadConversationId, context(), [items[2]])).toBeUndefined();
  });

  test('a stale snapshot for a replaced execution cannot mark the new work finished', () => {
    const old = makeDetail(); old.execution.status = 'completed';
    const result = resolveConversationDelegation(leadConversationId,
      { ...context(old), executionId: parseExecutionId('00000000-07d0-7000-8000-000000000099') }, items);
    expect(result?.detail).toBeNull();
    expect(result?.unfinished).toBe(true);
  });

  test.each(['planning', 'running', 'paused', 'waiting_input'] as const)('%s keeps the task open and the one overall clock running', status => {
    const detail = makeDetail(); detail.execution.status = status;
    const delegation = resolveConversationDelegation(leadConversationId, context(detail), items);
    const root = { running: false, state: 'completed' as const, defaultCollapsed: true, endAt: 3000 };
    expect(delegatedTurnPresentation(root, delegation)).toEqual({ ...root, running: true, state: 'running', defaultCollapsed: false });
    expect(root).toEqual({ running: false, state: 'completed', defaultCollapsed: true, endAt: 3000 });
  });

  test.each(['completed', 'completed_with_failures', 'failed', 'cancelled'] as const)('%s stops at the later task finish time and collapses', status => {
    const detail = makeDetail(); Object.assign(detail.execution, { status, updated_at: 5000 });
    const delegation = resolveConversationDelegation(leadConversationId, context(detail), items);
    const view = delegatedTurnPresentation({ running: false, state: 'completed' as const, defaultCollapsed: true, endAt: 3000 }, delegation);
    expect(delegation?.unfinished).toBe(false);
    expect(view.running).toBe(false);
    expect(view.defaultCollapsed).toBe(true);
    expect(view.endAt).toBe(5000);
    expect(view.state).toBe(status === 'cancelled' ? 'canceled' : status === 'completed' ? 'completed' : 'failed');
  });

  test('child completion cannot stop a still-running root turn or shorten its final interval', () => {
    const detail = makeDetail(); detail.execution.status = 'completed'; detail.execution.updated_at = 2000;
    const delegation = resolveConversationDelegation(leadConversationId, context(detail), items);
    const root = { running: true, state: 'running' as const, defaultCollapsed: false, endAt: 3000 };
    expect(delegatedTurnPresentation(root, delegation).running).toBe(true);
    expect(delegatedTurnPresentation({ ...root, running: false }, delegation).endAt).toBe(3000);
    expect(delegatedTurnPresentation(root, undefined)).toBe(root);
  });
});
