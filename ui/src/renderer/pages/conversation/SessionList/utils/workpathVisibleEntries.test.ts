/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { getVisibleWorkpathEntries } from './workpathVisibleEntries';
import type { SessionEntry, WorkpathNode } from './workpathTree';

const entry = (kind: SessionEntry['kind'], id: number): SessionEntry => ({
  kind,
  id,
  name: `${kind}-${id}`,
  pinned: false,
  pinnedAt: 0,
  activityAt: id,
  createdAt: id,
});

const node = (interactive: number[], terminal: number[]): WorkpathNode => ({
  key: '/repo/app',
  displayName: 'app',
  pinned: false,
  activityAt: 0,
  interactive: interactive.map((id) => entry('interactive', id)),
  terminal: terminal.map((id) => entry('terminal', id)),
});

describe('getVisibleWorkpathEntries', () => {
  test('keeps all entries visible when each session kind is within the collapsed limit', () => {
    const visible = getVisibleWorkpathEntries(node([1, 2, 3, 4], [10, 11]), false);

    expect(visible.hasOverflow).toBe(false);
    expect(visible.hiddenCount).toBe(0);
    expect(visible.interactive.map((item) => item.id)).toEqual([1, 2, 3, 4]);
    expect(visible.terminal.map((item) => item.id)).toEqual([10, 11]);
  });

  test('collapses each session kind independently while collapsed', () => {
    const visible = getVisibleWorkpathEntries(node([1, 2, 3, 4, 5, 6], [10, 11, 12, 13, 14, 15, 16]), false);

    expect(visible.hasOverflow).toBe(true);
    expect(visible.hiddenCount).toBe(3);
    expect(visible.interactive.map((item) => item.id)).toEqual([1, 2, 3, 4, 5]);
    expect(visible.terminal.map((item) => item.id)).toEqual([10, 11, 12, 13, 14]);
  });

  test('returns every child session when expanded', () => {
    const visible = getVisibleWorkpathEntries(node([1, 2, 3, 4], [10, 11, 12]), true);

    expect(visible.hiddenCount).toBe(0);
    expect(visible.interactive.map((item) => item.id)).toEqual([1, 2, 3, 4]);
    expect(visible.terminal.map((item) => item.id)).toEqual([10, 11, 12]);
  });
});
