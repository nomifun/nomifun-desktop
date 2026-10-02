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
const rejected = (name = 'report_completion') => ({
  id: 'rejected', type: 'tool_call', turn_id: 'turn-a', conversation_id: 'conversation-a',
  content: { call_id: 'rejected', name, status: 'error', output: JSON.stringify({
    status: 'not_executed', code: 'INVALID_TOOL_ARGUMENTS', tool: name,
    issues: [{ instance_path: '/criteria', schema_path: '/properties/criteria', error: 'invalid reference' }],
    message: 'No call in this batch was executed.',
  }) },
}) as TMessage;

describe('completion outcome display', () => {
  test('distinguishes a proven same-turn timeout without claiming success or changing counts', () => {
    const timeout = { ...receipt, state: 'timed_out', exit_code: undefined,
      cleanup: { ...receipt.cleanup, terminate_attempted: true } };
    const original = `The command reached its deadline.${footer()}`;
    expect(projectCompletionOutcomes(message, original, [tool(timeout)])).toEqual({
      body: 'The command reached its deadline.', toolCount: 1, commandCount: 1, kind: 'native_timeout', exitCodes: [],
    });
    for (const rows of [[], [tool(timeout, 'turn-b')], [tool(timeout, 'turn-a', 'conversation-b')],
      [tool({ ...timeout, cleanup: { ...timeout.cleanup, errors: ['failed to reap'] } })]]) {
      expect(projectCompletionOutcomes(message, original, rows)?.kind).toBe('counts');
    }
    expect(projectCompletionOutcomes(message, `Result${footer(2,1)}`, [tool(timeout)])?.kind).toBe('counts');
    expect(projectCompletionOutcomes(message, `Result${footer(2,1)}`, [tool(timeout), rejected()])?.kind).toBe('counts');
    expect(original).toBe(`The command reached its deadline.${footer()}`);
  });
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
    expect(projectCompletionOutcomes(message, `Result${footer(2,1)}`, [tool()])).toEqual({
      body: 'Result', toolCount: 2, commandCount: 1, kind: 'native_nonzero_and_unclassified',
      exitCodes: [1], otherCount: 1,
    });
  });
  test('separates a settled nonzero command from a rejected completion account', () => {
    const original = `Result${footer(2,1)}`;
    const display = projectCompletionOutcomes(message, original, [tool(), rejected()]);
    expect(display).toEqual({ body: 'Result', toolCount: 2, commandCount: 1,
      kind: 'native_nonzero_and_arguments', exitCodes: [1], argumentCount: 1 });
    expect(projectCompletionOutcomes(message, `Result${footer(1,0)}`, [rejected()])).toEqual({
      body: 'Result', toolCount: 1, commandCount: 0, kind: 'arguments_not_executed', exitCodes: [], argumentCount: 1,
    });
    expect(original).toBe(`Result${footer(2,1)}`);
  });
  test('never infers a complete breakdown from missing, unrelated or other fault results', () => {
    const wrongTurn = { ...rejected(), turn_id: 'turn-b' } as TMessage;
    const provider = rejected('mcp__remote__report_completion');
    for (const rows of [[tool(), provider],
      [tool({ ...receipt, signal: 9 }), rejected()],
      [tool({ ...receipt, cleanup: { ...receipt.cleanup, reaped: false } }), rejected()]]) {
      expect(projectCompletionOutcomes(message, `Result${footer(2,1)}`, rows)?.kind).toBe('counts');
    }
    for (const rows of [[tool()], [tool(), wrongTurn], [tool(), rejected()]]) {
      const display = projectCompletionOutcomes(message, `Result${footer(3,1)}`, rows);
      expect(display?.kind).toBe('native_nonzero_and_unclassified');
      expect(display?.argumentCount).toBeUndefined();
      expect(display?.otherCount).toBe(2);
    }
    expect(projectCompletionOutcomes(message, `Result${footer(2,2)}`, [tool(), rejected()])?.kind).toBe('counts');
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
