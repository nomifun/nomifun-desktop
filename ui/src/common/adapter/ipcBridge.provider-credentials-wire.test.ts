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

const initialModel = {
  model: 'gpt-4o',
  enabled: true,
  capabilities: [
    {
      task: 'chat' as const,
      protocol: 'openai.chat_text',
      connection_role: 'default',
      allow_cross_origin_credentials: false,
      provider_params: {},
    },
  ],
};

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe('provider credentials wire contract', () => {
  test('create sends only typed credentials and maps a secret-free response', async () => {
    let requestBody: Record<string, unknown> | undefined;
    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      requestBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
      return new Response(
        JSON.stringify({
          success: true,
          data: {
            provider_id: PROVIDER_ID,
            platform: 'openai',
            name: 'OpenAI',
            base_url: 'https://api.openai.com/v1',
            auth_scheme: 'bearer',
            has_credentials: true,
            models: [],
            enabled: true,
            sort_order: 0,
            created_at: 1,
            updated_at: 1,
          },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    const provider = await mode.createProvider.invoke({
      id: parseProviderId(PROVIDER_ID),
      platform: 'openai',
      name: 'OpenAI',
      base_url: 'https://api.openai.com/v1',
      auth_scheme: 'bearer',
      credentials: { api_keys: ['sk-test'] },
      initial_model: initialModel,
    });

    expect(requestBody?.credentials).toEqual({ api_keys: ['sk-test'] });
    expect(Object.prototype.hasOwnProperty.call(requestBody, 'api_key')).toBe(false);
    expect(provider.has_credentials).toBe(true);
    expect(Object.keys(provider).includes('credentials')).toBe(false);
  });

  test('reads saved provider API keys in plaintext for the edit form', async () => {
    let requestUrl = '';
    globalThis.fetch = (async (input: string | URL | Request) => {
      requestUrl = String(input);
      return new Response(
        JSON.stringify({
          success: true,
          data: ['sk-first', 'sk-second'],
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    const apiKeys = await mode.getProviderApiKeys.invoke({
      provider_id: parseProviderId(PROVIDER_ID),
    });

    expect(requestUrl.endsWith(`/api/providers/${PROVIDER_ID}/api-keys`)).toBe(true);
    expect(apiKeys).toEqual(['sk-first', 'sk-second']);
  });

  test('anonymous Bedrock discovery keeps secrets in credentials, not bedrock_config', async () => {
    let requestBody: Record<string, unknown> | undefined;
    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      requestBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
      return new Response(JSON.stringify({ success: true, data: { models: [] } }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;

    await mode.fetchModelList.invoke({
      platform: 'bedrock',
      base_url: '',
      auth_scheme: 'bedrock',
      credentials: {
        access_key_id: 'AKIA',
        secret_access_key: 'secret',
        session_token: 'session',
      },
      bedrock_config: { auth_method: 'accessKey', region: 'us-east-1' },
      try_fix: true,
    });

    expect(requestBody?.credentials).toEqual({
      access_key_id: 'AKIA',
      secret_access_key: 'secret',
      session_token: 'session',
    });
    expect(requestBody?.bedrock_config).toEqual({
      auth_method: 'accessKey',
      region: 'us-east-1',
    });
    expect(Object.prototype.hasOwnProperty.call(requestBody, 'api_key')).toBe(false);
  });
});
