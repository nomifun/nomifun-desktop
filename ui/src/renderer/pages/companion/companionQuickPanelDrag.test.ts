/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';
import { describe, expect, test } from 'bun:test';
import { isQuickPanelDragOrigin } from './companionQuickPanelDrag';

describe('expanded companion window drag surface', () => {
  test('background, header and empty-state content can move the window', () => {
    for (const className of ['nomi-companion-quick', 'nomi-companion-quick__header', 'nomi-companion-quick__empty']) {
      const node = document.createElement('div');
      node.className = className;
      expect(isQuickPanelDragOrigin(node)).toBe(true);
    }
  });

  test('interactive controls and selectable conversation content never start a window drag', () => {
    const panel = document.createElement('div');
    panel.innerHTML = `
      <button><span data-target="button-child">Action</span></button>
      <textarea data-target="textarea"></textarea>
      <div class="nomi-companion-quick__composer" data-target="composer-padding"></div>
      <div class="nomi-companion-quick__message"><span data-target="message-copy">Message</span></div>
      <div role="menu" data-target="menu-padding"></div>
    `;
    for (const node of panel.querySelectorAll('[data-target]')) {
      expect(isQuickPanelDragOrigin(node)).toBe(false);
    }
  });
});
