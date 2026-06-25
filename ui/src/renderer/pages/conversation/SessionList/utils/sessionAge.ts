/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

const DAY_MS = 24 * 60 * 60 * 1000;

export function getSessionAgeDays(createdAt: number | undefined | null, now = Date.now()): number | null {
  if (typeof createdAt !== 'number' || !Number.isFinite(createdAt) || createdAt <= 0) {
    return null;
  }

  return Math.max(0, Math.floor((now - createdAt) / DAY_MS));
}
