/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider } from '@/common/config/storage';
import { capabilityOf } from '@/common/utils/providerModels';
import { imageExts } from '@/renderer/services/FileService';

export type NomiVisionSendDecision =
  | { allowed: true }
  | { allowed: false; reason: 'capability_unavailable' | 'vision_not_supported' };

export const containsImageAttachment = (files: readonly string[]): boolean =>
  files.some((file) => {
    const lower = file.toLowerCase();
    return imageExts.some((extension) => lower.endsWith(extension));
  });

/**
 * Fail-closed image-send decision for Nomi chat.
 *
 * The sole capability authority is the exact `(provider id, model id, chat)`
 * row nested in `ProviderResponse.models[].capabilities[]`. Provider platform,
 * model-name patterns and traits on another task/model never grant vision.
 */
export const evaluateNomiVisionSend = ({
  files,
  providers,
  providerGraphResolved,
  providerId,
  model,
}: {
  files: readonly string[];
  providers: readonly IProvider[];
  providerGraphResolved: boolean;
  providerId?: string;
  model?: string;
}): NomiVisionSendDecision => {
  if (!containsImageAttachment(files)) return { allowed: true };
  if (!providerGraphResolved) return { allowed: false, reason: 'capability_unavailable' };

  const provider = providers.find((candidate) => candidate.id === providerId);
  const chatCapability = model ? capabilityOf(provider, model, 'chat') : undefined;
  return chatCapability?.traits.includes('vision_input')
    ? { allowed: true }
    : { allowed: false, reason: 'vision_not_supported' };
};
