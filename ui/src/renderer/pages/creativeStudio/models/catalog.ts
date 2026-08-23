/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ModelTask } from '@/common/config/storage';
import { capabilityOf } from '@/common/utils/providerModels';
import { modelDisplayLabel, modelPresentationRawId } from '@/common/utils/modelPresentation';
import { buildTaskModelGroups } from '@/renderer/hooks/agent/useModelsForTask';
import { getI18n } from 'react-i18next';

import type {
  CreativeModelCatalogSnapshot,
  CreativeModelCatalogSource,
  CreativeModelFilter,
  CreativeModelGroup,
  CreativeModelModality,
  CreativeModelOption,
  CreativeModelSelectionRef,
  CreativeModelSelectorState,
} from './types';

const MODALITY_TASK: Record<CreativeModelModality, ModelTask> = {
  text: 'chat',
  image: 'image_generation',
  video: 'video_generation',
  audio: 'speech_synthesis',
};

export const creativeModelTaskFor = (filter: CreativeModelFilter): ModelTask =>
  filter.capability === 'task' ? filter.task : MODALITY_TASK[filter.capability];

const fallbackProviderLabel = (provider: { name?: string; platform?: string }): string =>
  provider.name?.trim() ||
  provider.platform?.trim() ||
  getI18n()?.t('creativeStudio.models.select.unknownProvider', { defaultValue: 'Provider' }) ||
  'Provider';

/**
 * Build one exact task pool from NomiFun's nested capability graph. Disabled
 * providers/models and models with only a neighbouring task never enter the
 * result; model-name heuristics are intentionally absent.
 */
export const buildCreativeModelGroups = (
  providers: CreativeModelCatalogSnapshot['providers'],
  filter: CreativeModelFilter,
  providerLabel: (provider: { name?: string; platform?: string }) => string = fallbackProviderLabel
): CreativeModelGroup[] => {
  const task = creativeModelTaskFor(filter);
  const taskGroups = buildTaskModelGroups(providers, task, filter.traits ?? []);

  return taskGroups.map(({ provider, models }) => {
    const label = providerLabel(provider) || fallbackProviderLabel(provider);
    const options: CreativeModelOption[] = models.map((model) => {
      const capability = capabilityOf(provider, model, task);
      if (!capability) {
        // `buildTaskModelGroups` and this lookup share the same authoritative
        // nested row. Reaching this branch would indicate an inconsistent
        // provider snapshot, so fail closed instead of inventing metadata.
        throw new Error(`Missing ${task} capability for ${provider.id}/${model}`);
      }
      return {
        providerId: provider.id,
        providerName: label,
        platform: provider.platform,
        model,
        task,
        traits: capability.traits,
        protocol: capability.protocol,
        displayName: modelDisplayLabel(
          model,
          provider.models.find((candidate) => candidate.model === model)?.display_name
        ),
        rawModelId: modelPresentationRawId(
          model,
          provider.models.find((candidate) => candidate.model === model)?.display_name
        ),
      };
    });
    return {
      providerId: provider.id,
      providerName: label,
      platform: provider.platform,
      models: options,
    };
  });
};

export const flattenCreativeModelGroups = (
  groups: readonly CreativeModelGroup[]
): CreativeModelOption[] => groups.flatMap((group) => group.models);

export const findCreativeModelOption = (
  groups: readonly CreativeModelGroup[],
  value: CreativeModelSelectionRef | null
): CreativeModelOption | null => {
  if (!value) return null;
  return (
    groups
      .find((group) => group.providerId === value.providerId)
      ?.models.find((model) => model.model === value.model) ?? null
  );
};

/** View-state precedence preserves actionable load/error/empty explanations. */
export const creativeModelSelectorState = ({
  catalog,
  groups,
  disabled,
}: {
  catalog: CreativeModelCatalogSnapshot;
  groups: readonly CreativeModelGroup[];
  disabled: boolean;
}): CreativeModelSelectorState => {
  if (catalog.status === 'loading') return 'loading';
  if (catalog.status === 'error') return 'error';
  if (catalog.providers.length === 0) return 'no-provider';
  if (groups.length === 0) return 'no-compatible-model';
  return disabled ? 'disabled' : 'ready';
};

const asError = (value: unknown): Error =>
  value instanceof Error
    ? value
    : new Error(
        typeof value === 'string'
          ? value
          : getI18n()?.t('creativeStudio.models.catalog.loadFailed', {
              defaultValue: '模型目录加载失败',
            }) ?? '模型目录加载失败'
      );

/** Adapt the shared provider query without introducing a second model API. */
export const adaptCreativeModelCatalog = (
  source: CreativeModelCatalogSource
): CreativeModelCatalogSnapshot => {
  if (source.error !== undefined && source.error !== null) {
    return {
      status: 'error',
      providers: source.data ?? [],
      error: asError(source.error),
      refresh: source.refresh,
    };
  }
  if (source.isLoading || !Array.isArray(source.data)) {
    return {
      status: 'loading',
      providers: source.data ?? [],
      error: null,
      refresh: source.refresh,
    };
  }
  return {
    status: 'ready',
    providers: source.data,
    error: null,
    refresh: source.refresh,
  };
};
