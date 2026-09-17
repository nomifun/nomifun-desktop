/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const css = readFileSync(new URL('./LoginPage.css', import.meta.url), 'utf8');

describe('LoginPage desktop minimum-window layout', () => {
  test('fits normal and message states at 880×600 without an unsupported width breakpoint', () => {
    expect(css).toContain('@media (max-height: 680px) and (min-width: 880px)');
    expect(css).toContain('max-height: calc(100vh - 24px)');
    expect(css).toContain('max-width: 416px');
    expect(css).toContain('overflow-y: auto');
    expect(css).not.toMatch(/@media\s*\(\s*max-width\s*:/);
  });
});
