/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ProviderModelResponse } from '@/common/types/provider/providerModel';
import {
  capabilityOf,
  modelHealthOf,
  modelNamesOf,
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
  test('reads names and task-scoped capability health from the same model row', () => {
    const provider = { models: [row('gpt-4o'), row('o4-mini')] };
    expect(modelNamesOf(provider)).toEqual(['gpt-4o', 'o4-mini']);
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

  test('requires every requested trait on the selected task capability', () => {
    const model = row('multimodal');
    expect(modelSupportsTask(model, 'chat', ['vision_input'])).toBe(true);
    expect(modelSupportsTask(model, 'chat', ['vision_input', 'function_calling'])).toBe(false);
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
        },
      ],
    });
  });
});
