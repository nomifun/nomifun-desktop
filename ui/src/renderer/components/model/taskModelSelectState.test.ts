/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import type { TaskModelGroup } from '@/renderer/hooks/agent/useModelsForTask';
import { taskModelSelectState } from './taskModelSelectState';

const providerId = (suffix: string) => `0190f5fe-7c00-7a00-8000-0000000000${suffix}` as ProviderId;
const A = providerId('a1');
const B = providerId('b2');

const provider = (id: ProviderId, name: string): IProvider =>
  ({ id, name, platform: 'custom', enabled: true }) as unknown as IProvider;

const group = (id: ProviderId, name: string, models: string[]): TaskModelGroup => ({
  provider: provider(id, name),
  models,
});

describe('taskModelSelectState', () => {
  test('task scope lists only providers that can do the task; all-enabled lists them all', () => {
    const groups = [group(A, 'A', ['m1', 'm2'])];
    const enabledProviders = [provider(A, 'A'), provider(B, 'B')];

    const task = taskModelSelectState({
      groups,
      enabledProviders,
      scope: 'task',
      value: null,
      draftProviderId: null,
      isLoading: false,
    });
    expect(task.providers.map((p) => p.id)).toEqual([A]);

    const all = taskModelSelectState({
      groups,
      enabledProviders,
      scope: 'all-enabled',
      value: null,
      draftProviderId: null,
      isLoading: false,
    });
    expect(all.providers.map((p) => p.id)).toEqual([A, B]);
    // A provider with no task-capable model yields an empty model list rather
    // than disappearing — that is what lets the row explain itself.
    expect(
      taskModelSelectState({
        groups,
        enabledProviders,
        scope: 'all-enabled',
        value: null,
        draftProviderId: B,
        isLoading: false,
      }).models
    ).toEqual([]);
  });

  test('a deleted provider is stale, a vanished model is stale, and both are reported separately', () => {
    const groups = [group(A, 'A', ['m1'])];
    const enabledProviders = [provider(A, 'A')];

    const providerGone = taskModelSelectState({
      groups,
      enabledProviders,
      scope: 'all-enabled',
      value: { provider_id: B, model: 'm9' },
      draftProviderId: B,
      isLoading: false,
    });
    expect(providerGone.providerStale).toBe(true);
    expect(providerGone.modelStale).toBe(false);
    expect(providerGone.configured).toBe(false);

    const modelGone = taskModelSelectState({
      groups,
      enabledProviders,
      scope: 'all-enabled',
      value: { provider_id: A, model: 'retired' },
      draftProviderId: A,
      isLoading: false,
    });
    expect(modelGone.providerStale).toBe(false);
    expect(modelGone.modelStale).toBe(true);
    expect(modelGone.configured).toBe(false);

    const good = taskModelSelectState({
      groups,
      enabledProviders,
      scope: 'all-enabled',
      value: { provider_id: A, model: 'm1' },
      draftProviderId: A,
      isLoading: false,
    });
    expect(good.configured).toBe(true);
    expect(good.modelStale).toBe(false);
    expect(good.anyModel).toBe(true);
  });

  test('nothing is stale while the catalog is still loading', () => {
    // useModelsForTask keeps isLoading true whenever `data` is not an array, so
    // a failed resolve arrives here as "unknown", not as an empty catalog. If
    // this leaked through as staleness the row would tell the user to re-pick a
    // model that is perfectly fine — and the next click would overwrite a good
    // saved reference.
    const loading = taskModelSelectState({
      groups: [],
      enabledProviders: [],
      scope: 'task',
      value: { provider_id: A, model: 'm1' },
      draftProviderId: A,
      isLoading: true,
    });
    expect(loading.providerStale).toBe(false);
    expect(loading.modelStale).toBe(false);
    expect(loading.anyModel).toBe(false);
    expect(loading.configured).toBe(false);
  });

  test('a draft provider switch does not report the saved model as this provider’s', () => {
    const groups = [group(A, 'A', ['m1']), group(B, 'B', ['m2'])];
    const state = taskModelSelectState({
      groups,
      enabledProviders: [provider(A, 'A'), provider(B, 'B')],
      scope: 'task',
      value: { provider_id: A, model: 'm1' },
      draftProviderId: B,
      isLoading: false,
    });
    // The user picked provider B but has not picked a model yet: the model
    // select must be empty, not showing A's saved model under B.
    expect(state.models).toEqual(['m2']);
    expect(state.modelStale).toBe(false);
    expect(state.configured).toBe(false);
  });
});
