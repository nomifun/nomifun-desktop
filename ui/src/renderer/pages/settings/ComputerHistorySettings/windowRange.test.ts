/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  COMPUTER_HISTORY_WINDOWS,
  computerHistoryWindowRange,
  formatComputerHistoryDuration,
  formatComputerHistoryRollupDuration,
  formatComputerHistorySegmentTime,
} from './windowRange';

// 2026-09-03 is a Thursday. 12:30 local keeps every boundary unambiguous.
const NOW = new Date(2026, 8, 3, 12, 30, 0).getTime();
const DAY = 24 * 60 * 60 * 1000;

describe('computerHistoryWindowRange', () => {
  test('exposes the four windows in display order', () => {
    expect([...COMPUTER_HISTORY_WINDOWS]).toEqual(['today', 'yesterday', 'last_7_days', 'this_week']);
  });

  test('today starts at local midnight and ends now', () => {
    const { from_ms, to_ms } = computerHistoryWindowRange('today', NOW);
    expect(from_ms).toBe(new Date(2026, 8, 3, 0, 0, 0, 0).getTime());
    expect(to_ms).toBe(NOW);
  });

  test('yesterday is the full previous local day', () => {
    const { from_ms, to_ms } = computerHistoryWindowRange('yesterday', NOW);
    expect(from_ms).toBe(new Date(2026, 8, 2, 0, 0, 0, 0).getTime());
    expect(to_ms).toBe(new Date(2026, 8, 3, 0, 0, 0, 0).getTime());
  });

  test('last_7_days covers today plus the six days before it', () => {
    const { from_ms } = computerHistoryWindowRange('last_7_days', NOW);
    expect(from_ms).toBe(new Date(2026, 7, 28, 0, 0, 0, 0).getTime());
  });

  test('this_week starts on Monday (Thursday → Monday start)', () => {
    const { from_ms } = computerHistoryWindowRange('this_week', NOW);
    expect(from_ms).toBe(new Date(2026, 7, 31, 0, 0, 0, 0).getTime());
  });

  test('unknown window falls back to today', () => {
    const today = computerHistoryWindowRange('today', NOW);
    // A cast here is the point: the backend may add windows the UI does not
    // know yet, and the view must degrade to today rather than crash.
    const fallback = computerHistoryWindowRange('somewhere_in_time' as never, NOW);
    expect(fallback).toEqual(today);
  });

  test('range duration equals the window length', () => {
    const week = computerHistoryWindowRange('last_7_days', NOW);
    expect(week.to_ms - week.from_ms).toBeGreaterThanOrEqual(6 * DAY);
  });
});

describe('duration formatting', () => {
  test('segment duration buckets hours/minutes/seconds', () => {
    expect(formatComputerHistoryDuration(0)).toBe('0 s');
    expect(formatComputerHistoryDuration(45_000)).toBe('45 s');
    expect(formatComputerHistoryDuration(5 * 60 * 1000)).toBe('5 min 0 s');
    expect(formatComputerHistoryDuration(2 * 3600_000 + 5 * 60_000)).toBe('2 h 5 min');
    expect(formatComputerHistoryDuration(-3)).toBe('0 s');
  });

  test('rollup duration feeds the i18n interpolation params', () => {
    expect(formatComputerHistoryRollupDuration(125 * 60_000)).toEqual({ hours: 2, minutes: 5, minutesOnly: 125 });
    expect(formatComputerHistoryRollupDuration(45 * 60_000)).toEqual({ hours: 0, minutes: 45, minutesOnly: 45 });
  });

  test('segment time range is start – end and never reversed', () => {
    const start = new Date(2026, 8, 3, 9, 41).getTime();
    const end = new Date(2026, 8, 3, 10, 2).getTime();
    const rendered = formatComputerHistorySegmentTime(start, end);
    expect(rendered.startsWith(formatComputerHistorySegmentTime(start, start).split('–')[0].trim())).toBe(true);
    expect(formatComputerHistorySegmentTime(start, start - 1000)).toBe(
      formatComputerHistorySegmentTime(start, start)
    );
  });
});
