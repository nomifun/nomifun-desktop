/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeAssetLibraryPort,
  CreativeAssetPage,
  CreativeAssetQuery,
} from './types';

export const CREATIVE_ASSET_QUERY_CACHE_TTL_MS = 30_000;

const MAX_CACHE_ENTRIES_PER_CLIENT = 64;

interface CreativeAssetQueryCacheEntry {
  page?: CreativeAssetPage;
  expiresAt: number;
  lastAccessAt: number;
  promise?: Promise<CreativeAssetPage>;
  generation: number;
  scopeKey: string;
}

interface CreativeAssetClientCache {
  entries: Map<string, CreativeAssetQueryCacheEntry>;
  generation: number;
}

export interface CreativeAssetQueryCacheOptions {
  /**
   * Skip settled entries in the matching query scope. An active request for
   * the exact key is still shared so force reload cannot duplicate it.
   */
  force?: boolean;
}

export interface CreativeAssetQueryCacheConfiguration {
  ttlMs?: number;
  maxEntriesPerClient?: number;
  now?: () => number;
}

function queryKey(query: CreativeAssetQuery): string {
  return JSON.stringify([
    query.kind ?? null,
    query.collection ?? null,
    query.search ?? null,
    query.inLibrary ?? null,
    query.ungrouped ?? null,
    query.tag ?? null,
    query.sort ?? null,
    query.page ?? 1,
    query.pageSize ?? null,
  ]);
}

function queryScopeKey(query: CreativeAssetQuery): string {
  return JSON.stringify([
    query.kind ?? null,
    query.collection ?? null,
    query.search ?? null,
    query.inLibrary ?? null,
    query.ungrouped ?? null,
    query.tag ?? null,
    query.sort ?? null,
    query.pageSize ?? null,
  ]);
}

export class CreativeAssetQueryCache {
  private readonly cacheByClient = new WeakMap<
    CreativeAssetLibraryPort,
    CreativeAssetClientCache
  >();
  private readonly ttlMs: number;
  private readonly maxEntriesPerClient: number;
  private readonly now: () => number;

  constructor(configuration: CreativeAssetQueryCacheConfiguration = {}) {
    this.ttlMs = Math.max(
      0,
      Math.trunc(configuration.ttlMs ?? CREATIVE_ASSET_QUERY_CACHE_TTL_MS)
    );
    this.maxEntriesPerClient = Math.max(
      1,
      Math.trunc(configuration.maxEntriesPerClient ?? MAX_CACHE_ENTRIES_PER_CLIENT)
    );
    this.now = configuration.now ?? Date.now;
  }

  list(
    client: CreativeAssetLibraryPort,
    query: CreativeAssetQuery,
    options: CreativeAssetQueryCacheOptions = {}
  ): Promise<CreativeAssetPage> {
    const cache = this.getClientCache(client);
    const key = queryKey(query);
    const scopeKey = queryScopeKey(query);
    const now = this.now();
    const existing = cache.entries.get(key);

    if (existing?.promise) {
      if (options.force) this.removeScopeEntries(cache, scopeKey, key);
      existing.lastAccessAt = now;
      return existing.promise;
    }

    if (options.force) this.removeScopeEntries(cache, scopeKey);

    if (!options.force && existing?.page && existing.expiresAt > now) {
      existing.lastAccessAt = now;
      return Promise.resolve(existing.page);
    }

    if (existing) cache.entries.delete(key);

    const generation = cache.generation;
    let promise: Promise<CreativeAssetPage>;
    let request: Promise<CreativeAssetPage>;
    try {
      request = Promise.resolve(client.list(query));
    } catch (reason) {
      request = Promise.reject(reason);
    }
    promise = request
      .then((page) => {
        const current = cache.entries.get(key);
        if (
          current?.promise === promise &&
          current.generation === generation &&
          cache.generation === generation
        ) {
          current.page = page;
          current.expiresAt = this.now() + this.ttlMs;
          current.lastAccessAt = this.now();
          current.promise = undefined;
          this.pruneClientCache(cache);
        }
        return page;
      })
      .catch((reason) => {
        const current = cache.entries.get(key);
        if (current?.promise === promise && current.generation === generation) {
          cache.entries.delete(key);
        }
        throw reason;
      });

    cache.entries.set(key, {
      expiresAt: 0,
      lastAccessAt: now,
      promise,
      generation,
      scopeKey,
    });
    this.pruneClientCache(cache);
    return promise;
  }

  invalidate(client: CreativeAssetLibraryPort): void {
    const cache = this.cacheByClient.get(client);
    if (!cache) return;
    cache.generation += 1;
    cache.entries.clear();
  }

  generation(client: CreativeAssetLibraryPort): number {
    return this.getClientCache(client).generation;
  }

  private getClientCache(client: CreativeAssetLibraryPort): CreativeAssetClientCache {
    const existing = this.cacheByClient.get(client);
    if (existing) return existing;

    const created: CreativeAssetClientCache = {
      entries: new Map(),
      generation: 0,
    };
    this.cacheByClient.set(client, created);
    return created;
  }

  private removeScopeEntries(
    cache: CreativeAssetClientCache,
    scopeKey: string,
    exceptKey?: string
  ): void {
    for (const [key, entry] of cache.entries) {
      if (key !== exceptKey && entry.scopeKey === scopeKey) cache.entries.delete(key);
    }
  }

  private pruneClientCache(cache: CreativeAssetClientCache): void {
    if (cache.entries.size <= this.maxEntriesPerClient) return;

    const settledEntries = [...cache.entries.entries()]
      .filter(([, entry]) => !entry.promise)
      .sort(([, left], [, right]) => left.lastAccessAt - right.lastAccessAt);

    for (const [key] of settledEntries) {
      if (cache.entries.size <= this.maxEntriesPerClient) break;
      cache.entries.delete(key);
    }
  }
}

const creativeAssetQueryCache = new CreativeAssetQueryCache();

export function listCreativeAssetsCached(
  client: CreativeAssetLibraryPort,
  query: CreativeAssetQuery,
  options?: CreativeAssetQueryCacheOptions
): Promise<CreativeAssetPage> {
  return creativeAssetQueryCache.list(client, query, options);
}

export function invalidateCreativeAssetQueryCache(
  client: CreativeAssetLibraryPort
): void {
  creativeAssetQueryCache.invalidate(client);
}

export function getCreativeAssetQueryCacheGeneration(
  client: CreativeAssetLibraryPort
): number {
  return creativeAssetQueryCache.generation(client);
}
