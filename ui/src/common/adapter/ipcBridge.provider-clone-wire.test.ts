/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Wire contract for `POST /api/providers/{id}/clone`.
 *
 * The endpoint accepts an OPTIONAL `{ name }` body (contract fixed with the
 * backend lane): present → the copy takes that display name (the frontend
 * sends a localized "<source> <providerCopySuffix>"); absent → no body is sent
 * and the backend picks its default copy name.
 */

import { describe, expect, test } from 'bun:test';
import { parseProviderId } from '../types/ids';
import { mode } from './ipcBridge';

const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000002';
const CLONE_ID = '0190f5fe-7c00-7a00-8000-000000000003';

const realFetch = globalThis.fetch;

const cloneResponse = (name: string) => ({
  provider_id: CLONE_ID,
  platform: 'openai',
  name,
  base_url: 'https://api.openai.com',
  api_key: 'sk-test',
  models: ['gpt-4o'],
  enabled: true,
  is_full_url: false,
  sort_order: 1,
  created_at: 1,
  updated_at: 1,
});

describe('provider clone wire contract', () => {
  test('sends the optional localized name as the clone body', async () => {
    const calls: Array<{ method: string; url: string; body?: string }> = [];
    globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
      calls.push({
        method: init?.method ?? 'GET',
        url: String(input),
        body: typeof init?.body === 'string' ? init.body : undefined,
      });
      return new Response(JSON.stringify({ success: true, data: cloneResponse('OpenAI 副本') }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;

    try {
      const copied = await mode.cloneProvider.invoke({
        provider_id: parseProviderId(PROVIDER_ID),
        name: 'OpenAI 副本',
      });

      expect(calls.length).toBe(1);
      expect(calls[0].method).toBe('POST');
      expect(calls[0].url.endsWith(`/api/providers/${PROVIDER_ID}/clone`)).toBe(true);
      expect(JSON.parse(String(calls[0].body))).toEqual({ name: 'OpenAI 副本' });
      // Only `name` travels — never the provider_id locator or a whole record.
      expect(Object.keys(JSON.parse(String(calls[0].body)))).toEqual(['name']);
      expect(copied.id).toBe(parseProviderId(CLONE_ID));
      expect(copied.name).toBe('OpenAI 副本');
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('omits the body entirely when no name override is given', async () => {
    let capturedBody: unknown = 'unset';
    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      capturedBody = init?.body;
      return new Response(JSON.stringify({ success: true, data: cloneResponse('OpenAI Copy') }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;

    try {
      await mode.cloneProvider.invoke({ provider_id: parseProviderId(PROVIDER_ID) });
      expect(capturedBody).toBeUndefined();
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
