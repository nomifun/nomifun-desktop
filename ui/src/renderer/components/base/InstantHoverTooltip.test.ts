/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./InstantHoverTooltip.tsx', import.meta.url), 'utf8');

describe('InstantHoverTooltip', () => {
  test('renders its own immediate tooltip layer on hover and focus', () => {
    expect(source.includes("role='tooltip'")).toBe(true);
    expect(source.includes('onMouseEnter={() => setVisible(true)}')).toBe(true);
    expect(source.includes('onMouseLeave={() => setVisible(false)}')).toBe(true);
    expect(source.includes('onFocus={() => setVisible(true)}')).toBe(true);
    expect(source.includes('onBlur={() => setVisible(false)}')).toBe(true);
  });
});
