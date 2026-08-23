/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import React from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import type { CreativeAsset, CreativeAssetLibraryPort, CreativeAssetQuery } from '../types';
import CreativeAssetLibraryPage from './CreativeAssetLibraryPage';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
  interpolation: { escapeValue: false },
});

const createAsset = (index: number): CreativeAsset => ({
  id: `asset-${index}`,
  kind: 'image',
  title: `Asset ${index}`,
  collection: null,
  tags: [],
  mimeType: 'image/png',
  width: 100,
  height: 100,
  bytes: 100,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/assets/${index}`,
  thumbnailUrl: `/assets/${index}/thumb`,
  createdAt: index,
  updatedAt: index,
});

const assets = Array.from({ length: 25 }, (_, index) => createAsset(index + 1));

function createClient(calls: CreativeAssetQuery[]): CreativeAssetLibraryPort {
  return {
    list: async (query = {}) => {
      calls.push(query);
      const page = query.page ?? 1;
      const pageSize = query.pageSize ?? 10;
      const start = (page - 1) * pageSize;
      return { items: assets.slice(start, start + pageSize), total: assets.length };
    },
    upload: async () => createAsset(26),
    createText: async () => createAsset(26),
    update: async () => createAsset(1),
    remove: async () => undefined,
    renameCollection: async () => 0,
    url: (assetId) => `/assets/${assetId}`,
  };
}

const renderPage = (client: CreativeAssetLibraryPort) =>
  render(
    <I18nextProvider i18n={testI18n}>
      <CreativeAssetLibraryPage client={client} locale='zh-CN' />
    </I18nextProvider>
  );

afterEach(() => {
  cleanup();
});

describe('CreativeAssetLibraryPage pagination', () => {
  test('loads only the first backend page on mount', async () => {
    const calls: CreativeAssetQuery[] = [];
    const page = renderPage(createClient(calls));

    await waitFor(() => expect(page.getByText('Asset 1')).toBeTruthy());

    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ page: 1, pageSize: 10 });
    expect(page.queryByText('Asset 11')).toBeNull();
  });

  test('requests the next backend page only after the user navigates to it', async () => {
    const calls: CreativeAssetQuery[] = [];
    const page = renderPage(createClient(calls));

    await waitFor(() => expect(page.getByText('Asset 1')).toBeTruthy());
    fireEvent.click(page.getByRole('button', { name: '下一页' }));

    await waitFor(() => expect(page.getByText('Asset 11')).toBeTruthy());

    expect(calls).toHaveLength(2);
    expect(calls[0]).toMatchObject({ page: 1, pageSize: 10 });
    expect(calls[1]).toMatchObject({ page: 2, pageSize: 10 });
    expect(page.queryByText('Asset 1')).toBeNull();
  });

  test('retries a failed target page after refreshing the first page', async () => {
    const calls: CreativeAssetQuery[] = [];
    let pageTwoAttempts = 0;
    const client = createClient(calls);
    client.list = async (query = {}) => {
      calls.push(query);
      const requestedPage = query.page ?? 1;
      if (requestedPage === 2 && pageTwoAttempts++ === 0) {
        throw new Error('page 2 failed');
      }
      const pageSize = query.pageSize ?? 10;
      const start = (requestedPage - 1) * pageSize;
      return {
        items: assets.slice(start, start + pageSize),
        total: assets.length,
      };
    };

    const page = renderPage(client);
    await waitFor(() => expect(page.getByText('Asset 1')).toBeTruthy());
    fireEvent.click(page.getByRole('button', { name: '下一页' }));
    await waitFor(() => expect(page.getByText('page 2 failed')).toBeTruthy());

    fireEvent.click(page.getByRole('button', { name: '重试' }));
    await waitFor(() => expect(page.getByText('Asset 11')).toBeTruthy());

    expect(calls.map((query) => query.page ?? 1)).toEqual([1, 2, 1, 2]);
  });
});
