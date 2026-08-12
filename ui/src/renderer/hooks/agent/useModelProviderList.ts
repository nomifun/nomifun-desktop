import { ipcBridge } from '@/common';
import type { IProvider } from '@/common/config/storage';
import { useCallback, useMemo } from 'react';
import useSWR, { type SWRConfiguration } from 'swr';
import { orderModelSelectorProviders } from './modelSelectorProviderOrdering';

export interface ModelProviderListResult {
  /** Enabled providers in selector order. Task membership is filtered from
   * each provider's nested model capabilities by `useModelsForTask`. */
  providers: IProvider[];
  configuredProviders: IProvider[];
  isLoading: boolean;
  formatModelLabel: (provider: { platform?: string } | undefined, modelName?: string) => string;
}

export const PROVIDERS_SWR_KEY = 'providers';

// Provider config is local application state. Keep it stable after the initial
// load and refresh only through explicit mutate() calls after CRUD operations.
export const PROVIDERS_SWR_OPTIONS: SWRConfiguration<IProvider[], Error> = {
  revalidateOnFocus: false,
  revalidateOnReconnect: false,
  shouldRetryOnError: false,
};

export const fetchProviders = async (): Promise<IProvider[]> => {
  return (await ipcBridge.mode.listProviders.invoke()) ?? [];
};

export const useProvidersQuery = () => {
  return useSWR<IProvider[]>(PROVIDERS_SWR_KEY, fetchProviders, PROVIDERS_SWR_OPTIONS);
};

/**
 * Shared hook that builds the provider list and exposes provider
 * metadata/label helpers. Task-capable MODEL lists are
 * resolved by `useModelsForTask` from the nested provider response — this hook
 * deliberately no longer filters models by capability name heuristics.
 */
export const useModelProviderList = (): ModelProviderListResult => {
  const { data: modelConfig, isLoading: isProvidersLoading } = useProvidersQuery();

  const configuredProviders = useMemo(() => {
    const list: IProvider[] = Array.isArray(modelConfig) ? modelConfig : [];
    return list;
  }, [modelConfig]);

  const providers = useMemo(() => {
    // 过滤掉被禁用的 provider（默认为启用）。
    // 注意：不再按「是否有可用模型」过滤 —— 模型级别的可用性由
    // useModelsForTask（嵌套 task capability）决定，空组不会被渲染。
    return orderModelSelectorProviders(configuredProviders.filter((p) => p.enabled !== false));
  }, [configuredProviders]);

  const formatModelLabel = useCallback((_provider: { platform?: string } | undefined, modelName?: string) => {
    if (!modelName) return '';
    return modelName;
  }, []);

  return {
    providers,
    configuredProviders,
    // SWR clears `isLoading` after an error while `data` stays undefined. Keep
    // the catalog unresolved in that state so consumers never reinterpret a
    // failed provider request as an authoritative empty catalog and purge every
    // persisted model reference.
    isLoading: isProvidersLoading || !Array.isArray(modelConfig),
    formatModelLabel,
  };
};
