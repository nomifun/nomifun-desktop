/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ProviderModelCapabilityResponse } from '@/common/types/provider/providerModel';
import {
  capabilityDraftFromResponse,
  capabilityInputFromResponse,
  type ModelDefinitionDraft,
} from './providerModelAdvanced';
import { mergeTaskCapabilityEdit, resolveScopedModelTextEdit } from './modelTaskScopedEdit';

const capabilities: ProviderModelCapabilityResponse[] = [
  {
    task: 'chat', traits: ['vision_input'], protocol: 'openai.chat_text', connection_role: 'default',
    endpoint: '/chat/completions', allow_cross_origin_credentials: false, provider_params: { temperature: 0.3 },
    context_limit: 128_000, output_limit: 8_192, compaction_threshold_pct: 75,
    created_at: 1, updated_at: 1,
  },
  {
    task: 'speech_recognition', traits: [], protocol: 'openai.audio_transcriptions',
    connection_role: 'asr', base_url_override: 'https://audio.example.invalid/v1',
    endpoint: '/audio/transcriptions', allow_cross_origin_credentials: true,
    provider_params: { language: 'zh' }, created_at: 1, updated_at: 1,
  },
  {
    task: 'speech_synthesis', traits: [], protocol: 'openai.audio_speech', connection_role: 'default',
    endpoint: '/audio/speech', allow_cross_origin_credentials: false, provider_params: { voice: 'alloy', speed: 1.2 },
    created_at: 1, updated_at: 1,
  },
];
const definition = (): ModelDefinitionDraft => ({
  model: 'shared-model-id', capabilities: capabilities.map(capabilityDraftFromResponse),
});

describe('scenario model capability saves', () => {
  test('replaces ASR only and retains complete latest chat and TTS configuration', () => {
    const draft = definition();
    draft.capabilities[1].providerParamsJson = '{"language":"en"}';
    // The open editor still has the old chat snapshot. A newer chat update must
    // survive the full-list save required by the existing model API.
    const latest = capabilities.map((capability) => capability.task === 'chat'
      ? { ...capability, output_limit: 32_768, provider_params: { temperature: 0.8 } }
      : capability);
    const result = mergeTaskCapabilityEdit(draft, latest, 'speech_recognition', capabilityInputFromResponse(capabilities[1]));
    expect(result.error).toBeUndefined();
    expect(result.capabilities?.[0]).toEqual(capabilityInputFromResponse(latest[0]));
    expect(result.capabilities?.[1]).toEqual({
      ...capabilityInputFromResponse(capabilities[1]), provider_params: { language: 'en' },
    });
    expect(result.capabilities?.[2]).toEqual(capabilityInputFromResponse(capabilities[2]));
  });

  test('replaces TTS only while carrying chat limits and ASR connection independently', () => {
    const draft = definition();
    draft.capabilities[2].providerParamsJson = '{"voice":"nova","speed":1.2}';
    const result = mergeTaskCapabilityEdit(draft, capabilities, 'speech_synthesis', capabilityInputFromResponse(capabilities[2]));
    expect(result.capabilities?.[0]).toEqual(capabilityInputFromResponse(capabilities[0]));
    expect(result.capabilities?.[1]).toEqual(capabilityInputFromResponse(capabilities[1]));
    expect(result.capabilities?.[2]?.provider_params).toEqual({ voice: 'nova', speed: 1.2 });
  });

  test('preserves tasks added or removed elsewhere after the editor opened', () => {
    const draft = definition();
    const embedding: ProviderModelCapabilityResponse = {
      task: 'embedding', traits: [], protocol: 'openai.embeddings', connection_role: 'default',
      endpoint: '/embeddings', allow_cross_origin_credentials: false, provider_params: { dimensions: 1_024 }, created_at: 1, updated_at: 1,
    };
    const latest = [...capabilities.filter((capability) => capability.task !== 'speech_synthesis'), embedding];
    const result = mergeTaskCapabilityEdit(draft, latest, 'speech_recognition', capabilityInputFromResponse(capabilities[1]));
    expect(result.capabilities?.map((capability) => capability.task)).toEqual(['chat', 'speech_recognition', 'embedding']);
    expect(result.capabilities?.[2]).toEqual(capabilityInputFromResponse(embedding));
  });

  test('rejects saving a removed or duplicated scenario task instead of reintroducing it', () => {
    const draft = definition();
    const baseline = capabilityInputFromResponse(capabilities[1]);
    expect(mergeTaskCapabilityEdit(draft, [capabilities[0]], 'speech_recognition', baseline)).toEqual({ error: 'unavailable' });
    expect(mergeTaskCapabilityEdit(draft, [...capabilities, capabilities[1]], 'speech_recognition', baseline)).toEqual({ error: 'unavailable' });
    draft.capabilities = draft.capabilities.filter((capability) => capability.task !== 'speech_recognition');
    expect(mergeTaskCapabilityEdit(draft, capabilities, 'speech_recognition', baseline)).toEqual({ error: 'unavailable' });
  });

  test('rejects replacing externally changed task configuration but ignores health and timestamps', () => {
    const draft = definition();
    const baseline = capabilityInputFromResponse(capabilities[1]);
    const updated = capabilities.map((capability) => capability.task === 'speech_recognition'
      ? { ...capability, provider_params: { language: 'ja' } }
      : capability);
    expect(mergeTaskCapabilityEdit(draft, updated, 'speech_recognition', baseline)).toEqual({ error: 'changed' });
    const refreshed = capabilities.map((capability) => ({ ...capability, updated_at: 2 }));
    expect(mergeTaskCapabilityEdit(draft, refreshed, 'speech_recognition', baseline).error).toBeUndefined();
  });

  test('rejects malformed edited parameters instead of producing an incomplete capability list', () => {
    const draft = definition();
    draft.capabilities[1].providerParamsJson = '{';
    expect(mergeTaskCapabilityEdit(draft, capabilities, 'speech_recognition', capabilityInputFromResponse(capabilities[1]))).toEqual({ error: 'invalid' });
  });
});

describe('shared scenario model text', () => {
  test('retains the latest alias or description when that field was untouched in the draft', () => {
    expect(resolveScopedModelTextEdit('original alias', 'original alias', 'new alias')).toEqual({ value: 'new alias', conflict: false });
    expect(resolveScopedModelTextEdit(null, null, 'new description')).toEqual({ value: 'new description', conflict: false });
  });

  test('saves edited text or explicit removal when persisted text is unchanged', () => {
    expect(resolveScopedModelTextEdit(' new description ', 'original description', 'original description')).toEqual({ value: 'new description', conflict: false });
    expect(resolveScopedModelTextEdit('', 'original alias', 'original alias')).toEqual({ value: null, conflict: false });
  });

  test('rejects conflicting edits of the same shared field', () => {
    expect(resolveScopedModelTextEdit('my alias', 'original alias', 'another alias')).toEqual({ conflict: true });
    expect(resolveScopedModelTextEdit(null, 'original description', 'another description')).toEqual({ conflict: true });
  });

  test('accepts an external update that already matches the edited value', () => {
    expect(resolveScopedModelTextEdit('new alias', 'original alias', 'new alias')).toEqual({ value: 'new alias', conflict: false });
  });
});
