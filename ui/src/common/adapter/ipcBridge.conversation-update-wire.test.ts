/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { parseConversationId } from '@/common/types/ids';
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
  test('never exposes a legacy merge_extra mutation switch', () => {
    // The canonical metadata DTO rejects both `merge_extra` and `extra`.
    expect(source.includes('merge_extra')).toBe(false);
  });

  test('rejects mutable workspace binding after Session creation', async () => {
    const calls = recordPatch();

    await expect(conversation.update.invoke({
        conversation_id: parseConversationId(CONVERSATION_ID),
        updates: { extra: { workspace: '/home/me/project' } } as never,
      })).rejects.toThrow('AgentSession binding is immutable');
    expect(calls).toHaveLength(0);
  });

  test('still passes pinned/name through untouched', async () => {
    const calls = recordPatch();

    await conversation.update.invoke({
      conversation_id: parseConversationId(CONVERSATION_ID),
      updates: { name: 'renamed', pinned: true },
    });

    expect(JSON.parse(String(calls[0].body))).toEqual({ name: 'renamed', pinned: true });
    expect(calls[0].url.endsWith(`/api/agent-sessions/${CONVERSATION_ID}`)).toBe(true);
  });
});
