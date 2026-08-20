/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { parseAssetId } from '@/common/types/ids';

import type { WorkshopAssetApi, WorkshopAssetDto } from './api';
import { CreativeAssetClient, mapWorkshopAsset, toWorkshopAssetQuery } from './client';

const ASSET_ID = parseAssetId('0190f5fe-7c00-7a00-8000-000000000001');

function assetDto(overrides: Partial<WorkshopAssetDto> = {}): WorkshopAssetDto {
  return {
    asset_id: ASSET_ID,
    kind: 'image',
    title: 'Reference',
    collection: 'Moodboard',
    tags: ['warm', 'portrait'],
    mime: 'image/png',
    width: 1024,
    height: 768,
    bytes: 4096,
    in_library: true,
    text_content: null,
    origin: {
      provider_id: '0190f5fe-7c00-7a00-8000-000000000002',
      canvas_id: '0190f5fe-7c00-7a00-8000-000000000003',
      node_id: '0190f5fe-7c00-7a00-8000-000000000004',
      creation_task_id: '0190f5fe-7c00-7a00-8000-000000000005',
    },
    url: '/api/workshop/files/asset',
    thumb_url: '/api/workshop/files/asset?thumb=1',
    created_at: 100,
    updated_at: 200,
    ...overrides,
  };
}

function apiStub(overrides: Partial<WorkshopAssetApi> = {}): WorkshopAssetApi {
  return {
    list: async () => ({ items: [assetDto()], total: 1 }),
    upload: async () => assetDto(),
    createText: async () => assetDto({ kind: 'text', mime: null, width: null, height: null, bytes: null }),
    update: async () => assetDto(),
    remove: async () => undefined,
    renameCollection: async () => 0,
    fileUrl: (_assetId, thumbnail) => (thumbnail ? '/thumbnail' : '/original'),
    ...overrides,
  };
}

describe('CreativeAssetClient', () => {
  test('maps the Workshop wire shape into the Creative Studio domain shape', () => {
    const asset = mapWorkshopAsset(assetDto());

    expect(asset.id).toBe(ASSET_ID);
    expect(asset.mimeType).toBe('image/png');
    expect(asset.inLibrary).toBe(true);
    expect(asset.thumbnailUrl?.includes('thumb=1')).toBe(true);
    expect(asset.origin).toEqual({
      prompt: undefined,
      model: undefined,
      providerId: '0190f5fe-7c00-7a00-8000-000000000002',
      params: undefined,
      projectId: '0190f5fe-7c00-7a00-8000-000000000003',
      nodeId: '0190f5fe-7c00-7a00-8000-000000000004',
      generationTaskId: '0190f5fe-7c00-7a00-8000-000000000005',
    });
  });

  test('keeps ungrouped and named collection mutually exclusive on the wire', () => {
    expect(
      toWorkshopAssetQuery({
        kind: 'video',
        collection: 'ignored',
        search: 'launch',
        inLibrary: true,
        ungrouped: true,
        page: 2,
        pageSize: 24,
      })
    ).toEqual({
      kind: 'video',
      collection: undefined,
      q: 'launch',
      in_library: true,
      ungrouped: true,
      tag: undefined,
      sort: undefined,
      page: 2,
      page_size: 24,
    });
  });

  test('adapts list, update, upload, text creation, deletion, rename and URLs', async () => {
    const calls: Array<[string, unknown]> = [];
    const api = apiStub({
      list: async (query) => {
        calls.push(['list', query]);
        return { items: [assetDto()], total: 1 };
      },
      upload: async (_file, metadata, signal, onProgress) => {
        calls.push(['upload', { metadata, signal }]);
        onProgress?.(42);
        return assetDto();
      },
      createText: async (input) => {
        calls.push(['createText', input]);
        return assetDto({ kind: 'text', mime: null, width: null, height: null, bytes: null });
      },
      update: async (_id, patch) => {
        calls.push(['update', patch]);
        return assetDto({ title: 'Updated' });
      },
      remove: async (id) => {
        calls.push(['remove', id]);
      },
      renameCollection: async (from, to) => {
        calls.push(['rename', { from, to }]);
        return 3;
      },
    });
    const client = new CreativeAssetClient(api);
    const controller = new AbortController();
    const progress: number[] = [];

    const page = await client.list({ inLibrary: true });
    expect(page.total).toBe(1);
    expect(page.items[0]?.originalUrl).toBe('/original');
    expect(page.items[0]?.thumbnailUrl).toBe('/thumbnail');
    expect(
      (
        await client.upload(
          new File(['image'], 'sample.png', { type: 'image/png' }),
          { inLibrary: false },
          controller.signal,
          (percent) => progress.push(percent)
        )
      ).id
    ).toBe(ASSET_ID);
    expect((await client.createText({ title: 'Prompt', textContent: 'Hello' })).kind).toBe('text');
    expect((await client.update(ASSET_ID, { collection: null, inLibrary: false })).title).toBe('Updated');
    await client.remove(ASSET_ID);
    expect(await client.renameCollection('Old', 'New')).toBe(3);
    expect(client.url(ASSET_ID)).toBe('/original');
    expect(client.url(ASSET_ID, 'thumbnail')).toBe('/thumbnail');
    expect(progress).toEqual([42]);
    expect(calls.map(([name]) => name)).toEqual(['list', 'upload', 'createText', 'update', 'remove', 'rename']);
    expect(calls.find(([name]) => name === 'update')?.[1]).toEqual({
      title: undefined,
      collection: '',
      tags: undefined,
      in_library: false,
    });
  });

  test('rejects unknown backend asset kinds instead of guessing', () => {
    let error: unknown;
    try {
      mapWorkshopAsset(assetDto({ kind: 'archive' }));
    } catch (reason) {
      error = reason;
    }
    expect(error instanceof TypeError).toBe(true);
    expect(error instanceof Error ? error.message : '').toBe('Unknown creative asset kind: archive');
  });
});
