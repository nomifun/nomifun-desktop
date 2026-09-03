/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Time-window arithmetic for the computer-history activity view. Kept as a
 * pure module so the day/week boundaries are testable without React.
 */

import type { ComputerHistoryWindow } from '@/common/adapter/ipcBridge';

export const COMPUTER_HISTORY_WINDOWS: readonly ComputerHistoryWindow[] = [
  'today',
  'yesterday',
  'last_7_days',
  'this_week',
] as const;

const DAY_MS = 24 * 60 * 60 * 1000;

const startOfDay = (ms: number): number => {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
};

/** Inclusive `[from_ms, to_ms]` bracket for a named window, resolved against `now`. */
export const computerHistoryWindowRange = (
  window: ComputerHistoryWindow,
  now: number = Date.now()
): { from_ms: number; to_ms: number } => {
  const todayStart = startOfDay(now);
  switch (window) {
    case 'yesterday':
      return { from_ms: todayStart - DAY_MS, to_ms: todayStart };
    case 'last_7_days':
      return { from_ms: todayStart - 6 * DAY_MS, to_ms: now };
    case 'this_week': {
      const weekday = (new Date(now).getDay() + 6) % 7; // Monday-based, 0 = Monday
      return { from_ms: todayStart - weekday * DAY_MS, to_ms: now };
    }
    case 'today':
    default:
      return { from_ms: todayStart, to_ms: now };
  }
};

/** `1234567` → `20 m 34 s`-style short duration for a segment/rollup row. */
export const formatComputerHistoryDuration = (ms: number): string => {
  if (ms <= 0) return '0 s';
  const totalSeconds = Math.round(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) return `${hours} h ${minutes} min`;
  if (minutes > 0) return `${minutes} min ${seconds} s`;
  return `${seconds} s`;
};

/** Compact human duration for the top-apps rollup (`2 h 5 min`, `45 min`). */
export const formatComputerHistoryRollupDuration = (ms: number): { hours: number; minutes: number; minutesOnly: number } => {
  const totalMinutes = Math.max(0, Math.round(ms / 60000));
  return { hours: Math.floor(totalMinutes / 60), minutes: totalMinutes % 60, minutesOnly: totalMinutes };
};

/** `9:41 AM – 10:02 AM`-style local time range for one segment. */
export const formatComputerHistorySegmentTime = (startedAtMs: number, endedAtMs: number): string => {
  const fmt = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });
  return `${fmt.format(startedAtMs)} – ${fmt.format(Math.max(endedAtMs, startedAtMs))}`;
};
