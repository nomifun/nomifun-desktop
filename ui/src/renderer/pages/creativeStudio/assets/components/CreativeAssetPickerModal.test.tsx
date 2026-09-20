/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';

import creativeStudio from '../../../../services/i18n/locales/zh-CN/creativeStudio.json';
import type { CreativeAsset } from '../types';
import CreativeAssetPickerModal from './CreativeAssetPickerModal';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: false,
  resources: { 'zh-CN': { translation: { creativeStudio } } },
});

afterEach(cleanup);

const asset = (kind: CreativeAsset['kind']): CreativeAsset => ({
  id: `asset-${kind}`,
  kind,
  title: `${kind === 'image' ? '图片' : kind === 'video' ? '视频' : kind === 'audio' ? '音频' : '文本'}素材`,
  collection: null,
  tags: [],
  mimeType: kind === 'text' ? null : `${kind}/example`,
  width: kind === 'image' || kind === 'video' ? 1280 : null,
  height: kind === 'image' || kind === 'video' ? 720 : null,
  bytes: kind === 'text' ? null : 2048,
  inLibrary: true,
  textContent: kind === 'text' ? '文本素材内容' : null,
  origin: null,
  originalUrl: `/assets/${kind}`,
  thumbnailUrl: kind === 'image' || kind === 'video' ? `/assets/${kind}/thumbnail` : null,
  createdAt: 1,
  updatedAt: 2,
});

const assets = (['image', 'video', 'audio', 'text'] as const).map(asset);

describe('CreativeAssetPickerModal', () => {
  test('shows every authoritative asset kind while disabling unsupported selections', () => {
    const toggled: string[] = [];
    const page = render(
      <I18nextProvider i18n={i18n}>
        <CreativeAssetPickerModal
          open
          assets={assets}
          acceptedKinds={['image', 'text']}
          selectedIds={[]}
          loading={false}
          hasMore={false}
          onToggle={(item) => toggled.push(item.id)}
          onLoadMore={() => undefined}
          onCancel={() => undefined}
          onConfirm={() => undefined}
        />
      </I18nextProvider>
    );

    const listbox = page.getByRole('listbox');
    expect(within(listbox).getAllByRole('option')).toHaveLength(4);
    expect((within(listbox).getByRole('option', { name: '视频素材，当前创作模式不可选择' }) as HTMLButtonElement).disabled).toBe(true);
    expect((within(listbox).getByRole('option', { name: '音频素材，当前创作模式不可选择' }) as HTMLButtonElement).disabled).toBe(true);
    expect((within(listbox).getByRole('option', { name: '图片素材，未选择' }) as HTMLButtonElement).disabled).toBe(false);
    expect(document.querySelector('[data-creative-media-preview="video"]')).not.toBeNull();
    expect(document.querySelector('[data-asset-media-state="audio"]')).not.toBeNull();

    fireEvent.click(within(listbox).getByRole('option', { name: '视频素材，当前创作模式不可选择' }));
    fireEvent.click(within(listbox).getByRole('option', { name: '图片素材，未选择' }));
    expect(toggled).toEqual(['asset-image']);

    fireEvent.click(page.getByRole('button', { name: '视频' }));
    expect(within(page.getByRole('listbox')).getAllByRole('option')).toHaveLength(1);
    expect(within(page.getByRole('listbox')).getByRole('option', { name: '视频素材，当前创作模式不可选择' })).toBeDefined();
  });
});
