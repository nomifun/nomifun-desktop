/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeStudioAgentMessage } from '../types';

export function isDurableCreativeStudioAgentMessage(message: CreativeStudioAgentMessage): boolean {
  return (message.role === 'user' && message.status === 'complete') ||
    (message.role === 'assistant' && (
      message.status === 'complete' || message.status === 'stopped' ||
      (message.status === 'failed' && typeof message.errorMessage === 'string' &&
        message.errorMessage.trim().length > 0)
    ));
}

/**
 * Lossless, deterministic projection used at the resolver boundary. It is a
 * comparison key, not a cryptographic digest: the adapter compares the entire
 * serialized history so a missing or changed message cannot silently attach to
 * the wrong Nomi conversation.
 */
export function serializeCreativeStudioAgentHistory(
  history: readonly CreativeStudioAgentMessage[]
): string {
  return JSON.stringify(
    history.map((message) => ({
      id: message.id,
      role: message.role,
      status: message.status,
      text: message.text,
      ...(message.status === 'running'
        ? { activityLabel: message.activityLabel ?? null }
        : {}),
      ...(message.status === 'failed' ? { errorMessage: message.errorMessage } : {}),
      ...(message.status === 'paused' ? { pause: message.pause } : {}),
    }))
  );
}
