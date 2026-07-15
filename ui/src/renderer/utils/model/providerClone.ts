/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';

export function cloneProviderConfig(provider: IProvider, nextId: ProviderId, copyLabel: string): IProvider {
  return {
    ...provider,
    id: nextId,
    name: `${provider.name} ${copyLabel}`.trim(),
    model_health: undefined,
  };
}
