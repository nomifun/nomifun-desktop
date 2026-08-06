/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import zhNomi from '@/renderer/services/i18n/locales/zh-CN/nomi.json';
import enNomi from '@/renderer/services/i18n/locales/en-US/nomi.json';

const src = readFileSync(new URL('./ModelsSection.tsx', import.meta.url), 'utf8');

const OVERVIEW_SLOT_KEYS = [
  'mainChatModel',
  'fallbackChatModel',
  'fallbackChatModelHint',
  'fallbackChatUnset',
  'vadSlot',
  'vadSlotHint',
  'vadSensitivity',
  'vadMinSilence',
  'asrSlot',
  'asrSlotHint',
  'asrFallback',
  'visionSlot',
  'visionSlotHint',
  'visionFallback',
  'ttsSlot',
  'ttsSlotHint',
  'ttsFallback',
] as const;

describe('总览 model slots', () => {
  test('renders one row per slot in the designed order', () => {
    const order = [
      'nomi.overview.mainChatModel',
      'nomi.overview.fallbackChatModel',
      'nomi.overview.vadSlot',
      'nomi.overview.asrSlot',
      'nomi.overview.visionSlot',
      'nomi.overview.ttsSlot',
    ];
    const positions = order.map((key) => src.indexOf(`'${key}'`));
    expect(positions.every((p) => p > 0)).toBe(true);
    expect([...positions].sort((a, b) => a - b)).toEqual(positions);
  });

  test('every model slot goes through the shared selector, none re-implements one', () => {
    expect(src.match(/<TaskModelSelect/g)?.length).toBe(4);
    expect(src.includes("task='chat'")).toBe(true);
    expect(src.includes("task='speech_recognition'")).toBe(true);
    expect(src.includes("task='speech_synthesis'")).toBe(true);
    expect(src.includes("traits={['vision_input']}")).toBe(true);
    expect(src.includes('withVoice')).toBe(true);
    expect(src.includes('NomiSelect')).toBe(false);
  });

  test('the app-level "voice & perception" redirect row is gone', () => {
    // The row existed because TTS/ASR/VAD/vision had no per-companion setting.
    // They do now, so a redirect that sends the user away from the控件 would be
    // actively misleading.
    expect(src.includes('voicePerception')).toBe(false);
    expect(src.includes('useNavigate')).toBe(false);
    expect(src.includes('RowAction')).toBe(false);
  });

  test('an unset slot states its fallback instead of looking broken', () => {
    for (const key of ['fallbackChatUnset', 'asrFallback', 'visionFallback', 'ttsFallback']) {
      expect(src.includes(`nomi.overview.${key}`)).toBe(true);
    }
  });

  test('the VAD row is two local parameters, not a model picker', () => {
    expect(src.includes('NomiInputNumber')).toBe(true);
    expect(src.includes('vad: { sensitivity')).toBe(true);
    expect(src.includes('vad: { min_silence_ms')).toBe(true);
    expect(src.includes('min={200}')).toBe(true);
    expect(src.includes('max={3000}')).toBe(true);
  });

  test('copy exists in both locales and the retired keys are deleted', () => {
    const overview = (locale: Record<string, unknown>) =>
      (locale as { overview: Record<string, string> }).overview;
    for (const [name, locale] of [
      ['zh-CN', zhNomi as unknown as Record<string, unknown>],
      ['en-US', enNomi as unknown as Record<string, unknown>],
    ] as const) {
      for (const key of OVERVIEW_SLOT_KEYS) {
        expect(typeof overview(locale)[key]).toBe('string');
        expect(overview(locale)[key].trim().length > 0).toBe(true);
      }
      expect(overview(locale).voicePerception).toBeUndefined();
      expect(overview(locale).voicePerceptionHint).toBeUndefined();
      expect(overview(locale).goModelSettings).toBeUndefined();
      expect(name.length > 0).toBe(true);
    }
  });
});
