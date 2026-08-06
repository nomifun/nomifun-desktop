/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { ttsVoiceOptionsFor, TTS_VOICE_OPTIONS_BY_PLATFORM } from './ttsVoiceOptions';

const src = readFileSync(new URL('./TaskModelSelect.tsx', import.meta.url), 'utf8');
const companionControl = readFileSync(
  new URL('../../pages/nomi/CompanionModelControl.tsx', import.meta.url),
  'utf8'
);

describe('TaskModelSelect', () => {
  test('reads the catalog through the one shared hook and the one shared decision', () => {
    expect(src.includes('useModelsForTask(task, traits)')).toBe(true);
    expect(src.includes('taskModelSelectState({')).toBe(true);
    // No local re-derivation of staleness: that is exactly the drift this
    // component exists to remove.
    expect(src.includes('.includes(selectedModel)')).toBe(false);
  });

  test('a stale provider and a stale model are both rendered as disabled options', () => {
    expect(src.match(/disabled\s*>/g)?.length).toBeGreaterThanOrEqual(2);
    expect(src.includes("t('settings.taskModel.unavailableOption'")).toBe(true);
  });

  test('the voice select is free text with a candidate list, and only for the TTS variant', () => {
    expect(src.includes('withVoice')).toBe(true);
    expect(src.includes('showSearch')).toBe(true);
    expect(src.includes('allowCreate')).toBe(true);
    expect(src.includes("t('settings.taskModel.voicePlaceholder')")).toBe(true);
  });

  test('committing a model keeps the voice already chosen for that provider', () => {
    // Re-picking the model must not silently drop the voice: the user would
    // hear the provider default and have no idea why.
    expect(src.includes('voice: value?.provider_id === providerId ? value.voice : null')).toBe(true);
  });

  test('CompanionModelControl is now a thin wrapper, keeping its all-enabled scope', () => {
    expect(companionControl.includes('<TaskModelSelect')).toBe(true);
    expect(companionControl.includes("scope='all-enabled'")).toBe(true);
    expect(companionControl.includes('patchCompanion({ model:')).toBe(true);
    // Its own duplicated select markup is gone.
    expect(companionControl.includes('NomiSelect.Option')).toBe(false);
  });
});

describe('tts voice candidates', () => {
  test('only platforms whose voice ids are documented get a list', () => {
    expect(ttsVoiceOptionsFor('openai')).toEqual(TTS_VOICE_OPTIONS_BY_PLATFORM.openai);
    expect(ttsVoiceOptionsFor('openai').includes('alloy')).toBe(true);
    // Everything else is free text — inventing ids for a provider we have not
    // verified would offer the user values that just fail at synthesis time.
    expect(ttsVoiceOptionsFor('some-gateway')).toEqual([]);
    expect(ttsVoiceOptionsFor(undefined)).toEqual([]);
  });
});
