import { describe, expect, test } from 'bun:test';
import type { TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { creativeAcceptedTurnOwners, creativeMessageTurn, creativeProcessOwnerIds, creativeProcessSegments } from './canonicalProcessModel';
import { serializeCreativeStudioAgentHistory } from './adapters/history';

const session = parseConversationId('0190f5fe-7c00-7a00-8000-000000000501');
const first = parseMessageId('0190f5fe-7c00-7a00-8000-000000000502');
const later = parseMessageId('0190f5fe-7c00-7a00-8000-000000000503');
const answer = parseMessageId('0190f5fe-7c00-7a00-8000-000000000504');
const source = (id: string, type: TMessage['type'], content: unknown, turn = first, position = 'left'): TMessage => ({
  id, message_id: id, conversation_id: session, turn_id: turn, type, content, position, created_at: 100,
} as TMessage);

describe('Canvas canonical process projection', () => {
  test('uses the exact public assistant identity across reopened and later turns', () => {
    const history = [source(first, 'text', { content: '{"private_planning_input":true}' }, first, 'right'),
      source(answer, 'text', { content: 'Original response' }), source(later, 'text', { content: 'Later request' }, later, 'right')];
    const message = { id: answer, role: 'assistant', status: 'complete', text: 'Canvas visible response' } as const;
    expect(creativeMessageTurn(message, history, later)).toBe(first);
    expect(creativeMessageTurn({ ...message, id: 'unrelated' }, history, later)).toBeUndefined();
    expect(creativeMessageTurn({ id: 'local-draft', role: 'assistant', status: 'running', text: '' }, history)).toBeUndefined();
    expect(creativeMessageTurn({ id: 'local-draft', role: 'assistant', status: 'running', text: '' }, history, later)).toBe(later);
    const proof = serializeCreativeStudioAgentHistory([message]);
    creativeProcessSegments(history, true);
    expect(serializeCreativeStudioAgentHistory([message])).toBe(proof);
  });

  test('canonical question/error identities retain their exact Turn even as center tips', () => {
    const question = source(answer, 'tips', { content: 'Question', type: 'warning' }, first, 'center');
    expect(creativeMessageTurn({ id: answer, role: 'assistant', status: 'failed', text: '', errorMessage: 'failed' }, [question], later)).toBe(first);
  });

  test('delegation ownership keeps the accepted source UUID when the runtime root differs', () => {
    const canonical = [source(first, 'text', { content: 'User-visible request' }, first, 'right'),
      source(answer, 'text', { content: 'Response' }, later)];
    const owners = creativeAcceptedTurnOwners(canonical, later, first);
    expect(owners[0].turnId).toBe(later);
    expect(owners[0].sourceMessageIds).toContain(first);
    expect(owners[1].turnId).toBe(later);
  });

  test('one Turn with multiple public assistant members owns exactly one process and clock', () => {
    const second = parseMessageId('0190f5fe-7c00-7a00-8000-000000000505');
    const canonical = [source(answer, 'text', { content: 'First result' }), source(second, 'text', { content: 'Second result' })];
    const messages = [{ id: answer, role: 'assistant', status: 'complete', text: 'First result' },
      { id: second, role: 'assistant', status: 'complete', text: 'Second result' }] as const;
    expect([...creativeProcessOwnerIds(messages, canonical)]).toEqual([answer]);
    expect([...creativeProcessOwnerIds([...messages, { id: 'local-active', role: 'assistant', status: 'running', text: '' }], canonical, first)]).toEqual(['local-active']);
    expect([...creativeProcessOwnerIds(messages, canonical, later)]).toEqual([answer]);
  });

  test('shows canonical tool/reasoning order without model-input/planning JSON or private controls', () => {
    const timeline = (id: string, seq: number, kind: string) => ({ through_seq: 30, segments: [{ segment_id: id, group_id: `process:${first}`,
      first_seq: seq, last_seq: seq, kind, state: 'completed', ...(kind === 'tool' ? { call_id: id } : { start_utf16: 0, end_utf16: 7 }) }] });
    const command = source('command', 'tool_call', { call_id: 'command', name: 'start_process', status: 'completed', timeline: timeline('command', 10, 'tool') });
    const thought = source('reasoning', 'thinking', { content: 'Inspect', status: 'done', timeline: timeline('reasoning', 5, 'reasoning') });
    const privateRead = source('preflight', 'tool_call', { call_id: 'agent-instructions:read', name: 'read_file', status: 'completed' });
    const plan = source('plan', 'tool_call', { call_id: 'plan', name: 'update_plan', status: 'completed' });
    const rawPlanning = source(answer, 'text', { content: '{"kind":"model-planning-input","secret_context":"private"}' });
    const original = [rawPlanning, command, thought, privateRead, plan];
    expect(creativeProcessSegments(original, true).map(message => message.id)).toEqual(['reasoning', 'command']);
    expect(creativeProcessSegments(original, false).map(message => message.id)).toEqual(['command']);
    expect(JSON.stringify(creativeProcessSegments(original, true))).not.toContain('secret_context');
    expect(original[0]).toBe(rawPlanning);
  });

  test('completion receipt visibility matches chat without hiding same-name tools or the final response', () => {
    const makeReport = (id: string, extra: object = {}) => source(id, 'tool_call', {
      call_id: id, name: 'report_completion', status: 'completed', result_seq: 12, ...extra,
    });
    const finalText = 'report_completion remains part of this final answer';
    const final = source(answer, 'text', { content: finalText });
    const confirmed = makeReport('confirmed', { description: finalText });
    const pending = makeReport('pending', { result_seq: undefined });
    const failed = makeReport('failed', { status: 'error', error: 'Rejected account' });
    const held = makeReport('held', { runtime_hold: 'arguments_rejected' });
    const external = makeReport('external', { capability_id: 'third.party', action_id: 'third.party/report' });
    const original = [confirmed, pending, failed, held, external, final];
    const snapshot = JSON.stringify(original);
    expect(creativeProcessSegments(original, true).map(message => message.id)).toEqual(['pending', 'failed', 'held', 'external']);
    expect(creativeProcessSegments([confirmed, final], true)).toEqual([]);
    expect(JSON.stringify(original)).toBe(snapshot);
    expect(creativeMessageTurn({ id: answer, role: 'assistant', status: 'complete', text: 'Final response' }, original)).toBe(first);
  });
});
