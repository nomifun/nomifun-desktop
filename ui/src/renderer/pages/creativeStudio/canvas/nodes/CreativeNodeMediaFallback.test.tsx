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

test('image and panorama nodes recover from broken thumbnails without changing persisted fit or asset identity', () => {
  for (const kind of ['image', 'panorama'] as const) {
    const node = testNode(kind, 3000);
    node.data.assetId = testUuid(3001);
    if (node.type === 'image') node.data.fit = 'cover';
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
