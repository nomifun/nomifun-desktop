/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { TEXT_TO_SPEECH_CONFIG_KEY } from './textToSpeechConfig';

const src = readFileSync(new URL('./textToSpeechConfig.ts', import.meta.url), 'utf8');
const panel = readFileSync(
  new URL('../pages/modelHub/TextToSpeechContent.tsx', import.meta.url),
  'utf8'
);

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
    expect(src.includes('configService.set(TEXT_TO_SPEECH_CONFIG_KEY, undefined)')).toBe(true);
  });

  test('a failed write restores the persisted view instead of lying', () => {
    expect(src.includes('configService.reload()')).toBe(true);
  });

  test('the panel picks the model through the shared TTS selector variant', () => {
    expect(panel.includes('<TaskModelSelect')).toBe(true);
    expect(panel.includes("task='speech_synthesis'")).toBe(true);
    expect(panel.includes('withVoice')).toBe(true);
  });
});
