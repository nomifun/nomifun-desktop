/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider, TProviderWithModel } from '@/common/config/storage';
import type { ConfigKeyMap } from '@/common/config/configKeys';
import { configService } from '@/common/config/configService';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

/**
 * Build a unique key for a provider/model pair.
 */
const buildModelKey = (providerId?: string, modelName?: string) => {
  if (!providerId || !modelName) return null;
  return `${providerId}:${modelName}`;
};

/** Provider-based agent keys that share the model list UI */
type ProviderAgentKey = 'nomi';

/** Map agent key → storage key for persisting default model */
const MODEL_STORAGE_KEY: Record<ProviderAgentKey, 'nomi.defaultModel'> = {
  nomi: 'nomi.defaultModel',
};

type PersistedDefaultModel = NonNullable<ConfigKeyMap['nomi.defaultModel']>;

function isPersistedDefaultModel(value: unknown): value is PersistedDefaultModel {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const object = value as Record<string, unknown>;
  return (
    !('id' in object) &&
    typeof object.provider_id === 'string' &&
    typeof object.model === 'string'
  );
}

export type GuidModelSelectionResult = {
  modelList: IProvider[];
  formatGeminiModelLabel: (provider: { platform?: string } | undefined, modelName?: string) => string;
  current_model: TProviderWithModel | undefined;
  setCurrentModel: (model_info: TProviderWithModel) => Promise<void>;
};

/**
 * Hook that manages the model list and selection state for the Guid page.
 * @param agentKey - current provider-based agent (currently only 'nomi')
 */
export const useGuidModelSelection = (agentKey: ProviderAgentKey = 'nomi'): GuidModelSelectionResult => {
  // Chat-capable catalog from the unified backend resolve — replaces the old
  // duplicate guid/utils/modelUtils name-heuristic implementation.
  const { groups } = useModelsForTask('chat');

  const modelList = useMemo(() => groups.map((group) => group.provider), [groups]);
  const modelsByProvider = useMemo(
    () => new Map(groups.map((group) => [group.provider.id, group.models])),
    [groups]
  );

  const availableModelsFor = useCallback(
    (provider: IProvider | undefined): string[] =>
      provider ? (modelsByProvider.get(provider.id) ?? []) : [],
    [modelsByProvider]
  );

  /** Check if a model key still exists in the chat catalog. */
  const isModelKeyAvailable = useCallback(
    (key: string | null): boolean => {
      if (!key) return false;
      return groups.some((group) =>
        group.models.some((modelName) => buildModelKey(group.provider.id, modelName) === key)
      );
    },
    [groups]
  );

  const formatGeminiModelLabel = useCallback((_provider: { platform?: string } | undefined, modelName?: string) => {
    if (!modelName) return '';
    return modelName;
  }, []);

  const [current_model, _setCurrentModel] = useState<TProviderWithModel>();
  const selectedModelKeyRef = useRef<string | null>(null);
  const prevStorageKeyRef = useRef<string | null>(null);

  const storageKey = MODEL_STORAGE_KEY[agentKey];

  const setCurrentModel = useCallback(
    async (model_info: TProviderWithModel, persist = true) => {
      selectedModelKeyRef.current = buildModelKey(model_info.id, model_info.use_model);
      if (persist) {
        await configService.set(storageKey, {
          provider_id: model_info.id,
          model: model_info.use_model,
        }).catch((error) => {
          console.error('Failed to save default model:', error);
        });
      }
      _setCurrentModel(model_info);
    },
    [storageKey]
  );

  // Set default model when modelList or agent changes
  useEffect(() => {
    const setDefaultModel = async () => {
      if (!modelList || modelList.length === 0) {
        return;
      }
      // When agent switches, reset selection so we reload from the new storage key
      const agentChanged = prevStorageKeyRef.current !== null && prevStorageKeyRef.current !== storageKey;
      prevStorageKeyRef.current = storageKey;
      if (agentChanged) {
        selectedModelKeyRef.current = null;
      }

      const currentKey = selectedModelKeyRef.current || buildModelKey(current_model?.id, current_model?.use_model);
      if (!agentChanged && isModelKeyAvailable(currentKey)) {
        if (!selectedModelKeyRef.current && currentKey) {
          selectedModelKeyRef.current = currentKey;
        }
        return;
      }
      const rawSavedModel: unknown = configService.get(storageKey);
      const savedModel = isPersistedDefaultModel(rawSavedModel) ? rawSavedModel : undefined;
      const canPersistFallback = rawSavedModel === undefined || savedModel !== undefined;
      if (rawSavedModel !== undefined && savedModel === undefined) {
        console.warn(`Ignoring invalid persisted default model for ${storageKey}; no legacy migration is performed.`);
      }

      // First-available enabled model — the fallback whenever nothing valid was
      // saved. `modelList` mirrors the catalog groups (only providers with at
      // least one chat-capable model), so the first provider is guaranteed to
      // expose at least one selectable model. Use the CATALOG list the picker
      // shows rather than raw `provider.models[0]`, which can be a model that
      // is not chat-capable and thus never appears in the picker — picking it
      // would leave current_model pointing at an unselectable model. This
      // guarantees the lead (主管) model is always set and editable, so submit
      // is never silently blocked in auto/range mode.
      const firstProvider = modelList[0];
      const firstAvailableModel = availableModelsFor(firstProvider)[0] ?? '';

      let defaultModel: IProvider | undefined;
      let resolvedUseModel: string;

      if (savedModel) {
        const { provider_id, model } = savedModel;
        const exactMatch = modelList.find((m) => m.id === provider_id);
        if (exactMatch && availableModelsFor(exactMatch).includes(model)) {
          defaultModel = exactMatch;
          resolvedUseModel = model;
        } else {
          defaultModel = firstProvider;
          resolvedUseModel = firstAvailableModel;
        }
      } else {
        defaultModel = firstProvider;
        resolvedUseModel = firstAvailableModel;
      }

      if (!defaultModel || !resolvedUseModel) return;

      await setCurrentModel({
        ...defaultModel,
        use_model: resolvedUseModel,
      }, canPersistFallback);
    };

    setDefaultModel().catch((error) => {
      console.error('Failed to set default model:', error);
    });
    // availableModelsFor / isModelKeyAvailable derive from the same catalog
    // groups as modelList, so modelList is the single change signal.
  }, [modelList, storageKey]);

  return {
    modelList,
    formatGeminiModelLabel,
    current_model,
    setCurrentModel,
  };
};
