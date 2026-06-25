/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { WorkpathNode } from './workpathTree';
import {
  getBatchSelectionScopeState,
  getWorkpathBatchSelectionScope,
  toggleBatchSelectionScope,
} from './batchSelectionScopes';

const node = (
  key: string,
  interactive: number[],
  terminal: number[],
  displayName = key
): WorkpathNode => ({
  key,
  displayName,
  pinned: false,
  activityAt: 0,
  interactive: interactive.map((id) => ({ kind: 'interactive', id, name: `c${id}`, pinned: false, pinnedAt: 0, activityAt: id, createdAt: id })),
  terminal: terminal.map((id) => ({ kind: 'terminal', id, name: `t${id}`, pinned: false, pinnedAt: 0, activityAt: id, createdAt: id })),
});

describe('getWorkpathBatchSelectionScope', () => {
  test('returns every descendant row for a workpath aggregate node', () => {
    expect(getWorkpathBatchSelectionScope(node('/repo/app', [2, 3], [20], 'app'))).toEqual({
      conversationIds: [2, 3],
      terminalIds: [20],
    });
  });

  test('narrows a workpath aggregate node to one session kind', () => {
    const workpath = node('/repo/app', [2, 3], [20], 'app');

    expect(getWorkpathBatchSelectionScope(workpath, 'interactive')).toEqual({
      conversationIds: [2, 3],
      terminalIds: [],
    });
    expect(getWorkpathBatchSelectionScope(workpath, 'terminal')).toEqual({
      conversationIds: [],
      terminalIds: [20],
    });
  });
});

describe('toggleBatchSelectionScope', () => {
  test('adds a partially selected scope without dropping existing selections', () => {
    const next = toggleBatchSelectionScope(
      { conversationIds: [2, 3], terminalIds: [20] },
      { conversationIds: new Set([1, 2]), terminalIds: new Set<number>() }
    );

    expect([...next.conversationIds].sort()).toEqual([1, 2, 3]);
    expect([...next.terminalIds].sort()).toEqual([20]);
  });

  test('removes a fully selected scope and preserves other selections', () => {
    const next = toggleBatchSelectionScope(
      { conversationIds: [2, 3], terminalIds: [20] },
      { conversationIds: new Set([1, 2, 3]), terminalIds: new Set([20, 30]) }
    );

    expect([...next.conversationIds].sort()).toEqual([1]);
    expect([...next.terminalIds].sort()).toEqual([30]);
  });
});

describe('getBatchSelectionScopeState', () => {
  test('returns unchecked when no descendant rows are selected', () => {
    expect(
      getBatchSelectionScopeState(
        { conversationIds: [1, 2], terminalIds: [10] },
        { conversationIds: new Set(), terminalIds: new Set() }
      )
    ).toMatchObject({ checked: false, indeterminate: false, disabled: false });
  });

  test('returns checked when every descendant row is selected', () => {
    expect(
      getBatchSelectionScopeState(
        { conversationIds: [1, 2], terminalIds: [10] },
        { conversationIds: new Set([1, 2]), terminalIds: new Set([10]) }
      )
    ).toMatchObject({ checked: true, indeterminate: false, disabled: false });
  });

  test('returns indeterminate when only part of the aggregate node is selected', () => {
    expect(
      getBatchSelectionScopeState(
        { conversationIds: [1, 2], terminalIds: [10] },
        { conversationIds: new Set([2]), terminalIds: new Set() }
      )
    ).toMatchObject({ checked: true, indeterminate: true, disabled: false });
  });

  test('returns disabled for aggregate nodes without selectable descendants', () => {
    expect(
      getBatchSelectionScopeState(
        { conversationIds: [], terminalIds: [] },
        { conversationIds: new Set([1]), terminalIds: new Set([10]) }
      )
    ).toMatchObject({ checked: false, indeterminate: false, disabled: true });
  });
});
