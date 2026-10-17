/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { UpdaterInstallReason } from '@/common/adapter/tauriUpdateInstall';

export type UpdateErrorMessageKey =
  | 'update.releaseFeedUnavailable'
  | 'update.packageNoLongerReady'
  | 'update.checkFailed';

export function getUpdateErrorMessageKey(message: unknown): UpdateErrorMessageKey {
  const normalized = String(message ?? '').toLowerCase();
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

export type UpdateInstallBlockMessageKey =
  | 'update.installBlockedMountedVolume'
  | 'update.installBlockedTranslocation'
  | 'update.installBlockedCrossDevice'
  | 'update.installBlockedMetadataUnavailable'
  | 'update.installBlockedBundleNotFound';

const INSTALL_BLOCK_MESSAGE_KEYS: Record<UpdaterInstallReason, UpdateInstallBlockMessageKey> = {
  mounted_volume: 'update.installBlockedMountedVolume',
  app_translocation: 'update.installBlockedTranslocation',
  cross_device: 'update.installBlockedCrossDevice',
  metadata_unavailable: 'update.installBlockedMetadataUnavailable',
  app_bundle_not_found: 'update.installBlockedBundleNotFound',
};

export function getInstallBlockMessageKey(reason: UpdaterInstallReason | null): UpdateInstallBlockMessageKey {
  return INSTALL_BLOCK_MESSAGE_KEYS[reason ?? 'metadata_unavailable'];
}
