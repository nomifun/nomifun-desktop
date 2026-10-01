import { describe, expect, test } from 'bun:test';
import type { IMessageText, TMessage } from '@/common/chat/chatLib';
import { projectCompletionOutcomes } from './completionOutcomeDisplayModel';

const footer = (tools = 1, commands = 1) => `${tools ? `\n\nUnsuccessful tool attempts in this turn: ${tools} (including argument checks and command outcomes). Details remain available in the execution steps.` : ''}${commands ? `\n\nUnsuccessful command attempts in this turn: ${commands}. Each command's exit status and output explain the result.` : ''}`;
const message = { id: 'final', type: 'text', position: 'left', turn_id: 'turn-a',
  conversation_id: 'conversation-a', content: { content: `Diagnostic exit 1.${footer()}` } } as IMessageText;
const receipt = { state: 'exited', exit_code: 1, signal: null, success: false,
  process_id: 'owned', output: { text: '0 pass, 1 fail' },
  cleanup: { reaped: true, errors: [], interrupt_attempted: false, terminate_attempted: false, force_kill_attempted: false } };
const tool = (output: unknown = receipt, turn = 'turn-a', conversation = 'conversation-a') => ({
  id: 'diagnostic', type: 'tool_call', turn_id: turn, conversation_id: conversation,
  content: { call_id: 'diagnostic', name: 'exec_command', status: 'error', output: JSON.stringify(output) },
}) as TMessage;

describe('completion outcome display', () => {
  test('uses proven native nonzero outcomes without changing source text or counts', () => {
    const original = message.content.content;
    const display = projectCompletionOutcomes(message, original, [tool()])!;
    expect(display).toEqual({ body: 'Diagnostic exit 1.', toolCount: 1, commandCount: 1,
      kind: 'native_nonzero', exitCodes: [1] });
    expect(message.content.content).toBe(original);
    expect(projectCompletionOutcomes(message, `Result${footer(0,1)}`, [tool()])?.kind).toBe('native_nonzero');
  });
  test('does not use another turn, conversation or incomplete history as an outcome', () => {
    for (const rows of [[], [tool(receipt,'turn-b')], [tool(receipt,'turn-a','conversation-b')]]) {
      expect(projectCompletionOutcomes(message, message.content.content, rows)?.kind).toBe('counts');
    }
    expect(projectCompletionOutcomes({ ...message, turn_id: undefined }, message.content.content, [tool()])?.kind).toBe('counts');
    expect(projectCompletionOutcomes(message, `Result${footer(2,1)}`, [tool()])?.kind).toBe('counts');
  });
  test('keeps signals, unproven cleanup, infrastructure and argument failures distinct', () => {
    for (const result of [
      { ...receipt, signal: 9 }, { ...receipt, state: 'lost' },
      { ...receipt, cleanup: { ...receipt.cleanup, reaped: false } },
      { ...receipt, cleanup: { ...receipt.cleanup, errors: ['unproven'] } },
    ]) expect(projectCompletionOutcomes(message, message.content.content, [tool(result)])?.kind).toBe('counts');
    const rejected = { ...tool(), id: 'rejected', content: { call_id: 'rejected', name: 'read_file', status: 'error',
      output: JSON.stringify({ status: 'not_executed', code: 'INVALID_TOOL_ARGUMENTS', tool: 'read_file', issues: [] }) } } as TMessage;
    expect(projectCompletionOutcomes(message, message.content.content, [tool(),rejected])?.kind).toBe('counts');
  });
  test('preserves user content, code examples, modified disclosures and out-of-range counts', () => {
    expect(projectCompletionOutcomes({ ...message, position: 'right' }, message.content.content, [tool()])).toBeUndefined();
    expect(projectCompletionOutcomes(message, `\x60\x60\x60text\nExample${footer()}`, [tool()])).toBeUndefined();
    expect(projectCompletionOutcomes(message, message.content.content.replace('Details remain', 'Details did remain'), [tool()])).toBeUndefined();
    expect(projectCompletionOutcomes(message, `Result${footer(4294967296,1)}`, [tool()])).toBeUndefined();
  });
});
