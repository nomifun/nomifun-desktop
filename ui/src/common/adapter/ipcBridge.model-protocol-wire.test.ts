/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { modelProtocol } from './ipcBridge';

const realFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe('model protocol manifest wire contract', () => {
  test('sends the selected Custom model as an encoded query hint', async () => {
    let requestedUrl = '';
    globalThis.fetch = (async (input: string | URL | Request) => {
      requestedUrl = String(input);
      return new Response(
        JSON.stringify({
          success: true,
          data: {
            tasks: ['chat'],
            preset: 'custom',
            platform: 'custom',
            requested_task: 'chat',
            platform_default_base_url: null,
            requires_user_input: true,
            default_auth_scheme: 'bearer',
            auth_schemes: [],
            recommendation: null,
            protocols: [],
          },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    await modelProtocol.list.invoke({
      preset: 'custom',
      task: 'chat',
      base_url: 'https://gateway.example/api/v1',
      model: 'vendor/model latest:free',
    });

    const query = new URL(requestedUrl).searchParams;
    expect(query.get('preset')).toBe('custom');
    expect(query.get('task')).toBe('chat');
    expect(query.get('base_url')).toBe('https://gateway.example/api/v1');
    expect(query.get('model')).toBe('vendor/model latest:free');
  });

  test('omits the optional model hint when no model is selected', async () => {
    let requestedUrl = '';
    globalThis.fetch = (async (input: string | URL | Request) => {
      requestedUrl = String(input);
      return new Response(
        JSON.stringify({
          success: true,
          data: {
            tasks: ['chat'],
            preset: 'custom',
            platform: 'custom',
            requested_task: 'chat',
            platform_default_base_url: null,
            requires_user_input: true,
            default_auth_scheme: 'bearer',
            auth_schemes: [],
            recommendation: null,
            protocols: [],
          },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    await modelProtocol.list.invoke({ preset: 'custom', task: 'chat' });

    expect(new URL(requestedUrl).searchParams.has('model')).toBe(false);
  });
});
