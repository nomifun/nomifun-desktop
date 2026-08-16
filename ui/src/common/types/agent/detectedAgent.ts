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

/** Execution engine kinds — each uses a different protocol or runtime */
export type DetectedAgentKind = 'nomi';
