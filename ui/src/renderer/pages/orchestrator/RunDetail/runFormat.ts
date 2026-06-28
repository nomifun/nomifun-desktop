/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Format a millisecond duration into a compact human label: `45s`, `12m`,
 * `3m 20s`, `1h 4m`. Used for run elapsed time and per-task pacing. Negative /
 * NaN inputs clamp to `0s` (a clock skew between created/updated must never
 * render a nonsense negative duration).
 */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return '0s';
  const totalSec = Math.floor(ms / 1000);
  if (totalSec < 60) return `${totalSec}s`;
  const totalMin = Math.floor(totalSec / 60);
  if (totalMin < 60) {
    const sec = totalSec % 60;
    return sec > 0 ? `${totalMin}m ${sec}s` : `${totalMin}m`;
  }
  const hours = Math.floor(totalMin / 60);
  const min = totalMin % 60;
  return min > 0 ? `${hours}h ${min}m` : `${hours}h`;
}

/** Task statuses that have a meaningful elapsed time worth surfacing (a pending
 * task hasn't started, so its created→updated gap is just queue time). */
const TIMED_TASK_STATUSES = new Set(['running', 'done', 'completed', 'failed', 'needs_review']);

/** Whether a task's status warrants showing a duration chip. */
export function taskHasTiming(status: string): boolean {
  return TIMED_TASK_STATUSES.has(status);
}
