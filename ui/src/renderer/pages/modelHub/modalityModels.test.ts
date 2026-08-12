/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IProvider } from '@/common/config/storage';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ProviderModelResponse } from '@/common/types/provider/providerModel';
import type { ProviderId } from '@/common/types/ids';
import { buildModalityGroups, MODALITY_SPECS, rowMatchesModality } from './modalityModels';

const A = '0190f5fe-7c00-7a00-8000-0000000000a1' as ProviderId;
const B = '0190f5fe-7c00-7a00-8000-0000000000b2' as ProviderId;

const row = (
  providerId: ProviderId,
  name: string,
  task: ModelTask,
  overrides: Partial<ProviderModelResponse> = {}
): ProviderModelResponse => ({
  provider_id: providerId,
  model: name,
  enabled: true,
  sort_order: 0,
  capabilities: [
    {
      task,
      traits: [],
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
  ...overrides,
});

const provider = (
  id: ProviderId,
  name: string,
  models: ProviderModelResponse[],
  enabled = true
): IProvider => ({
  id,
  platform: 'custom',
  name,
  base_url: 'https://example.test',
  auth_scheme: 'bearer',
  has_credentials: true,
  models,
  enabled,
});

describe('modality specs', () => {
  test('represents all nine tasks independently', () => {
    expect(Object.values(MODALITY_SPECS).map((spec) => spec.task)).toEqual([
      'chat',
      'realtime_conversation',
      'chat',
      'image_generation',
      'image_edit',
      'video_generation',
      'speech_synthesis',
      'speech_recognition',
      'embedding',
      'rerank',
    ]);
    expect(MODALITY_SPECS.vision.traits).toEqual(['vision_input']);
  });

  test('matches task and traits on the same capability', () => {
    const vision = row(A, 'vision-chat', 'chat');
    vision.capabilities[0].traits = ['vision_input'];
    expect(rowMatchesModality(vision, MODALITY_SPECS.chat)).toBe(true);
    expect(rowMatchesModality(vision, MODALITY_SPECS.vision)).toBe(true);
    expect(rowMatchesModality(vision, MODALITY_SPECS.realtime)).toBe(false);
  });

  test('never merges edit with generation or rerank with embedding', () => {
    expect(rowMatchesModality(row(A, 'edit', 'image_edit'), MODALITY_SPECS.image)).toBe(false);
    expect(rowMatchesModality(row(A, 'edit', 'image_edit'), MODALITY_SPECS.image_edit)).toBe(true);
    expect(rowMatchesModality(row(A, 'rank', 'rerank'), MODALITY_SPECS.embedding)).toBe(false);
    expect(rowMatchesModality(row(A, 'rank', 'rerank'), MODALITY_SPECS.rerank)).toBe(true);
  });
});

describe('buildModalityGroups', () => {
  test('retains disabled providers and models in management', () => {
    const groups = buildModalityGroups(
      [
        provider(A, 'A', [
          row(A, 'on', 'chat'),
          row(A, 'off', 'chat', { enabled: false }),
        ], false),
      ],
      MODALITY_SPECS.chat
    );
    expect(groups[0].enabled).toBe(false);
    expect(groups[0].models.map((model) => [model.model, model.enabled])).toEqual([
      ['on', true],
      ['off', false],
    ]);
  });

  test('preserves provider and nested model order and drops empty groups', () => {
    const groups = buildModalityGroups(
      [
        provider(B, 'B', [row(B, 'b-image', 'image_generation')]),
        provider(A, 'A', [row(A, 'a1', 'chat'), row(A, 'a2', 'chat')]),
      ],
      MODALITY_SPECS.chat
    );
    expect(groups.map((group) => group.providerId)).toEqual([A]);
    expect(groups[0].models.map((model) => model.model)).toEqual(['a1', 'a2']);
  });
});
