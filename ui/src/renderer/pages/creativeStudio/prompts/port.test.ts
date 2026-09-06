/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseAssetId } from '@/common/types/ids';
import type { CreativeAsset, CreativeAssetLibraryPort } from '../assets';
import {
  createNomiPromptLibraryPort,
  mapNomiTextAssetToPromptLibraryItem,
  promptAssetIdentity,
} from './port';

const ASSET_ID = parseAssetId('0190f5fe-7c00-7a00-8000-000000000004');

const TEXT_ASSET: CreativeAsset = {
  id: ASSET_ID,
  kind: 'text',
  title: 'Composition notes',
  collection: 'Visual language',
  tags: ['composition'],
  mimeType: null,
  width: null,
  height: null,
  bytes: null,
  inLibrary: true,
  textContent: 'Describe subject placement, framing, and light.',
  origin: null,
  originalUrl: '/api/creative-studio/files/asset',
  thumbnailUrl: null,
  createdAt: 10,
  updatedAt: 20,
};

function assets(items: CreativeAsset[]): CreativeAssetLibraryPort {
  return {
    list: async () => ({ items, total: items.length }),
    upload: async () => { throw new Error('not used'); },
    createText: async () => { throw new Error('not used'); },
    update: async () => { throw new Error('not used'); },
    remove: async () => undefined,
    renameCollection: async () => 0,
    url: () => '',
  };
}

describe('Nomi prompt library port', () => {
  test('maps independent library text assets with real content', () => {
    expect(mapNomiTextAssetToPromptLibraryItem(TEXT_ASSET)?.prompt).toBe(
      'Describe subject placement, framing, and light.'
    );
    expect(mapNomiTextAssetToPromptLibraryItem({ ...TEXT_ASSET, kind: 'image' })).toBeNull();
    expect(mapNomiTextAssetToPromptLibraryItem({ ...TEXT_ASSET, inLibrary: false })).toBeNull();
    expect(mapNomiTextAssetToPromptLibraryItem({ ...TEXT_ASSET, textContent: ' ' })).toBeNull();
  });

  test('recognizes only catalog provenance mirrors', () => {
    const mirror: CreativeAsset = {
      ...TEXT_ASSET,
      origin: {
        promptLibrarySource: 'catalog',
        promptLibraryId: 'catalog-prompt-1',
        promptCatalogId: 'catalog-prompt-1',
      },
    };
    expect(promptAssetIdentity(mirror)).toEqual({ source: 'catalog', id: 'catalog-prompt-1' });
    expect(mapNomiTextAssetToPromptLibraryItem(mirror)).toBeNull();
  });

  test('combines the dedicated catalog with user text assets', async () => {
    const catalogItem = {
      id: 'catalog-prompt-1',
      source: 'catalog',
      title: 'Catalog prompt',
      description: null,
      prompt: 'Catalog prompt body',
      category: null,
      tags: [],
      knowledgeBaseIds: [],
      coverUrl: null,
      preview: null,
      sourceUrl: null,
      license: null,
      licenseUrl: null,
      createdAt: null,
      updatedAt: null,
      savedToAssets: false,
    };
    const port = createNomiPromptLibraryPort({
      catalog: { list: async () => [catalogItem] },
      assets: assets([TEXT_ASSET]),
    });
    const result = (await port.list()) as Array<{ source: string }>;
    expect(result.map((item) => item.source)).toEqual(['catalog', 'asset']);
  });

  test('marks a catalog prompt saved only while its provenance mirror exists', async () => {
    const catalogItem = {
      id: 'catalog-prompt-1',
      source: 'catalog',
      title: 'Catalog prompt',
      description: null,
      prompt: 'Catalog prompt body',
      category: null,
      tags: [],
      knowledgeBaseIds: [],
      coverUrl: null,
      preview: null,
      sourceUrl: null,
      license: null,
      licenseUrl: null,
      createdAt: null,
      updatedAt: null,
      savedToAssets: false,
    };
    const mirror: CreativeAsset = {
      ...TEXT_ASSET,
      origin: {
        promptLibrarySource: 'catalog',
        promptLibraryId: catalogItem.id,
        promptCatalogId: catalogItem.id,
      },
    };
    let visible = true;
    const mutableAssets: CreativeAssetLibraryPort = {
      ...assets([]),
      list: async (query) => {
        const items = visible && query?.inLibrary ? [mirror] : [];
        return { items, total: items.length };
      },
    };
    const port = createNomiPromptLibraryPort({
      catalog: { list: async () => [catalogItem] },
      assets: mutableAssets,
    });
    expect((await port.list() as Array<{ savedToAssets: boolean }>)[0]?.savedToAssets).toBe(true);
    visible = false;
    expect((await port.list() as Array<{ savedToAssets: boolean }>)[0]?.savedToAssets).toBe(false);
  });
});
