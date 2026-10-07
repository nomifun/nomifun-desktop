/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { IProvider, TProviderWithModel } from '@/common/config/storage';
import type { ConversationContextValue } from '@/renderer/hooks/context/ConversationContext';

/** Conversation models are identity references; account UI metadata comes from the configured provider catalog. */
export function currentModelProviderTarget(
  model: Pick<TProviderWithModel, 'id' | 'use_model'> | undefined,
  providers: readonly Pick<IProvider, 'id' | 'platform'>[]
): ConversationContextValue['currentModel'] {
  if (!model) return undefined;
  const provider = providers.find((candidate) => candidate.id === model.id);
  if (!provider) return undefined;
  return { id: provider.id, platform: provider.platform, use_model: model.use_model };
}
