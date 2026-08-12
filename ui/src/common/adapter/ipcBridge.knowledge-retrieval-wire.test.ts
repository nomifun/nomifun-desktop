/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { parseProviderId } from '../types/ids';
import { knowledge } from './ipcBridge';

const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000002';
const realFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe('knowledge retrieval wire contract', () => {
  test('PUT keeps embedding and rerank as separate tagged exact-model stages', async () => {
    let requestBody: unknown;
    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      requestBody = JSON.parse(String(init?.body));
      return new Response(JSON.stringify({ success: true, data: requestBody }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;

    const config = await knowledge.setRetrievalConfig.invoke({
      embedding: { mode: 'local' },
      rerank: {
        mode: 'remote',
        provider_id: parseProviderId(PROVIDER_ID),
        model: 'rerank-v3',
      },
    });

    expect(requestBody).toEqual({
      embedding: { mode: 'local' },
      rerank: { mode: 'remote', provider_id: PROVIDER_ID, model: 'rerank-v3' },
    });
    expect(config.rerank).toEqual({
      mode: 'remote',
      provider_id: parseProviderId(PROVIDER_ID),
      model: 'rerank-v3',
    });
  });
});
