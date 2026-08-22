/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'vitest';

import { createCreativePromptCatalogPort } from './catalogPort';

const ITEM = {
  id: 'awesome-gpt-image-001',
  title: '纸雕海报',
  coverUrl: 'https://raw.githubusercontent.com/example/image.jpg',
  prompt: 'Create a paper poster',
  tags: ['poster'],
  category: 'awesome-gpt-image',
  sourceUrl: 'https://github.com/ZeroLu/awesome-gpt-image',
  license: 'MIT',
  licenseUrl: 'https://github.com/ZeroLu/awesome-gpt-image/blob/main/LICENSE',
  preview: '![](https://raw.githubusercontent.com/example/image.jpg)',
  createdAt: null,
  updatedAt: '2026-08-21T00:00:00Z',
};

function page(items: unknown[], stale: boolean) {
  return { items, total: items.length, tags: [], categories: [], syncedAt: null, stale, sources: [] };
}

describe('Creative prompt catalog HTTP port', () => {
  test('synchronizes an empty cache and maps attribution', async () => {
    let syncCalls = 0;
    const port = createCreativePromptCatalogPort({
      transport: {
        list: async () => page([], true),
        sync: async () => {
          syncCalls += 1;
          return page([ITEM], false);
        },
      },
    });

    const [first, second] = await Promise.all([port.list(), port.list()]);
    expect(syncCalls).toBe(1);
    expect(first).toEqual(second);
    const mapped = (first as Array<Record<string, unknown>>)[0];
    expect(mapped?.source).toBe('catalog');
    expect(mapped?.title).toBe('纸雕海报');
    expect(mapped?.sourceUrl).toBe('https://github.com/ZeroLu/awesome-gpt-image');
    expect(mapped?.license).toBe('MIT');
    expect(mapped?.updatedAt).toBe(Date.parse('2026-08-21T00:00:00Z'));
  });

  test('keeps a valid stale cache when refresh is offline', async () => {
    const port = createCreativePromptCatalogPort({
      transport: {
        list: async () => page([ITEM], true),
        sync: async () => {
          throw new Error('offline');
        },
      },
    });
    expect(await port.list()).toHaveLength(1);
  });

  test('fails closed for inconsistent responses and empty offline bootstrap', async () => {
    const invalid = createCreativePromptCatalogPort({
      transport: {
        list: async () => ({ ...page([ITEM], false), total: 2 }),
        sync: async () => page([], false),
      },
    });
    let invalidError: unknown;
    try {
      await invalid.list();
    } catch (error) {
      invalidError = error;
    }
    expect((invalidError as Error).message.includes('total is inconsistent')).toBe(true);

    const offline = createCreativePromptCatalogPort({
      transport: {
        list: async () => page([], true),
        sync: async () => {
          throw new Error('offline');
        },
      },
    });
    let offlineError: unknown;
    try {
      await offline.list();
    } catch (error) {
      offlineError = error;
    }
    expect((offlineError as Error).message).toBe('offline');
  });
});
