/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import { modelHealthOf } from '@/common/utils/providerModels';

/** Health dots are keyed by the exact provider, model, and Chat capability. */
export const exactChatHealthDotColor = (
  providers: readonly IProvider[] | undefined,
  providerId: ProviderId,
  model: string
): string | null => {
  const provider = providers?.find((candidate) => candidate.id === providerId);
  const status = modelHealthOf(provider, model, 'chat')?.status ?? 'unknown';
  if (status === 'unknown') return null;
  if (status === 'healthy') return 'bg-green-500';
  if (status === 'unhealthy') return 'bg-red-500';
  return 'bg-gray-400';
};
