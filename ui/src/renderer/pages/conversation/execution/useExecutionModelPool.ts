import type { TExecutionModelPool, TExecutionModelRef } from '@/common/types/agentExecution/agentExecutionTypes';
import { useModelProviderList } from '@/renderer/hooks/agent/useModelProviderList';
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
  const { providers, configuredProviders, isLoading, getAvailableModels, formatModelLabel } = useModelProviderList();

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
      providers.flatMap((provider) =>
        getAvailableModels(provider).map((model) => ({
          provider_id: provider.id,
          model,
        })),
      ),
    [providers, getAvailableModels],
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
    isLoading,
    configuredPairs,
    allPairs,
    hasModels: allPairs.length > 0,
    buildModelPool,
  };
}
