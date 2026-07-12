/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import controlContractCss from '@renderer/styles/theme-control-contract.css?raw';

export const THEME_CONTROL_CONTRACT_STYLE_ID = 'theme-control-contract';

/**
 * Keep core interactive states after the runtime-injected preset stylesheet.
 * The helper is shared by the main shell and standalone companion windows.
 */
export function ensureThemeControlContract(): void {
  if (typeof document === 'undefined') return;

  const existing = document.getElementById(THEME_CONTROL_CONTRACT_STYLE_ID) as HTMLStyleElement | null;
  if (existing?.textContent === controlContractCss && existing === document.head.lastElementChild) return;

  existing?.remove();
  const styleEl = document.createElement('style');
  styleEl.id = THEME_CONTROL_CONTRACT_STYLE_ID;
  styleEl.type = 'text/css';
  styleEl.textContent = controlContractCss;
  document.head.appendChild(styleEl);
}

export function removeThemeControlContract(): void {
  if (typeof document === 'undefined') return;
  document.getElementById(THEME_CONTROL_CONTRACT_STYLE_ID)?.remove();
}
