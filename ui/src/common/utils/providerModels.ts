/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Row-level provider-model catalog readers.
 *
 * `IProvider.models_detail` (the `provider_models` projection) is the
 * authoritative per-model source; the legacy whole-provider maps
 * (`model_health`, `model_enabled`, …) are write-frozen and only kept for
 * managed/legacy providers without rows. New reads go through these helpers so
 * no new call sites touch the legacy maps directly.
 */

import type { IProvider } from '@/common/config/storage';
import type { ModelHealthStatus } from '@/common/types/provider/providerModel';

/**
 * Health snapshot for one model, read from its authoritative `models_detail`
 * row. Returns `undefined` when the provider has no row for the model — the
 * legacy `model_health` map is deliberately NOT consulted (server-side probes
 * persist into rows; a missing row means "never probed" for the UI).
 */
export const modelHealthOf = (
  provider: Pick<IProvider, 'models_detail'> | undefined,
  model: string
): ModelHealthStatus | undefined => provider?.models_detail?.find((row) => row.model === model)?.health;

/**
 * Model names for a provider: authoritative `models_detail` rows first,
 * falling back to the legacy `models` string array when the provider has no
 * rows (managed/legacy providers only).
 */
export const modelNamesOf = (provider: Pick<IProvider, 'models' | 'models_detail'>): string[] => {
  if (provider.models_detail && provider.models_detail.length > 0) {
    return provider.models_detail.map((row) => row.model);
  }
  return provider.models ?? [];
};
