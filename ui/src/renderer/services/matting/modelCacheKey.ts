/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Shared Cache Storage coordinates for the MODNet matting model.
 *
 * The model is fetched on the **main thread** (authenticated, from the local
 * backend proxy `GET /api/companion/matting-model`) and read back on the **worker
 * thread** during inference. Both run on the same origin, so a key resolved
 * against `self.location.origin` is identical in window and worker contexts —
 * the page-seeded entry is readable from the matting worker.
 */

export const MODEL_CACHE_NAME = 'nomifun-matting-v1';

/** Stable, origin-absolute Cache Storage key (same string in window + worker). */
export function modelCacheKey(): string {
  try {
    return new URL('/__matting_model_v1', self.location.origin).toString();
  } catch {
    return '/__matting_model_v1';
  }
}
