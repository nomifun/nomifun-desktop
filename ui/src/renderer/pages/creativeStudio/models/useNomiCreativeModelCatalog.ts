/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useMemo } from 'react';

import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';

import { adaptCreativeModelCatalog } from './catalog';
import type { CreativeModelCatalogSnapshot } from './types';

/**
 * Thin adapter over NomiFun's one SWR provider catalog. Creative Studio never
 * fetches a source-product model profile or stores credentials of its own.
 */
export function useNomiCreativeModelCatalog(): CreativeModelCatalogSnapshot {
  const { data, error, isLoading, mutate } = useProvidersQuery();
  const refresh = useCallback(() => {
    void mutate();
  }, [mutate]);

  return useMemo(
    () => adaptCreativeModelCatalog({ data, error, isLoading, refresh }),
    [data, error, isLoading, refresh]
  );
}
