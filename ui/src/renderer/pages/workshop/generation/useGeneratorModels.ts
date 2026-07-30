/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Resolve the models a generation card can pick, by mode — every mode reads the
 * authoritative catalog (`useModelsForTask` / `useCreationModels`, i.e.
 * `POST /api/model-profiles/resolve`; no name heuristics):
 *  - image → resolve(image_generation) ∪ resolve(image_edit)
 *  - video → resolve(video_generation)
 *  - text  → resolve(chat)
 *  - tts   → resolve(speech_synthesis)
 *
 * Grouped by provider and flattened, plus a `hasProviders` signal so the picker
 * can tell "no platforms configured" apart from "no matching models".
 */

import { useMemo } from 'react';
import { useProvidersQuery } from '@renderer/hooks/agent/useModelProviderList';
import { useModelsForTask, type TaskModelGroup } from '@renderer/hooks/agent/useModelsForTask';
import { filterCreationModels, useCreationModels } from '@renderer/pages/modelHub/creationModels';
import type { GenMode, ModelGroup, ModelOption } from './genTypes';
import { useModelSelectorProviderLabel } from '@renderer/hooks/agent/useModelSelectorProviderLabel';

export interface GeneratorModels {
  groups: ModelGroup[];
  flat: ModelOption[];
  /** Any enabled provider exposes at least one usable model at all. */
  hasProviders: boolean;
}

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

export function useGeneratorModels(mode: GenMode): GeneratorModels {
  const { data: rawProviders } = useProvidersQuery();
  // 对话/语音合成模型来自统一 task catalog resolve（无名称启发式）。
  const { groups: chatGroups } = useModelsForTask('chat');
  const { groups: ttsGroups } = useModelsForTask('speech_synthesis');
  const { entries: creationEntries } = useCreationModels();
  const providerLabel = useModelSelectorProviderLabel();

  return useMemo<GeneratorModels>(() => {
    const hasProviders =
      (rawProviders ?? []).some((p) => p.enabled !== false && (p.models ?? []).length > 0) || chatGroups.length > 0;

    if (mode === 'text' || mode === 'tts') {
      const flat = flattenTaskGroups(mode === 'text' ? chatGroups : ttsGroups, providerLabel);
      return { groups: group(flat), flat, hasProviders };
    }

    const cap = mode === 'video' ? 'video_generation' : 'image_generation';
    const flat: ModelOption[] = filterCreationModels(creationEntries, cap).map((e) => ({
      providerId: e.providerId,
      providerName: e.providerName,
      platform: e.platform,
      model: e.model,
    }));
    return { groups: group(flat), flat, hasProviders };
  }, [mode, rawProviders, chatGroups, ttsGroups, creationEntries, providerLabel]);
}
