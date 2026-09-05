/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const mainSource = readFileSync(new URL('./main.tsx', import.meta.url), 'utf8');

describe('renderer bootstrap storage boundary', () => {
  test('routes nullable system-info generation through the tolerant initializer', () => {
    expect(mainSource.includes('initializeBrowserStorageGeneration(info?.storageGeneration)')).toBe(true);
    expect(mainSource.includes('setBrowserStorageGeneration(info.storageGeneration)')).toBe(false);
  });

  test('keeps genuine storage bootstrap failures visible', () => {
    expect(mainSource.includes("console.error('Failed to initialize browser storage generation:', err);")).toBe(true);
    expect(mainSource.includes('throw err;')).toBe(true);
    expect(mainSource.includes('setConfigError(error instanceof Error ? error : new Error(String(error)))')).toBe(true);
  });
});
