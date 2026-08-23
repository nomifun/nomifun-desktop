/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { IProvider, ModelTask, ModelTrait } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import type { ProviderCredentials } from '@/common/types/provider/providerApi';
import { useRef } from 'react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';

export interface FetchedModelOption {
  label: string;
  value: string;
  displayName?: string;
  tasks: ModelTask[];
  traits: ModelTrait[];
  /**
   * Context window the provider's own catalog declared. Absent when it declared
   * none — the app then falls back to its 200k assumption, so a real value here
   * is the only thing that calibrates compaction correctly.
   */
  contextLimit?: number;
}

export interface UseModeModelListOptions {
  platform: string;
  /** Existing providers use the server-side encrypted credentials. */
  providerId?: ProviderId;
  /** The remaining fields are for anonymous discovery before provider creation. */
  baseUrl?: string;
  authScheme?: string;
  credentials?: ProviderCredentials;
  bedrockConfig?: IProvider['bedrock_config'];
  tryFix?: boolean;
}

const sortGeminiModels = (models: FetchedModelOption[]) =>
  models.toSorted((a, b) => {
    const aPro = a.value.toLowerCase().includes('pro');
    const bPro = b.value.toLowerCase().includes('pro');
    if (aPro && !bPro) return -1;
    if (!aPro && bPro) return 1;
    const extractVersion = (name: string) => {
      const match = name.match(/(\d+\.?\d*)/);
      return match ? parseFloat(match[1]) : 0;
    };
    const versionDifference = extractVersion(b.value) - extractVersion(a.value);
    return versionDifference || a.value.localeCompare(b.value);
  });

const useModeModeList = (options: UseModeModelListOptions) => {
  const { t } = useTranslation();
  const credentialState = useRef({ snapshot: '', revision: 0 });
  const credentialSnapshot =
    options.credentials === undefined ? 'missing' : JSON.stringify(options.credentials);
  if (credentialState.current.snapshot !== credentialSnapshot) {
    credentialState.current = {
      snapshot: credentialSnapshot,
      revision: credentialState.current.revision + 1,
    };
  }
  const canFetchStored = Boolean(options.providerId);
  const canFetchAnonymous = Boolean(options.authScheme) && options.credentials !== undefined;
  const cacheKey = canFetchStored
    ? ['provider-models', options.providerId, options.tryFix]
    : canFetchAnonymous
      ? [
          'anonymous-provider-models',
          options.platform,
          options.baseUrl,
          options.authScheme,
          // A local monotonic revision triggers a refresh without placing any
          // credential material (or a reusable hash of it) in SWR's global key.
          credentialState.current.revision,
          options.bedrockConfig,
          options.tryFix,
        ]
      : null;

  return useSWR(
    cacheKey,
    async (): Promise<{ models: FetchedModelOption[]; fix_base_url?: string }> => {
      try {
        const response = options.providerId
          ? await ipcBridge.mode.fetchProviderModels.invoke({
              provider_id: options.providerId,
              try_fix: options.tryFix,
            })
          : await ipcBridge.mode.fetchModelList.invoke({
              platform: options.platform,
              base_url: options.baseUrl ?? '',
              auth_scheme: options.authScheme ?? '',
              credentials: options.credentials ?? {},
              bedrock_config: options.bedrockConfig,
              try_fix: options.tryFix,
            });
        let models = response.models.map((model) => ({
          label: model.name || model.id,
          value: model.id,
          ...(model.name && model.name !== model.id ? { displayName: model.name } : {}),
          tasks: model.tasks ?? [],
          traits: model.traits ?? [],
          // Only present when the provider's own catalog declares a window.
          ...(model.context_limit && model.context_limit > 0
            ? { contextLimit: model.context_limit }
            : {}),
        }));
        if (options.platform.includes('gemini')) {
          models = sortGeminiModels(models);
        }
        return {
          models,
          ...(response.fixed_base_url ? { fix_base_url: response.fixed_base_url } : {}),
        };
      } catch (error) {
        if (isBackendHttpError(error)) {
          switch (error.code) {
            case 'UNAUTHORIZED':
              throw new Error(t('settings.modelCatalogUnauthorized'));
            case 'FORBIDDEN':
              throw new Error(t('settings.modelCatalogForbidden'));
            case 'RATE_LIMITED':
              throw new Error(t('settings.modelCatalogRateLimited'));
            case 'TIMEOUT':
            case 'BAD_GATEWAY':
              throw new Error(t('settings.modelCatalogUnavailable'));
            default:
              throw new Error(error.backendMessage || t('settings.modelCatalogFetchFailed'));
          }
        }
        throw error;
      }
    },
    { shouldRetryOnError: false }
  );
};

export default useModeModeList;
