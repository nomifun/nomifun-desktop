/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const workspaceSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const controlCss = readFileSync(new URL('../../../styles/theme-control-contract.css', import.meta.url), 'utf8');

describe('requirements view toggle', () => {
  test('gives the selected list or board view an unambiguous theme-aware active state', () => {
    expect(workspaceSource.includes("className='requirements-view-toggle'")).toBe(true);
    expect(controlCss.includes('.requirements-view-toggle [role=\'tab\'][aria-selected=\'true\']')).toBe(true);
    expect(controlCss.includes('--control-selected-bg')).toBe(true);
    expect(controlCss.includes('--control-selected-fg')).toBe(true);
  });
});
