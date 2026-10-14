/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

const CONTRACT_EVOLUTION_REJECTED = 'AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED';

/** Recognize the Host's exact conflict envelope; quoted diagnostic text is not a code. */
export function conversationConfigurationErrorCode(
  payload: { code?: unknown; error?: unknown } | undefined,
): typeof CONTRACT_EVOLUTION_REJECTED | undefined {
  if (payload?.code === CONTRACT_EVOLUTION_REJECTED) return CONTRACT_EVOLUTION_REJECTED;
  if (payload?.code === 'CONFLICT' && typeof payload.error === 'string'
    && /^(?:Conflict:\s*)?AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED(?:\s*:|$)/.test(payload.error)) {
    return CONTRACT_EVOLUTION_REJECTED;
  }
  return undefined;
}
