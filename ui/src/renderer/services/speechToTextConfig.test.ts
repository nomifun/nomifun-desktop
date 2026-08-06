/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { hasLegacyEmbeddedSpeechBlocks, normalizeSpeechToTextConfig } from './speechToTextConfig';

const speechSection = readFileSync(
  new URL('../pages/modelHub/SpeechToTextContent.tsx', import.meta.url),
  'utf8'
);

const providerId = '0190f5fe-7c00-7a00-8000-0000000000a1' as never;

describe('speech-to-text config is a catalog reference now', () => {
  test('a legacy embedded-credential block is detected and stripped', () => {
    const legacy = {
      enabled: true,
      provider: 'openai' as const,
      openai: { api_key: 'sk-legacy', model: 'whisper-1', language: 'zh' },
    };
    expect(hasLegacyEmbeddedSpeechBlocks(legacy)).toBe(true);
    const normalized = normalizeSpeechToTextConfig(legacy);
    // The model/language carried by the retired block are preserved (they are
    // the user's actual choice) but the credential shape is gone — the backend
    // has refused embedded credentials since the catalog migration, so keeping
    // them only risks re-persisting a secret.
    expect(normalized.model).toBe('whisper-1');
    expect(normalized.language).toBe('zh');
    expect(normalized.openai).toBeUndefined();
    expect(normalized.deepgram).toBeUndefined();
    expect(hasLegacyEmbeddedSpeechBlocks(normalized)).toBe(false);
  });

  test('a catalog-shaped config round-trips untouched', () => {
    const current = {
      enabled: true,
      provider: 'openai' as const,
      provider_id: providerId,
      model: 'whisper-1',
      language: '',
    };
    expect(normalizeSpeechToTextConfig(current)).toEqual(current);
    expect(hasLegacyEmbeddedSpeechBlocks(current)).toBe(false);
  });

  test('the section performs the one-time migration and uses the shared selector', () => {
    expect(speechSection.includes('hasLegacyEmbeddedSpeechBlocks')).toBe(true);
    expect(speechSection.includes('<TaskModelSelect')).toBe(true);
    expect(speechSection.includes("task='speech_recognition'")).toBe(true);
  });
});
