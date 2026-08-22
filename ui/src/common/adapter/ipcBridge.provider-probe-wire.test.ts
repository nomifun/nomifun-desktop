/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { parseProviderId } from '../types/ids';
import { mode } from './ipcBridge';

const PROVIDER_ID = parseProviderId('0190f5fe-7c00-7a00-8000-000000000002');
const realFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe('provider connection probe wire contract', () => {
  test('forwards the draft model used by protocol endpoint templates', async () => {
    let body: Record<string, unknown> | undefined;
    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      body = JSON.parse(String(init?.body));
      return new Response(
        JSON.stringify({
          success: true,
          data: {
            reachability: 'reachable',
            protocol: 'gemini.generate_text',
            task: 'chat',
            root_shape: 'origin_root',
            attempted_url:
              'https://gateway.example/v1beta/models/gemini-2.5-pro:streamGenerateContent',
            elapsed_ms: 12,
            candidates: [],
          },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    await mode.probeProviderConnection.invoke({
      provider_id: PROVIDER_ID,
      protocol: 'gemini.generate_text',
      task: 'chat',
      model: 'gemini-2.5-pro',
      probe_candidates: true,
    });

    expect(body).toEqual({
      protocol: 'gemini.generate_text',
      task: 'chat',
      model: 'gemini-2.5-pro',
      probe_candidates: true,
    });
  });
});
