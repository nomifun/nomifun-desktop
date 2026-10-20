import { describe, expect, test } from 'bun:test';
import type { NormalizedToolCall } from '@/common/chat/normalizeToolCall';
import { buildToolReceiptRows, countBoundedSearchResults, getToolReceiptIcon } from './toolReceiptModel';

const tool = (overrides: Partial<NormalizedToolCall>): NormalizedToolCall => ({ key: 'call', name: 'start_process', status: 'completed', ...overrides });

describe('one tool receipt projection', () => {
  test('trusted catalogue actions choose file/command affordances; arbitrary MCP names remain generic', () => {
    const rows = buildToolReceiptRows([
      tool({ name: 'read_file', description: 'src/app.ts' }), tool({ name: 'write_file', description: 'src/app.ts' }),
      tool({ name: 'platform__truncated__0123456789abcdefabcd', capabilityId: 'workspace.process', actionId: 'workspace.process/start' }),
      tool({ name: 'mcp__server__read_file__abcdefghijklmnop', description: 'remote value' }),
    ]);
    expect(rows.map(row => row.action)).toEqual(['read_files', 'edit_files', 'run_commands', 'generic']);
    expect(rows[3].target).toContain('remote value');
    expect(getToolReceiptIcon(rows.slice(0, 1))).toBe('file');
    expect(getToolReceiptIcon(rows.slice(1, 2))).toBe('edit');
  });

  test('retains proven exit, timeout, preflight and cancellation outcomes without reading output prose', () => {
    const rows = buildToolReceiptRows([
      tool({ commandExitCode: 0 }), tool({ status: 'error', commandExitCode: 1, nonFatalFailure: true }),
      tool({ status: 'error', commandTimedOut: true, error: 'cleanup completed' }),
      tool({ status: 'canceled', notExecutedReason: 'invalid_arguments' }), tool({ status: 'canceled' }),
    ]);
    expect(rows.map(row => row.state)).toEqual(['completed', 'completed', 'failed', 'completed', 'canceled']);
    expect(rows[1].commandExitCode).toBe(1);
    expect(rows[2]).toMatchObject({ commandTimedOut: true, error: 'cleanup completed' });
    expect(rows[3].notExecutedReason).toBe('invalid_arguments');
  });

  test('the same row projection selects active and failed icons without building unused count summaries', () => {
    const rows = buildToolReceiptRows([tool({ name: 'write_file', status: 'error' }), tool({ name: 'read_file', status: 'running' })]);
    expect(getToolReceiptIcon(rows)).toBe('file');
    expect(getToolReceiptIcon(rows.slice(0, 1))).toBe('edit');
    expect(getToolReceiptIcon([])).toBeUndefined();
    expect(countBoundedSearchResults([tool({ boundedResult: 'search_context_withheld' }), tool({ status: 'error' })])).toBe(1);
  });
});
