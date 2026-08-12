/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import type { IProvider, ModelTask, ModelTrait } from '@/common/config/storage';
import type { ProviderModelResponse } from '@/common/types/provider/providerModel';
import { parseProviderId } from '@/common/types/ids';
import { buildTaskModelGroups } from './useModelsForTask';

const PROVIDER_A = parseProviderId('0190f5fe-7c00-7a00-8000-00000000000a');
const PROVIDER_B = parseProviderId('0190f5fe-7c00-7a00-8000-00000000000b');

const model = (
  provider_id: IProvider['id'],
  name: string,
  task: ModelTask,
  traits: ModelTrait[] = [],
  enabled = true,
  sort_order = 0
): ProviderModelResponse => ({
  provider_id,
  model: name,
  enabled,
  sort_order,
  capabilities: [
    {
      task,
      traits,
      protocol: `test.${task}`,
      connection_role: 'default',
      allow_cross_origin_credentials: false,
      provider_params: {},
      created_at: 1,
      updated_at: 1,
    },
  ],
  created_at: 1,
  updated_at: 1,
});

const provider = (
  id: IProvider['id'],
  name: string,
  models: ProviderModelResponse[],
  enabled = true,
  sort_order = 0
): IProvider => ({
  id,
  platform: 'test',
  name,
  base_url: 'https://example.test',
  auth_scheme: 'bearer',
  has_credentials: true,
  models,
  enabled,
  sort_order,
});

describe('buildTaskModelGroups', () => {
  test('filters task capability while preserving provider and model order', () => {
    const providers = [
      provider(PROVIDER_A, 'First', [
        model(PROVIDER_A, 'a-chat-1', 'chat'),
        model(PROVIDER_A, 'a-chat-2', 'chat'),
      ], true, 10),
      provider(PROVIDER_B, 'Second', [
        model(PROVIDER_B, 'b-chat', 'chat'),
        model(PROVIDER_B, 'b-edit', 'image_edit'),
      ], true, 20),
    ];

    const groups = buildTaskModelGroups(providers, 'chat');
    expect(groups.map((group) => group.provider.id)).toEqual([PROVIDER_A, PROVIDER_B]);
    expect(groups[0].models).toEqual(['a-chat-1', 'a-chat-2']);
    expect(groups[1].models).toEqual(['b-chat']);
  });

  test('excludes disabled providers and models from runtime selection', () => {
    const providers = [
      provider(PROVIDER_A, 'Enabled', [
        model(PROVIDER_A, 'enabled-chat', 'chat'),
        model(PROVIDER_A, 'disabled-chat', 'chat', [], false),
      ]),
      provider(PROVIDER_B, 'Disabled provider', [model(PROVIDER_B, 'chat', 'chat')], false),
    ];
    expect(buildTaskModelGroups(providers, 'chat').map((group) => group.models)).toEqual([
      ['enabled-chat'],
    ]);
  });

  test('does not merge image edit with generation or rerank with embedding', () => {
    const providers = [
      provider(PROVIDER_A, 'Modalities', [
        model(PROVIDER_A, 'edit', 'image_edit'),
        model(PROVIDER_A, 'image', 'image_generation'),
        model(PROVIDER_A, 'reranker', 'rerank'),
        model(PROVIDER_A, 'embedder', 'embedding'),
      ]),
    ];
    expect(buildTaskModelGroups(providers, 'image_edit')[0].models).toEqual(['edit']);
    expect(buildTaskModelGroups(providers, 'image_generation')[0].models).toEqual(['image']);
    expect(buildTaskModelGroups(providers, 'rerank')[0].models).toEqual(['reranker']);
    expect(buildTaskModelGroups(providers, 'embedding')[0].models).toEqual(['embedder']);
  });

  test('requires traits on the selected task capability', () => {
    const providers = [
      provider(PROVIDER_A, 'Traits', [
        model(PROVIDER_A, 'vision-chat', 'chat', ['vision_input']),
        model(PROVIDER_A, 'plain-chat', 'chat'),
      ]),
    ];
    expect(buildTaskModelGroups(providers, 'chat', ['vision_input'])[0].models).toEqual([
      'vision-chat',
    ]);
  });
});

describe('useModelsForTask wiring', () => {
  const source = readFileSync(new URL('./useModelsForTask.ts', import.meta.url), 'utf8');

  test('uses the nested provider response directly', () => {
    expect(source.includes('useProvidersQuery()')).toBe(true);
    expect(source.includes('useSWR')).toBe(false);
    expect(source.includes('model.enabled && modelSupportsTask')).toBe(true);
  });
});
