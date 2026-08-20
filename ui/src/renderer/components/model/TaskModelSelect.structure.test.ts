/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  ttsUsesModelIdAsVoice,
  ttsVoiceOptionsFor,
  TTS_VOICE_OPTIONS_BY_PROTOCOL,
} from './ttsVoiceOptions';

const src = readFileSync(new URL('./TaskModelSelect.tsx', import.meta.url), 'utf8');
const companionControl = readFileSync(
  new URL('../../pages/nomi/CompanionModelControl.tsx', import.meta.url),
  'utf8'
);
const generatorCard = readFileSync(
  new URL('../../pages/workshop/generation/GeneratorCard.tsx', import.meta.url),
  'utf8'
);
const failoverContent = readFileSync(
  new URL('../../pages/modelHub/ModelFailoverContent.tsx', import.meta.url),
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

  test('can hand its resolved warning to a packaged setting-row description', () => {
    expect(src.includes('onHintChange?: (hint: string) => void')).toBe(true);
    expect(src.includes('onHintChange?.(hint)')).toBe(true);
    expect(src.includes('!hideHint && hint')).toBe(true);
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
    expect(src.includes('!ttsUsesModelIdAsVoice(nextSpeechSynthesisProtocol)')).toBe(true);
  });

  test('voice behavior comes from the exact speech-synthesis capability protocol', () => {
    expect(src.includes("capabilityOf(selectedProvider, selectedModel, 'speech_synthesis')")).toBe(true);
    expect(src.includes('ttsVoiceOptionsFor(speechSynthesisProtocol')).toBe(true);
    expect(src.includes('currentPlatform')).toBe(false);
    expect(src.includes('.platform')).toBe(false);
  });

  test('CompanionModelControl is now a thin wrapper, keeping its all-enabled scope', () => {
    expect(companionControl.includes('<TaskModelSelect')).toBe(true);
    expect(companionControl.includes("scope='all-enabled'")).toBe(true);
    expect(companionControl.includes('patchCompanion({ model:')).toBe(true);
    // Its own duplicated select markup is gone.
    expect(companionControl.includes('NomiSelect.Option')).toBe(false);
  });

  test('generation and failover reuse this selector instead of owning parallel pickers', () => {
    expect(generatorCard.includes("components/model/TaskModelSelect'")).toBe(true);
    expect(generatorCard.includes('onChange={setTaskModel}')).toBe(true);
    expect(failoverContent.includes("components/model/TaskModelSelect'")).toBe(true);
    expect(failoverContent.includes('onChange={setDraft}')).toBe(true);
  });
});

describe('tts voice candidates', () => {
  test('only exact protocols whose voice ids are documented get a list', () => {
    expect(ttsVoiceOptionsFor('openai.audio_speech')).toEqual(
      TTS_VOICE_OPTIONS_BY_PROTOCOL['openai.audio_speech']
    );
    expect(ttsVoiceOptionsFor('openai.audio_speech').includes('alloy')).toBe(true);
    // StepFun voices are verified against its live system-voices API, so they
    // are offered as suggestions (still free text for cloned/newer ids).
    expect(ttsVoiceOptionsFor('stepfun.audio_speech').includes('cixingnansheng')).toBe(true);
    expect(
      ttsVoiceOptionsFor('siliconflow.audio_speech', 'FunAudioLLM/CosyVoice2-0.5B').includes(
        'FunAudioLLM/CosyVoice2-0.5B:alex'
      )
    ).toBe(true);
    expect(
      ttsVoiceOptionsFor('siliconflow.audio_speech', 'fnlp/MOSS-TTSD-v0.5').includes(
        'fnlp/MOSS-TTSD-v0.5:diana'
      )
    ).toBe(true);
    // Do not guess model-prefixed voice ids for a newly launched model.
    expect(ttsVoiceOptionsFor('siliconflow.audio_speech', 'vendor/new-speech-model')).toEqual([]);
    expect(ttsVoiceOptionsFor('siliconflow.audio_speech')).toEqual([]);
    // Everything else is free text — inventing ids for a provider we have not
    // verified would offer the user values that just fail at synthesis time.
    expect(ttsVoiceOptionsFor('future.tts_protocol')).toEqual([]);
    expect(ttsVoiceOptionsFor(undefined)).toEqual([]);
  });

  test('Deepgram uses the selected Aura model id as its voice', () => {
    expect(ttsUsesModelIdAsVoice('deepgram.speak_rest')).toBe(true);
    expect(ttsUsesModelIdAsVoice('openai.audio_speech')).toBe(false);
    expect(ttsUsesModelIdAsVoice('deepgram')).toBe(false);
    expect(src.includes('withVoice && !modelIdIsVoice')).toBe(true);
  });
});
