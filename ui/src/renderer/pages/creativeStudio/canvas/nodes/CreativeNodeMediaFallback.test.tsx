/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import { testNode, testUuid } from '../core/testFixtures';
import { CreativeNodeView } from './CreativeNodeViews';

afterEach(cleanup);

test('unknown image dimensions are read from the original instead of a potentially cropped thumbnail', () => {
  const node = testNode('image', 3010);
  node.data.assetId = 'portrait';
  const sizes: Array<{ width: number; height: number }> = [];
  const { container } = render(withCanvasTestI18n(
    <CreativeNodeView node={node} asset={{ src: '/square-thumbnail.jpg', originalSrc: '/portrait.png' }}
      onMediaSize={(size) => sizes.push(size)} />
  ));
  const image = container.querySelector('img')!;
  expect(image.getAttribute('src')).toBe('/portrait.png');
  Object.defineProperty(image, 'naturalWidth', { value: 600 });
  Object.defineProperty(image, 'naturalHeight', { value: 900 });
  fireEvent.load(image);
  expect(sizes).toEqual([{ width: 600, height: 900 }]);
});

test('image and panorama nodes recover from broken thumbnails without changing persisted fit or asset identity', () => {
  for (const kind of ['image', 'panorama'] as const) {
    const node = testNode(kind, 3000);
    node.data.assetId = testUuid(3001);
    if (node.type === 'image') {
      node.data.fit = 'cover';
      node.data.naturalSize = { width: 1200, height: 800 };
    }
    const before = JSON.stringify(node);
    const { container, unmount } = render(withCanvasTestI18n(
      <CreativeNodeView node={node} asset={{ src: '/thumb.jpg', originalSrc: '/original.png' }} />
    ));
    expect(container.querySelector('img')?.getAttribute('src')).toBe('/thumb.jpg');
    fireEvent.error(container.querySelector('img')!);
    const image = container.querySelector('img')!;
    expect(image.getAttribute('src')).toBe('/original.png');
    if (kind === 'image') expect(image.style.objectFit).toBe('cover');
    expect(JSON.stringify(node)).toBe(before);
    fireEvent.error(image);
    expect(container.querySelector('[data-asset-media-state="missing"]')).not.toBeNull();
    unmount();
  }
});
