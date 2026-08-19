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
  BROWSER_DISPLAY_MODE_PREVIOUS_POLICY_VERSION,
  buildBrowserResourcePolicyAdvancedSaveRequest,
  buildBrowserResourcePolicyPresetRequest,
  isBrowserDisplayMode,
  migrateBrowserDisplayMode,
  normalizeBrowserResourcePolicy,
} from './browserSettings';

describe('migrateBrowserDisplayMode', () => {
  test('keeps the renderer config schema on the backend lineage marker', () => {
    const configVersion: NonNullable<
      ConfigKeyMap['agent.browserUse.displayModeVersion']
    > = BROWSER_DISPLAY_MODE_POLICY_VERSION;

    expect(configVersion).toBe(3);
    // The previous marker must stay representable: migration still reads it to
    // decide whether a stored `external` was a real choice.
    const previous: NonNullable<
      ConfigKeyMap['agent.browserUse.displayModeVersion']
    > = BROWSER_DISPLAY_MODE_PREVIOUS_POLICY_VERSION;
    expect(previous).toBe(2);
  });

  test('publishes headless, auto and external as the three user policies', () => {
    expect(BROWSER_DISPLAY_MODES).toEqual(['headless', 'auto', 'external']);
    expect(isBrowserDisplayMode('headless')).toBe(true);
    expect(isBrowserDisplayMode('auto')).toBe(true);
    expect(isBrowserDisplayMode('external')).toBe(true);
    expect(isBrowserDisplayMode('embedded')).toBe(false);
  });

  test('preserves valid choices under the current lineage marker', () => {
    for (const displayMode of ['headless', 'auto', 'external'] as const) {
      expect(
        migrateBrowserDisplayMode({
          displayMode,
          displayModeVersion: BROWSER_DISPLAY_MODE_POLICY_VERSION,
        })
      ).toEqual({ displayMode, shouldPersist: false, source: 'displayMode' });
    }
    expect(
      migrateBrowserDisplayMode({
        displayMode: 'external',
        displayModeVersion: '  "3"  ',
      })
    ).toEqual({
      displayMode: 'external',
      shouldPersist: false,
      source: 'displayMode',
    });
  });

  test('carries a v2 explicit external choice forward instead of silencing it', () => {
    // A v2 marker proves the user deliberately chose a visible window, so the
    // new auto default must not take it away.
    for (const version of [BROWSER_DISPLAY_MODE_PREVIOUS_POLICY_VERSION, '2', '  "2"  ']) {
      expect(
        migrateBrowserDisplayMode({
          displayMode: 'external',
          displayModeVersion: version,
        })
      ).toEqual({
        displayMode: 'external',
        shouldPersist: true,
        source: 'lineage',
      });
    }
  });

  test("moves v2's universal headless default onto auto", () => {
    // v2 persisted `headless` for every installation, so it reflects the old
    // default rather than a deliberate "never show me a window".
    expect(
      migrateBrowserDisplayMode({
        displayMode: 'headless',
        displayModeVersion: BROWSER_DISPLAY_MODE_PREVIOUS_POLICY_VERSION,
      })
    ).toEqual({ displayMode: 'auto', shouldPersist: true, source: 'lineage' });
  });

  test('fails every unversioned or older value closed to auto', () => {
    // Crucially this includes an unversioned `external`, which may have been
    // *inferred* from the removed `silent=false` setting rather than chosen. It
    // must not reopen a foreground window. `auto` still launches silently.
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
        displayMode: 'auto',
        shouldPersist: true,
        source: 'lineage',
      });
    }
  });

  test('defaults a fresh install to auto and requests persistence', () => {
    expect(migrateBrowserDisplayMode({})).toEqual({
      displayMode: 'auto',
      shouldPersist: true,
      source: 'default',
    });
  });

  test('repairs malformed current-version state to auto', () => {
    for (const displayMode of ['embedded', 'visible', null, undefined]) {
      expect(
        migrateBrowserDisplayMode({
          displayMode,
          displayModeVersion: BROWSER_DISPLAY_MODE_POLICY_VERSION,
          silent: false,
        })
      ).toEqual({
        displayMode: 'auto',
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
      max_task_memory_bytes: 1536 * 1024 * 1024,
      max_task_active_operations: 2,
      max_task_open_lanes: 4,
      max_task_tabs: 16,
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

  test('moves user-edited advanced values to the custom policy', () => {
    const edited = {
      ...persisted,
      advanced: { ...persisted.advanced, max_open_lanes: 24 },
    };
    expect(
      buildBrowserResourcePolicyPresetRequest('high_concurrency', edited, persisted)
    ).toEqual({ preset: 'custom', advanced: edited.advanced });

    const cleared = {
      ...persisted,
      advanced: (({ max_open_lanes: _dropped, ...rest }) => rest)(persisted.advanced),
    };
    expect(
      buildBrowserResourcePolicyPresetRequest('resource_saving', cleared, persisted)
    ).toEqual({ preset: 'custom', advanced: cleared.advanced });
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

describe('buildBrowserResourcePolicyAdvancedSaveRequest', () => {
  const persisted = {
    preset: 'automatic' as const,
    advanced: {
      max_memory_ratio: 0.5,
      max_task_memory_bytes: 1536 * 1024 * 1024,
      max_task_active_operations: 2,
      max_task_open_lanes: 4,
      max_task_tabs: 16,
      reserved_memory_bytes: 512 * 1024 * 1024,
      max_active_operations: 4,
      max_open_lanes: 16,
      max_queued_requests: 64,
      max_owner_queued_requests: 8,
    },
  };

  test('resolves user-edited advanced values to the custom preset', () => {
    // An explicit reserved_memory_bytes is honored by the scheduler only
    // under the custom preset; a named preset silently re-floors it to 20%
    // of total memory. Saving edited advanced values must therefore hand
    // control to custom end-to-end.
    const edited = {
      ...persisted,
      advanced: { ...persisted.advanced, reserved_memory_bytes: 2 * 1024 * 1024 * 1024 },
    };
    expect(buildBrowserResourcePolicyAdvancedSaveRequest(edited, persisted)).toEqual({
      preset: 'custom',
      advanced: edited.advanced,
    });

    const cleared = {
      ...persisted,
      advanced: (({ reserved_memory_bytes: _dropped, ...rest }) => rest)(persisted.advanced),
    };
    expect(buildBrowserResourcePolicyAdvancedSaveRequest(cleared, persisted)).toEqual({
      preset: 'custom',
      advanced: cleared.advanced,
    });
  });

  test('keeps the current preset when advanced values merely echo the server state', () => {
    expect(buildBrowserResourcePolicyAdvancedSaveRequest(persisted, persisted)).toEqual({
      preset: 'automatic',
    });
    expect(
      buildBrowserResourcePolicyAdvancedSaveRequest(
        { ...persisted, preset: 'custom', advanced: { ...persisted.advanced } },
        { ...persisted, preset: 'custom' }
      )
    ).toEqual({ preset: 'custom' });
  });
});

describe('normalizeBrowserResourcePolicy', () => {
  test('preserves the backend custom preset instead of collapsing it to automatic', () => {
    // Dropping 'custom' on GET would make the next advanced save re-attach a
    // named preset and re-floor the explicitly configured memory reserve.
    expect(
      normalizeBrowserResourcePolicy({
        preset: 'custom',
        advanced: { reserved_memory_bytes: 2 * 1024 * 1024 * 1024 },
      })
    ).toEqual({
      preset: 'custom',
      advanced: { reserved_memory_bytes: 2 * 1024 * 1024 * 1024 },
    });
  });

  test('preserves the final nested wire shape', () => {
    expect(
      normalizeBrowserResourcePolicy({
        preset: 'resource_saving',
        advanced: {
          max_memory_ratio: 0.3,
          max_task_memory_bytes: 768 * 1024 * 1024,
          max_task_tabs: 8,
          max_open_lanes: 24,
        },
      })
    ).toEqual({
      preset: 'resource_saving',
      advanced: {
        max_memory_ratio: 0.3,
        max_task_memory_bytes: 768 * 1024 * 1024,
        max_task_tabs: 8,
        max_open_lanes: 24,
      },
    });
  });

  test('ignores the obsolete installation-wide absolute memory field', () => {
    expect(
      normalizeBrowserResourcePolicy({
        preset: 'automatic',
        advanced: {
          max_memory_bytes: 8 * 1024 * 1024 * 1024,
          max_task_memory_bytes: 1024 * 1024 * 1024,
        },
      })
    ).toEqual({
      preset: 'automatic',
      advanced: { max_task_memory_bytes: 1024 * 1024 * 1024 },
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
