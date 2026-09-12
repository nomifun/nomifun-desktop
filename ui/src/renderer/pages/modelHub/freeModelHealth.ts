/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ManagedModelHealthResult } from '@/common/types/provider/managedModelService';

// A successful one-shot probe is not an indefinite promise of availability.
export const HEALTH_FRESHNESS_MS = 5 * 60 * 1000;

export const isHealthResultStale = (
  result: { status: ManagedModelHealthResult['status']; checkedAt?: number } | undefined,
  now: number
): boolean =>
  result?.status === 'healthy' &&
  typeof result.checkedAt === 'number' &&
  now - result.checkedAt > HEALTH_FRESHNESS_MS;
