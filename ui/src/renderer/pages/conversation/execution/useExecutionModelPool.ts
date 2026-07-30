import type { TExecutionModelPool, TExecutionModelRef } from '@/common/types/agentExecution/agentExecutionTypes';
import type { IProvider } from '@/common/config/storage';
import { useModelProviderList } from '@/renderer/hooks/agent/useModelProviderList';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { useCallback, useMemo } from 'react';
import { parseProviderId } from '@/common/types/ids';

const PAIR_SEPARATOR = '\u0000';

export const encodePair = (ref: TExecutionModelRef): string => `${ref.provider_id}${PAIR_SEPARATOR}${ref.model}`;

export const decodePair = (value: string): TExecutionModelRef => {
  const separatorIndex = value.indexOf(PAIR_SEPARATOR);
  return {
    provider_id: parseProviderId(value.slice(0, separatorIndex)),
    model: value.slice(separatorIndex + PAIR_SEPARATOR.length),
  };
};

export type ExecutionModelMode = 'single' | 'automatic' | 'range';

export type ExecutionModelPoolSource = {
  mode: ExecutionModelMode;
  single?: string;
  range?: string[];
};

export function useExecutionModelPool() {
  const { configuredProviders, isLoading: isProvidersLoading, formatModelLabel } = useModelProviderList();
  // Selectable execution models come from the unified chat catalog resolve —
  // the same source as every other chat-family selector (heuristics gone).
  const { groups, isLoading: isCatalogLoading } = useModelsForTask('chat');

  const providers = useMemo(() => groups.map((group) => group.provider), [groups]);
  const modelsByProvider = useMemo(
    () => new Map(groups.map((group) => [group.provider.id, group.models])),
    [groups]
  );
  const getAvailableModels = useCallback(
    (provider: IProvider): string[] => modelsByProvider.get(provider.id) ?? [],
    [modelsByProvider]
  );

  // The full configured universe (raw catalog membership, not task-filtered):
  // used to distinguish "temporarily unavailable" from "removed" refs.
  const configuredPairs = useMemo<TExecutionModelRef[]>(
    () =>
      configuredProviders.flatMap((provider) =>
        (provider.models ?? []).map((model) => ({
          provider_id: provider.id,
          model,
        })),
      ),
    [configuredProviders],
  );

  const allPairs = useMemo<TExecutionModelRef[]>(
    () =>
      groups.flatMap((group) =>
        group.models.map((model) => ({
          provider_id: group.provider.id,
          model,
        })),
      ),
    [groups],
  );

  const buildModelPool = useCallback((source: ExecutionModelPoolSource): TExecutionModelPool | null => {
    if (source.mode === 'automatic') return { mode: 'automatic' };
    if (source.mode === 'single') {
      return source.single ? { mode: 'single', model: decodePair(source.single) } : null;
    }
    const models = (source.range ?? []).map(decodePair);
    return models.length > 0 ? { mode: 'range', models } : null;
  }, []);

  return {
    providers,
    getAvailableModels,
    formatModelLabel,
    // Loading covers BOTH sources: consumers gate destructive reconciliation on
    // this flag, so a failed/unfinished catalog resolve must read as loading.
    isLoading: isProvidersLoading || isCatalogLoading,
    configuredPairs,
    allPairs,
    hasModels: allPairs.length > 0,
    buildModelPool,
  };
}
