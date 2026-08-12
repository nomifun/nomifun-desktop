/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { normalizeSpeechToTextConfig } from './speechToTextConfig';

const speechSection = readFileSync(
  new URL('../pages/modelHub/SpeechToTextContent.tsx', import.meta.url),
  'utf8'
);

const providerId = '0190f5fe-7c00-7a00-8000-0000000000a1' as never;

describe('speech-to-text config is one model reference', () => {
  test('normalizes the current shape without a provider enum or embedded credentials', () => {
    const current = {
      enabled: true,
      provider_id: providerId,
      model: 'step-asr',
      language: 'zh',
    };
    expect(normalizeSpeechToTextConfig(current)).toEqual(current);
    expect('provider' in normalizeSpeechToTextConfig(current)).toBe(false);
    expect('openai' in normalizeSpeechToTextConfig(current)).toBe(false);
    expect('deepgram' in normalizeSpeechToTextConfig(current)).toBe(false);
  });

  test('uses the shared speech-recognition selector', () => {
    expect(speechSection.includes('<TaskModelSelect')).toBe(true);
    expect(speechSection.includes("task='speech_recognition'")).toBe(true);
    expect(speechSection.includes("provider: 'openai'")).toBe(false);
  });
});
