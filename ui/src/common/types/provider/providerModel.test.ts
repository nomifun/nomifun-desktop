/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type {
  ProviderModelKeyRequest,
  ProviderModelResponse,
  SaveProviderModelRequest,
} from './providerModel';

const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000002';
const wire = (value: unknown) => JSON.parse(JSON.stringify(value)) as Record<string, unknown>;

const response: ProviderModelResponse = {
  provider_id: PROVIDER_ID,
  model: 'step-tts-mini',
  display_name: 'Step TTS Mini',
  enabled: true,
  sort_order: 3,
  description: 'speech model',
  capabilities: [
    {
      task: 'speech_synthesis',
      traits: ['audio_output', 'streaming'],
      protocol: 'stepfun.audio_speech',
      connection_role: 'default',
      endpoint: '/audio/speech',
      allow_cross_origin_credentials: false,
      provider_params: { voice: 'cixingnansheng' },
      health: { status: 'healthy', latency: 320 },
      health_checked_at: 1712345678000,
      created_at: 1,
      updated_at: 2,
    },
  ],
  created_at: 1,
  updated_at: 2,
};

describe('provider model wire contract', () => {
  test('returns one model with nested task-scoped capabilities', () => {
    const body = wire(response);
    expect(Object.keys(body).sort()).toEqual([
      'capabilities',
      'created_at',
      'description',
      'display_name',
      'enabled',
      'model',
      'provider_id',
      'sort_order',
      'updated_at',
    ]);
    expect((body['capabilities'] as unknown[])).toHaveLength(1);
    expect((body['capabilities'] as Array<Record<string, unknown>>)[0]).toMatchObject({
      task: 'speech_synthesis',
      protocol: 'stepfun.audio_speech',
      connection_role: 'default',
    });
  });

  test('full save contains identity plus one complete model input', () => {
    const request: SaveProviderModelRequest = {
      provider_id: PROVIDER_ID,
      model: {
        model: response.model,
        display_name: response.display_name,
        enabled: false,
        description: response.description,
        sort_order: response.sort_order,
        capabilities: response.capabilities.map((capability) => ({
          task: capability.task,
          traits: capability.traits,
          protocol: capability.protocol,
          connection_role: capability.connection_role,
          endpoint: capability.endpoint,
          allow_cross_origin_credentials: capability.allow_cross_origin_credentials,
          provider_params: capability.provider_params,
        })),
      },
    };
    expect(Object.keys(wire(request)).sort()).toEqual(['model', 'provider_id']);
    expect((wire(request)['model'] as Record<string, unknown>)['capabilities']).toHaveLength(1);
  });

  test('delete key carries exactly the composite identity', () => {
    const key: ProviderModelKeyRequest = { provider_id: PROVIDER_ID, model: response.model };
    expect(wire(key)).toEqual({ provider_id: PROVIDER_ID, model: 'step-tts-mini' });
  });
});
