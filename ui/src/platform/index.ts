/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * In-house replacement for the former third-party platform package.
 *
 * Exposes the same import shape the app already uses:
 *   import { bridge, storage, theme, logger } from '@/platform';
 *
 * Only the surface actually consumed by this project is implemented (bridge
 * pub/sub + RPC, namespaced storage, a few design tokens, a logger facade).
 * The wire protocol is preserved so the Rust backend and mobile client are
 * unaffected. See ./bridge.ts for the protocol contract.
 */
export * as bridge from './bridge';
export * as storage from './storage';
export * as theme from './theme';
export * as logger from './logger';
