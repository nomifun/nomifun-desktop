/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { parseKnowledgeBaseId } from '../types/ids';
import { agentPlatform, knowledge } from './ipcBridge';

const realFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe('mutable Knowledge binding wire contract', () => {
  test('rejects the retired conversation side channel before network I/O', async () => {
    let called = false;
    globalThis.fetch = (async () => {
      called = true;
      throw new Error('must not reach fetch');
    }) as typeof fetch;

    await expect(
      knowledge.getBinding.invoke({
        kind: 'conversation',
        target_id: '0190f5fe-7c00-7a00-8000-000000000201',
      } as never)
    ).rejects.toThrow('unsupported mutable Knowledge binding kind: conversation');
    expect(called).toBe(false);
  });

  test('uses the dedicated AgentSession command for live conversation changes', async () => {
    const sessionId = '0190f5fe-7c00-7a00-8000-000000000201';
    const kbId = parseKnowledgeBaseId('0190f5fe-7c00-7a00-8000-000000000202');
    let requestUrl = '';
    let requestBody: unknown;
    globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
      requestUrl = String(input);
      requestBody = JSON.parse(String(init?.body));
      return new Response(JSON.stringify({ success: true, data: requestBody }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;

    const saved = await agentPlatform.sessions.updateKnowledge.invoke({
      agent_session_id: sessionId,
      binding: {
        enabled: true,
        writeback: true,
        writeback_eagerness: 'auto',
        kb_ids: [kbId],
      },
    });

    expect(requestUrl.endsWith(`/api/agent-sessions/${sessionId}/knowledge`)).toBe(true);
    expect(requestBody).toEqual({
      enabled: true,
      writeback: true,
      writeback_eagerness: 'auto',
      kb_ids: [kbId],
    });
    expect(saved.kb_ids).toEqual([kbId]);
  });
});
