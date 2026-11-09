/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  IProvider,
  ModelTask,
  ModelTechnicalCapability,
  ModelTrait,
} from '@/common/config/storage';
import { modelSupportsTask } from '@/common/utils/providerModels';
import { useCallback, useMemo } from 'react';
import { useProvidersQuery } from './useModelProviderList';
import { useTaskModelProtocols, type TaskModelProtocolRequirements } from './useTaskModelProtocols';
export type { TaskModelProtocolRequirements } from './useTaskModelProtocols';

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
 * adapter input requirements match the same capability object. Disabled
 * providers and disabled models are never runnable.
 */
export const buildTaskModelGroups = (
  providers: readonly IProvider[],
  task: ModelTask,
  requiredTraits: readonly ModelTrait[] = [],
  requiredTechnicalCapabilities: readonly ModelTechnicalCapability[] = [],
  allowedProtocols?: ReadonlySet<string>
): TaskModelGroup[] =>
  providers
    .filter((provider) => provider.enabled !== false)
    .flatMap((provider) => {
      const models = provider.models
        .filter(
          (model) =>
            model.enabled &&
            modelSupportsTask(model, task, requiredTraits, requiredTechnicalCapabilities) &&
            (!allowedProtocols || model.capabilities.some(capability => capability.task === task && allowedProtocols.has(capability.protocol)))
        )
        .map((model) => model.model);
      return models.length === 0 ? [] : [{ provider, models }];
    });

/**
 * Single runtime selector source for every modality. The provider response
 * already contains the complete model/capability graph. Execution-scoped
 * callers additionally constrain the saved protocol using the server registry.
 */
export function useModelsForTask(
  task: ModelTask,
  requiredTraits?: ModelTrait[],
  requiredTechnicalCapabilities?: ModelTechnicalCapability[],
  requiredProtocols?: TaskModelProtocolRequirements
): ModelsForTaskResult {
  const { data, error, isLoading, mutate } = useProvidersQuery();
  const protocolScope = useTaskModelProtocols(task, requiredProtocols);

  const groups = useMemo(
    () => protocolScope.isLoading ? [] : buildTaskModelGroups(data ?? [], task, requiredTraits, requiredTechnicalCapabilities, protocolScope.protocols),
    [data, requiredTechnicalCapabilities, requiredTraits, task, protocolScope.isLoading, protocolScope.protocols]
  );

  const refresh = useCallback(() => {
    void mutate().catch(() => undefined);
    void protocolScope.refresh().catch(() => undefined);
  }, [mutate, protocolScope.refresh]);

  return {
    groups,
    // A failed provider request is unresolved, never an authoritative empty
    // model catalog that may cause a caller to clear a persisted selection.
    isLoading: isLoading || !Array.isArray(data) || protocolScope.isLoading,
    error: error ?? protocolScope.error,
    refresh,
  };
}
