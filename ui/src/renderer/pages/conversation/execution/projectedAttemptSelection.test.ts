import { describe, expect, test } from 'bun:test';
import { parseExecutionAttemptId } from '@/common/types/ids';
import { executionId, makeAttempt, makeStep } from '../../../../../test/fixtures/conversationDelegation';
import { executionProjectionKey, selectProjectedAttempt } from './projectedAttemptSelection';

describe('historical execution selection', () => {
  const step = makeStep(1);
  const original = makeAttempt(step, { status: 'completed' });
  const retry = makeAttempt(step, {
    attempt_id: parseExecutionAttemptId('00000000-09c4-7000-8000-000000000031'), attempt_no: 2,
  });

  test('a completion record keeps its original attempt after a retry arrives', () => {
    expect(selectProjectedAttempt([original, retry], step.step_id, original.attempt_id)).toBe(original);
    expect(selectProjectedAttempt([original, retry], step.step_id)).toBe(retry);
  });

  test('a missing historical attempt never falls through to the current attempt', () => {
    expect(selectProjectedAttempt([retry], step.step_id, original.attempt_id)).toBeUndefined();
    expect(selectProjectedAttempt([original], makeStep(2).step_id, original.attempt_id)).toBeUndefined();
  });

  test('switching historical attempts resets task-local drafts even with the same step key', () => {
    const base = { executionId, step, projectionKey: step.step_id };
    expect(executionProjectionKey({ ...base, attemptId: original.attempt_id }))
      .not.toBe(executionProjectionKey({ ...base, attemptId: retry.attempt_id }));
    expect(executionProjectionKey(base)).not.toBe(executionProjectionKey({ ...base, attemptId: original.attempt_id }));
  });
});
