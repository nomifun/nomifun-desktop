/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import { parseAssetId } from '@/common/types/ids';

import type {
  CreativeAsset,
  CreativeAssetLibraryPort,
  CreativePromptAssetPort,
} from '../../assets';
import { listCreativeAssetsCached } from '../../assets/creativeAssetQueryCache';
import type { PromptLibraryItem, PromptLibraryPort } from '..';
import { CreativeStudioPromptsRoute } from './CreativeStudioPromptsRoute';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: {} } },
  interpolation: { escapeValue: false },
});

const ITEM: PromptLibraryItem = {
  id: 'catalog-prompt-1',
  source: 'catalog',
  title: 'Catalog prompt',
  description: null,
  prompt: 'Create a precise product photograph.',
  category: 'Photography',
  tags: ['product'],
  knowledgeBaseIds: [],
  coverUrl: null,
  preview: null,
  sourceUrl: 'https://example.test/source',
  license: 'MIT',
  licenseUrl: 'https://example.test/license',
  createdAt: null,
  updatedAt: null,
  savedToAssets: false,
};

const CREATED_ASSET: CreativeAsset = {
  id: parseAssetId('0190f5fe-7c00-7a00-8000-000000000099'),
  kind: 'text',
  title: ITEM.title,
  collection: 'Prompts',
  tags: ITEM.tags,
  mimeType: null,
  width: null,
  height: null,
  bytes: null,
  inLibrary: true,
  textContent: ITEM.prompt,
  origin: {
    promptLibrarySource: 'catalog',
    promptLibraryId: ITEM.id,
    promptCatalogId: ITEM.id,
  },
  originalUrl: '/api/creative-studio/files/saved-prompt',
  thumbnailUrl: null,
  createdAt: 100,
  updatedAt: 100,
};

afterEach(() => cleanup());

describe('Creative Studio prompt save interaction', () => {
  test('adds, atomically removes, reopens as absent, and can add again', async () => {
    const port: PromptLibraryPort = { list: async () => [ITEM] };
    let createCalls = 0;
    let finishCreate: ((asset: CreativeAsset) => void) | undefined;
    const createResult = new Promise<CreativeAsset>((resolve) => {
      finishCreate = resolve;
    });
    let removeCalls = 0;
    let listCalls = 0;
    let removedIdentity: [string, string] | null = null;
    let finishRemove: ((matched: number) => void) | undefined;
    const removeResult = new Promise<number>((resolve) => {
      finishRemove = resolve;
    });
    const assetPort: CreativeAssetLibraryPort & CreativePromptAssetPort = {
      list: async () => {
        listCalls += 1;
        return { items: [], total: 0 };
      },
      upload: async () => {
        throw new Error('not used');
      },
      createText: async () => {
        createCalls += 1;
        return createResult;
      },
      removePromptAsset: async (source, promptId) => {
        removeCalls += 1;
        removedIdentity = [source, promptId];
        return removeResult;
      },
      update: async () => {
        throw new Error('not used');
      },
      remove: async () => undefined,
      renameCollection: async () => 0,
      url: () => '',
    };
    await listCreativeAssetsCached(assetPort, { inLibrary: true });
    await listCreativeAssetsCached(assetPort, { inLibrary: true });
    expect(listCalls).toBe(1);

    const { container } = render(
      <I18nextProvider i18n={testI18n}>
        <CreativeStudioPromptsRoute
          port={port}
          assetPort={assetPort}
          locale='en-US'
          notifySuccess={() => {
            throw new Error('toast unavailable');
          }}
          notifyError={() => undefined}
        />
      </I18nextProvider>
    );
    const page = within(container.ownerDocument.body);

    const openDetails = await page.findByRole('button', {
      name: 'View prompt: Catalog prompt',
    });
    fireEvent.click(openDetails);
    const addButton = await page.findByRole('button', { name: 'Add to my assets' });
    fireEvent.click(addButton);
    fireEvent.click(addButton);
    expect(createCalls).toBe(1);

    await act(async () => finishCreate?.(CREATED_ASSET));
    const removeButton = await page.findByRole('button', { name: 'Remove from my assets' });
    await listCreativeAssetsCached(assetPort, { inLibrary: true });
    expect(listCalls).toBe(2);
    fireEvent.click(removeButton);
    fireEvent.click(removeButton);
    expect(removeCalls).toBe(1);
    expect(removedIdentity).toEqual(['catalog', ITEM.id]);

    await act(async () => finishRemove?.(1));
    await page.findByRole('button', { name: 'Add to my assets' });
    await listCreativeAssetsCached(assetPort, { inLibrary: true });
    expect(listCalls).toBe(3);

    const closeButton = container.ownerDocument.querySelector<HTMLElement>(
      '.arco-modal-close-icon'
    );
    expect(closeButton).not.toBeNull();
    fireEvent.click(closeButton!);
    fireEvent.click(openDetails);

    const rememberedButton = await page.findByRole('button', { name: 'Add to my assets' });
    fireEvent.click(rememberedButton);
    await page.findByRole('button', { name: 'Remove from my assets' });
    expect(createCalls).toBe(2);
    await listCreativeAssetsCached(assetPort, { inLibrary: true });
    expect(listCalls).toBe(4);
  });

  test('keeps a failed removal retryable and remembers a later success', async () => {
    const savedItem = { ...ITEM, savedToAssets: true };
    const port: PromptLibraryPort = { list: async () => [savedItem] };
    let removeCalls = 0;
    const assetPort: CreativeAssetLibraryPort & CreativePromptAssetPort = {
      list: async () => ({ items: [], total: 0 }),
      upload: async () => {
        throw new Error('not used');
      },
      createText: async () => CREATED_ASSET,
      removePromptAsset: async () => {
        removeCalls += 1;
        if (removeCalls === 1) throw new Error('remove denied');
        return 1;
      },
      update: async () => {
        throw new Error('not used');
      },
      remove: async () => undefined,
      renameCollection: async () => 0,
      url: () => '',
    };

    const { container } = render(
      <I18nextProvider i18n={testI18n}>
        <CreativeStudioPromptsRoute
          port={port}
          assetPort={assetPort}
          locale='en-US'
          notifySuccess={() => undefined}
          notifyError={() => undefined}
        />
      </I18nextProvider>
    );
    const page = within(container.ownerDocument.body);
    const openDetails = await page.findByRole('button', {
      name: 'View prompt: Catalog prompt',
    });
    fireEvent.click(openDetails);
    fireEvent.click(await page.findByRole('button', { name: 'Remove from my assets' }));

    expect((await page.findByRole('alert')).textContent?.includes('remove denied')).toBe(true);
    const retryButton = page.getByRole('button', { name: 'Remove from my assets' });
    fireEvent.click(retryButton);
    await page.findByRole('button', { name: 'Add to my assets' });
    expect(removeCalls).toBe(2);

    const closeButton = container.ownerDocument.querySelector<HTMLElement>(
      '.arco-modal-close-icon'
    );
    fireEvent.click(closeButton!);
    fireEvent.click(openDetails);
    await page.findByRole('button', { name: 'Add to my assets' });
  });
});
