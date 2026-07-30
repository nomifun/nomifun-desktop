/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IProvider, ModelTask, ModelTrait } from '@/common/config/storage';
import type { CatalogModelRef, ResolveModelsRequest } from '@/common/types/provider/providerApi';
import { useCallback, useMemo } from 'react';
import useSWR, { type SWRConfiguration } from 'swr';
import { useModelProviderList } from './useModelProviderList';

/** One provider's task-capable models, in catalog (sort_order) order. */
export interface TaskModelGroup {
  provider: IProvider;
  models: string[];
}

export interface ModelsForTaskResult {
  groups: TaskModelGroup[];
  isLoading: boolean;
  error?: Error;
  refresh: () => void;
}

// Catalog resolution is local application state (same lifecycle as the
// provider list): stable after the initial load, refreshed via explicit
// refresh()/mutate calls after CRUD operations.
const RESOLVE_SWR_OPTIONS: SWRConfiguration<CatalogModelRef[], Error> = {
  revalidateOnFocus: false,
  revalidateOnReconnect: false,
  shouldRetryOnError: false,
};

/**
 * SWR cache key for one (task, traits) resolution. Trait order is normalized
 * so equivalent trait sets share one cache entry.
 */
export const modelsForTaskKey = (task: ModelTask, requiredTraits?: ModelTrait[]): string =>
  `models-for-task:${task}:${[...(requiredTraits ?? [])].sort().join(',')}`;

/** Request body for `modelProfile.resolve` (trait filtering is server-side). */
export const buildResolveModelsRequest = (
  task: ModelTask,
  requiredTraits?: ModelTrait[]
): ResolveModelsRequest =>
  requiredTraits && requiredTraits.length > 0 ? { task, required_traits: requiredTraits } : { task };

/**
 * Group resolved (provider, model) refs by provider, preserving BOTH the
 * provider ordering of `providers` (the selector ordering authority) and the
 * backend catalog ordering of each provider's models. Refs whose provider is
 * not in `providers` (deleted/unknown metadata) are dropped; providers without
 * any matching model produce no group.
 */
export const buildTaskModelGroups = (
  refs: readonly CatalogModelRef[],
  providers: readonly IProvider[]
): TaskModelGroup[] => {
  const modelsByProvider = new Map<string, string[]>();
  for (const ref of refs) {
    const models = modelsByProvider.get(ref.provider_id);
    if (!models) {
      modelsByProvider.set(ref.provider_id, [ref.model]);
    } else if (!models.includes(ref.model)) {
      models.push(ref.model);
    }
  }
  const groups: TaskModelGroup[] = [];
  for (const provider of providers) {
    const models = modelsByProvider.get(provider.id);
    if (models && models.length > 0) {
      groups.push({ provider, models });
    }
  }
  return groups;
};

/**
 * Unified "which models can do this task" hook — the single source every
 * chat-family model selector reads. Resolves the authoritative catalog via
 * `POST /api/model-profiles/resolve` (profiles + backend inference; no
 * frontend name heuristics) and joins provider metadata from
 * `useModelProviderList`.
 */
export function useModelsForTask(task: ModelTask, requiredTraits?: ModelTrait[]): ModelsForTaskResult {
  const { providers, isLoading: isProvidersLoading } = useModelProviderList();

  const key = modelsForTaskKey(task, requiredTraits);
  const { data, error, isLoading, mutate } = useSWR<CatalogModelRef[]>(
    key,
    async () => {
      const response = await ipcBridge.modelProfile.resolve.invoke(
        buildResolveModelsRequest(task, requiredTraits)
      );
      return response?.models ?? [];
    },
    RESOLVE_SWR_OPTIONS
  );

  const groups = useMemo(() => buildTaskModelGroups(data ?? [], providers), [data, providers]);

  const refresh = useCallback(() => {
    void mutate();
  }, [mutate]);

  return {
    groups,
    // Mirror useModelProviderList's fail-safe: after an error SWR clears
    // `isLoading` while `data` stays undefined. Keep the catalog unresolved in
    // that state so consumers never treat a failed resolve as an authoritative
    // empty catalog (and e.g. purge persisted model references).
    isLoading: isLoading || isProvidersLoading || !Array.isArray(data),
    error,
    refresh,
  };
}
