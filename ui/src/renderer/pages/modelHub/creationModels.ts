/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Creative Workshop model discovery.
 *
 * Answers "which providers/models can generate images / videos?" for the
 * Model Hub 创作模型 view AND for the workshop generation card. The signal is
 * the authoritative catalog resolution (`useModelsForTask` →
 * `POST /api/model-profiles/resolve`) — per-model task tags maintained on the
 * model management page. The former three-tier fallback (profile >
 * provider-level override > model-name heuristics) is gone: resolve is the
 * single source, with `image_edit` folded into the workshop's
 * `image_generation` capability (an edit-capable model can also produce
 * images, so the image picker offers it).
 */

import { useMemo } from 'react';
import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import { useModelsForTask, type TaskModelGroup } from '@/renderer/hooks/agent/useModelsForTask';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';

/** The two Creative-Workshop generation capabilities. */
export type CreationCapability = 'image_generation' | 'video_generation';

export const CREATION_CAPABILITIES: CreationCapability[] = ['image_generation', 'video_generation'];

/** One generation-capable model resolved against a provider. */
export interface CreationModelEntry {
  providerId: ProviderId;
  providerName: string;
  platform: string;
  model: string;
  /** Non-empty subset of {@link CreationCapability}. */
  capabilities: CreationCapability[];
}

/** Generation-capable models grouped under their provider. */
export interface CreationProviderGroup {
  providerId: ProviderId;
  providerName: string;
  platform: string;
  models: CreationModelEntry[];
}

/** One task resolution feeding a workshop capability. */
export interface CreationModelSource {
  capability: CreationCapability;
  groups: readonly TaskModelGroup[];
}

/**
 * Union per-task catalog resolutions into per-model creation entries. A model
 * resolved by several sources (e.g. `image_generation` + `image_edit`, or
 * image + video) yields ONE entry carrying the merged capability set.
 * Providers keep their first-seen order across sources; each provider's models
 * keep the backend catalog order of their first source.
 */
export const buildCreationModelEntries = (
  sources: readonly CreationModelSource[],
  providerName: (provider: IProvider) => string = (provider) => provider.name
): CreationModelEntry[] => {
  const providerOrder: IProvider[] = [];
  const modelsByProvider = new Map<string, Map<string, Set<CreationCapability>>>();

  for (const { capability, groups } of sources) {
    for (const { provider, models } of groups) {
      let providerModels = modelsByProvider.get(provider.id);
      if (!providerModels) {
        providerModels = new Map();
        modelsByProvider.set(provider.id, providerModels);
        providerOrder.push(provider);
      }
      for (const model of models) {
        let capabilities = providerModels.get(model);
        if (!capabilities) {
          capabilities = new Set();
          providerModels.set(model, capabilities);
        }
        capabilities.add(capability);
      }
    }
  }

  const out: CreationModelEntry[] = [];
  for (const provider of providerOrder) {
    const providerModels = modelsByProvider.get(provider.id);
    if (!providerModels) continue;
    for (const [model, capabilities] of providerModels) {
      out.push({
        providerId: provider.id,
        providerName: providerName(provider),
        platform: provider.platform,
        model,
        capabilities: CREATION_CAPABILITIES.filter((cap) => capabilities.has(cap)),
      });
    }
  }
  return out;
};

/** Restrict a flat entry list to one capability (undefined = keep all). */
export const filterCreationModels = (
  entries: readonly CreationModelEntry[],
  filter?: CreationCapability
): CreationModelEntry[] =>
  filter ? entries.filter((entry) => entry.capabilities.includes(filter)) : [...entries];

/** Group the flat entry list by provider, preserving provider order. */
export const groupCreationModelsByProvider = (
  entries: readonly CreationModelEntry[]
): CreationProviderGroup[] => {
  const groups = new Map<string, CreationProviderGroup>();
  for (const entry of entries) {
    let group = groups.get(entry.providerId);
    if (!group) {
      group = {
        providerId: entry.providerId,
        providerName: entry.providerName,
        platform: entry.platform,
        models: [],
      };
      groups.set(entry.providerId, group);
    }
    group.models.push(entry);
  }
  return [...groups.values()];
};

export interface CreationModelsResult {
  /** All creation-capable models (image ∪ image_edit ∪ video), catalog order. */
  entries: CreationModelEntry[];
  isLoading: boolean;
}

/**
 * Resolve every creation-capable model from the authoritative catalog
 * (image mode unions the `image_generation` and `image_edit` tasks; video mode
 * is `video_generation`). The three underlying task resolutions are
 * unconditional hook calls (React rules); SWR de-duplicates them across all
 * consumers.
 */
export function useCreationModels(): CreationModelsResult {
  const providerLabel = useModelSelectorProviderLabel();
  const imageGeneration = useModelsForTask('image_generation');
  const imageEdit = useModelsForTask('image_edit');
  const videoGeneration = useModelsForTask('video_generation');

  const entries = useMemo(
    () =>
      buildCreationModelEntries(
        [
          { capability: 'image_generation', groups: imageGeneration.groups },
          { capability: 'image_generation', groups: imageEdit.groups },
          { capability: 'video_generation', groups: videoGeneration.groups },
        ],
        providerLabel
      ),
    [imageGeneration.groups, imageEdit.groups, videoGeneration.groups, providerLabel]
  );

  return {
    entries,
    isLoading: imageGeneration.isLoading || imageEdit.isLoading || videoGeneration.isLoading,
  };
}
