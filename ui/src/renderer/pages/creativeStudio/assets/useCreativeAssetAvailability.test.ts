/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { notifyCreativeAssetDeleted } from './assetDeletion';
import type { CreativeAsset } from './types';
import { useCreativeAssetAvailability } from './useCreativeAssetAvailability';

afterEach(cleanup);
const asset = (deletedAt: number | null = null): CreativeAsset => ({
  id: '0190f5fe-7c00-7a00-8000-000000000001', kind: 'image', title: 'Original',
  collection: null, tags: [], mimeType: 'image/png', width: 10, height: 10,
  bytes: 10, inLibrary: deletedAt === null, textContent: null, origin: null,
  originalUrl: '/original', thumbnailUrl: '/thumbnail', createdAt: 1, updatedAt: 1, deletedAt,
});

describe('history asset availability', () => {
  test('identifies deletion from metadata and distinguishes request failures', async () => {
    const client = { get: async (id: string) => { if (id === 'offline') throw new Error('offline'); return asset(2); } };
    const hook = renderHook(() => useCreativeAssetAvailability([asset().id, 'offline'], client));
    await waitFor(() => expect(hook.result.current.get(asset().id)).toBe('deleted'));
    expect(hook.result.current.get('offline')).toBe('unavailable');
  });

  test('does not let a pending metadata read undo a deletion notification', async () => {
    let finish!: (asset: CreativeAsset) => void;
    const pending = new Promise<CreativeAsset>((resolve) => { finish = resolve; });
    const client = { get: () => pending };
    const hook = renderHook(() => useCreativeAssetAvailability([asset().id], client));
    act(() => notifyCreativeAssetDeleted(client, asset().id));
    await act(async () => finish(asset()));
    expect(hook.result.current.get(asset().id)).toBe('deleted');
  });

  test('rechecks visible historical assets when another window returns focus', async () => {
    let deleted = false;
    const client = { get: async () => asset(deleted ? 2 : null) };
    const hook = renderHook(() => useCreativeAssetAvailability([asset().id], client));
    await waitFor(() => expect(hook.result.current.get(asset().id)).toBe('available'));
    deleted = true;
    act(() => window.dispatchEvent(new Event('focus')));
    await waitFor(() => expect(hook.result.current.get(asset().id)).toBe('deleted'));
  });
});
