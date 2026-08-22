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
  requestKey: string;
  manifests: ModelProtocolManifestMap;
  errorTasks: ModelTask[];
}

const orderedUniqueTasks = (tasks: readonly ModelTask[]): ModelTask[] => [...new Set(tasks)];
const EMPTY_TASKS: readonly ModelTask[] = [];
const EMPTY_MANIFESTS: ModelProtocolManifestMap = {};

/**
 * Fetch provider-preset × task protocol manifests from the backend registry.
 * Custom and New API compatibility defaults are task-scoped and no longer wait
 * for model-id input, so typing a model never reloads or temporarily hides the
 * manifest.
 */
export interface UseModelProtocolManifestsOptions {
  preset?: string;
  tasks: readonly ModelTask[];
  bootstrapTask?: ModelTask;
  baseUrlHint?: string;
}

export const useModelProtocolManifests = ({
  preset,
  tasks,
  bootstrapTask,
  baseUrlHint,
}: UseModelProtocolManifestsOptions) => {
  const requestedTasks = useMemo(
    () => orderedUniqueTasks([...(bootstrapTask ? [bootstrapTask] : []), ...tasks]),
    [bootstrapTask, tasks]
  );
  const requestKey = JSON.stringify([preset ?? '', baseUrlHint ?? '', requestedTasks]);
  const swrKey =
    preset && requestedTasks.length > 0
      ? ['model-protocol-manifests', requestKey]
      : null;
  const state = useSWR<ManifestBatch>(
    swrKey,
    async () => {
      const settled = await Promise.allSettled(
        requestedTasks.map((task) =>
          ipcBridge.modelProtocol.list.invoke({
            preset: preset!,
            task,
            ...(baseUrlHint ? { base_url: baseUrlHint } : {}),
          })
        )
      );
      const manifests: ModelProtocolManifestMap = {};
      const errorTasks: ModelTask[] = [];
      settled.forEach((result, index) => {
        const task = requestedTasks[index];
        if (result.status === 'fulfilled') manifests[task] = result.value as ModelProtocolManifest;
        else errorTasks.push(task);
      });
      return { requestKey, manifests, errorTasks };
    },
    { keepPreviousData: true, revalidateOnFocus: false }
  );

  const current = state.data?.requestKey === requestKey ? state.data : undefined;
  const manifestPending = swrKey !== null && !current;

  return {
    manifests: current?.manifests ?? EMPTY_MANIFESTS,
    errorTasks: current?.errorTasks ?? EMPTY_TASKS,
    loadingTasks: manifestPending ? requestedTasks : EMPTY_TASKS,
    mutate: state.mutate,
  };
};

export default useModelProtocolManifests;
