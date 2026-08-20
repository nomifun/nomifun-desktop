/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  canGenerateAudioWorkbench,
  clampAudioWorkbenchSpeed,
  isAudioWorkbenchBusy,
  type AudioWorkbenchValue,
} from './types';

const value = (overrides: Partial<AudioWorkbenchValue> = {}): AudioWorkbenchValue => ({
  text: '欢迎来到 NomiFun 创意工坊。',
  instructions: '自然、温暖，适合产品旁白。',
  voice: 'alloy',
  format: 'mp3',
  speed: 1,
  model: { providerId: 'provider-openai', model: 'gpt-4o-mini-tts' },
  ...overrides,
});

describe('AudioWorkbench controlled contract helpers', () => {
  test('recognizes only queued and running as busy', () => {
    expect(
      ['idle', 'queued', 'running', 'succeeded', 'failed', 'canceled'].map((state) =>
        isAudioWorkbenchBusy(state as Parameters<typeof isAudioWorkbenchBusy>[0])
      )
    ).toEqual([false, true, true, false, false, false]);
  });

  test('clamps source-compatible speed without leaking an invalid number', () => {
    expect(clampAudioWorkbenchSpeed(0.1)).toBe(0.25);
    expect(clampAudioWorkbenchSpeed(5)).toBe(4);
    expect(clampAudioWorkbenchSpeed(1.237)).toBe(1.24);
    expect(clampAudioWorkbenchSpeed(Number.NaN)).toBe(1);
    expect(clampAudioWorkbenchSpeed(3, { min: 0.5, max: 2, step: 0.1 })).toBe(2);
  });

  test('gates submit on model, text, length, task activity and required reference', () => {
    expect(canGenerateAudioWorkbench(value(), 'idle', 0)).toBe(true);
    expect(canGenerateAudioWorkbench(value({ model: null }), 'idle', 0)).toBe(false);
    expect(canGenerateAudioWorkbench(value({ text: '   ' }), 'idle', 0)).toBe(false);
    expect(canGenerateAudioWorkbench(value({ text: '12345' }), 'idle', 0, { maxTextLength: 4 })).toBe(false);
    expect(canGenerateAudioWorkbench(value(), 'queued', 0)).toBe(false);
    expect(canGenerateAudioWorkbench(value(), 'running', 0)).toBe(false);
    expect(canGenerateAudioWorkbench(value(), 'idle', 0, { referenceRequired: true })).toBe(false);
    expect(canGenerateAudioWorkbench(value(), 'idle', 1, { referenceRequired: true })).toBe(true);
    expect(canGenerateAudioWorkbench(value(), 'succeeded', 1, { disabled: true })).toBe(false);
  });
});
