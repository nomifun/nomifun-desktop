import { expect, test } from 'bun:test';
import { parseConversationId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import { conversation } from './ipcBridge';

test('context clear sends the same explicit action key on retry and never accepts a client-authored reason', async () => {
  const conversationId = parseConversationId(uuidv7());
  const key = uuidv7();
  const requests: Array<{ url: string; body: unknown }> = [];
  const realFetch = globalThis.fetch;
  try {
    globalThis.fetch = (async (url, init) => {
      requests.push({ url: String(url), body: JSON.parse(String(init?.body)) });
      return new Response(JSON.stringify({ success: true, data: null }), {
        status: 200, headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;
    const command = { conversation_id: conversationId, idempotency_key: key, reason: 'forged' };
    await conversation.clearContext.invoke(command);
    await conversation.clearContext.invoke(command);
    expect(requests).toEqual([1, 2].map(() => ({
      url: requests[0].url, body: { idempotency_key: key },
    })));
    expect(requests[0].url.endsWith(`/api/agent-sessions/${conversationId}/clear-context`)).toBe(true);
    await expect(conversation.clearContext.invoke({ conversation_id: conversationId, idempotency_key: '' })).rejects.toThrow();
    expect(requests).toHaveLength(2);
  } finally { globalThis.fetch = realFetch; }
});
