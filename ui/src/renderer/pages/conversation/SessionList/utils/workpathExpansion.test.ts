/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { getRenderedExpansionState } from './workpathExpansion';

describe('getRenderedExpansionState', () => {
  test('active routes request a persisted expand without forcing the current render open forever', () => {
    expect(getRenderedExpansionState({ active: true, persistedExpanded: false })).toEqual({
      expanded: false,
      shouldSyncExpanded: true,
    });
  });

  test('manual collapse stays collapsed after the active route has already been synced', () => {
    expect(getRenderedExpansionState({ active: true, persistedExpanded: false, activeRouteSynced: true })).toEqual({
      expanded: false,
      shouldSyncExpanded: false,
    });
  });
});
