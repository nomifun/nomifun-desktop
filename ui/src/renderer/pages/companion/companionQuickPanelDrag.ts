/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

const NON_DRAG_SELECTOR = [
  'button',
  'input',
  'textarea',
  'select',
  'a',
  '[role="menu"]',
  '[role="menuitem"]',
  '[data-no-window-drag]',
  '.nomi-companion-quick__message',
  '.nomi-companion-quick__live',
  '.nomi-companion-quick__composer',
].join(',');

/** True when a primary-pointer press should move the native companion window. */
export function isQuickPanelDragOrigin(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(NON_DRAG_SELECTOR) === null;
}
