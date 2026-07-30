/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

import type { IProvider } from '@/common/config/storage';
import type { CatalogModelRef } from '@/common/types/provider/providerApi';
import { parseProviderId } from '@/common/types/ids';
import {
  buildResolveModelsRequest,
  buildTaskModelGroups,
  modelsForTaskKey,
} from './useModelsForTask';

const PROVIDER_A = parseProviderId('0190f5fe-7c00-7a00-8000-00000000000a');
const PROVIDER_B = parseProviderId('0190f5fe-7c00-7a00-8000-00000000000b');
const PROVIDER_GONE = parseProviderId('0190f5fe-7c00-7a00-8000-00000000000c');

const provider = (id: IProvider['id'], name: string): IProvider =>
  ({
    id,
    platform: 'openai',
    name,
    base_url: 'https://x.test/v1',
    api_key: 'k',
    models: [],
  }) as unknown as IProvider;

const ref = (provider_id: IProvider['id'], model: string): CatalogModelRef => ({ provider_id, model });

describe('buildTaskModelGroups (mocked resolve payload → grouped selector data)', () => {
  test('groups refs by provider preserving provider sort order and per-provider model order', () => {
    const providers = [provider(PROVIDER_A, 'First'), provider(PROVIDER_B, 'Second')];
    // Resolve payload deliberately interleaves providers; provider order must
    // come from the providers list, model order from the payload (catalog order).
    const refs = [
      ref(PROVIDER_B, 'b-model-1'),
      ref(PROVIDER_A, 'a-model-1'),
      ref(PROVIDER_B, 'b-model-2'),
      ref(PROVIDER_A, 'a-model-2'),
    ];

    const groups = buildTaskModelGroups(refs, providers);

    expect(groups.map((g) => g.provider.id)).toEqual([PROVIDER_A, PROVIDER_B]);
    expect(groups[0].models).toEqual(['a-model-1', 'a-model-2']);
    expect(groups[1].models).toEqual(['b-model-1', 'b-model-2']);
  });

  test('drops refs for unknown providers and providers without matching models', () => {
    const providers = [provider(PROVIDER_A, 'First'), provider(PROVIDER_B, 'No chat models')];
    const refs = [ref(PROVIDER_A, 'a-model-1'), ref(PROVIDER_GONE, 'orphan-model')];

    const groups = buildTaskModelGroups(refs, providers);

    // The deleted/unknown provider's ref is dropped (no metadata to render);
    // provider B has no matching models so it yields NO empty group.
    expect(groups).toHaveLength(1);
    expect(groups[0].provider.id).toBe(PROVIDER_A);
    expect(groups[0].models).toEqual(['a-model-1']);
  });

  test('deduplicates repeated refs', () => {
    const providers = [provider(PROVIDER_A, 'First')];
    const refs = [ref(PROVIDER_A, 'a-model-1'), ref(PROVIDER_A, 'a-model-1')];

    expect(buildTaskModelGroups(refs, providers)[0].models).toEqual(['a-model-1']);
  });

  test('empty catalog resolves to zero groups (empty-state signal)', () => {
    expect(buildTaskModelGroups([], [provider(PROVIDER_A, 'First')])).toEqual([]);
  });
});

describe('trait filtering wiring (server-side filter, request + cache key)', () => {
  test('request carries required_traits only when refinement is requested', () => {
    expect(buildResolveModelsRequest('chat')).toEqual({ task: 'chat' });
    expect(buildResolveModelsRequest('chat', [])).toEqual({ task: 'chat' });
    expect(buildResolveModelsRequest('chat', ['vision_input'])).toEqual({
      task: 'chat',
      required_traits: ['vision_input'],
    });
  });

  test('SWR key separates task and trait combinations', () => {
    expect(modelsForTaskKey('chat')).not.toBe(modelsForTaskKey('embedding'));
    expect(modelsForTaskKey('chat')).not.toBe(modelsForTaskKey('chat', ['vision_input']));
    expect(modelsForTaskKey('chat', ['vision_input'])).not.toBe(
      modelsForTaskKey('chat', ['function_calling'])
    );
  });

  test('SWR key is trait-order insensitive (equivalent sets share one cache entry)', () => {
    expect(modelsForTaskKey('chat', ['vision_input', 'function_calling'])).toBe(
      modelsForTaskKey('chat', ['function_calling', 'vision_input'])
    );
  });
});

describe('useModelsForTask wiring (structure)', () => {
  const source = readFileSync(new URL('./useModelsForTask.ts', import.meta.url), 'utf8');

  test('resolves through ipcBridge.modelProfile.resolve under a task+traits SWR key', () => {
    expect(source.includes('ipcBridge.modelProfile.resolve.invoke(')).toBe(true);
    expect(source.includes('useSWR<CatalogModelRef[]>')).toBe(true);
    expect(source.includes('modelsForTaskKey(task, requiredTraits)')).toBe(true);
  });

  test('joins provider metadata from useModelProviderList and stays unresolved after errors', () => {
    expect(source.includes("from './useModelProviderList'")).toBe(true);
    expect(source.includes('buildTaskModelGroups(data ?? [], providers)')).toBe(true);
    // Error fail-safe: an errored resolve must read as loading, never as an
    // authoritative empty catalog.
    expect(source.includes('!Array.isArray(data)')).toBe(true);
  });
});
