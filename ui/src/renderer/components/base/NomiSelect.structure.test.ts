/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./NomiSelect.tsx', import.meta.url), 'utf8');

describe('NomiSelect content-fit structure', () => {
  test('uses the visible value as the intrinsic width source', () => {
    expect(source.includes("'[&_.arco-select-view]:w-max'")).toBe(true);
    expect(source.includes("'[&_.arco-select-view-selector]:w-max'")).toBe(true);
    expect(source.includes("'[&_.arco-select-view-value]:w-max'")).toBe(true);
    expect(source.includes("'[&_.arco-select-view-value-mirror]:w-max'")).toBe(true);
    expect(source.includes('contentFit && CONTENT_FIT_CLASS')).toBe(true);
  });
});
