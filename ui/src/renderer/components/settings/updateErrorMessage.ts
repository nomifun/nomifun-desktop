/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type UpdateErrorMessageKey =
  | 'update.releaseFeedUnavailable'
  | 'update.crossDeviceInstallUnsupported'
  | 'update.packageNoLongerReady'
  | 'update.checkFailed';

export function getUpdateErrorMessageKey(message: unknown): UpdateErrorMessageKey {
  const normalized = String(message ?? '').toLowerCase();
  if (
    normalized.includes('nomifun_updater_auto_install_unsupported') ||
    normalized.includes('cross-device link') ||
    normalized.includes('crosses devices') ||
    normalized.includes('os error 18')
  ) {
    return 'update.crossDeviceInstallUnsupported';
  }
  // The native side refused an install it never started, because it no longer
  // holds the package for that version. The recovery is to download again — the
  // generic fallback below used to tell the user the CHECK had failed, which
  // pointed them at the wrong action entirely.
  if (
    normalized.includes('nomifun_update_not_retained') ||
    normalized.includes('no downloaded update is ready to install')
  ) {
    return 'update.packageNoLongerReady';
  }
  if (
    normalized.includes('valid release json') ||
    normalized.includes('release json') ||
    normalized.includes('latest.json')
  ) {
    return 'update.releaseFeedUnavailable';
  }
  return 'update.checkFailed';
}
