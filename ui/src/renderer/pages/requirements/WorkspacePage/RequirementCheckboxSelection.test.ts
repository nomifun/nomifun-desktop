/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const rowSource = readFileSync(new URL('./RequirementListRow.tsx', import.meta.url), 'utf8');
const filtersSource = readFileSync(new URL('./RequirementFilters.tsx', import.meta.url), 'utf8');
const controlCss = readFileSync(new URL('../../../styles/theme-control-contract.css', import.meta.url), 'utf8');

describe('requirements checkbox selection treatment', () => {
  test('scopes the enhanced selected state to the requirements row and select-all controls', () => {
    expect(rowSource.includes("className='requirements-selection-checkbox'")).toBe(true);
    expect(filtersSource.includes("className='requirements-selection-checkbox'")).toBe(true);
    expect(controlCss.includes('.arco-checkbox-mask {')).toBe(true);
    expect(controlCss.includes('.arco-checkbox-checked .arco-checkbox-mask')).toBe(true);
    expect(controlCss.includes('.arco-checkbox-mask-icon')).toBe(true);
    expect(controlCss.includes('@media (prefers-reduced-motion: reduce)')).toBe(true);
  });
});
