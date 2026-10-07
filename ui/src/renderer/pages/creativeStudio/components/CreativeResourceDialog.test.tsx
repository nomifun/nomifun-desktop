/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';

import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';

import CreativeResourceDialog from './CreativeResourceDialog';

afterEach(() => {
  cleanup();
  document.querySelectorAll('[data-test-fullscreen-popup-host]').forEach((node) => node.remove());
});

describe('CreativeResourceDialog portal ownership', () => {
  test('mounts inside an explicit fullscreen descendant instead of document.body', () => {
    const fullscreenHost = document.createElement('div');
    fullscreenHost.dataset.testFullscreenPopupHost = 'true';
    document.body.appendChild(fullscreenHost);

    render(
      <CreativeResourceDialog
        kind='assets'
        title='资产库'
        scope='canvas'
        popupContainer={fullscreenHost}
        onClose={() => undefined}
      >
        <div>FULLSCREEN ASSET CONTENT</div>
      </CreativeResourceDialog>
    );

    const content = fullscreenHost.querySelector(
      '[data-canvas-resource-dialog="assets"]'
    );
    expect(content).not.toBeNull();
    expect(fullscreenHost.textContent).toContain('FULLSCREEN ASSET CONTENT');
  });
});
