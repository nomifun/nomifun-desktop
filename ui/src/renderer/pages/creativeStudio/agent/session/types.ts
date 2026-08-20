/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  NomiCreativeStudioAgentSessionResolution,
  NomiCreativeStudioAgentSessionResolutionInput,
} from '../adapters';

/**
 * Immutable request passed to the durable backend boundary. The port must
 * atomically resolve or create exactly one exclusive Nomi conversation for
 * `(owner, projectId, sessionId)` and return the persisted binding.
 *
 * Cancellation is deliberately absent here. React StrictMode may abort one
 * caller while its replacement is already waiting for the same durable
 * resolution. The controller applies cancellation per waiter without
 * interrupting the shared persistence operation halfway through.
 */
export type CreativeStudioAgentSessionPersistenceRequest = Readonly<
  Omit<NomiCreativeStudioAgentSessionResolutionInput, 'signal'>
>;

/**
 * Server-backed authority for Creative Studio Agent session ownership.
 * Implementations must not infer ownership from the active chat route, a
 * renderer cache, localStorage, sessionStorage, or a client-writable marker.
 */
export interface CreativeStudioAgentSessionPersistencePort {
  resolveOrCreateExclusive(
    input: CreativeStudioAgentSessionPersistenceRequest
  ): Promise<NomiCreativeStudioAgentSessionResolution>;
}

export type CreativeStudioAgentSessionResolutionErrorCode =
  | 'INVALID_INPUT'
  | 'HISTORY_PROJECTION_MISMATCH'
  | 'PORT_CONTRACT_VIOLATION';

export class CreativeStudioAgentSessionResolutionError extends Error {
  readonly code: CreativeStudioAgentSessionResolutionErrorCode;

  constructor(code: CreativeStudioAgentSessionResolutionErrorCode, message: string) {
    super(message);
    this.name = 'CreativeStudioAgentSessionResolutionError';
    this.code = code;
  }
}
