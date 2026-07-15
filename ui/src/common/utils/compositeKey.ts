/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Encodes a tuple without ambiguous string-concatenation boundaries.
 *
 * Intended for React keys and small in-memory composite identities. Persistent
 * browser state must use browserStorageKey instead.
 */
export function compositeKey(...segments: readonly string[]): string {
  return segments.map((segment) => `${segment.length}:${segment}`).join('|');
}
