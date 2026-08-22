/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { IProvider, ModelTask, ModelTrait } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';

import {
  adaptCreativeModelCatalog,
  buildCreativeModelGroups,
  creativeModelSelectorState,
  creativeModelTaskFor,
  findCreativeModelOption,
} from './catalog';

const providerId = (suffix: string) =>
  `0190f5fe-7c00-7a00-8000-0000000000${suffix}` as ProviderId;

const capability = (task: ModelTask, traits: ModelTrait[] = []) => ({
  task,
  traits,
  protocol: `test.${task}`,
  connection_role: 'default',
  allow_cross_origin_credentials: false,
  provider_params: {},
  created_at: 1,
  updated_at: 1,
});

const provider = ({
  id,
  name,
  enabled = true,
  models,
}: {
  id: ProviderId;
  name: string;
  enabled?: boolean;
  models: Array<{
    name: string;
    enabled?: boolean;
    capabilities: ReturnType<typeof capability>[];
  }>;
}): IProvider => ({
  id,
  name,
  enabled,
  platform: 'custom',
  base_url: 'https://example.invalid',
  auth_scheme: 'bearer',
  has_credentials: true,
  models: models.map((model, index) => ({
    provider_id: id,
    model: model.name,
    enabled: model.enabled ?? true,
    sort_order: index,
    capabilities: model.capabilities,
    created_at: 1,
    updated_at: 1,
  })),
});

const A = providerId('a1');
const B = providerId('b2');

describe('Creative Studio model task mapping', () => {
  test('maps product modalities to one exact NomiFun task', () => {
    expect(creativeModelTaskFor({ capability: 'text' })).toBe('chat');
    expect(creativeModelTaskFor({ capability: 'image' })).toBe('image_generation');
    expect(creativeModelTaskFor({ capability: 'video' })).toBe('video_generation');
    expect(creativeModelTaskFor({ capability: 'audio' })).toBe('speech_synthesis');
    expect(creativeModelTaskFor({ capability: 'task', task: 'image_edit' })).toBe('image_edit');
    expect(creativeModelTaskFor({ capability: 'task', task: 'speech_recognition' })).toBe(
      'speech_recognition'
    );
  });
});

describe('Creative Studio model catalog', () => {
  const providers = [
    provider({
      id: A,
      name: 'Provider A',
      models: [
        { name: 'chat-only', capabilities: [capability('chat')] },
        {
          name: 'image-with-vision',
          capabilities: [capability('image_generation', ['vision_input'])],
        },
        { name: 'disabled-image', enabled: false, capabilities: [capability('image_generation')] },
      ],
    }),
    provider({
      id: B,
      name: 'Provider B',
      enabled: false,
      models: [{ name: 'hidden-image', capabilities: [capability('image_generation')] }],
    }),
  ];

  test('filters from exact nested capabilities and excludes disabled rows', () => {
    const groups = buildCreativeModelGroups(providers, { capability: 'image' });
    expect(groups).toHaveLength(1);
    expect(groups[0].providerId).toBe(A);
    expect(groups[0].models.map((model) => model.model)).toEqual(['image-with-vision']);
    expect(groups[0].models[0]).toMatchObject({
      task: 'image_generation',
      protocol: 'test.image_generation',
      traits: ['vision_input'],
    });
  });

  test('requires traits on the same task capability', () => {
    expect(
      buildCreativeModelGroups(providers, {
        capability: 'task',
        task: 'image_generation',
        traits: ['vision_input'],
      })[0].models.map((model) => model.model)
    ).toEqual(['image-with-vision']);
    expect(
      buildCreativeModelGroups(providers, {
        capability: 'task',
        task: 'image_generation',
        traits: ['audio_output'],
      })
    ).toEqual([]);
  });

  test('never broadens an exact task to a neighbouring modality task', () => {
    expect(
      buildCreativeModelGroups(providers, {
        capability: 'task',
        task: 'image_edit',
      })
    ).toEqual([]);
    expect(
      buildCreativeModelGroups(providers, {
        capability: 'task',
        task: 'image_generation',
      })[0].models.map((model) => model.model)
    ).toEqual(['image-with-vision']);
  });

  test('resolves a saved provider/model pair only from the compatible pool', () => {
    const groups = buildCreativeModelGroups(providers, { capability: 'image' });
    expect(findCreativeModelOption(groups, { providerId: A, model: 'image-with-vision' })?.task).toBe(
      'image_generation'
    );
    expect(findCreativeModelOption(groups, { providerId: A, model: 'chat-only' })).toBeNull();
  });
});

describe('Creative Studio model selector states', () => {
  const compatible = provider({
    id: A,
    name: 'A',
    models: [{ name: 'image', capabilities: [capability('image_generation')] }],
  });
  const readyCatalog = adaptCreativeModelCatalog({ data: [compatible], isLoading: false });
  const readyGroups = buildCreativeModelGroups(readyCatalog.providers, { capability: 'image' });

  test('keeps loading, no-provider, no-compatible, disabled, ready and error distinct', () => {
    const loading = adaptCreativeModelCatalog({ data: undefined, isLoading: false });
    expect(creativeModelSelectorState({ catalog: loading, groups: [], disabled: false })).toBe(
      'loading'
    );

    const empty = adaptCreativeModelCatalog({ data: [], isLoading: false });
    expect(creativeModelSelectorState({ catalog: empty, groups: [], disabled: false })).toBe(
      'no-provider'
    );

    expect(creativeModelSelectorState({ catalog: readyCatalog, groups: [], disabled: false })).toBe(
      'no-compatible-model'
    );
    expect(creativeModelSelectorState({ catalog: readyCatalog, groups: readyGroups, disabled: true })).toBe(
      'disabled'
    );
    expect(creativeModelSelectorState({ catalog: readyCatalog, groups: readyGroups, disabled: false })).toBe(
      'ready'
    );

    const failed = adaptCreativeModelCatalog({
      data: [compatible],
      isLoading: false,
      error: new Error('offline'),
    });
    expect(creativeModelSelectorState({ catalog: failed, groups: readyGroups, disabled: false })).toBe(
      'error'
    );
    expect(failed.error?.message).toBe('offline');
  });

  test('a failed provider request is never reinterpreted as an empty catalog', () => {
    const failed = adaptCreativeModelCatalog({
      data: undefined,
      isLoading: false,
      error: 'bridge unavailable',
    });
    expect(failed.status).toBe('error');
    expect(failed.providers).toEqual([]);
    expect(failed.error?.message).toBe('bridge unavailable');
  });
});
