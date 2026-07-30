/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import useSWR, { type SWRConfiguration } from 'swr';

import { ipcBridge } from '@/common';
import type { ProviderConnectionResponse } from '@/common/types/provider/providerConnection';
import type { ProviderId } from '@/common/types/ids';

export const providerConnectionsSwrKey = (providerId: ProviderId) => `provider-connections:${providerId}`;

const SWR_OPTIONS: SWRConfiguration<ProviderConnectionResponse[], Error> = {
  revalidateOnFocus: false,
  revalidateOnReconnect: false,
  shouldRetryOnError: false,
};

/**
 * Non-default per-role connection profiles of one provider
 * (`GET /api/providers/{id}/connections`). Pass `enabled: false` to defer the
 * fetch until the UI actually needs the list (collapsed sections, popovers).
 */
export const useProviderConnections = (providerId: ProviderId, enabled = true) => {
  const { data, error, isLoading, mutate } = useSWR<ProviderConnectionResponse[]>(
    enabled ? providerConnectionsSwrKey(providerId) : null,
    () => ipcBridge.providerConnection.list.invoke({ provider_id: providerId }),
    SWR_OPTIONS
  );

  return { connections: data ?? [], error, isLoading, mutate };
};
