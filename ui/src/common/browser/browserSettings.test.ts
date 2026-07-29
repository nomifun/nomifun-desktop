/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ConfigKeyMap } from '@/common/config/configKeys';
import {
  BROWSER_DISPLAY_MODES,
  BROWSER_DISPLAY_MODE_POLICY_VERSION,
  buildBrowserResourcePolicyPresetRequest,
  isBrowserDisplayMode,
  migrateBrowserDisplayMode,
  normalizeBrowserResourcePolicy,
} from './browserSettings';

describe('migrateBrowserDisplayMode', () => {
  test('keeps the renderer config schema on the backend v2 lineage marker', () => {
    const configVersion: NonNullable<
      ConfigKeyMap['agent.browserUse.displayModeVersion']
    > = BROWSER_DISPLAY_MODE_POLICY_VERSION;

    expect(configVersion).toBe(2);
  });

  test('publishes headless and external as the two user policies', () => {
    expect(BROWSER_DISPLAY_MODES).toEqual(['headless', 'external']);
    expect(isBrowserDisplayMode('headless')).toBe(true);
    expect(isBrowserDisplayMode('external')).toBe(true);
    expect(isBrowserDisplayMode('embedded')).toBe(false);
  });

  test('preserves valid choices only across the v2 lineage boundary', () => {
    expect(
      migrateBrowserDisplayMode({
        displayMode: 'headless',
        displayModeVersion: BROWSER_DISPLAY_MODE_POLICY_VERSION,
      })
    ).toEqual({
      displayMode: 'headless',
      shouldPersist: false,
      source: 'displayMode',
    });
    expect(
      migrateBrowserDisplayMode({
        displayMode: 'external',
        displayModeVersion: '  "2"  ',
      })
    ).toEqual({
      displayMode: 'external',
      shouldPersist: false,
      source: 'displayMode',
    });
  });

  test('fails every unversioned or old-version value closed to headless', () => {
    for (const input of [
      { displayMode: 'external' },
      { displayMode: 'headless' },
      { displayMode: 'external', displayModeVersion: 1 },
      { displayMode: 'external', displayModeVersion: '1' },
      { displayMode: 'external', displayModeVersion: null },
      { silent: false },
      { silent: 'false' },
      { silent: true },
    ]) {
      expect(migrateBrowserDisplayMode(input)).toEqual({
        displayMode: 'headless',
        shouldPersist: true,
        source: 'lineage',
      });
    }
  });

  test('defaults a fresh install to headless and requests persistence', () => {
    expect(migrateBrowserDisplayMode({})).toEqual({
      displayMode: 'headless',
      shouldPersist: true,
      source: 'default',
    });
  });

  test('repairs malformed v2 state to headless', () => {
    for (const displayMode of ['embedded', 'visible', null, undefined]) {
      expect(
        migrateBrowserDisplayMode({
          displayMode,
          displayModeVersion: BROWSER_DISPLAY_MODE_POLICY_VERSION,
          silent: false,
        })
      ).toEqual({
        displayMode: 'headless',
        shouldPersist: true,
        source: 'lineage',
      });
    }
  });
});

describe('buildBrowserResourcePolicyPresetRequest', () => {
  const persisted = {
    preset: 'automatic' as const,
    advanced: {
      max_memory_ratio: 0.5,
      reserved_memory_bytes: 512 * 1024 * 1024,
      max_active_operations: 4,
      max_open_lanes: 16,
      max_queued_requests: 64,
      max_owner_queued_requests: 8,
    },
  };

  test('sends only the preset when advanced fields merely echo the server state', () => {
    // The GET endpoint materializes every advanced field; echoing them back
    // makes the backend treat them as authoritative overrides and the preset
    // transition becomes a no-op.
    expect(
      buildBrowserResourcePolicyPresetRequest('high_concurrency', persisted, persisted)
    ).toEqual({ preset: 'high_concurrency' });
    expect(
      buildBrowserResourcePolicyPresetRequest(
        'resource_saving',
        { ...persisted, advanced: { ...persisted.advanced } },
        persisted
      )
    ).toEqual({ preset: 'resource_saving' });
  });

  test('keeps user-edited advanced values as intentional overrides', () => {
    const edited = {
      ...persisted,
      advanced: { ...persisted.advanced, max_open_lanes: 24 },
    };
    expect(
      buildBrowserResourcePolicyPresetRequest('high_concurrency', edited, persisted)
    ).toEqual({ preset: 'high_concurrency', advanced: edited.advanced });

    const cleared = {
      ...persisted,
      advanced: (({ max_open_lanes: _dropped, ...rest }) => rest)(persisted.advanced),
    };
    expect(
      buildBrowserResourcePolicyPresetRequest('resource_saving', cleared, persisted)
    ).toEqual({ preset: 'resource_saving', advanced: cleared.advanced });
  });

  test('treats both-absent advanced as untouched', () => {
    expect(
      buildBrowserResourcePolicyPresetRequest(
        'automatic',
        { preset: 'resource_saving' },
        { preset: 'resource_saving' }
      )
    ).toEqual({ preset: 'automatic' });
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
