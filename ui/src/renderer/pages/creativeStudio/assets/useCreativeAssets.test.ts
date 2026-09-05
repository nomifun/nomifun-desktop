/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';

import { parseAssetId } from '@/common/types/ids';

import {
  CreativeAssetQueryCache,
  listCreativeAssetsCached,
  type CreativeAssetQueryCacheConfiguration,
} from './creativeAssetQueryCache';
import type {
  CreativeAsset,
  CreativeAssetLibraryPort,
  CreativeAssetPage,
} from './types';
import { creativeAssetMatchesQuery } from './useCreativeAssets';
import { useCreativeAssets } from './useCreativeAssets';

const ASSET: CreativeAsset = {
  id: parseAssetId('0190f5fe-7c00-7a00-8000-000000000001'),
  kind: 'image',
  title: 'Golden hour portrait',
  collection: 'People',
  tags: ['warm', 'portrait'],
  mimeType: 'image/png',
  width: 100,
  height: 100,
  bytes: 10,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: '/asset',
  thumbnailUrl: '/thumb',
  createdAt: 1,
  updatedAt: 1,
};

const UPDATED_ASSET: CreativeAsset = {
  ...ASSET,
  title: 'Updated portrait',
  updatedAt: 2,
};

interface Deferred<T> {
  promise: Promise<T>;
  resolve(value: T): void;
  reject(reason: unknown): void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, resolve, reject };
}

function page(items: CreativeAsset[] = [ASSET]): CreativeAssetPage {
  return { items, total: items.length };
}

function createPort(
  list: CreativeAssetLibraryPort['list'],
  overrides: Partial<CreativeAssetLibraryPort> = {}
): CreativeAssetLibraryPort {
  return {
    list,
    upload: async () => ASSET,
    createText: async () => ASSET,
    update: async () => UPDATED_ASSET,
    remove: async () => undefined,
    renameCollection: async () => 0,
    url: () => ASSET.originalUrl,
    ...overrides,
  };
}

const QUERY = {
  kind: 'image' as const,
  search: 'portrait',
  sort: 'created_desc' as const,
  page: 1,
  pageSize: 40,
};

function createCache(configuration: CreativeAssetQueryCacheConfiguration = {}) {
  return new CreativeAssetQueryCache(configuration);
}

afterEach(() => {
  cleanup();
});

describe('creativeAssetMatchesQuery', () => {
  test('matches kind, collection, tag, library and search filters together', () => {
    expect(
      creativeAssetMatchesQuery(ASSET, {
        kind: 'image',
        collection: 'People',
        tag: 'warm',
        inLibrary: true,
        search: 'HOUR PORTRAIT',
      })
    ).toBe(true);
    expect(creativeAssetMatchesQuery(ASSET, { kind: 'video' })).toBe(false);
    expect(creativeAssetMatchesQuery(ASSET, { tag: 'cold' })).toBe(false);
  });

  test('treats the ungrouped filter as mutually exclusive with collection', () => {
    expect(creativeAssetMatchesQuery(ASSET, { ungrouped: true })).toBe(false);
    expect(creativeAssetMatchesQuery({ ...ASSET, collection: null }, { ungrouped: true })).toBe(true);
  });
});

describe('CreativeAssetQueryCache', () => {
  test('deduplicates concurrent requests for the same client and query', async () => {
    const cache = createCache();
    const pending = deferred<CreativeAssetPage>();
    let calls = 0;
    const client = createPort(() => {
      calls += 1;
      return pending.promise;
    });

    const first = cache.list(client, QUERY);
    const second = cache.list(client, { ...QUERY });

    expect(calls).toBe(1);
    expect(second).toBe(first);

    pending.resolve(page());
    expect(await first).toEqual(page());
  });

  test('serves settled entries within the TTL and refetches after expiry', async () => {
    let now = 1_000;
    const cache = createCache({
      ttlMs: 100,
      now: () => now,
    });
    let calls = 0;
    const client = createPort(async () => {
      calls += 1;
      return page();
    });

    await cache.list(client, QUERY);
    now = 1_099;
    await cache.list(client, QUERY);
    expect(calls).toBe(1);

    now = 1_100;
    await cache.list(client, QUERY);
    expect(calls).toBe(2);
  });

  test('isolates cache entries by client, query and page size', async () => {
    const cache = createCache();
    let clientACalls = 0;
    let clientBCalls = 0;
    const clientA = createPort(async () => {
      clientACalls += 1;
      return page();
    });
    const clientB = createPort(async () => {
      clientBCalls += 1;
      return page();
    });

    await Promise.all([
      cache.list(clientA, QUERY),
      cache.list(clientA, { ...QUERY, search: 'landscape' }),
      cache.list(clientA, { ...QUERY, pageSize: 80 }),
      cache.list(clientA, { ...QUERY, page: 2 }),
      cache.list(clientB, QUERY),
    ]);

    expect(clientACalls).toBe(4);
    expect(clientBCalls).toBe(1);
  });

  test('does not cache an old in-flight result after invalidation', async () => {
    const cache = createCache();
    const oldPending = deferred<CreativeAssetPage>();
    const freshPending = deferred<CreativeAssetPage>();
    let calls = 0;
    const client = createPort(() => {
      calls += 1;
      return calls === 1 ? oldPending.promise : freshPending.promise;
    });

    const oldRequest = cache.list(client, QUERY);
    cache.invalidate(client);
    const freshRequest = cache.list(client, QUERY);

    expect(calls).toBe(2);

    const whileFreshIsPending = cache.list(client, QUERY);
    expect(whileFreshIsPending).toBe(freshRequest);
    expect(calls).toBe(2);

    freshPending.resolve(page([UPDATED_ASSET]));
    expect(await freshRequest).toEqual(page([UPDATED_ASSET]));

    oldPending.resolve(page([ASSET]));
    expect(await oldRequest).toEqual(page([ASSET]));

    expect(await cache.list(client, QUERY)).toEqual(page([UPDATED_ASSET]));
    expect(calls).toBe(2);
  });

  test('force bypasses settled cache but shares an active same-key request', async () => {
    const cache = createCache();
    let calls = 0;
    const client = createPort(async () => {
      calls += 1;
      return page();
    });

    await cache.list(client, QUERY);
    await cache.list(client, QUERY);
    expect(calls).toBe(1);

    await cache.list(client, QUERY, { force: true });
    expect(calls).toBe(2);

    const pending = deferred<CreativeAssetPage>();
    const activeClient = createPort(() => {
      calls += 1;
      return pending.promise;
    });
    const active = cache.list(activeClient, QUERY);
    const forcedActive = cache.list(activeClient, QUERY, { force: true });
    expect(forcedActive).toBe(active);
    expect(calls).toBe(3);

    pending.resolve(page());
    await active;
  });

  test('force refresh evicts settled sibling pages in the same query scope', async () => {
    const cache = createCache();
    const calls: number[] = [];
    const client = createPort(async (query) => {
      calls.push(query?.page ?? 1);
      return page();
    });

    await cache.list(client, QUERY);
    await cache.list(client, { ...QUERY, page: 2 });
    expect(calls).toEqual([1, 2]);

    await cache.list(client, QUERY, { force: true });
    await cache.list(client, { ...QUERY, page: 2 });
    expect(calls).toEqual([1, 2, 1, 2]);
  });

  test('does not cache rejected requests', async () => {
    const cache = createCache();
    let calls = 0;
    const failure = new Error('temporary failure');
    const client = createPort(async () => {
      calls += 1;
      throw failure;
    });

    let firstFailure: unknown;
    let secondFailure: unknown;
    try {
      await cache.list(client, QUERY);
    } catch (reason) {
      firstFailure = reason;
    }
    try {
      await cache.list(client, QUERY);
    } catch (reason) {
      secondFailure = reason;
    }
    expect(firstFailure).toBe(failure);
    expect(secondFailure).toBe(failure);
    expect(calls).toBe(2);
  });
});

describe('useCreativeAssets mutation reconciliation', () => {
  test('removes a deleted asset and invalidates an in-flight list that still contains it', async () => {
    const staleList = deferred<CreativeAssetPage>();
    let listCalls = 0;
    let removedId: string | undefined;
    const client = createPort(async () => {
      listCalls += 1;
      if (listCalls === 1) return page();
      if (listCalls === 2) return staleList.promise;
      return page([]);
    }, {
      remove: async (assetId) => { removedId = assetId; },
    });
    const hook = renderHook(() =>
      useCreativeAssets({ client, query: { kind: 'image' }, pageSize: 40 })
    );
    await waitFor(() => {
      expect(hook.result.current.assets).toEqual([ASSET]);
    });

    let reload!: Promise<void>;
    act(() => { reload = hook.result.current.reload(); });
    await waitFor(() => { expect(listCalls).toBe(2); });
    await act(async () => { await hook.result.current.remove(ASSET.id); });

    expect(removedId).toBe(ASSET.id);
    expect(hook.result.current.assets).toEqual([]);
    expect(hook.result.current.total).toBe(0);
    await act(async () => {
      staleList.resolve(page());
      await reload;
    });
    expect(hook.result.current.assets).toEqual([]);
    expect(hook.result.current.total).toBe(0);
    expect(hook.result.current.mutationError).toBeNull();

    expect(await listCreativeAssetsCached(client, {
      kind: 'image', page: 1, pageSize: 40,
    })).toEqual(page([]));
    expect(listCalls).toBe(3);
  });

  test('preserves an asset, count, and cached list when deletion is rejected', async () => {
    const failure = new Error('Asset is still referenced by a canvas');
    let listCalls = 0;
    const client = createPort(async () => {
      listCalls += 1;
      return page();
    }, {
      remove: async () => { throw failure; },
    });
    const hook = renderHook(() =>
      useCreativeAssets({ client, query: { kind: 'image' }, pageSize: 40 })
    );
    await waitFor(() => {
      expect(hook.result.current.assets).toEqual([ASSET]);
    });
    await act(async () => {
      const error = await hook.result.current.remove(ASSET.id).catch((reason) => reason);
      expect(error).toBe(failure);
    });

    expect(hook.result.current.assets).toEqual([ASSET]);
    expect(hook.result.current.total).toBe(1);
    expect(hook.result.current.mutationError).toBe(failure);
    expect(hook.result.current.mutating).toBe(false);
    expect(await listCreativeAssetsCached(client, {
      kind: 'image', page: 1, pageSize: 40,
    })).toEqual(page());
    expect(listCalls).toBe(1);
  });

  test('keeps local update reconciliation while invalidating the query cache', async () => {
    let listCalls = 0;
    const client = createPort(async () => {
      listCalls += 1;
      return page();
    }, {
      update: async () => UPDATED_ASSET,
    });

    const hook = renderHook(() =>
      useCreativeAssets({
        client,
        query: { kind: 'image' },
        pageSize: 40,
      })
    );

    await waitFor(() => {
      expect(hook.result.current.assets).toEqual([ASSET]);
    });
    expect(listCalls).toBe(1);

    await act(async () => {
      await hook.result.current.update(ASSET.id, { title: UPDATED_ASSET.title });
    });

    expect(hook.result.current.assets).toEqual([UPDATED_ASSET]);
    expect(hook.result.current.total).toBe(1);

    const cachedPage = await listCreativeAssetsCached(client, {
      kind: 'image',
      page: 1,
      pageSize: 40,
    });
    expect(cachedPage).toEqual(page());
    expect(listCalls).toBe(2);
  });

  test('ignores an initial list response invalidated by a successful mutation', async () => {
    const pendingList = deferred<CreativeAssetPage>();
    let listCalls = 0;
    const client = createPort(() => {
      listCalls += 1;
      return pendingList.promise;
    }, {
      update: async () => UPDATED_ASSET,
    });

    const hook = renderHook(() =>
      useCreativeAssets({
        client,
        query: { kind: 'image' },
        pageSize: 40,
      })
    );

    await waitFor(() => {
      expect(listCalls).toBe(1);
      expect(hook.result.current.loading).toBe(true);
    });

    await act(async () => {
      await hook.result.current.update(ASSET.id, { title: UPDATED_ASSET.title });
    });
    expect(hook.result.current.assets).toEqual([UPDATED_ASSET]);
    expect(hook.result.current.total).toBe(1);

    pendingList.resolve(page([ASSET]));
    await waitFor(() => {
      expect(hook.result.current.loading).toBe(false);
    });
    expect(hook.result.current.assets).toEqual([UPDATED_ASSET]);
    expect(hook.result.current.total).toBe(1);
  });

});
