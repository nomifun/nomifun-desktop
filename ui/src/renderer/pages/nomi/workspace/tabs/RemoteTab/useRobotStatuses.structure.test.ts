/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('./useRobotStatuses.ts', import.meta.url), 'utf8');

describe('useRobotStatuses is the three-part realtime shape', () => {
  test('takes a snapshot on mount', () => {
    // The socket has no replay buffer: a robot that was already speaking before
    // this section mounted is only learnable by asking.
    expect(src.includes('ipcBridge.robot.statuses.invoke()')).toBe(true);
  });

  test('merges incremental events', () => {
    expect(src.includes('ipcBridge.robot.onStatus.on(')).toBe(true);
  });

  test('re-snapshots when the socket reconnects', () => {
    // Frames dropped while the socket was down are never replayed, and a stale
    // "speaking" is the worst possible lie for this pill.
    expect(src.includes('ipcBridge.conversation.reconnected.on(')).toBe(true);
  });

  test('the newer changed_at wins so an out-of-order delivery cannot walk state back', () => {
    expect(src.includes('prev.changed_at > next.changed_at')).toBe(true);
  });

  test('listeners are installed before the snapshot is requested', () => {
    // Otherwise a transition emitted mid-flight falls into a subscribe gap.
    const offStatus = src.indexOf('ipcBridge.robot.onStatus.on(');
    const firstSnapshot = src.indexOf('resnapshot();', offStatus);
    expect(offStatus).toBeGreaterThan(0);
    expect(firstSnapshot).toBeGreaterThan(offStatus);
  });

  test('keyed by robot_id, so a rebound robot keeps its live phase', () => {
    expect(src.includes('Record<string, IApiRobotStatus>')).toBe(true);
    expect(src.includes('[next.robot_id]: next')).toBe(true);
  });
});
