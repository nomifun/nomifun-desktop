/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

describe('WorkpathDrawer structure', () => {
  test('keeps copy path in the hover action group instead of a standalone resting icon', () => {
    const source = readFileSync(join(dirname(fileURLToPath(import.meta.url)), 'WorkpathDrawer.tsx'), 'utf8');
    const hoverOpsIndex = source.indexOf('{/* Hover ops:');
    const copyButtonIndex = source.indexOf('<CopyIconButton');

    expect(hoverOpsIndex).toBeGreaterThan(-1);
    expect(copyButtonIndex).toBeGreaterThan(hoverOpsIndex);
    expect(source.includes('always visible (real workpaths only)')).toBe(false);
  });
});
