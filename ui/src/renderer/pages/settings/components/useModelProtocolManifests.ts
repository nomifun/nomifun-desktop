/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import { useMemo } from 'react';
import useSWR from 'swr';
import type { ModelProtocolManifest, ModelProtocolManifestMap } from './providerModelAdvanced';

interface ManifestBatch {
  manifests: ModelProtocolManifestMap;
  errorTasks: ModelTask[];
}

const orderedUniqueTasks = (tasks: readonly ModelTask[]): ModelTask[] => [...new Set(tasks)];

/**
 * Fetch provider-preset × task protocol manifests from the backend registry.
 * No frontend protocol table or platform heuristic participates in the result.
 */
export const useModelProtocolManifests = (
  preset: string | undefined,
  tasks: readonly ModelTask[],
  bootstrapTask?: ModelTask,
  baseUrlHint?: string
) => {
  const requestedTasks = useMemo(
    () => orderedUniqueTasks([...(bootstrapTask ? [bootstrapTask] : []), ...tasks]),
    [bootstrapTask, tasks]
  );
  const taskKey = requestedTasks.join(',');
  const state = useSWR<ManifestBatch>(
    preset && requestedTasks.length > 0 ? ['model-protocol-manifests', preset, baseUrlHint ?? '', taskKey] : null,
    async () => {
      const settled = await Promise.allSettled(
        requestedTasks.map((task) =>
          ipcBridge.modelProtocol.list.invoke({ preset: preset!, task, ...(baseUrlHint ? { base_url: baseUrlHint } : {}) })
        )
      );
      const manifests: ModelProtocolManifestMap = {};
      const errorTasks: ModelTask[] = [];
      settled.forEach((result, index) => {
        const task = requestedTasks[index];
        if (result.status === 'fulfilled') manifests[task] = result.value as ModelProtocolManifest;
        else errorTasks.push(task);
      });
      return { manifests, errorTasks };
    },
    { keepPreviousData: true, revalidateOnFocus: false }
  );

  return {
    manifests: state.data?.manifests ?? {},
    errorTasks: state.data?.errorTasks ?? [],
    loadingTasks: state.isLoading ? requestedTasks : [],
    mutate: state.mutate,
  };
};

export default useModelProtocolManifests;
