/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider, ModelTask, ModelTrait } from '@/common/config/storage';
import { modelSupportsTask } from '@/common/utils/providerModels';
import { useCallback, useMemo } from 'react';
import { useProvidersQuery } from './useModelProviderList';
import { orderModelSelectorProviders } from './modelSelectorProviderOrdering';

/** One enabled provider's enabled task-capable models, in stored model order. */
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

/**
 * Resolve a runtime model list from the nested provider response. Task and
 * traits are matched against the same capability object; disabled providers
 * and disabled models are never runnable.
 */
export const buildTaskModelGroups = (
  providers: readonly IProvider[],
  task: ModelTask,
  requiredTraits: readonly ModelTrait[] = []
): TaskModelGroup[] =>
  orderModelSelectorProviders(providers.filter((provider) => provider.enabled !== false)).flatMap(
    (provider) => {
      const models = provider.models
        .filter((model) => model.enabled && modelSupportsTask(model, task, requiredTraits))
        .map((model) => model.model);
      return models.length === 0 ? [] : [{ provider, models }];
    }
  );

/**
 * Single runtime selector source for every modality. The provider response
 * already contains the complete model/capability graph, so no second profile
 * request, bridge join, name heuristic, or fallback cache is involved.
 */
export function useModelsForTask(task: ModelTask, requiredTraits?: ModelTrait[]): ModelsForTaskResult {
  const { data, error, isLoading, mutate } = useProvidersQuery();

  const groups = useMemo(
    () => buildTaskModelGroups(data ?? [], task, requiredTraits),
    [data, requiredTraits, task]
  );

  const refresh = useCallback(() => {
    void mutate();
  }, [mutate]);

  return {
    groups,
    // A failed provider request is unresolved, never an authoritative empty
    // model catalog that may cause a caller to clear a persisted selection.
    isLoading: isLoading || !Array.isArray(data),
    error,
    refresh,
  };
}
