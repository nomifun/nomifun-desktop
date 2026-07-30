/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IProvider } from '@/common/config/storage';
import type { TaskModelGroup } from '@/renderer/hooks/agent/useModelsForTask';
import {
  buildCreationModelEntries,
  filterCreationModels,
  groupCreationModelsByProvider,
} from './creationModels';

const provider = (id: string, name: string): IProvider =>
  ({
    id,
    name,
    platform: 'openai',
    enabled: true,
    models: [],
  }) as unknown as IProvider;

const providerA = provider('0190f5fe-7c00-7a00-8000-000000000001', 'Provider A');
const providerB = provider('0190f5fe-7c00-7a00-8000-000000000002', 'Provider B');

const groups = (provider_: IProvider, models: string[]): TaskModelGroup[] => [
  { provider: provider_, models },
];

describe('resolve-backed creation model entries', () => {
  test('unions image_generation and image_edit under the image capability', () => {
    const entries = buildCreationModelEntries([
      { capability: 'image_generation', groups: groups(providerA, ['gen-only']) },
      { capability: 'image_generation', groups: groups(providerA, ['edit-only', 'gen-only']) },
      { capability: 'video_generation', groups: [] },
    ]);

    expect(entries.map((e) => e.model)).toEqual(['gen-only', 'edit-only']);
    expect(entries.every((e) => e.capabilities.includes('image_generation'))).toBe(true);
    expect(filterCreationModels(entries, 'video_generation')).toEqual([]);
  });

  test('a model resolved for image and video yields one entry with both capabilities', () => {
    const entries = buildCreationModelEntries([
      { capability: 'image_generation', groups: groups(providerA, ['multi-modal']) },
      { capability: 'video_generation', groups: groups(providerA, ['multi-modal', 'video-only']) },
    ]);

    expect(entries).toHaveLength(2);
    expect(entries[0].model).toBe('multi-modal');
    expect(entries[0].capabilities).toEqual(['image_generation', 'video_generation']);
    expect(entries[1].capabilities).toEqual(['video_generation']);
  });

  test('capability filter and provider grouping preserve order', () => {
    const entries = buildCreationModelEntries([
      { capability: 'image_generation', groups: [...groups(providerA, ['a-img']), ...groups(providerB, ['b-img'])] },
      { capability: 'video_generation', groups: groups(providerB, ['b-vid']) },
    ]);

    expect(filterCreationModels(entries, 'image_generation').map((e) => e.model)).toEqual(['a-img', 'b-img']);

    const grouped = groupCreationModelsByProvider(entries);
    expect(grouped.map((g) => g.providerId)).toEqual([providerA.id, providerB.id]);
    expect(grouped[1].models.map((m) => m.model)).toEqual(['b-img', 'b-vid']);
  });

  test('applies the provider label function to every entry', () => {
    const entries = buildCreationModelEntries(
      [{ capability: 'image_generation', groups: groups(providerA, ['gen']) }],
      (p) => `label:${p.name}`
    );

    expect(entries[0].providerName).toBe('label:Provider A');
  });
});
