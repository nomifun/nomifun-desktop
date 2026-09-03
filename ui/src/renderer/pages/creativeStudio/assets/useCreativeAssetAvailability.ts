/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useEffect, useMemo, useState } from 'react';
import { subscribeCreativeAssetDeletion } from './assetDeletion';
import { creativeAssetClient } from './client';
import { isCreativeAssetDeleted, type CreativeAsset } from './types';

export type CreativeAssetAvailability = 'loading' | 'available' | 'deleted' | 'unavailable';

/** History keeps the original ids; only an authoritative tombstone means deleted. */
export function useCreativeAssetAvailability(
  assetIds: readonly string[],
  client: { get(assetId: string): Promise<CreativeAsset> } = creativeAssetClient
): ReadonlyMap<string, CreativeAssetAvailability> {
  const key = JSON.stringify([...new Set(assetIds)].sort());
  const ids = useMemo(() => JSON.parse(key) as string[], [key]);
  const [state, setState] = useState<ReadonlyMap<string, CreativeAssetAvailability>>(new Map());
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    const reload = () => setRefresh((value) => value + 1);
    window.addEventListener('focus', reload);
    return () => window.removeEventListener('focus', reload);
  }, []);

  useEffect(() => {
    let active = true;
    const deleted = new Set<string>();
    const publish = (assetId: string, status: CreativeAssetAvailability) => {
      if (!active || (deleted.has(assetId) && status !== 'deleted')) return;
      if (status === 'deleted') deleted.add(assetId);
      setState((current) => new Map(current).set(assetId, status));
    };
    setState(new Map(ids.map((id) => [id, 'loading'])));
    const unsubscribe = subscribeCreativeAssetDeletion(client, (assetId) => {
      if (ids.includes(assetId)) publish(assetId, 'deleted');
    });
    ids.forEach((id) => {
      void client.get(id).then(
        (asset) => publish(id, isCreativeAssetDeleted(asset) ? 'deleted' : 'available'),
        () => publish(id, 'unavailable')
      );
    });
    return () => { active = false; unsubscribe(); };
  }, [client, ids, refresh]);

  return state;
}
