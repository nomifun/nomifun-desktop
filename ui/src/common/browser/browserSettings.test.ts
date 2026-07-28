/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  BROWSER_DISPLAY_MODES,
  isBrowserDisplayMode,
  migrateBrowserDisplayMode,
  normalizeBrowserResourcePolicy,
} from './browserSettings';

describe('migrateBrowserDisplayMode', () => {
  test('publishes headless and external as the two user policies', () => {
    expect(BROWSER_DISPLAY_MODES).toEqual(['headless', 'external']);
    expect(isBrowserDisplayMode('headless')).toBe(true);
    expect(isBrowserDisplayMode('external')).toBe(true);
    expect(isBrowserDisplayMode('embedded')).toBe(false);
  });

  test('preserves explicit user choices without a rewrite', () => {
    expect(migrateBrowserDisplayMode({ displayMode: 'headless', silent: false })).toEqual({
      displayMode: 'headless',
      shouldPersist: false,
      source: 'displayMode',
    });
    expect(migrateBrowserDisplayMode({ displayMode: 'external', silent: true })).toEqual({
      displayMode: 'external',
      shouldPersist: false,
      source: 'displayMode',
    });
  });

  test('migrates the removed embedded viewer to headless', () => {
    expect(migrateBrowserDisplayMode({ displayMode: 'embedded', silent: false })).toEqual({
      displayMode: 'headless',
      shouldPersist: true,
      source: 'displayMode',
    });
  });

  test('migrates silent=false to the explicit visible default', () => {
    expect(migrateBrowserDisplayMode({ silent: false })).toEqual({
      displayMode: 'external',
      shouldPersist: true,
      source: 'silent',
    });
  });

  test('migrates silent=true to headless and requests persistence', () => {
    expect(migrateBrowserDisplayMode({ silent: true })).toEqual({
      displayMode: 'headless',
      shouldPersist: true,
      source: 'silent',
    });
  });

  test('defaults a fresh install to headless and requests persistence', () => {
    expect(migrateBrowserDisplayMode({})).toEqual({
      displayMode: 'headless',
      shouldPersist: true,
      source: 'default',
    });
  });

  test('repairs an invalid new mode to headless without consulting legacy silent', () => {
    expect(migrateBrowserDisplayMode({ displayMode: 'visible', silent: false })).toEqual({
      displayMode: 'headless',
      shouldPersist: true,
      source: 'displayMode',
    });
  });

  test('treats an explicitly present null mode as malformed new configuration', () => {
    expect(migrateBrowserDisplayMode({ displayMode: null, silent: false })).toEqual({
      displayMode: 'headless',
      shouldPersist: true,
      source: 'displayMode',
    });
  });
});

describe('normalizeBrowserResourcePolicy', () => {
  test('preserves the final nested wire shape', () => {
    expect(
      normalizeBrowserResourcePolicy({
        preset: 'resource_saving',
        advanced: {
          max_memory_ratio: 0.3,
          max_open_lanes: 24,
        },
      })
    ).toEqual({
      preset: 'resource_saving',
      advanced: {
        max_memory_ratio: 0.3,
        max_open_lanes: 24,
      },
    });
  });

  test('accepts the legacy mode alias and top-level numeric fields', () => {
    expect(
      normalizeBrowserResourcePolicy({
        mode: 'high_concurrency',
        max_active_operations: 32,
        max_queued_requests: 128,
      })
    ).toEqual({
      preset: 'high_concurrency',
      advanced: {
        max_active_operations: 32,
        max_queued_requests: 128,
      },
    });
  });

  test('prefers nested values and drops invalid numeric input', () => {
    expect(
      normalizeBrowserResourcePolicy({
        preset: 'automatic',
        max_open_lanes: 90,
        max_memory_ratio: '0.4',
        advanced: {
          max_open_lanes: 40,
          max_active_operations: Number.POSITIVE_INFINITY,
        },
      })
    ).toEqual({
      preset: 'automatic',
      advanced: {
        max_open_lanes: 40,
      },
    });
  });
});
