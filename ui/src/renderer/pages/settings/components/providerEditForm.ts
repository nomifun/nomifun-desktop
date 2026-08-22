/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * A provider edit is a partial update. Merely opening the detail form must not
 * rewrite persisted authentication with a manifest/default fallback.
 */
export const buildAuthSchemeEditPatch = (
  storedScheme: string,
  draftScheme: string,
  dirty: boolean
): { auth_scheme?: string } => {
  const normalizedDraft = draftScheme.trim();
  return dirty && normalizedDraft !== storedScheme.trim()
    ? { auth_scheme: normalizedDraft }
    : {};
};
