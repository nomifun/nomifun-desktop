/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import { ResourceSpaceChromeProvider } from '../../app/ResourceSpaceChrome';
import type { CreativeAsset, CreativeAssetLibraryPort, CreativeAssetQuery } from '../types';
import CreativeAssetLibraryPage from './CreativeAssetLibraryPage';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
  interpolation: { escapeValue: false },
});

const assets: CreativeAsset[] = [1, 2].map((index) => ({
  id: `resource-asset-${index}`, kind: 'image', title: `Asset ${index}`, collection: '合集', tags: [],
  mimeType: 'image/png', width: 1280, height: 720, bytes: 2048, inLibrary: true,
  textContent: null, origin: null, originalUrl: `/assets/${index}`, thumbnailUrl: `/assets/${index}/thumb`,
  createdAt: index, updatedAt: index,
}));

function createClient(calls: CreativeAssetQuery[] = []): CreativeAssetLibraryPort {
  return {
    list: async (query = {}) => {
      calls.push(query);
      const items = query.search ? assets.filter((asset) => asset.title.includes(query.search!)) : assets;
      return { items, total: items.length };
    },
    upload: async () => assets[0],
    createText: async () => assets[0],
    update: async () => assets[0],
    remove: async () => undefined,
    renameCollection: async () => 0,
    url: (id) => `/assets/${id}`,
  };
}

function renderPage(client: CreativeAssetLibraryPort, headerActions?: HTMLElement) {
  const page = <CreativeAssetLibraryPage client={client} locale='zh-CN' />;
  return render(
    <I18nextProvider i18n={testI18n}>
      {headerActions ? (
        <ResourceSpaceChromeProvider value={{ section: 'assets', headerActions, taskActions: null }}>
          {page}
        </ResourceSpaceChromeProvider>
      ) : page}
    </I18nextProvider>
  );
}

function createHeaderActions() {
  const header = document.createElement('div');
  header.dataset.testResourceActions = 'true';
  document.body.append(header);
  return header;
}

afterEach(() => {
  cleanup();
  document.querySelectorAll('[data-test-resource-actions]').forEach((element) => element.remove());
});

describe('CreativeAssetLibraryPage in Resource Space', () => {
  test('moves only the page chrome and preserves the original asset card and batch actions', async () => {
    const client = createClient();
    const standalone = renderPage(client);
    await standalone.findByText('Asset 1');
    const originalCard = standalone.container.querySelector('[data-asset-id="resource-asset-1"]')!.outerHTML;
    standalone.unmount();

    const header = createHeaderActions();
    const page = renderPage(client, header);
    await page.findByText('Asset 1');
    expect(page.container.querySelector('[data-asset-id="resource-asset-1"]')!.outerHTML).toBe(originalCard);
    expect(page.container.querySelector('[data-resource-space-embedded="true"]')).toBeTruthy();
    expect(page.container.querySelector('main')).toBeNull();
    expect(page.container.querySelector('h1')).toBeNull();
    expect(page.container.querySelector('[role="search"]')).toBeNull();
    expect(within(header).getByRole('searchbox', { name: '搜索' })).toBeTruthy();
    expect(within(header).getByRole('button', { name: '上传素材' })).toBeTruthy();
    expect(within(header).getByRole('button', { name: '新建文本' })).toBeTruthy();
    expect(page.getByRole('navigation', { name: '素材分页' })).toBeTruthy();

    fireEvent.click(page.getByRole('checkbox', { name: '选择素材: Asset 1' }));
    const toolbar = within(page.container.querySelector('[data-asset-selection-toolbar]') as HTMLElement);
    expect(toolbar.getByText('已选择 1 项')).toBeTruthy();
    expect(toolbar.getByRole('button', { name: '删除' })).toBeTruthy();
    expect(page.queryByRole('button', { name: '列表视图' })).toBeNull();
    expect(page.queryByRole('button', { name: '网格视图' })).toBeNull();
    expect(page.container.querySelector('[data-asset-view="grid"]')).toBeTruthy();
    expect((page.getByRole('checkbox', { name: '选择素材: Asset 1' }) as HTMLInputElement).checked).toBe(true);
  });

  test('keeps portaled search submit, clear, text creation and upload controls connected to the page', async () => {
    const calls: CreativeAssetQuery[] = [];
    const header = createHeaderActions();
    const page = renderPage(createClient(calls), header);
    await page.findByText('Asset 1');
    const actions = within(header);
    const search = actions.getByRole('searchbox', { name: '搜索' });
    fireEvent.change(search, { target: { value: 'Asset 2' } });
    fireEvent.submit(actions.getByRole('search'));
    await waitFor(() => expect(calls.some((query) => query.search === 'Asset 2')).toBe(true));
    await waitFor(() => expect(page.queryByText('Asset 1')).toBeNull());
    fireEvent.click(actions.getByRole('button', { name: '清除搜索' }));
    await page.findByText('Asset 1');
    expect((search as HTMLInputElement).value).toBe('');

    const fileInput = page.container.querySelector('input[type="file"]') as HTMLInputElement;
    const filePicker = spyOn(fileInput, 'click').mockImplementation(() => undefined);
    fireEvent.click(actions.getByRole('button', { name: '上传素材' }));
    expect(filePicker).toHaveBeenCalledTimes(1);
    filePicker.mockRestore();
    expect(fileInput.accept).toBe('image/*,video/*');

    fireEvent.click(actions.getByRole('button', { name: '新建文本' }));
    await waitFor(() => expect(document.querySelector('[data-create-text-asset-form]')).toBeTruthy());
  });
});
