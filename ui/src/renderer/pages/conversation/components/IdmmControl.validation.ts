/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ProviderId } from '@/common/types/ids';

export type IdmmBackupValidationKey = 'idmm.backupRequired' | 'idmm.backupModelIncomplete';

export type IdmmWatchBackupConfig = {
  enabled: boolean;
  tier: string;
  bypass_model: {
    provider_id?: ProviderId | null;
    model?: string | null;
  };
};

const hasText = (value?: string | null): boolean => Boolean(value?.trim());

/**
 * `fallbackResolved` is whether a bypass model resolves WITHOUT the watch naming
 * one — i.e. the supervised session lends its own model. Conversations do;
 * terminals do not (their agent CLI owns the model). There is no global-default
 * tier behind this any more, so the two inputs are all there is.
 */
export const getWatchBackupValidationErrorKey = (
  watch: IdmmWatchBackupConfig,
  fallbackResolved: boolean
): IdmmBackupValidationKey | null => {
  if (!watch.enabled || watch.tier !== 'rule_plus_model') return null;

  const hasBackupProvider = watch.bypass_model.provider_id != null;
  const hasBackupModel = hasText(watch.bypass_model.model);

  if (hasBackupProvider !== hasBackupModel) return 'idmm.backupModelIncomplete';
  if (!hasBackupProvider && !fallbackResolved) return 'idmm.backupRequired';
  return null;
};

export const isWatchBackupReady = (watch: IdmmWatchBackupConfig, fallbackResolved: boolean): boolean =>
  getWatchBackupValidationErrorKey(watch, fallbackResolved) === null;
