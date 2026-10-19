import { afterEach, expect, test } from 'bun:test';
import { agentPlatform, companion } from './ipcBridge';
import { parseCompanionId, parseConversationId } from '../types/ids';

const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000051');
const conversationId = parseConversationId('019b0000-0000-7000-8000-000000000052');
const originalFetch = globalThis.fetch;
afterEach(() => { globalThis.fetch = originalFetch; });

test('Session list, explicit new conversation and desktop routing have distinct wire commands', async () => {
  const calls: Array<{ url: string; method: string; body: unknown }> = [];
  const session = { conversation_id: conversationId, companion_id: companionId, role: 'companion', robot_id: null,
    title: 'New conversation', created_at: 1, updated_at: 1, archived: false };
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? 'GET';
    calls.push({ url, method, body: typeof init?.body === 'string' ? JSON.parse(init.body) : null });
    const data = method === 'GET' ? { items: [session], total: 1, has_more: false }
      : url.endsWith('/active') ? { conversation_id: conversationId } : session;
    return new Response(JSON.stringify({ success: true, data }), { headers: { 'Content-Type': 'application/json' } });
  }) as typeof fetch;
  expect((await companion.listCompanionSessions.invoke({ companion_id: companionId })).items[0].role).toBe('companion');
  await companion.createCompanionSession.invoke({ companion_id: companionId, creation_key: 'stable-retry-key' });
  await companion.continueInCompanionWindow.invoke({ companion_id: companionId, conversation_id: conversationId });
  expect(calls.map(call => call.method)).toEqual(['GET', 'POST', 'POST']);
  expect(calls[0].url).toContain(`/companions/${companionId}/companion/threads`);
  expect(calls[1].body).toEqual({ new_session: true, creation_key: 'stable-retry-key' });
  expect(calls[2].url).toContain(`/companions/${companionId}/companion/active`);
  expect(calls[2].body).toEqual({ conversation_id: conversationId });
});

test('device Session Agent options use the unified companion target', async () => {
  let requestUrl = '';
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    requestUrl = String(input);
    return new Response(JSON.stringify({ success: true, data: { selection: null, needs_model: true, options: [] } }),
      { headers: { 'Content-Type': 'application/json' } });
  }) as typeof fetch;
  await agentPlatform.productBindingOptions.invoke({ target_kind: 'companion', target_id: companionId, conversation_id: conversationId });
  expect(requestUrl).toContain(`/product-agent-bindings/companion/${companionId}?conversation_id=${conversationId}`);
});
