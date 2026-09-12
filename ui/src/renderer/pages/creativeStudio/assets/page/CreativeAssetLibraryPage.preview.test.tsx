/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import { notifyCreativeAssetDeleted } from '../assetDeletion';
import type { CreativeAsset, CreativeAssetLibraryPort } from '../types';

// Arco reads DOM availability on import, so initialize the test DOM first.
const { default: CreativeAssetLibraryPage } = await import('./CreativeAssetLibraryPage');
const { default: CreativeAssetPreviewModal } = await import('./CreativeAssetPreviewModal');

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: {} } },
  interpolation: { escapeValue: false },
});

const asset: CreativeAsset = {
  id: 'preview-video', kind: 'video', title: '竖屏视频', collection: null, tags: [],
  mimeType: 'video/mp4', width: 720, height: 1280, bytes: 1024, inLibrary: true,
  textContent: null, origin: null, originalUrl: '/clip.mp4', thumbnailUrl: '/cover.jpg',
  createdAt: 1, updatedAt: 1,
};

afterEach(cleanup);

describe('CreativeAssetLibraryPage video preview', () => {
  test.each(['image', 'video', 'audio', 'text'] as const)('shows %s in the shared layout with full prompt and optional metadata', async (kind) => {
    const prompt = '纯白色纸张破洞视角，洞边缘有撕纸纤维质感，蓝色头发的小女孩从洞中探出。';
    const current: CreativeAsset = {
      ...asset, kind, title: prompt, origin: { prompt, model: 'agnes-image-2.1-flash', providerId: 'agnes' },
      textContent: kind === 'text' ? '完整文本内容\n第二行' : null,
    };
    const props = { asset: current, onClose: () => undefined, onDownload: () => undefined, locale: 'zh-CN' };
    const page = render(<I18nextProvider i18n={testI18n}><CreativeAssetPreviewModal {...props} /></I18nextProvider>);
    const preview = await waitFor(() => {
      const element = document.querySelector(`[data-creative-asset-preview="${kind}"]`) as HTMLElement;
      expect(element).not.toBeNull();
      return element;
    });
    const visual = preview.querySelector('[data-creative-detail-visual]')!;
    const info = preview.querySelector('[data-creative-detail-info]') as HTMLElement;
    expect(visual.querySelector(kind === 'image' ? 'img' : kind === 'text' ? 'pre' : kind)).not.toBeNull();
    expect(within(info).getByText(prompt)).not.toBeNull();
    expect(within(info).getByRole('heading').textContent).toBe('纯白色纸张破洞视角，洞边缘有撕');
    expect(within(info).getByRole('button', { name: '复制提示词' })).not.toBeNull();
    expect(within(info).getByText('agnes')).not.toBeNull();
    expect(within(info).getByText('720 × 1280')).not.toBeNull();
    expect(within(info).queryByText('合集')).toBeNull();
    expect(within(info).queryByText('素材标签')).toBeNull();
    expect(preview.textContent?.includes('未分组')).toBe(false);
    expect(preview.textContent?.includes('无标签')).toBe(false);

    page.rerender(<I18nextProvider i18n={testI18n}><CreativeAssetPreviewModal {...props} asset={{ ...current, collection: '品牌素材', tags: ['蓝色', ''] }} /></I18nextProvider>);
    expect(within(info).getByText('品牌素材')).not.toBeNull();
    expect(within(info).getByText('蓝色')).not.toBeNull();
  });

  test('uses the shared player and preserves download, deletion and close behavior', async () => {
    const client: CreativeAssetLibraryPort = {
      list: async () => ({ items: [asset], total: 1 }),
      upload: async () => asset,
      createText: async () => asset,
      update: async () => asset,
      remove: async () => undefined,
      renameCollection: async () => 0,
      url: () => asset.originalUrl,
    };
    const page = render(
      <I18nextProvider i18n={testI18n}>
        <CreativeAssetLibraryPage client={client} locale='zh-CN' />
      </I18nextProvider>
    );

    const trigger = await page.findByRole('button', { name: `更多：${asset.title}` });
    fireEvent.click(trigger);
    const menu = await within(document.body).findByRole('menu');
    fireEvent.click(within(menu).getByRole('menuitem', { name: '查看' }));

    await waitFor(() => expect(document.querySelector('[data-creative-asset-preview="video"]')).toBeTruthy());
    const preview = document.querySelector('[data-creative-asset-preview="video"]') as HTMLElement;
    const player = preview.querySelector('[data-creative-video-player]');
    const video = player?.querySelector('video');
    expect(video?.getAttribute('src')).toBe(asset.originalUrl);
    expect(video?.getAttribute('poster')).toBe(asset.thumbnailUrl);
    expect(video?.hasAttribute('controls')).toBe(false);
    expect(video?.hasAttribute('disablepictureinpicture')).toBe(true);
    expect(preview.textContent?.includes('画中画')).toBe(false);

    const downloads: Array<{ href: string | null; filename: string }> = [];
    const anchorClick = HTMLAnchorElement.prototype.click;
    HTMLAnchorElement.prototype.click = function () {
      downloads.push({ href: this.getAttribute('href'), filename: this.download });
    };
    try {
      const download = within(preview).getByRole('button', { name: '下载原始文件' }) as HTMLButtonElement;
      fireEvent.click(download);
      expect(downloads).toEqual([{ href: asset.originalUrl, filename: `${asset.title}.mp4` }]);

      act(() => notifyCreativeAssetDeleted(client, asset.id));
      await waitFor(() => expect(preview.querySelector('video')).toBeNull());
      expect(within(preview).getByRole('status').textContent).toBe('素材已删除');
      expect(download.disabled).toBe(true);
      fireEvent.click(download);
      expect(downloads).toHaveLength(1);
    } finally {
      HTMLAnchorElement.prototype.click = anchorClick;
    }

    fireEvent.click(within(preview).getByRole('button', { name: '关闭' }));
    await waitFor(() => expect(document.querySelector('[data-creative-asset-preview]')).toBeNull());
  });
});
