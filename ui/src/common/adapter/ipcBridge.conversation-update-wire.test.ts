/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { parseConversationId } from '@/common/types/ids';
import type { TChatConversation } from '../config/storage';
import { conversation } from './ipcBridge';

const CONVERSATION_ID = '0190f5fe-7c00-7a00-8000-000000000301';
const source = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');
const realFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = realFetch;
});

function recordPatch(): Array<{ method: string; url: string; body?: string }> {
  const calls: Array<{ method: string; url: string; body?: string }> = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    calls.push({
      method: init?.method ?? 'GET',
      url: String(input),
      body: typeof init?.body === 'string' ? init.body : undefined,
    });
    return new Response(JSON.stringify({ success: true, data: true }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }) as typeof fetch;
  return calls;
}

describe('conversation update wire contract', () => {
  test('never sends merge_extra: UpdateConversationRequest is deny_unknown_fields', () => {
    // The server DTO (nomifun-api-types/src/conversation.rs) has no such field,
    // so shipping it turned every extra PATCH into a 400. `extra` is always
    // merged server-side; there is nothing to switch on.
    expect(source.includes('merge_extra')).toBe(false);
  });

  test('binds a workspace by sending exactly {extra:{workspace}}', async () => {
    const calls = recordPatch();

    await conversation.update.invoke({
      conversation_id: parseConversationId(CONVERSATION_ID),
      updates: { extra: { workspace: '/home/me/project' } as TChatConversation['extra'] },
    });

    expect(calls).toHaveLength(1);
    expect(calls[0].method).toBe('PATCH');
    expect(calls[0].url.endsWith(`/api/conversations/${CONVERSATION_ID}`)).toBe(true);
    expect(JSON.parse(String(calls[0].body))).toEqual({
      extra: { workspace: '/home/me/project' },
    });
  });

  test('still passes pinned/name through untouched', async () => {
    const calls = recordPatch();

    await conversation.update.invoke({
      conversation_id: parseConversationId(CONVERSATION_ID),
      updates: { name: 'renamed', pinned: true } as Partial<TChatConversation> & { pinned?: boolean },
    });

    expect(JSON.parse(String(calls[0].body))).toEqual({ name: 'renamed', pinned: true });
  });
});
