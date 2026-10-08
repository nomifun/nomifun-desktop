/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export function parseApiKeyList(value?: string | null): string[] {
  if (!value) return [];
  return value
    .split(/[,\n]/)
    .map((key) => key.trim())
    .filter(Boolean);
}
