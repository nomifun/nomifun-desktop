/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { configService } from '@/common/config/configService';
import type { ProviderId } from '@/common/types/ids';
import type { TextToSpeechConfig } from '@/common/types/provider/speech';
import {
  getTextToSpeechConfig,
  saveTextToSpeechConfig,
  TEXT_TO_SPEECH_CONFIG_KEY,
} from './textToSpeechConfig';

const src = readFileSync(new URL('./textToSpeechConfig.ts', import.meta.url), 'utf8');
const panel = readFileSync(
  new URL('../pages/modelHub/TextToSpeechContent.tsx', import.meta.url),
  'utf8'
);
const realFetch = globalThis.fetch;

const persistedConfig: TextToSpeechConfig = {
  provider_id: '019feebc-3f84-7400-ab90-f29fda42725e' as ProviderId,
  model: 'step-tts-mini',
  voice: 'cixingnansheng',
};

afterEach(() => {
  globalThis.fetch = realFetch;
  configService.reset();
});

describe('tools.textToSpeech client service', () => {
  test('uses the key the backend registered as a Provider reference', () => {
    expect(TEXT_TO_SPEECH_CONFIG_KEY).toBe('tools.textToSpeech');
  });

  test('there is no enabled switch to disagree with the key itself', () => {
    expect(src.includes('enabled')).toBe(false);
  });

  test('clearing the default deletes the key rather than storing a blank object', () => {
    // The backend registers this key as a REQUIRED model reference: an object
    // with an empty provider_id would be rejected at the write boundary, so
    // "no default" has to be expressed as key deletion (null value).
    expect(src.includes('configService.remove(TEXT_TO_SPEECH_CONFIG_KEY)')).toBe(true);
    expect(src.includes('configService.set(TEXT_TO_SPEECH_CONFIG_KEY, undefined)')).toBe(false);
  });

  test('a failed write restores the persisted view instead of lying', () => {
    expect(src.includes('configService.reload()')).toBe(true);
  });

  test('clearing sends a null deletion wire value and evicts the cached default', async () => {
    let request: { url: string; method?: string; body?: string } | undefined;
    configService.setLocal(TEXT_TO_SPEECH_CONFIG_KEY, persistedConfig);
    globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
      request = {
        url: String(input),
        method: init?.method,
        body: typeof init?.body === 'string' ? init.body : undefined,
      };
      return new Response(null, { status: 204 });
    }) as typeof fetch;

    await saveTextToSpeechConfig(null);

    expect(request).toEqual({
      url: 'http://127.0.0.1:13400/api/settings/client',
      method: 'PUT',
      body: JSON.stringify({ [TEXT_TO_SPEECH_CONFIG_KEY]: null }),
    });
    expect(getTextToSpeechConfig()).toBeUndefined();
  });

  test('a rejected deletion reloads the persisted default into the cache', async () => {
    const methods: string[] = [];
    configService.setLocal(TEXT_TO_SPEECH_CONFIG_KEY, persistedConfig);
    globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
      const method = init?.method ?? 'GET';
      methods.push(method);
      if (method === 'PUT') {
        return new Response('write rejected', { status: 500 });
      }
      return new Response(
        JSON.stringify({ data: { [TEXT_TO_SPEECH_CONFIG_KEY]: persistedConfig } }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      );
    }) as typeof fetch;

    let caught: unknown;
    try {
      await saveTextToSpeechConfig(null);
    } catch (error) {
      caught = error;
    }

    expect(caught instanceof Error).toBe(true);
    expect((caught as Error).message.includes('write rejected')).toBe(true);
    expect(methods).toEqual(['PUT', 'GET']);
    expect(getTextToSpeechConfig()).toEqual(persistedConfig);
  });

  test('the panel picks the model through the shared TTS selector variant', () => {
    expect(panel.includes('<TaskModelSelect')).toBe(true);
    expect(panel.includes("task='speech_synthesis'")).toBe(true);
    expect(panel.includes('withVoice')).toBe(true);
    expect(panel.includes('hideHint')).toBe(false);
  });
});
