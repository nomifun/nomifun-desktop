/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const drawerSource = readFileSync(new URL('./PresetEditDrawer.tsx', import.meta.url), 'utf8');
const controlCss = readFileSync(new URL('../../../styles/theme-control-contract.css', import.meta.url), 'utf8');

describe('preset skill checkbox selection treatment', () => {
  test('applies the enhanced selected state to every editable preset skill source', () => {
    expect(drawerSource.match(/preset-skill-selection-checkbox/g)?.length).toBe(5);
    expect(controlCss.includes('.preset-skill-selection-checkbox .arco-checkbox-mask')).toBe(true);
    expect(controlCss.includes('.preset-skill-selection-checkbox.arco-checkbox-checked .arco-checkbox-mask')).toBe(true);
    expect(controlCss.includes('.preset-skill-selection-checkbox .arco-checkbox-mask-icon')).toBe(true);
  });
});
