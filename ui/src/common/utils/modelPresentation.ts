/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Generic model presentation helpers.
 *
 * The runtime model id is never rewritten. Providers may use opaque ids,
 * deployment ids, or endpoint ids; a separate optional display name is the
 * only portable way to show a friendly label without guessing provider
 * semantics in shared UI code.
 */
export const modelDisplayLabel = (model: string, displayName?: string): string =>
  displayName?.trim() || model;

export const modelPresentationRawId = (
  model: string,
  displayName?: string
): string | undefined =>
  displayName?.trim() && displayName.trim() !== model ? model : undefined;
