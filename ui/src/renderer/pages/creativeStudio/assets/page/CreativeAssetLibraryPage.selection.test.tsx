/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import { BackendHttpError } from '@/common/adapter/httpBridge';
import { notifyCreativeAssetDeleted } from '../assetDeletion';
import type { CreativeAsset, CreativeAssetLibraryPort } from '../types';
import CreativeAssetLibraryPage from './CreativeAssetLibraryPage';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
  interpolation: { escapeValue: false },
});

const createAsset = (index: number): CreativeAsset => ({
  id: `asset-${index}`, kind: 'image', title: `Asset ${index}`, collection: '合集', tags: [],
  mimeType: 'image/png', width: 100, height: 100, bytes: 100, inLibrary: true,
  textContent: null, origin: null, originalUrl: `/assets/${index}`, thumbnailUrl: null,
  createdAt: index, updatedAt: index,
});

function createClient(count = 3, beforeRemove?: (id: string) => Promise<void>) {
  let assets = Array.from({ length: count }, (_, index) => createAsset(index + 1));
  const removedIds: string[] = [];
  const client: CreativeAssetLibraryPort = {
    list: async (query = {}) => {
      const filtered = query.kind ? assets.filter((asset) => asset.kind === query.kind) : assets;
      const pageSize = query.pageSize ?? 10;
      const start = ((query.page ?? 1) - 1) * pageSize;
      return { items: filtered.slice(start, start + pageSize), total: filtered.length };
    },
    remove: async (id) => {
      removedIds.push(id);
      await beforeRemove?.(id);
      assets = assets.filter((asset) => asset.id !== id);
    },
    upload: async () => createAsset(count + 1),
    createText: async () => createAsset(count + 1),
    update: async () => createAsset(1),
    renameCollection: async () => 0,
    url: (id) => `/assets/${id}`,
  };
  return { client, removedIds };
}

const renderPage = (client: CreativeAssetLibraryPort) => render(
  <I18nextProvider i18n={testI18n}>
    <CreativeAssetLibraryPage client={client} locale='zh-CN' />
  </I18nextProvider>
);

const selectionToolbar = () => within(document.querySelector('[data-asset-selection-toolbar]') as HTMLElement);
const deleteDialog = () => within(document.querySelector('.arco-modal') as HTMLElement);

beforeEach(() => {
  spyOn(Message, 'success').mockReturnValue(() => {});
});
afterEach(() => {
  cleanup();
  mock.restore();
});

describe('CreativeAssetLibraryPage selection', () => {
  test('selects without opening a preview, supports select all and cancel, then deletes only checked assets', async () => {
    const { client, removedIds } = createClient();
    const page = renderPage(client);
    const first = await page.findByRole('checkbox', { name: '选择素材: Asset 1' });
    const third = page.getByRole('checkbox', { name: '选择素材: Asset 3' });
    expect(page.queryByRole('button', { name: '删除' })).toBeNull();
    expect(document.querySelector('[data-asset-selection-bar]')).toBeNull();
    fireEvent.click(first);
    fireEvent.click(third);
    expect(document.querySelector('[data-creative-asset-preview]')).toBeNull();
    expect(selectionToolbar().getByText('已选择 2 项')).toBeTruthy();

    fireEvent.click(selectionToolbar().getByRole('checkbox', { name: '全选本页' }));
    expect(page.getAllByRole('checkbox').every((input) => (input as HTMLInputElement).checked)).toBe(true);
    fireEvent.click(selectionToolbar().getByRole('checkbox', { name: '全选本页' }));
    expect(page.getAllByRole('checkbox').some((input) => (input as HTMLInputElement).checked)).toBe(false);
    expect(selectionToolbar().queryByRole('button', { name: '删除' })).toBeNull();

    fireEvent.click(selectionToolbar().getByRole('checkbox', { name: '全选本页' }));
    fireEvent.click(selectionToolbar().getByRole('button', { name: '取消选择' }));
    expect(page.getAllByRole('checkbox').some((input) => (input as HTMLInputElement).checked)).toBe(false);

    fireEvent.click(first);
    fireEvent.click(third);
    fireEvent.click(selectionToolbar().getByRole('button', { name: '删除' }));
    await waitFor(() => expect(deleteDialog().getByText(/确定永久删除选中的 2 项素材/)).toBeTruthy());
    fireEvent.click(deleteDialog().getByRole('button', { name: '取消' }));
    expect(removedIds).toEqual([]);
    expect((first as HTMLInputElement).checked).toBe(true);

    fireEvent.click(selectionToolbar().getByRole('button', { name: '删除' }));
    fireEvent.click(deleteDialog().getByRole('button', { name: '永久删除' }));
    await waitFor(() => {
      expect(page.queryByText('Asset 1')).toBeNull();
      expect(page.queryByText('Asset 3')).toBeNull();
      expect(page.getByText('Asset 2')).toBeTruthy();
      expect(selectionToolbar().queryByRole('button', { name: '删除' })).toBeNull();
    });
    expect(removedIds).toEqual(['asset-1', 'asset-3']);
  }, 15000);

  test('continues after a failed deletion and retries only failed assets', async () => {
    let blocked = true;
    const { client, removedIds } = createClient(3, async (id) => {
      if (id === 'asset-2' && blocked) throw new BackendHttpError({
        method: 'DELETE', path: '/assets/asset-2', status: 409, body: { error: 'in use' },
      });
    });
    const page = renderPage(client);
    fireEvent.click(await page.findByRole('checkbox', { name: '选择素材: Asset 1' }));
    fireEvent.click(selectionToolbar().getByRole('checkbox', { name: '全选本页' }));
    fireEvent.click(selectionToolbar().getByRole('button', { name: '删除' }));
    fireEvent.click(deleteDialog().getByRole('button', { name: '永久删除' }));

    await waitFor(() => {
      expect(deleteDialog().getByRole('alert').textContent).toContain('已删除 2 项，1 项未能删除');
      expect(deleteDialog().getByRole('alert').textContent).toContain('正在执行的生成任务');
      expect((page.getByRole('checkbox', { name: '选择素材: Asset 2' }) as HTMLInputElement).checked).toBe(true);
      expect((deleteDialog().getByRole('button', { name: '永久删除' }) as HTMLButtonElement).disabled).toBe(false);
    });
    expect(removedIds).toEqual(['asset-1', 'asset-2', 'asset-3']);

    blocked = false;
    fireEvent.click(deleteDialog().getByRole('button', { name: '永久删除' }));
    await waitFor(() => expect(page.queryByRole('checkbox', { name: '选择素材: Asset 2' })).toBeNull());
    expect(removedIds).toEqual(['asset-1', 'asset-2', 'asset-3', 'asset-2']);
  }, 15000);

  test('locks selection during deletion and prevents duplicate submissions', async () => {
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    const { client, removedIds } = createClient(3, () => pending);
    const page = renderPage(client);
    fireEvent.click(await page.findByRole('checkbox', { name: '选择素材: Asset 1' }));
    fireEvent.click(selectionToolbar().getByRole('button', { name: '删除' }));
    const confirm = deleteDialog().getByRole('button', { name: '永久删除' });
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    expect(removedIds).toEqual(['asset-1']);
    expect(page.getAllByRole('checkbox').every((input) => (input as HTMLInputElement).disabled)).toBe(true);
    expect((selectionToolbar().getByRole('button', { name: '取消选择' }) as HTMLButtonElement).disabled).toBe(true);
    await act(async () => { finish(); await pending; });
    await waitFor(() => expect(page.queryByRole('checkbox', { name: '选择素材: Asset 1' })).toBeNull());
    expect(removedIds).toEqual(['asset-1']);
  });

  test('clears selection on pagination, filtering and external deletion', async () => {
    const { client } = createClient(12);
    const page = renderPage(client);
    fireEvent.click(await page.findByRole('checkbox', { name: '选择素材: Asset 1' }));
    fireEvent.click(page.getByRole('button', { name: '下一页' }));
    const next = await page.findByRole('checkbox', { name: '选择素材: Asset 11' });
    expect(selectionToolbar().queryByRole('button', { name: '删除' })).toBeNull();
    fireEvent.click(next);
    act(() => notifyCreativeAssetDeleted(client, 'asset-11'));
    expect(page.queryByText('已选择 1 项')).toBeNull();

    fireEvent.click(page.getByRole('button', { name: '上一页' }));
    fireEvent.click(await page.findByRole('checkbox', { name: '选择素材: Asset 1' }));
    fireEvent.click(within(page.getByRole('group', { name: '类型' })).getByRole('button', { name: '图片' }));
    await waitFor(() => expect(page.queryByText('已选择 1 项')).toBeNull());
    expect((await page.findByRole('checkbox', { name: '选择素材: Asset 1' }) as HTMLInputElement).checked).toBe(false);
  });
});
