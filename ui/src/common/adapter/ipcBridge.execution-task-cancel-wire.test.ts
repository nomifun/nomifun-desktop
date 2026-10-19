import { expect, test } from 'bun:test';
import { executionId, leadConversationId, makeDetail, makeStep } from '../../../test/fixtures/conversationDelegation';
import { agentExecution } from './ipcBridge';

test('task cancellation sends its captured task, versions and explicit delegation scope', async () => {
  const step = makeStep(0);
  const requests: Array<{ url: string; method?: string; body: Record<string, unknown> }> = [];
  const originalFetch = globalThis.fetch;
  try {
    globalThis.fetch = (async (url, init) => {
      requests.push({ url: String(url), method: init?.method, body: JSON.parse(String(init?.body)) });
      return new Response(JSON.stringify({ success: true, data: makeDetail({ steps: [{ ...step, status: 'cancelled' }] }) }), {
        status: 200, headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;
    const updates = { expected_execution_version: 41, expected_step_version: 7, scope: 'task_and_delegated' as const };
    const result = await agentExecution.cancelStep.invoke({ execution_id: executionId, step_id: step.step_id, updates });
    expect(requests).toHaveLength(1);
    expect(requests[0].method).toBe('POST');
    expect(requests[0].url.endsWith(`/api/agent-executions/${executionId}/steps/${step.step_id}/cancel`)).toBe(true);
    expect(requests[0].body).toEqual(updates);
    expect(result.execution.execution_id).toBe(executionId);
    expect(result.steps[0].status).toBe('cancelled');
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('Execution detail preserves AutoWork source identity even before its root Step exists', async () => {
  const original = globalThis.fetch;
  const automation = { root_step_id: null, lead_conversation_id: leadConversationId,
    requirement_id: 'original-requirement', claim_generation: 7 };
  try {
    globalThis.fetch = (async () => Response.json({ success: true, data: makeDetail({ automation_control: automation }) })) as typeof fetch;
    const detail = await agentExecution.get.invoke({ execution_id: executionId });
    expect(detail?.automation_control).toEqual(automation);
    expect(detail?.steps).toHaveLength(0);
  } finally { globalThis.fetch = original; }
});
