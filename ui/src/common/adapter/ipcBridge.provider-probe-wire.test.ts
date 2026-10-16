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

  for (const savedProvider of [false, true]) {
    test(`cancels an in-flight ${savedProvider ? 'saved provider' : 'draft connection'} probe`, async () => {
      let receivedSignal: AbortSignal | null | undefined;
      let requestBody: Record<string, unknown> | undefined;
      let requestUrl: string | undefined;
      globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
        receivedSignal = init?.signal;
        requestBody = JSON.parse(String(init?.body));
        requestUrl = String(input);
        return new Promise<Response>((_resolve, reject) => {
          receivedSignal?.addEventListener('abort', () => {
            reject(new DOMException('aborted', 'AbortError'));
          }, { once: true });
        });
      }) as typeof fetch;

      const controller = new AbortController();
      const probe = {
        protocol: 'openai.chat_text',
        task: 'chat' as const,
        model: 'test-model',
        probe_candidates: true,
      };
      const draft = {
        platform: 'custom',
        base_url: 'https://gateway.example/v1',
        auth_scheme: 'bearer',
        credentials: { api_keys: ['test-secret'] },
        ...probe,
      };
      const pending = savedProvider
        ? mode.probeProviderConnection.invoke({ provider_id: PROVIDER_ID, ...probe }, controller.signal)
        : mode.probeConnection.invoke(draft, controller.signal);

      expect(receivedSignal?.aborted).toBe(false);
      expect(requestBody).toEqual(savedProvider ? probe : draft);
      expect(new URL(requestUrl!, 'http://127.0.0.1').pathname).toBe(
        savedProvider ? `/api/providers/${PROVIDER_ID}/probe-connection` : '/api/providers/probe-connection'
      );
      expect(requestUrl).not.toContain('test-secret');
      controller.abort();

      await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
      expect(receivedSignal?.aborted).toBe(true);
    });
  }
});
