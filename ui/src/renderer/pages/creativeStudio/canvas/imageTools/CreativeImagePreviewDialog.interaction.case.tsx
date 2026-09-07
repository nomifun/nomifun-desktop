/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider } from 'react-i18next';
import common from '../../../../services/i18n/locales/zh-CN/common.json';
import zh from '../../../../services/i18n/locales/zh-CN/creativeStudio.json';
import type { CreativeAsset } from '../../assets';
import type { CreativeCanvasNode } from '../../domain';
import CreativeCanvasImageToolbar from './CreativeCanvasImageToolbar';
import CreativeImagePreviewDialog from './CreativeImagePreviewDialog';

afterEach(cleanup);
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
      <div id='creative-studio-portal-root' />
      </section>
    </I18nextProvider>
  );
};

describe('canvas image preview', () => {
  test('opens the original image, zooms independently and restores focus after closing', async () => {
    let canvasInputs = 0;
    const view = render(<Harness resolveAsset={async () => asset} onCanvasInput={() => { canvasInputs += 1; }} />);
    const previewButton = view.getByRole('button', { name: '预览图片' });
    previewButton.focus();
    fireEvent.click(previewButton);
    const image = await view.findByRole('img', { name: '猫咪原图' });
    const dialog = view.getByRole('dialog');
    expect(view.getByTestId('canvas-content').contains(dialog)).toBe(false);
    expect(dialog.querySelector('.arco-modal-footer')).toBeNull();
    expect(dialog.querySelectorAll('[role="button"], button').length).toBe(1);
    expect(image.getAttribute('src')).toBe('/original.png');
    fireEvent.load(image);
    const container = image.parentElement!;
    expect(container.style.transform).toBe('scale(1, 1)');
    fireEvent.keyDown(view.getByRole('button', { name: 'Close' }), { key: 'ArrowUp' });
    expect(container.style.transform).toBe('scale(1.1, 1.1)');
    fireEvent.wheel(image, { deltaY: 100 });
    expect(container.style.transform).toBe('scale(1, 1)');
    fireEvent.keyDown(view.getByRole('button', { name: 'Close' }), { key: 'Delete' });
    expect(canvasInputs).toBe(0);
    fireEvent.keyDown(view.getByRole('button', { name: 'Close' }), { key: 'Escape' });
    await waitFor(() => expect(view.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(previewButton));
    fireEvent.click(previewButton);
    const reopened = await view.findByRole('img', { name: '猫咪原图' });
    expect(reopened.parentElement!.style.transform).toBe('scale(1, 1)');
    fireEvent.click(view.getByRole('button', { name: 'Close' }));
    expect(view.queryByRole('dialog')).toBeNull();
  });

  test('can close during loading and ignores the late asset response', async () => {
    let finish!: (value: CreativeAsset) => void;
    const pending = new Promise<CreativeAsset>((resolve) => { finish = resolve; });
    const view = render(<Harness resolveAsset={() => pending} />);
    fireEvent.click(view.getByRole('button', { name: '预览图片' }));
    expect(view.getByRole('status').textContent?.includes(common.loading)).toBe(true);
    fireEvent.click(view.getByRole('button', { name: 'Close' }));
    await act(async () => { finish(asset); await pending; });
    expect(view.queryByRole('dialog')).toBeNull();
    expect(view.queryByRole('img', { name: '猫咪原图' })).toBeNull();
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
    fireEvent.error(await view.findByRole('img', { name: '猫咪原图' }));
    expect(view.getByRole('alert').textContent?.includes('图片加载失败')).toBe(true);
    fireEvent.click(view.getByRole('button', { name: '重试' }));
    fireEvent.load(await view.findByRole('img', { name: '猫咪原图' }));
    expect(view.queryByRole('alert')).toBeNull();
    expect(attempts).toBe(3);
  });
});
