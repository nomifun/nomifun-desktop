import { describe, expect, test } from 'bun:test';
import { canSteerExecutionAttempt } from './executionStatusMeta';

describe('canSteerExecutionAttempt', () => {
  test('allows a running attempt while the aggregate is active', () => {
    expect(canSteerExecutionAttempt('running', 'running')).toBe(true);
    expect(canSteerExecutionAttempt('running', 'waiting_input')).toBe(true);
  });

  test('rejects attempts that are not running', () => {
    expect(canSteerExecutionAttempt('waiting_input', 'waiting_input')).toBe(false);
    expect(canSteerExecutionAttempt('queued', 'running')).toBe(false);
    expect(canSteerExecutionAttempt('completed', 'running')).toBe(false);
  });

  test('rejects inactive and terminal aggregate states', () => {
    expect(canSteerExecutionAttempt('running', 'planning')).toBe(false);
    expect(canSteerExecutionAttempt('running', 'paused')).toBe(false);
    expect(canSteerExecutionAttempt('running', 'completed')).toBe(false);
    expect(canSteerExecutionAttempt('running', 'completed_with_failures')).toBe(false);
    expect(canSteerExecutionAttempt('running', 'failed')).toBe(false);
    expect(canSteerExecutionAttempt('running', 'cancelled')).toBe(false);
  });
});
