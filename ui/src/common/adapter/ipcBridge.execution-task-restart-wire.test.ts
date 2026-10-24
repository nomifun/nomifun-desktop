import { expect, test } from 'bun:test';
import { executionId, makeAttempt, makeStep } from '../../../test/fixtures/conversationDelegation';
import { agentExecution } from './ipcBridge';

test('task restart sends its immutable identity and receives only the original command acceptance', async () => {
  const step = makeStep(0), attempt = makeAttempt(step);
  const requests: Array<{ url: string; method?: string; body: unknown }> = [];
  const previous = globalThis.fetch;
  try {
    const receipt = { execution_id: executionId, step_ids: [step.step_id], sequence: 42,
      operation_id: 'restart-step:original-command', replayed: false };
    globalThis.fetch = (async (url, init) => {
      requests.push({ url: String(url), method: init?.method, body: JSON.parse(String(init?.body)) });
      return Response.json({ success: true, data: receipt });
    }) as typeof fetch;
    const updates = { idempotency_key: 'same-ui-intent', expected_execution_version: 31,
      expected_step_version: 7, expected_attempt_id: attempt.attempt_id };
    expect(await agentExecution.restart.invoke({ execution_id: executionId, step_id: step.step_id, updates })).toEqual(receipt);
    expect(requests).toHaveLength(1);
    expect(requests[0]).toMatchObject({ method: 'POST', body: updates });
    expect(requests[0].url.endsWith(`/api/agent-executions/${executionId}/steps/${step.step_id}/restart`)).toBe(true);
    expect('steps' in receipt).toBe(false);
  } finally { globalThis.fetch = previous; }
});

test('restart preserves a never-started target and rejects a malformed acceptance receipt', async () => {
  const previous = globalThis.fetch, step = makeStep(0);
  try {
    let malformed = false;
    globalThis.fetch = (async (_url, init) => {
      expect(JSON.parse(String(init?.body)).expected_attempt_id).toBeNull();
      return Response.json({ success: true, data: { execution_id: executionId, step_ids: [step.step_id],
        sequence: malformed ? -1 : 42, operation_id: 'exact-restart', replayed: true } });
    }) as typeof fetch;
    const request = { execution_id: executionId, step_id: step.step_id,
      updates: { idempotency_key: 'never-started', expected_execution_version: 1, expected_step_version: 1, expected_attempt_id: null } };
    expect((await agentExecution.restart.invoke(request)).replayed).toBe(true);
    malformed = true;
    await expect(agentExecution.restart.invoke(request)).rejects.toThrow('exact acceptance receipt');
  } finally { globalThis.fetch = previous; }
});
