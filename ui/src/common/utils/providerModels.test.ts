/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ProviderModelResponse } from '@/common/types/provider/providerModel';
import { modelHealthOf, modelNamesOf } from './providerModels';

const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000002';

const row = (model: string, extra?: Partial<ProviderModelResponse>): ProviderModelResponse => ({
  provider_id: PROVIDER_ID,
  model,
  enabled: true,
  sort_order: 0,
  tasks: ['chat'],
  traits: [],
  params: null,
  source: 'inferred',
  created_at: 1,
  updated_at: 1,
  ...extra,
});

describe('modelHealthOf', () => {
  test('reads health from the authoritative models_detail row', () => {
    const provider = {
      models_detail: [row('gpt-4o', { health: { status: 'healthy', latency: 120, last_check: 42 } })],
    };
    expect(modelHealthOf(provider, 'gpt-4o')).toEqual({ status: 'healthy', latency: 120, last_check: 42 });
  });

  test('never falls back to the legacy model_health map', () => {
    const provider = {
      models_detail: [row('gpt-4o')],
      model_health: { 'gpt-4o': { status: 'unhealthy' as const } },
    };
    // Row exists but carries no health → undefined, even though the legacy map
    // has an entry. The legacy map is write-frozen and must not be read here.
    expect(modelHealthOf(provider, 'gpt-4o')).toBeUndefined();
    // No row at all → undefined too.
    expect(modelHealthOf(provider, 'missing-model')).toBeUndefined();
    expect(modelHealthOf(undefined, 'gpt-4o')).toBeUndefined();
    expect(modelHealthOf({}, 'gpt-4o')).toBeUndefined();
  });
});

describe('modelNamesOf', () => {
  test('prefers models_detail rows over the legacy models array', () => {
    const provider = {
      models: ['legacy-only'],
      models_detail: [row('gpt-4o'), row('o4-mini')],
    };
    expect(modelNamesOf(provider)).toEqual(['gpt-4o', 'o4-mini']);
  });

  test('falls back to legacy models when there are no rows', () => {
    expect(modelNamesOf({ models: ['a', 'b'], models_detail: [] })).toEqual(['a', 'b']);
    expect(modelNamesOf({ models: ['a'] })).toEqual(['a']);
    expect(modelNamesOf({ models: undefined as unknown as string[] })).toEqual([]);
  });
});
