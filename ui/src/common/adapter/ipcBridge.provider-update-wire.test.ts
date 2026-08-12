/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { parseProviderId } from '../types/ids';
import { mode } from './ipcBridge';

const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000002';
const realFetch = globalThis.fetch;

const providerResponse = {
  provider_id: PROVIDER_ID,
  platform: 'openai',
  name: 'StepFun edited',
  base_url: 'https://api.stepfun.com/v1',
  auth_scheme: 'bearer',
  has_credentials: true,
  models: [],
  enabled: true,
  sort_order: 0,
  created_at: 1,
  updated_at: 2,
};

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe('provider update wire contract', () => {
  test('allow-lists backend fields when a renderer/form-shaped object is passed', async () => {
    const calls: Array<{ method: string; url: string; body?: string }> = [];
    globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
      calls.push({
        method: init?.method ?? 'GET',
        url: String(input),
        body: typeof init?.body === 'string' ? init.body : undefined,
      });
      return new Response(JSON.stringify({ success: true, data: providerResponse }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;

    const dirtyRendererInput = {
      provider_id: parseProviderId(PROVIDER_ID),
      platform: 'openai',
      name: 'StepFun edited',
      base_url: 'https://api.stepfun.com/v1',
      auth_scheme: 'bearer',
      credentials: { api_keys: ['sk-test'] },
      models: [],
      enabled: true,
      sort_order: 0,
      // Runtime-only fields previously leaked from EditModeModal and caused
      // UpdateProviderRequest's deny_unknown_fields deserializer to return 400.
      // `platform` is also immutable after creation and must be stripped.
      model: 'step-3.7-flash',
      bedrockAuthMethod: 'accessKey',
      bedrockRegion: 'us-east-1',
      bedrockAccessKeyId: '',
      bedrockSecretAccessKey: '',
      bedrockProfile: '',
    } as unknown as Parameters<typeof mode.updateProvider.invoke>[0];

    await mode.updateProvider.invoke(dirtyRendererInput);

    expect(calls).toHaveLength(1);
    expect(calls[0].method).toBe('PUT');
    expect(calls[0].url.endsWith(`/api/providers/${PROVIDER_ID}`)).toBe(true);
    expect(JSON.parse(String(calls[0].body))).toEqual({
      name: 'StepFun edited',
      base_url: 'https://api.stepfun.com/v1',
      auth_scheme: 'bearer',
      credentials: { api_keys: ['sk-test'] },
      enabled: true,
      sort_order: 0,
    });
  });
});
