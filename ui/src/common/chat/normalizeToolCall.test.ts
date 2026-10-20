import { describe, expect, it } from 'bun:test';
import type { IMessageToolCall } from './chatLib';
import { normalizeToolCallContent } from './chatLib';
import { normalizeToolCall, normalizeToolGroup } from './normalizeToolCall';

const normalize = (content: Record<string, unknown>) => normalizeToolCall({
  type: 'tool_call', content: normalizeToolCallContent({ call_id: 'call', name: 'start_process', ...content }),
} as IMessageToolCall);

describe('public tool outcomes', () => {
  it('retains native command labels and exact host exit outcomes without tool bodies', () => {
    const result = normalize({ description: 'bun test', status: 'error', command_exit_code: 1, non_fatal_failure: true });
    expect(result).toMatchObject({ description: 'bun test', status: 'error', commandExitCode: 1, nonFatalFailure: true });
    expect(result).not.toHaveProperty('input');
    expect(result).not.toHaveProperty('output');
  });

  it('keeps launch refusals and clean timeouts failed while preserving their public diagnostic', () => {
    for (const [field, normalized] of [['command_not_started', 'commandNotStarted'], ['command_timed_out', 'commandTimedOut']]) {
      const result = normalize({ status: 'error', [field]: true, error: 'The requested command did not complete.' });
      expect(result?.status).toBe('error');
      expect(result).toHaveProperty(normalized, true);
      expect(result?.error).toBe('The requested command did not complete.');
      expect(result?.nonFatalFailure).toBeUndefined();
    }
  });

  it('shows a host-rejected process reference as unexecuted', () => {
    expect(normalize({ status: 'error', process_reference_invalid: true })).toMatchObject({
      status: 'canceled', notExecutedReason: 'process_reference',
    });
  });

  it('keeps bounded search status separate from execution failure', () => {
    expect(normalize({ name: 'search_files', status: 'error', bounded_result: 'search_context_withheld', non_fatal_failure: true }))
      .toMatchObject({ status: 'error', boundedResult: 'search_context_withheld', nonFatalFailure: true });
  });

  it('uses typed local holds for argument refusals, skipped calls and preflight outcomes', () => {
    expect(normalize({ status: 'error', runtime_hold: 'arguments_rejected' })).toMatchObject({ status: 'canceled', notExecutedReason: 'invalid_arguments' });
    expect(normalize({ status: 'error', runtime_hold: 'prior_call_failed' })).toMatchObject({ status: 'canceled', skipped: true });
    expect(normalize({ status: 'error', runtime_hold: 'input_superseded' })).toMatchObject({ status: 'canceled', notExecutedReason: 'runtime_preflight' });
    expect(normalize({ status: 'error', runtime_hold: 'runtime_gate' })).toMatchObject({ status: 'canceled', notExecutedReason: 'runtime_preflight' });
  });

  it('never infers status from raw results or preserves a rejected write proposal', () => {
    const raw = { status: 'error', args: { path: 'app.ts', content: 'PRIVATE FILE BODY' },
      input: { patch: 'PRIVATE PATCH' }, output: 'Exit code: 1\nPRIVATE STDOUT',
      command_exit_code: -1, command_not_started: 'true', non_fatal_failure: 'true', bounded_result: 'unknown' };
    const content = normalizeToolCallContent({ call_id: 'call', name: 'start_process', ...raw });
    expect(content).not.toHaveProperty('args');
    expect(content).not.toHaveProperty('input');
    expect(content).not.toHaveProperty('output');
    expect(JSON.stringify(content)).not.toContain('PRIVATE');
    const result = normalize(raw);
    expect(result?.status).toBe('error');
    expect(result?.commandExitCode).toBeUndefined();
    expect(result?.commandNotStarted).toBeUndefined();
    expect(result?.nonFatalFailure).toBeUndefined();
    expect(result?.boundedResult).toBeUndefined();
  });

  it('accepts an exit code of zero and ignores outcomes on a running frame', () => {
    expect(normalize({ status: 'completed', command_exit_code: 0 })?.commandExitCode).toBe(0);
    expect(normalize({ status: 'running', command_exit_code: 0, command_timed_out: true, error: 'stale' }))
      .toMatchObject({ status: 'running' });
    expect(normalize({ status: 'running', command_exit_code: 0 })?.commandExitCode).toBeUndefined();
    expect(normalize({ status: 'completed', error: 'stale' })?.error).toBeUndefined();
  });

  it('preserves actual cancellation and rejects missing call identities', () => {
    expect(normalize({ status: 'canceled' })?.status).toBe('canceled');
    expect(normalize({ call_id: '' })).toBeUndefined();
  });

  it('does not interpret ordinary remote error prose as local preflight evidence', () => {
    const result = normalize({ name: 'mcp__server__write_file__abcdefghijklmnop', status: 'error',
      error: 'Operations not executed: remote service error' });
    expect(result?.status).toBe('error');
    expect(result?.notExecutedReason).toBeUndefined();
    expect(result?.error).toContain('remote service error');
  });

  it('retains group labels and statuses without adding a second input carrier', () => {
    const result = normalizeToolGroup({ type: 'tool_group', content: [
      { call_id: 'one', name: 'read_file', description: 'src/app.ts', status: 'Success' },
      { call_id: 'two', name: 'write_file', description: 'src/next.ts', status: 'Executing' },
    ] } as any);
    expect(result.map(row => [row.description, row.status])).toEqual([['src/app.ts', 'completed'], ['src/next.ts', 'running']]);
    expect(result[0]).not.toHaveProperty('input');
    expect(normalizeToolGroup({ type: 'tool_group', content: {} } as any)).toEqual([]);
  });
});
