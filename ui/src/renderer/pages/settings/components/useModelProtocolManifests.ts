/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import { useEffect, useMemo, useState } from 'react';
import useSWR from 'swr';
import type { ModelProtocolManifest, ModelProtocolManifestMap } from './providerModelAdvanced';

interface ManifestBatch {
  requestKey: string;
  manifests: ModelProtocolManifestMap;
  errorTasks: ModelTask[];
}

const orderedUniqueTasks = (tasks: readonly ModelTask[]): ModelTask[] => [...new Set(tasks)];
const MODEL_HINT_DEBOUNCE_MS = 250;

/**
 * Fetch provider-preset × task protocol manifests from the backend registry.
 * Protocol selection stays backend-owned; the frontend only scopes and
 * debounces the optional Custom model hint.
 */
export interface UseModelProtocolManifestsOptions {
  preset?: string;
  tasks: readonly ModelTask[];
  bootstrapTask?: ModelTask;
  baseUrlHint?: string;
  modelHint?: string;
}

export const useModelProtocolManifests = ({
  preset,
  tasks,
  bootstrapTask,
  baseUrlHint,
  modelHint,
}: UseModelProtocolManifestsOptions) => {
  const requestedTasks = useMemo(
    () => orderedUniqueTasks([...(bootstrapTask ? [bootstrapTask] : []), ...tasks]),
    [bootstrapTask, tasks]
  );
  // Only the Custom preset has a model-gated compatibility recommendation.
  // Built-in manifests stay stable while a user types a model id, and new-api
  // keeps its existing explicit-protocol contract.
  const customPreset = preset?.trim().toLowerCase() === 'custom';
  const normalizedModel = customPreset ? modelHint?.trim() || undefined : undefined;
  const [debouncedModel, setDebouncedModel] = useState<string>();
  useEffect(() => {
    if (!normalizedModel) {
      setDebouncedModel((current) => (current === undefined ? current : undefined));
      return;
    }
    const timeout = globalThis.setTimeout(
      () => setDebouncedModel(normalizedModel),
      MODEL_HINT_DEBOUNCE_MS
    );
    return () => globalThis.clearTimeout(timeout);
  }, [normalizedModel]);
  const recommendationModel = customPreset ? debouncedModel : undefined;
  const modelHintPending = customPreset && normalizedModel !== recommendationModel;
  const requestKey = JSON.stringify([
    preset ?? '',
    baseUrlHint ?? '',
    recommendationModel ?? '',
    requestedTasks,
  ]);
  const swrKey =
    preset && requestedTasks.length > 0 && !modelHintPending
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
            ...(recommendationModel ? { model: recommendationModel } : {}),
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

  // `keepPreviousData` is useful when another task is added, but once the model
  // changes it must not let model A's recommendation mutate model B's draft.
  const current =
    !modelHintPending && state.data?.requestKey === requestKey ? state.data : undefined;
  const manifestPending = modelHintPending || (swrKey !== null && !current);

  return {
    manifests: current?.manifests ?? {},
    errorTasks: current?.errorTasks ?? [],
    loadingTasks: manifestPending ? requestedTasks : [],
    mutate: state.mutate,
  };
};

export default useModelProtocolManifests;
