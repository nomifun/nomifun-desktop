/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const layoutSource = readFileSync(new URL('./Layout.tsx', import.meta.url), 'utf8');
const modalSource = readFileSync(new URL('../settings/UpdateModal.tsx', import.meta.url), 'utf8');
const layoutCss = readFileSync(new URL('../../styles/layout.css', import.meta.url), 'utf8');

describe('global update availability entry', () => {
  test('shows the Logo update button only when a new version is available', () => {
    expect(layoutSource.includes('updateAvailability.available && !collapsed')).toBe(true);
    expect(layoutSource.includes("className='sidebar-update-button'")).toBe(true);
    expect(layoutSource.includes("detail: { source: 'sidebar' }")).toBe(true);
  });

  test('keeps startup and modal checks connected to the shared state', () => {
    expect(layoutSource.includes('reportUpdateAvailable(res.data.updateInfo.version)')).toBe(true);
    expect(layoutSource.includes('reportNoUpdateAvailable()')).toBe(true);
    expect(modalSource.includes('reportUpdateAvailable(res.data.latest.version)')).toBe(true);
    expect(modalSource.includes('reportUpdateAvailable(evt.version)')).toBe(true);
  });

  test('uses the requested blue circular treatment with reduced-motion support', () => {
    expect(layoutCss.includes('.sidebar-update-button {')).toBe(true);
    expect(layoutCss.includes('width: 18px')).toBe(true);
    expect(layoutCss.includes('height: 18px')).toBe(true);
    expect(layoutSource.includes("<Download theme='outline' size={11}")).toBe(true);
    expect(layoutCss.includes('border-radius: 999px')).toBe(true);
    expect(layoutCss.includes('background: rgb(var(--primary-6))')).toBe(true);
    expect(layoutCss.includes('.sidebar-update-button::before')).toBe(true);
    expect(layoutCss.includes('@media (prefers-reduced-motion: reduce)')).toBe(true);
  });
});
