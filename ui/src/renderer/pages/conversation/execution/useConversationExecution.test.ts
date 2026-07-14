import { describe, expect, test } from 'bun:test';
import { shouldDiscoverExecutionRelation } from './useConversationExecution';

describe('ConversationExecutionLink discovery', () => {
  test('discovers the first execution created while the conversation is already open', () => {
    expect(
      shouldDiscoverExecutionRelation(null, {
        execution_id: 'execution-new',
        change_kind: 'created',
      }),
    ).toBe(true);
  });

  test('discovers a replacement execution but leaves current execution updates to the detail subscription', () => {
    expect(
      shouldDiscoverExecutionRelation('execution-old', {
        execution_id: 'execution-new',
        change_kind: 'plan_changed',
      }),
    ).toBe(true);
    expect(
      shouldDiscoverExecutionRelation('execution-old', {
        execution_id: 'execution-old',
        change_kind: 'step_changed',
      }),
    ).toBe(false);
  });

  test('re-resolves the relation after the current execution is deleted', () => {
    expect(
      shouldDiscoverExecutionRelation('execution-old', {
        execution_id: 'execution-old',
        change_kind: 'deleted',
      }),
    ).toBe(true);
  });
});
