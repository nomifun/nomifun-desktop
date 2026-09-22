/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseCompanionId } from '@/common/types/ids';
import {
  switchCompanionDesktopWindow,
  type CompanionSwitchWindow,
} from './companionWindowSwitch';

const currentId = parseCompanionId('019f0000-0000-7000-8000-000000000001');
const targetId = parseCompanionId('019f0000-0000-7000-8000-000000000002');

const windowFixture = (events: string[]): CompanionSwitchWindow => ({
  async show() { events.push('show'); },
  async setFocus() { events.push('focus'); },
});

describe('desktop companion quick switching', () => {
  test('a hidden target replaces the current companion only after it is ready', async () => {
    const events: string[] = [];
    const target = windowFixture(events);
    let reads = 0;
    const result = await switchCompanionDesktopWindow({
      currentId,
      targetId,
      roster: [
        { companion_id: currentId, enabled: true },
        { companion_id: targetId, enabled: false },
      ],
      async getCurrentPosition() { events.push('position'); return { x: 80, y: 120 }; },
      async getWindow() { reads += 1; events.push(`window-${reads}`); return reads >= 2 ? target : null; },
      async enableTarget(_, position) { events.push(`enable-${position?.x}-${position?.y}`); },
      async disableCurrent() { events.push('disable-current'); },
      async syncWindows(specs) {
        expect(specs.find((item) => item.companion_id === targetId)?.enabled).toBe(true);
        events.push('sync');
      },
      async placeTarget(_, position) { events.push(`place-${position.x}-${position.y}`); },
      async wait() { events.push('wait'); },
    });

    expect(result).toBe('replaced');
    expect(events).toEqual([
      'position',
      'window-1',
      'enable-80-120',
      'sync',
      'window-2',
      'place-80-120',
      'show',
      'focus',
      'disable-current',
    ]);
  });

  test('an enabled target is focused without hiding the current companion', async () => {
    const events: string[] = [];
    const target = windowFixture(events);
    const result = await switchCompanionDesktopWindow({
      currentId,
      targetId,
      roster: [
        { companion_id: currentId, enabled: true },
        { companion_id: targetId, enabled: true },
      ],
      async getCurrentPosition() { throw new Error('position must not be read'); },
      async getWindow() { events.push('window'); return target; },
      async enableTarget() { throw new Error('target must not be enabled again'); },
      async disableCurrent() { events.push('disable-current'); },
      async syncWindows() { events.push('sync'); },
      async placeTarget() { events.push('place'); },
      async wait() { events.push('wait'); },
    });

    expect(result).toBe('focused');
    expect(events).toEqual(['window', 'show', 'focus']);
  });

  test('an enabled target whose window is late is reconciled and then focused', async () => {
    const events: string[] = [];
    const target = windowFixture(events);
    let reads = 0;
    const result = await switchCompanionDesktopWindow({
      currentId,
      targetId,
      roster: [
        { companion_id: currentId, enabled: true },
        { companion_id: targetId, enabled: true },
      ],
      async getCurrentPosition() { throw new Error('position must not be read'); },
      async getWindow() { reads += 1; events.push(`window-${reads}`); return reads >= 3 ? target : null; },
      async enableTarget() { throw new Error('target must not be enabled again'); },
      async disableCurrent() { events.push('disable-current'); },
      async syncWindows() { events.push('sync'); },
      async placeTarget() { events.push('place'); },
      async wait() { events.push('wait'); },
    });

    expect(result).toBe('focused');
    expect(events).toEqual(['window-1', 'sync', 'window-2', 'wait', 'window-3', 'show', 'focus']);
  });

  test('a failed target creation preserves the current companion', async () => {
    const events: string[] = [];
    const result = await switchCompanionDesktopWindow({
      currentId,
      targetId,
      roster: [
        { companion_id: currentId, enabled: true },
        { companion_id: targetId, enabled: false },
      ],
      async getCurrentPosition() { return null; },
      async getWindow() { return null; },
      async enableTarget() { events.push('enable'); },
      async disableCurrent() { events.push('disable-current'); },
      async syncWindows() { events.push('sync'); },
      async placeTarget() { events.push('place'); },
      async wait() {},
    });

    expect(result).toBe('missing');
    expect(events).toEqual(['enable', 'sync']);
  });
});
