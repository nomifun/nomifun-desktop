/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Resolve the models a generation card can pick, by mode:
 *  - image → providers' `image_generation`-capable models (M6 heuristic + override)
 *  - video → providers' `video_generation`-capable models
 *  - text  → providers' conversation models (the Model Hub "available" set, which
 *            already excludes image/video generators via `excludeFromPrimary`)
 *
 * Grouped by provider and flattened, plus a `hasProviders` signal so the picker
 * can tell "no platforms configured" apart from "no matching models".
 */

import { useMemo } from 'react';
import { useProvidersQuery } from '@renderer/hooks/agent/useModelProviderList';
import { useModelsForTask } from '@renderer/hooks/agent/useModelsForTask';
import { useModelProfiles } from '@renderer/hooks/agent/useModelProfiles';
import { getCreationModels } from '@renderer/pages/modelHub/creationModels';
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

export function useGeneratorModels(mode: GenMode): GeneratorModels {
  const { data: rawProviders } = useProvidersQuery();
  // text 模式的对话模型来自统一 chat catalog resolve（无名称启发式）。
  const { groups: chatGroups } = useModelsForTask('chat');
  const { profiles } = useModelProfiles();
  const providerLabel = useModelSelectorProviderLabel();

  return useMemo<GeneratorModels>(() => {
    const hasProviders =
      (rawProviders ?? []).some((p) => p.enabled !== false && (p.models ?? []).length > 0) || chatGroups.length > 0;

    if (mode === 'text') {
      const flat: ModelOption[] = [];
      for (const { provider, models } of chatGroups) {
        for (const model of models) {
          flat.push({ providerId: provider.id, providerName: providerLabel(provider), platform: provider.platform, model });
        }
      }
      return { groups: group(flat), flat, hasProviders };
    }

    const cap = mode === 'video' ? 'video_generation' : 'image_generation';
    const flat: ModelOption[] = getCreationModels(rawProviders, cap, profiles).map((e) => ({
      providerId: e.providerId,
      providerName: providerLabel({ name: e.providerName, platform: e.platform }),
      platform: e.platform,
      model: e.model,
    }));
    return { groups: group(flat), flat, hasProviders };
  }, [mode, rawProviders, chatGroups, profiles, providerLabel]);
}
