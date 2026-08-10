/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * The mini-app feature's relative-time phrasing, in one place.
 *
 * Two surfaces list solidified mini-apps — the library grid at `/mini-apps` and
 * the read-only quick panel in the conversation's right-hand rail — and both show
 * "updated <when>". Keeping one formatter means the two lists can never disagree
 * about what "yesterday" means or drift onto different keys.
 *
 * The keys live in the `miniApps` namespace on purpose: reading the workshop
 * gallery's copy would silently couple two unrelated surfaces and freeze that
 * namespace's wording.
 */
import type { TFunction } from 'i18next';

export function formatMiniAppRelativeTime(epochMs: number, t: TFunction): string {
  const diff = Date.now() - epochMs;
  const minutes = Math.floor(diff / 60000);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);
  if (minutes < 1) return t('miniApps.time.justNow');
  if (minutes < 60) return t('miniApps.time.minutesAgo', { count: minutes });
  if (hours < 24) return t('miniApps.time.hoursAgo', { count: hours });
  if (days === 1) return t('miniApps.time.yesterday');
  if (days < 7) return t('miniApps.time.daysAgo', { count: days });
  return t('miniApps.time.weeksAgo');
}
