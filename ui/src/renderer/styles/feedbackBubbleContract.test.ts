/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const contractCss = readFileSync(new URL('./feedback-bubble-contract.css', import.meta.url), 'utf8');
const arcoOverrideCss = readFileSync(new URL('./arco-override.css', import.meta.url), 'utf8');
const mainSource = readFileSync(new URL('../main.tsx', import.meta.url), 'utf8');

describe('feedback bubble visual contract', () => {
  test('is loaded globally for portal-rendered feedback', () => {
    expect(mainSource.includes("import './styles/feedback-bubble-contract.css';")).toBe(true);
  });

  test('shares one opaque surface token across Message and Notification', () => {
    expect(contractCss.includes('.arco-message,\n.arco-notification')).toBe(true);
    expect(contractCss.includes('background: var(--feedback-bubble-bg, var(--color-bg-popup)) !important;')).toBe(
      true
    );
  });

  test('layers every status tint over the shared solid surface', () => {
    for (const status of ['success', 'warning', 'info', 'error']) {
      expect(arcoOverrideCss.includes(`.arco-message.arco-message-${status}`)).toBe(true);
    }
    expect(arcoOverrideCss.match(/var\(--feedback-bubble-bg, var\(--color-bg-popup\)\)/g)?.length).toBe(8);
  });
});
