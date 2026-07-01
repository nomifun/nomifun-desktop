/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type UpdateErrorMessageKey = 'update.releaseFeedUnavailable' | 'update.checkFailed';

export function getUpdateErrorMessageKey(message: unknown): UpdateErrorMessageKey {
  const normalized = String(message ?? '').toLowerCase();
  if (
    normalized.includes('valid release json') ||
    normalized.includes('release json') ||
    normalized.includes('latest.json')
  ) {
    return 'update.releaseFeedUnavailable';
  }
  return 'update.checkFailed';
}
