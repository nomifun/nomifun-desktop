/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');

describe('text color theme tokens', () => {
  test('defines the quaternary text utility as a real theme color', () => {
    const unoConfig = readFileSync(join(uiRoot, 'uno.config.ts'), 'utf8');

    expect(unoConfig.includes("'t-quaternary': 'var(--text-secondary)'")).toBe(true);
  });
});
