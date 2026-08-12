/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseProviderId } from '@/common/types/ids';
import {
  fromProviderResponse,
  toCreateProviderRequest,
  toUpdateProviderRequest,
  type ProviderResponse,
  type UpdateProviderRequest,
} from './providerApi';
import type { ProviderModelInput, ProviderModelResponse } from './providerModel';

const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000002';

const initialModel: ProviderModelInput = {
  model: 'gpt-4o',
  enabled: true,
  sort_order: 0,
  capabilities: [
    {
      task: 'chat',
      traits: ['vision_input'],
      protocol: 'openai.chat_text',
      connection_role: 'default',
      allow_cross_origin_credentials: false,
      provider_params: {},
      context_limit: 128_000,
    },
  ],
};

const modelResponse = (): ProviderModelResponse => ({
  provider_id: PROVIDER_ID,
  model: initialModel.model,
  enabled: initialModel.enabled ?? true,
  sort_order: 0,
  capabilities: initialModel.capabilities.map((capability) => ({
    ...capability,
    traits: capability.traits ?? [],
    allow_cross_origin_credentials: capability.allow_cross_origin_credentials ?? false,
    provider_params: capability.provider_params ?? {},
    created_at: 1,
    updated_at: 1,
  })),
  created_at: 1,
  updated_at: 1,
});

const response = (provider_id: string): ProviderResponse => ({
  provider_id,
  platform: 'openai',
  name: 'OpenAI',
  base_url: 'https://api.openai.com',
  auth_scheme: 'bearer',
  has_credentials: true,
  models: [modelResponse()],
  enabled: true,
  sort_order: 0,
  created_at: 1,
  updated_at: 1,
});

const expectThrow = (action: () => unknown) => {
  try {
    action();
  } catch {
    return;
  }
  throw new Error('Expected action to throw');
};

describe('provider wire contract', () => {
  test('maps provider_id and preserves the complete nested capability graph', () => {
    const provider = fromProviderResponse(response(PROVIDER_ID));
    expect(provider.id).toBe(parseProviderId(PROVIDER_ID));
    expect(provider.models[0].model).toBe('gpt-4o');
    expect(provider.models[0].capabilities[0]).toMatchObject({
      task: 'chat',
      protocol: 'openai.chat_text',
      context_limit: 128_000,
    });
    expect(Object.keys(provider).sort()).toEqual([
      'auth_scheme',
      'base_url',
      'bedrock_config',
      'enabled',
      'has_credentials',
      'id',
      'models',
      'name',
      'platform',
      'sort_order',
    ]);
  });

  test('rejects non-canonical provider ids at the wire boundary', () => {
    expectThrow(() => fromProviderResponse(response(`prov_${PROVIDER_ID}`)));
    expectThrow(() => fromProviderResponse(response(PROVIDER_ID.toUpperCase())));
  });

  test('creates provider and first complete model atomically in one request', () => {
    const request = toCreateProviderRequest({
      id: parseProviderId(PROVIDER_ID),
      platform: 'openai',
      name: 'OpenAI',
      base_url: 'https://api.openai.com',
      auth_scheme: 'bearer',
      credentials: { api_keys: ['sk-test'] },
      enabled: true,
      initial_model: initialModel,
    });
    expect(request.provider_id).toBe(parseProviderId(PROVIDER_ID));
    expect(request.credentials).toEqual({ api_keys: ['sk-test'] });
    expect(request.initial_model).toEqual(initialModel);
    expect(Object.prototype.hasOwnProperty.call(request, 'id')).toBe(false);
    expect(Object.prototype.hasOwnProperty.call(request, 'models')).toBe(false);
  });

  test('provider updates strip immutable platform and response-only fields', () => {
    const request = toUpdateProviderRequest({
      platform: 'other-provider',
      name: 'Renamed provider',
      models: [modelResponse()],
      enabled: false,
    } as unknown as UpdateProviderRequest);

    expect(request).toEqual({
      name: 'Renamed provider',
      enabled: false,
    });
    expect(Object.prototype.hasOwnProperty.call(request, 'platform')).toBe(false);
    expect(Object.prototype.hasOwnProperty.call(request, 'models')).toBe(false);
  });

  test('provider update keeps credentials omitted or forwards one typed replacement', () => {
    expect(toUpdateProviderRequest({ name: 'Keep existing' })).toEqual({ name: 'Keep existing' });
    expect(
      toUpdateProviderRequest({ credentials: { api_keys: ['replacement'] } })
    ).toEqual({ credentials: { api_keys: ['replacement'] } });
  });
});
