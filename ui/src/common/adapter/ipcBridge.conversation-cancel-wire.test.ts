/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { expect, test } from 'bun:test';
import { CANONICAL_UUID_V7, parseConversationId, parseMessageId } from '@/common/types/ids';
import { conversation } from './ipcBridge';

test('targeted cancellation sends the observed Turn root and ordinary stop retains its existing contract', async () => {
  const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000221');
  const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000222');
  const requests: Array<{ url: string; body: Record<string, unknown> }> = [];
  const realFetch = globalThis.fetch;
  try {
    globalThis.fetch = (async (url, init) => {
      requests.push({ url: String(url), body: JSON.parse(String(init?.body)) });
      return new Response(JSON.stringify({ success: true, data: null }), {
        status: 200, headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;
    await conversation.stop.invoke({ conversation_id: conversationId, expected_turn_id: turnId });
    await conversation.stop.invoke({ conversation_id: conversationId });
    expect(requests).toHaveLength(2);
    expect(requests[0].url.endsWith(`/api/agent-sessions/${conversationId}/turns/cancel`)).toBe(true);
    expect(requests[0].body.expected_turn_id).toBe(turnId);
    expect(requests[0].body.idempotency_key).toMatch(CANONICAL_UUID_V7);
    expect(requests[1].body.expected_turn_id).toBeUndefined();
    expect(requests[1].body.idempotency_key).toMatch(CANONICAL_UUID_V7);
    expect(requests[0].body.idempotency_key).not.toBe(requests[1].body.idempotency_key);
  } finally {
    globalThis.fetch = realFetch;
  }
});
