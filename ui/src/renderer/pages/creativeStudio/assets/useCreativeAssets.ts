/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { creativeAssetClient } from './client';
import { subscribeCreativeAssetDeletion } from './assetDeletion';
import { isCreativeAssetDeleted } from './types';
import {
  getCreativeAssetQueryCacheGeneration,
  invalidateCreativeAssetQueryCache,
  listCreativeAssetsCached,
} from './creativeAssetQueryCache';
import type {
  CreateCreativeTextAsset,
  CreativeAsset,
  CreativeAssetLibraryPort,
  CreativeAssetMetadata,
  CreativeAssetPatch,
  CreativeAssetQuery,
  CreativeAssetUploadProgress,
  CreativeAssetVariant,
} from './types';

export const CREATIVE_ASSET_PAGE_SIZE = 40;

export interface UseCreativeAssetsOptions {
  enabled?: boolean;
  query?: Omit<CreativeAssetQuery, 'page' | 'pageSize'>;
  pageSize?: number;
  client?: CreativeAssetLibraryPort;
}

export interface UseCreativeAssetsResult {
  assets: CreativeAsset[];
  total: number;
  loading: boolean;
  loadingMore: boolean;
  mutating: boolean;
  error: Error | null;
  mutationError: Error | null;
  hasMore: boolean;
  reload(): Promise<void>;
  loadMore(): Promise<void>;
  upload(
    file: File,
    metadata?: CreativeAssetMetadata,
    signal?: AbortSignal,
    onProgress?: CreativeAssetUploadProgress
  ): Promise<CreativeAsset>;
  createText(input: CreateCreativeTextAsset): Promise<CreativeAsset>;
  update(assetId: string, patch: CreativeAssetPatch): Promise<CreativeAsset>;
  remove(assetId: string): Promise<void>;
  renameCollection(from: string, to: string): Promise<number>;
  url(assetId: string, variant?: CreativeAssetVariant): string;
}

function asError(value: unknown): Error {
  return value instanceof Error ? value : new Error(String(value));
}

export function creativeAssetMatchesQuery(
  asset: CreativeAsset,
  query: Omit<CreativeAssetQuery, 'page' | 'pageSize'>
): boolean {
  if (isCreativeAssetDeleted(asset)) return false;
  if (query.kind && asset.kind !== query.kind) return false;
  if (query.inLibrary !== undefined && asset.inLibrary !== query.inLibrary) return false;
  if (query.ungrouped && asset.collection) return false;
  if (!query.ungrouped && query.collection && asset.collection !== query.collection) return false;
  if (query.tag && !asset.tags.includes(query.tag)) return false;
  const search = query.search?.trim().toLocaleLowerCase();
  if (search) {
    const searchable = [asset.title, asset.collection ?? '', ...asset.tags].join('\n').toLocaleLowerCase();
    if (!searchable.includes(search)) return false;
  }
  return true;
}

function addOrReplaceAsset(items: CreativeAsset[], asset: CreativeAsset, prepend: boolean): CreativeAsset[] {
  const index = items.findIndex((entry) => entry.id === asset.id);
  if (index < 0) return prepend ? [asset, ...items] : [...items, asset];
  const next = [...items];
  next[index] = asset;
  return next;
}

function appendUniqueAssets(items: CreativeAsset[], additions: CreativeAsset[]): CreativeAsset[] {
  const known = new Set(items.map((asset) => asset.id));
  return [...items, ...additions.filter((asset) => !known.has(asset.id))];
}

export function useCreativeAssets(options: UseCreativeAssetsOptions = {}): UseCreativeAssetsResult {
  const enabled = options.enabled ?? true;
  const port = options.client ?? creativeAssetClient;
  const pageSize = Math.max(1, Math.trunc(options.pageSize ?? CREATIVE_ASSET_PAGE_SIZE));
  const query = options.query ?? {};
  const queryKind = query.kind;
  const queryCollection = query.collection;
  const querySearch = query.search;
  const queryInLibrary = query.inLibrary;
  const queryUngrouped = query.ungrouped;
  const queryTag = query.tag;
  const querySort = query.sort;

  const stableQuery = useMemo<Omit<CreativeAssetQuery, 'page' | 'pageSize'>>(
    () => ({
      kind: queryKind,
      collection: queryCollection,
      search: querySearch,
      inLibrary: queryInLibrary,
      ungrouped: queryUngrouped,
      tag: queryTag,
      sort: querySort,
    }),
    [queryKind, queryCollection, querySearch, queryInLibrary, queryUngrouped, queryTag, querySort]
  );

  const [assets, setAssets] = useState<CreativeAsset[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [pendingMutations, setPendingMutations] = useState(0);
  const [error, setError] = useState<Error | null>(null);
  const [mutationError, setMutationError] = useState<Error | null>(null);

  const mountedRef = useRef(true);
  const requestRef = useRef(0);
  const pageRef = useRef(1);
  const loadingMoreRef = useRef(false);
  const assetsRef = useRef<CreativeAsset[]>([]);
  const totalRef = useRef(0);

  useEffect(() => {
    assetsRef.current = assets;
  }, [assets]);
  useEffect(() => {
    totalRef.current = total;
  }, [total]);
  useEffect(() => {
    // React StrictMode intentionally replays effect setup/cleanup in dev.
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      requestRef.current += 1;
    };
  }, []);

  const buildQuery = useCallback(
    (page: number): CreativeAssetQuery => ({ ...stableQuery, page, pageSize }),
    [stableQuery, pageSize]
  );

  const loadFirstPage = useCallback(async (force: boolean) => {
    if (!enabled) return;
    const request = ++requestRef.current;
    const cacheGeneration = getCreativeAssetQueryCacheGeneration(port);
    setLoading(true);
    setError(null);
    try {
      const result = await listCreativeAssetsCached(port, buildQuery(1), { force });
      if (
        !mountedRef.current ||
        request !== requestRef.current ||
        cacheGeneration !== getCreativeAssetQueryCacheGeneration(port)
      ) {
        return;
      }
      assetsRef.current = result.items;
      totalRef.current = result.total;
      setAssets(result.items);
      setTotal(result.total);
      pageRef.current = 1;
    } catch (reason) {
      if (
        !mountedRef.current ||
        request !== requestRef.current ||
        cacheGeneration !== getCreativeAssetQueryCacheGeneration(port)
      ) {
        return;
      }
      assetsRef.current = [];
      totalRef.current = 0;
      setAssets([]);
      setTotal(0);
      setError(asError(reason));
    } finally {
      if (mountedRef.current && request === requestRef.current) setLoading(false);
    }
  }, [enabled, port, buildQuery]);

  const reload = useCallback(() => loadFirstPage(true), [loadFirstPage]);

  useEffect(() => {
    const unsubscribe = subscribeCreativeAssetDeletion(port, (assetId) => {
      if (!assetsRef.current.some((asset) => asset.id === assetId)) return;
      const remaining = assetsRef.current.filter((asset) => asset.id !== assetId);
      assetsRef.current = remaining;
      totalRef.current = Math.max(0, totalRef.current - 1);
      setAssets(remaining);
      setTotal(totalRef.current);
    });
    const refresh = () => { if (enabled) void reload(); };
    window.addEventListener('focus', refresh);
    return () => { unsubscribe(); window.removeEventListener('focus', refresh); };
  }, [enabled, port, reload]);

  useEffect(() => {
    if (!enabled) {
      requestRef.current += 1;
      setLoading(false);
      return;
    }
    void loadFirstPage(false);
  }, [enabled, loadFirstPage]);

  const loadMore = useCallback(async () => {
    if (!enabled || loadingMoreRef.current || assetsRef.current.length >= totalRef.current) return;
    const request = requestRef.current;
    const cacheGeneration = getCreativeAssetQueryCacheGeneration(port);
    const nextPage = pageRef.current + 1;
    loadingMoreRef.current = true;
    setLoadingMore(true);
    setError(null);
    try {
      const result = await listCreativeAssetsCached(port, buildQuery(nextPage));
      if (
        !mountedRef.current ||
        request !== requestRef.current ||
        cacheGeneration !== getCreativeAssetQueryCacheGeneration(port)
      ) {
        return;
      }
      const nextAssets = appendUniqueAssets(assetsRef.current, result.items);
      assetsRef.current = nextAssets;
      totalRef.current = result.total;
      setAssets(nextAssets);
      setTotal(result.total);
      pageRef.current = nextPage;
    } catch (reason) {
      if (
        mountedRef.current &&
        request === requestRef.current &&
        cacheGeneration === getCreativeAssetQueryCacheGeneration(port)
      ) {
        setError(asError(reason));
      }
    } finally {
      loadingMoreRef.current = false;
      if (mountedRef.current && request === requestRef.current) setLoadingMore(false);
    }
  }, [enabled, port, buildQuery]);

  const mutate = useCallback(async <T,>(operation: () => Promise<T>): Promise<T> => {
    setPendingMutations((count) => count + 1);
    setMutationError(null);
    try {
      return await operation();
    } catch (reason) {
      const nextError = asError(reason);
      if (mountedRef.current) setMutationError(nextError);
      throw nextError;
    } finally {
      if (mountedRef.current) setPendingMutations((count) => Math.max(0, count - 1));
    }
  }, []);

  const reconcileAsset = useCallback(
    (asset: CreativeAsset, prepend: boolean) => {
      const current = assetsRef.current;
      const existed = current.some((entry) => entry.id === asset.id);
      const matches = creativeAssetMatchesQuery(asset, stableQuery);
      const next = matches
        ? addOrReplaceAsset(current, asset, prepend)
        : current.filter((entry) => entry.id !== asset.id);
      const totalDelta = matches && !existed ? 1 : !matches && existed ? -1 : 0;
      assetsRef.current = next;
      setAssets(next);
      if (totalDelta !== 0) {
        totalRef.current = Math.max(0, totalRef.current + totalDelta);
        setTotal(totalRef.current);
      }
    },
    [stableQuery]
  );

  const upload = useCallback(
    (file: File, metadata: CreativeAssetMetadata = {}, signal?: AbortSignal, onProgress?: CreativeAssetUploadProgress) =>
      mutate(async () => {
        const asset = await port.upload(file, metadata, signal, onProgress);
        invalidateCreativeAssetQueryCache(port);
        if (mountedRef.current) reconcileAsset(asset, true);
        return asset;
      }),
    [mutate, port, reconcileAsset]
  );

  const createText = useCallback(
    (input: CreateCreativeTextAsset) =>
      mutate(async () => {
        const asset = await port.createText(input);
        invalidateCreativeAssetQueryCache(port);
        if (mountedRef.current) reconcileAsset(asset, true);
        return asset;
      }),
    [mutate, port, reconcileAsset]
  );

  const update = useCallback(
    (assetId: string, patch: CreativeAssetPatch) =>
      mutate(async () => {
        const asset = await port.update(assetId, patch);
        invalidateCreativeAssetQueryCache(port);
        if (mountedRef.current) reconcileAsset(asset, false);
        return asset;
      }),
    [mutate, port, reconcileAsset]
  );

  const remove = useCallback(
    (assetId: string) =>
      mutate(async () => {
        await port.remove(assetId);
        invalidateCreativeAssetQueryCache(port);
        if (!mountedRef.current) return;
        if (!assetsRef.current.some((asset) => asset.id === assetId)) return;
        const next = assetsRef.current.filter((asset) => asset.id !== assetId);
        assetsRef.current = next;
        totalRef.current = Math.max(0, totalRef.current - 1);
        setAssets(next);
        setTotal(totalRef.current);
      }),
    [mutate, port]
  );

  const renameCollection = useCallback(
    (from: string, to: string) =>
      mutate(async () => {
        const updated = await port.renameCollection(from, to);
        invalidateCreativeAssetQueryCache(port);
        if (mountedRef.current) await reload();
        return updated;
      }),
    [mutate, port, reload]
  );

  const url = useCallback(
    (assetId: string, variant: CreativeAssetVariant = 'original') => port.url(assetId, variant),
    [port]
  );

  return {
    assets,
    total,
    loading,
    loadingMore,
    mutating: pendingMutations > 0,
    error,
    mutationError,
    hasMore: assets.length < total,
    reload,
    loadMore,
    upload,
    createText,
    update,
    remove,
    renameCollection,
    url,
  };
}
