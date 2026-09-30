/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider } from 'react-i18next';
import common from '../../../../services/i18n/locales/zh-CN/common.json';
import zh from '../../../../services/i18n/locales/zh-CN/creativeStudio.json';
import type { CreativeAsset } from '../../assets';
import type { CreativeCanvasNode } from '../../domain';
import type { CreativeModelCatalogSnapshot } from '../../models';
import CreativeCanvasImageToolbar from './CreativeCanvasImageToolbar';
import CreativeImageCropDialog from './CreativeImageCropDialog';
import CreativeImageMaskEditDialog from './CreativeImageMaskEditDialog';
import CreativeImagePreviewDialog from './CreativeImagePreviewDialog';
import CreativeImageSplitDialog from './CreativeImageSplitDialog';

afterEach(() => {
  cleanup();
  document.getElementById('resource-page-portal-root')?.remove();
});
const i18n = createInstance();
await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { creativeStudio: zh, common } } } });

const node: Extract<CreativeCanvasNode, { type: 'image' }> = {
  id: 'image-node', type: 'image', position: { x: 100, y: 80 },
  size: { width: 320, height: 220 }, groupId: null, zIndex: 1, locked: false,
  data: { assetId: 'image-asset', caption: '猫咪', alt: '猫咪', fit: 'contain', naturalSize: { width: 1920, height: 1080 }, composer: null },
};
const asset: CreativeAsset = {
  id: 'image-asset', kind: 'image', title: '猫咪原图', collection: null, tags: [],
  mimeType: 'image/png', width: 1920, height: 1080, bytes: 1, inLibrary: true,
  textContent: null, origin: null, originalUrl: '/original.png', thumbnailUrl: '/thumbnail.png',
  createdAt: 1, updatedAt: 1,
};
const emptyCatalog: CreativeModelCatalogSnapshot = {
  status: 'ready',
  providers: [],
  error: null,
};

const Harness = ({ resolveAsset, onCanvasInput = () => {} }: {
  resolveAsset: () => Promise<CreativeAsset>;
  onCanvasInput?: () => void;
}) => {
  const [visible, setVisible] = useState(false);
  return (
    <I18nextProvider i18n={i18n}>
      <section data-testid='canvas-content'>
      <div onClick={onCanvasInput} onKeyDown={onCanvasInput}>
        <CreativeCanvasImageToolbar
          nodeId={node.id} visible hasImageContent disabled
          onPreview={() => setVisible(true)}
          onInfo={() => {}} onDelete={() => {}} onUpload={() => {}}
          onCrop={() => {}} onDownload={() => {}} onSplit={() => {}}
        ><article>image node</article></CreativeCanvasImageToolbar>
      </div>
      {visible ? <CreativeImagePreviewDialog node={node} resolveAsset={resolveAsset} onClose={() => setVisible(false)} /> : null}
      <div id='resource-page-portal-root' />
      </section>
    </I18nextProvider>
  );
};

describe('canvas image preview', () => {
  test.each([
    ['preview', () => (
      <CreativeImagePreviewDialog
        node={node}
        resolveAsset={() => new Promise<CreativeAsset>(() => {})}
        onClose={() => {}}
      />
    )],
    ['crop', () => (
      <CreativeImageCropDialog visible asset={asset} onClose={() => {}} onConfirm={() => {}} />
    )],
    ['mask edit', () => (
      <CreativeImageMaskEditDialog
        visible
        asset={asset}
        catalog={emptyCatalog}
        model={null}
        onModelChange={() => {}}
        onClose={() => {}}
        onConfirm={() => {}}
      />
    )],
    ['split', () => (
      <CreativeImageSplitDialog visible asset={asset} onClose={() => {}} onConfirm={() => {}} />
    )],
  ] as const)('mounts the %s modal above viewport-portaled canvas controls', (_name, dialog) => {
    const resourcePortal = document.createElement('div');
    resourcePortal.id = 'resource-page-portal-root';
    document.body.append(resourcePortal);

    const view = render(<I18nextProvider i18n={i18n}>{dialog()}</I18nextProvider>);
    const modal = view.getByRole('dialog');
    const wrapper = modal.closest<HTMLElement>('.arco-modal-wrapper');
    const mask = wrapper?.parentElement?.querySelector<HTMLElement>('.arco-modal-mask');

    expect(wrapper?.style.zIndex).toBe('1700');
    expect(mask?.style.zIndex).toBe('1700');
    expect(resourcePortal.contains(wrapper)).toBe(false);
    expect(document.body.contains(wrapper)).toBe(true);
  });

  test('opens the original image, zooms independently and restores focus after closing', async () => {
    let canvasInputs = 0;
    const view = render(<Harness resolveAsset={async () => asset} onCanvasInput={() => { canvasInputs += 1; }} />);
    const previewButton = view.getByRole('button', { name: '预览图片' });
    previewButton.focus();
    fireEvent.click(previewButton);
    const image = await view.findByAltText('猫咪原图');
    const dialog = view.getByRole('dialog');
    expect(view.getByTestId('canvas-content').contains(dialog)).toBe(false);
    expect(dialog.querySelector('.arco-modal-footer')).toBeNull();
    expect(view.getByRole('group', { name: '图片缩放' })).not.toBeNull();
    expect(within(dialog).getByRole('button', { name: '图片另存为' })).not.toBeNull();
    expect(image.getAttribute('src')).toBe('/original.png');
    Object.defineProperties(image, { naturalWidth: { value: 1920 }, naturalHeight: { value: 1080 } });
    fireEvent.load(image);
    const fitButton = view.getByRole('button', { name: '适应窗口' });
    const fitted = fitButton.textContent;
    const zoomInButton = view.getByRole('button', { name: '放大图片' });
    fireEvent.click(zoomInButton);
    fireEvent.click(zoomInButton);
    expect(fitButton.textContent).not.toBe(fitted);
    fireEvent.click(fitButton);
    expect(fitButton.textContent).toBe(fitted);
    const closeButton = view.getByRole('button', { name: '关闭图片预览' });
    fireEvent.keyDown(closeButton, { key: 'Delete' });
    expect(canvasInputs).toBe(0);
    fireEvent.keyDown(closeButton, { key: 'Escape' });
    await waitFor(() => expect(view.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(previewButton));
    fireEvent.click(previewButton);
    const reopened = await view.findByAltText('猫咪原图');
    Object.defineProperties(reopened, { naturalWidth: { value: 1920 }, naturalHeight: { value: 1080 } });
    fireEvent.load(reopened);
    expect(view.getByRole('button', { name: '适应窗口' }).textContent).toBe(fitted);
    fireEvent.click(view.getByRole('button', { name: '关闭图片预览' }));
    expect(view.queryByRole('dialog')).toBeNull();
  });

  test('can close during loading and ignores the late asset response', async () => {
    let finish!: (value: CreativeAsset) => void;
    const pending = new Promise<CreativeAsset>((resolve) => { finish = resolve; });
    const view = render(<Harness resolveAsset={() => pending} />);
    fireEvent.click(view.getByRole('button', { name: '预览图片' }));
    expect(view.getByRole('status').textContent?.includes(common.imagePreview.loading)).toBe(true);
    fireEvent.click(view.getByRole('button', { name: '关闭图片预览' }));
    await act(async () => { finish(asset); await pending; });
    expect(view.queryByRole('dialog')).toBeNull();
    expect(view.queryByAltText('猫咪原图')).toBeNull();
  });

  test('recovers from asset lookup and original image loading failures', async () => {
    let attempts = 0;
    const view = render(<Harness resolveAsset={async () => {
      attempts += 1;
      if (attempts === 1) throw new Error('network unavailable');
      return asset;
    }} />);
    fireEvent.click(view.getByRole('button', { name: '预览图片' }));
    expect((await view.findByRole('alert')).textContent?.includes('图片加载失败')).toBe(true);
    fireEvent.click(view.getByRole('button', { name: '重试' }));
    fireEvent.error(await view.findByAltText('猫咪原图'));
    expect(view.getByRole('alert').textContent?.includes('图片加载失败')).toBe(true);
    fireEvent.click(view.getByRole('button', { name: '重试' }));
    fireEvent.load(await view.findByAltText('猫咪原图'));
    expect(view.queryByRole('alert')).toBeNull();
    expect(attempts).toBe(3);
  });
});
