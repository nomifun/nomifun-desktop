/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeStudioAgentSessionPersistencePort } from './types';

export const CREATIVE_STUDIO_AGENT_SESSION_BACKEND_GAP = Object.freeze({
  code: 'ATOMIC_EXCLUSIVE_SESSION_BINDING_UNAVAILABLE',
  requiredContract: Object.freeze([
    'An authenticated owner-scoped resolve-or-create operation keyed by canvasId and sessionId',
    'A database uniqueness constraint on ownerId, the legacy Canvas storage ID, and sessionId',
    'Conversation creation and binding persistence in one transaction',
    'A server-owned exclusive-ownership marker that public conversation create/update cannot forge',
    'Authoritative model and Creative Studio history-projection verification on every resolve',
  ]),
  currentLimitation:
    'The current public conversation API exposes list/create/update separately and conversation.extra is client-writable, while Creative Studio Canvas CAS persistence is a separate service transaction.',
} as const);

/**
 * Explicit production failure while the backend lacks an atomic ownership
 * contract. This is safer than silently attaching the panel to the ordinary
 * active conversation or pretending a browser cache is durable persistence.
 */
export class CreativeStudioAgentSessionBackendUnavailableError extends Error {
  readonly code = CREATIVE_STUDIO_AGENT_SESSION_BACKEND_GAP.code;
  readonly requiredContract = CREATIVE_STUDIO_AGENT_SESSION_BACKEND_GAP.requiredContract;
  readonly currentLimitation = CREATIVE_STUDIO_AGENT_SESSION_BACKEND_GAP.currentLimitation;

  constructor() {
    super(
      'Creative Studio Agent session persistence is unavailable until the backend provides an atomic exclusive session-binding contract'
    );
    this.name = 'CreativeStudioAgentSessionBackendUnavailableError';
  }
}

export function createFailClosedCreativeStudioAgentSessionPort(): CreativeStudioAgentSessionPersistencePort {
  return {
    async resolveOrCreateExclusive() {
      throw new CreativeStudioAgentSessionBackendUnavailableError();
    },
  };
}
