/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Resolve the models a generation card can pick, by mode — every mode reads the
 * nested model capability graph through `useModelsForTask` (no name
 * heuristics and no cross-task unions): image_generation, video_generation,
 * chat, and speech_synthesis are each resolved independently.
 *
 * Grouped by provider and flattened, plus a `hasProviders` signal so the picker
 * can tell "no platforms configured" apart from "no matching models".
 */

import { useMemo } from 'react';
import { useProvidersQuery } from '@renderer/hooks/agent/useModelProviderList';
import { useModelsForTask, type TaskModelGroup } from '@renderer/hooks/agent/useModelsForTask';
import type { GenMode, ImageGeneratorTask, ModelGroup, ModelOption } from './genTypes';
import { useModelSelectorProviderLabel } from '@renderer/hooks/agent/useModelSelectorProviderLabel';

export interface GeneratorModels {
  groups: ModelGroup[];
  flat: ModelOption[];
  /** Any enabled provider exposes at least one usable model at all. */
  hasProviders: boolean;
}

export type GeneratorModelTask =
  | 'chat'
  | 'speech_synthesis'
  | 'video_generation'
  | ImageGeneratorTask;

export type GeneratorTaskPools<T> = Record<GeneratorModelTask, readonly T[]>;

function group(flat: ModelOption[]): ModelGroup[] {
  const groups = new Map<string, ModelGroup>();
  for (const m of flat) {
    let g = groups.get(m.providerId);
    if (!g) {
      g = { providerId: m.providerId, providerName: m.providerName, platform: m.platform, models: [] };
      groups.set(m.providerId, g);
    }
    g.models.push(m);
  }
  return [...groups.values()];
}

function flattenTaskGroups(
  taskGroups: readonly TaskModelGroup[],
  providerLabel: (provider: { name?: string; platform?: string }) => string
): ModelOption[] {
  const flat: ModelOption[] = [];
  for (const { provider, models } of taskGroups) {
    for (const model of models) {
      flat.push({ providerId: provider.id, providerName: providerLabel(provider), platform: provider.platform, model });
    }
  }
  return flat;
}

export function generatorTaskForMode(
  mode: GenMode,
  imageTask: ImageGeneratorTask
): GeneratorModelTask {
  if (mode === 'text') return 'chat';
  if (mode === 'tts') return 'speech_synthesis';
  if (mode === 'video') return 'video_generation';
  return imageTask;
}

/** Select one exact pool; a missing task can never fall through to another. */
export function exactGeneratorTaskPool<T>(
  mode: GenMode,
  imageTask: ImageGeneratorTask,
  pools: GeneratorTaskPools<T>
): readonly T[] {
  return pools[generatorTaskForMode(mode, imageTask)];
}

export function useGeneratorModels(
  mode: GenMode,
  imageTask: ImageGeneratorTask
): GeneratorModels {
  const { data: rawProviders } = useProvidersQuery();
  // Every mode reads its exact task capability; image_edit is not generation.
  const { groups: chatGroups } = useModelsForTask('chat');
  const { groups: ttsGroups } = useModelsForTask('speech_synthesis');
  const { groups: imageGenerationGroups } = useModelsForTask('image_generation');
  const { groups: imageEditGroups } = useModelsForTask('image_edit');
  const { groups: videoGroups } = useModelsForTask('video_generation');
  const providerLabel = useModelSelectorProviderLabel();

  return useMemo<GeneratorModels>(() => {
    const hasProviders =
      (rawProviders ?? []).some(
        (provider) =>
          provider.enabled !== false &&
          provider.models.some((model) => model.enabled && model.capabilities.length > 0)
      ) || chatGroups.length > 0;

    const selectedGroups = exactGeneratorTaskPool(mode, imageTask, {
      chat: chatGroups,
      speech_synthesis: ttsGroups,
      video_generation: videoGroups,
      image_generation: imageGenerationGroups,
      image_edit: imageEditGroups,
    });
    const flat = flattenTaskGroups(selectedGroups, providerLabel);
    return { groups: group(flat), flat, hasProviders };
  }, [
    mode,
    imageTask,
    rawProviders,
    chatGroups,
    ttsGroups,
    imageGenerationGroups,
    imageEditGroups,
    videoGroups,
    providerLabel,
  ]);
}
