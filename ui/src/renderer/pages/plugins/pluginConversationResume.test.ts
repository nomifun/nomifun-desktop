import { afterEach, expect, test } from 'bun:test';
import { parseConversationId } from '@/common/types/ids';
import { continuePluginConversation, replyToPluginConversation } from './pluginConversationResume';
import type { NativeAgentExecution } from '@/common/types/agentPlatform';

const realFetch = globalThis.fetch;
const id = '0190f5fe-7c00-7a00-8000-000000000202';
const message = {
  conversation_id: parseConversationId(id), input: 'Approved; continue this existing plugin.',
  idempotency_key: 'plugin-approval:fixture', plugin_delivery: { draft_id: '0190f5fe-7c00-7a00-8000-000000000204' },
};
const paused = {
  state: 'paused', operation_id: 'original-turn', checkpoint_revision: 7,
  checkpoint_digest: 'a'.repeat(64), checkpoint_retained: true,
  pause: { revision: 2, reason: 'EXECUTION_USER_REQUESTED', cleanup_proven: true },
};
type Call = { path: string; method: string; body: unknown };
const calls: Call[] = [];

function fixture(responses: Array<{ data: unknown; status?: number }>) {
  globalThis.fetch = (async (input, init) => {
    calls.push({ path: new URL(String(input), 'http://127.0.0.1').pathname,
      method: init?.method ?? 'GET', body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined });
    const response = responses.shift();
    if (!response) throw new Error('Unexpected request');
    return new Response(JSON.stringify(response.status === 409
      ? { success: false, code: 'CONFLICT', error: 'Already running' }
      : { success: true, data: response.data }), {
      status: response.status ?? 200, headers: { 'Content-Type': 'application/json' },
    });
  }) as typeof fetch;
}

afterEach(() => { globalThis.fetch = realFetch; calls.length = 0; });

test('approval resumes the exact paused turn with its input, without starting or steering another turn', async () => {
  fixture([{ data: paused }, { data: { duplicate: false } }]);
  await continuePluginConversation(message);
  expect(calls).toEqual([
    { method: 'GET', path: `/api/agent-sessions/${id}/execution`, body: undefined },
    { method: 'POST', path: `/api/agent-sessions/${id}/plugin-continuation`, body: {
      request: {
        operation_id: 'original-turn', idempotency_key: message.idempotency_key,
        expected_pause_revision: 2, expected_checkpoint_revision: 7,
        expected_checkpoint_digest: 'a'.repeat(64), budget: {},
      },
      input: { content: 'Approved; continue this existing plugin.' },
    } },
  ]);
});

test('approval never fabricates resource cleanup or authorizes an unrelated stalled pause', async () => {
  for (const execution of [
    { ...paused, pause: { ...paused.pause, cleanup_proven: false } },
    { ...paused, pause: { ...paused.pause, reason: 'EXECUTION_NO_PROGRESS' } },
  ]) {
    fixture([{ data: execution }]);
    await expect(continuePluginConversation(message)).rejects.toThrow('PLUGIN_CONTINUATION_NOT_AVAILABLE');
  }
  expect(calls.every(call => call.method === 'GET')).toBe(true);
});

test('a pause committed during send uses its fresh checkpoint instead of the steering path', async () => {
  fixture([{ data: { ...paused, state: 'running', pause: null } },
    { data: null, status: 409 }, { data: paused }, { data: { duplicate: false } }]);
  await continuePluginConversation(message);
  expect(calls.map(call => call.path)).toEqual([
    `/api/agent-sessions/${id}/execution`, `/api/agent-sessions/${id}/turns`,
    `/api/agent-sessions/${id}/execution`, `/api/agent-sessions/${id}/plugin-continuation`,
  ]);
});

test('supplemental input and exact resume are submitted together without a new turn', async () => {
  fixture([{ data: { duplicate: false } }]);
  const execution = { ...paused, pause: { ...paused.pause, reason: 'PLUGIN_DELIVERY_REQUIRED' } } as NativeAgentExecution;
  await replyToPluginConversation({ ...message, input: 'Use uppercase text' }, execution);
  expect(calls).toEqual([{ method: 'POST', path: `/api/agent-sessions/${id}/plugin-continuation`, body: {
    request: { operation_id: 'original-turn', idempotency_key: message.idempotency_key,
      expected_pause_revision: 2, expected_checkpoint_revision: 7,
      expected_checkpoint_digest: 'a'.repeat(64), budget: {} },
    input: { content: 'Use uppercase text' },
  } }]);
});

test('reply cannot resume an unrelated pause or claim unproven cleanup', async () => {
  for (const execution of [paused,
    { ...paused, pause: { ...paused.pause, reason: 'PLUGIN_DELIVERY_REQUIRED', cleanup_proven: false } },
  ]) {
    await expect(replyToPluginConversation(message, execution as NativeAgentExecution)).rejects.toThrow('PLUGIN_CONTINUATION_NOT_AVAILABLE');
  }
  expect(calls).toHaveLength(0);
});
