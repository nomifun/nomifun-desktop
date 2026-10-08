/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ProviderModelResponse } from '@/common/types/provider/providerModel';
import {
  capabilityOf,
  capabilitySupportsTrait,
  modelHealthOf,
  modelSupportsTask,
  toProviderModelInput,
} from './providerModels';

const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000002';

const row = (model: string, extra?: Partial<ProviderModelResponse>): ProviderModelResponse => ({
  provider_id: PROVIDER_ID,
  model,
  enabled: true,
  sort_order: 0,
  description: 'test model',
  capabilities: [
    {
      task: 'chat',
      traits: ['vision_input'],
      protocol: 'openai.chat_text',
      connection_role: 'default',
      allow_cross_origin_credentials: false,
      provider_params: {},
      output_limit: 16_384,
      health: { status: 'healthy', latency: 120 },
      health_checked_at: 42,
      created_at: 1,
      updated_at: 1,
    },
  ],
  created_at: 1,
  updated_at: 1,
  ...extra,
});

describe('nested provider models', () => {
  test('reads task-scoped capability health from the same model row', () => {
    const provider = { models: [row('gpt-4o'), row('o4-mini')] };
    expect(capabilityOf(provider, 'gpt-4o', 'chat')?.protocol).toBe('openai.chat_text');
    expect(modelHealthOf(provider, 'gpt-4o', 'chat')).toEqual({ status: 'healthy', latency: 120 });
    expect(modelHealthOf(provider, 'gpt-4o', 'embedding')).toBeUndefined();
  });

  test('keeps image edit and generation, embedding and rerank independent', () => {
    const image = row('image-model', {
      capabilities: [
        {
          task: 'image_edit',
          traits: [],
          protocol: 'openai.images_edit',
          connection_role: 'default',
          allow_cross_origin_credentials: false,
          provider_params: {},
          created_at: 1,
          updated_at: 1,
        },
      ],
    });
    expect(modelSupportsTask(image, 'image_edit')).toBe(true);
    expect(modelSupportsTask(image, 'image_generation')).toBe(false);
    expect(modelSupportsTask(image, 'rerank')).toBe(false);
  });

  test('native input eligibility depends on adapter representation without catalog checkboxes', () => {
    const model = row('brand-new-unknown-model');
    model.capabilities[0]!.traits = [];
    expect(modelSupportsTask(model, 'chat', ['vision_input'])).toBe(true);
    expect(modelSupportsTask(model, 'chat', ['audio_input'])).toBe(true);
    expect(modelSupportsTask(model, 'chat', ['vision_input', 'web_search'])).toBe(false);
    expect(modelSupportsTask(model, 'chat', ['video_input'])).toBe(false);
    expect(modelSupportsTask(model, 'chat', [], ['function_calling'])).toBe(true);

    model.capabilities[0]!.protocol = 'openai.responses';
    expect(modelSupportsTask(model, 'chat', ['vision_input', 'web_search'])).toBe(true);

    const limited = row('limited');
    limited.capabilities[0]!.health = {
      status: 'unknown',
      unsupported_technical_capabilities: ['function_calling'],
    };
    expect(modelSupportsTask(limited, 'chat', [], ['function_calling'])).toBe(false);
  });

  test('saved traits cannot grant representation missing from an adapter or another task', () => {
    const model = row('mislabelled', {
      capabilities: [{ ...row('source').capabilities[0]!, protocol: 'unknown.chat' }],
    });
    expect(modelSupportsTask(model, 'chat', ['vision_input'])).toBe(false);
    expect(capabilitySupportsTrait(undefined, 'vision_input')).toBe(false);
    expect(capabilitySupportsTrait({ ...model.capabilities[0]!, task: 'image_generation' }, 'vision_input')).toBe(false);
  });

  test('all registered image Chat adapters accept untagged image candidates', () => {
    for (const protocol of [
      'openai.chat_text', 'openai.responses', 'anthropic.messages',
      'gemini.generate_text', 'bedrock.anthropic_messages', 'vertex.anthropic_messages',
    ]) {
      const capability = { ...row('untagged').capabilities[0]!, protocol, traits: [] };
      expect(capabilitySupportsTrait(capability, 'vision_input')).toBe(true);
      expect(capabilitySupportsTrait(capability, 'video_input')).toBe(false);
      expect(capabilitySupportsTrait(capability, 'audio_input')).toBe(
        ['openai.chat_text', 'openai.responses', 'gemini.generate_text'].includes(protocol)
      );
    }
  });

  test('strips health and timestamps from full save input', () => {
    expect(toProviderModelInput(row('gpt-4o'))).toEqual({
      model: 'gpt-4o',
      enabled: true,
      description: 'test model',
      sort_order: 0,
      capabilities: [
        {
          task: 'chat',
          traits: ['vision_input'],
          protocol: 'openai.chat_text',
          connection_role: 'default',
          base_url_override: undefined,
          endpoint: undefined,
          poll_endpoint: undefined,
          content_endpoint: undefined,
          realtime_endpoint: undefined,
          allow_cross_origin_credentials: false,
          provider_params: {},
          context_limit: undefined,
          output_limit: 16_384,
          compaction_threshold_pct: undefined,
        },
      ],
    });
  });

  test('enabled and description saves preserve every authored task limit and inference parameter', () => {
    const configured = row('custom-model');
    configured.capabilities[0] = {
      ...configured.capabilities[0]!, context_limit: 1_000_000, output_limit: 100_000,
      compaction_threshold_pct: 90, provider_params: { reasoning_effort: 'high', temperature: 0.2 },
    };
    const saved = toProviderModelInput({ ...configured, enabled: false, description: 'changed' });
    expect(saved.enabled).toBe(false);
    expect(saved.description).toBe('changed');
    expect(saved.capabilities[0]).toMatchObject({ context_limit: 1_000_000, output_limit: 100_000,
      compaction_threshold_pct: 90, provider_params: { reasoning_effort: 'high', temperature: 0.2 } });
    expect(saved.capabilities[0]).not.toHaveProperty('health');
    expect(toProviderModelInput(row('unknown')).capabilities[0]?.context_limit).toBeUndefined();
  });
});
