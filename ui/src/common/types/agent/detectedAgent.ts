/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Detection layer types — represents available execution engines in the system.
 *
 * Each `kind` corresponds to a distinct execution engine / communication protocol.
 * Presets are a reusable configuration layer that references these execution
 * engines; they are not detected Agents themselves.
 */

/** Remote agent communication protocol */
export type RemoteAgentProtocol = 'openclaw' | 'zeroclaw' | 'acp';

/** Remote agent authentication method */
export type RemoteAgentAuthType = 'bearer' | 'password' | 'none';

/** Execution engine kinds — each uses a different protocol or runtime */
export type DetectedAgentKind = 'acp' | 'remote' | 'nomi' | 'openclaw-gateway' | 'nanobot';
