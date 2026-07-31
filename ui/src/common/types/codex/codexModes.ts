/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

// Native session-mode ids of the current codex ACP bridge
// (@agentclientprotocol/codex-acp, migration-022 swap). The previous
// @zed-industries bridge advertised `read-only` / `auto` / `full-access`;
// normalizeCodexMode folds that whole lineage onto the current ids so
// persisted conversations keep working.
export const CODEX_MODE_READ_ONLY = 'read-only';
export const CODEX_MODE_NATIVE_DEFAULT = 'agent';
export const CODEX_MODE_NATIVE_FULL_ACCESS = 'agent-full-access';

// Legacy values kept for backward compatibility with persisted config.
// Only consumed internally by normalizeCodexMode, no external callers.
const CODEX_MODE_AUTO_EDIT = 'autoEdit';
const CODEX_MODE_FULL_AUTO = 'yolo';
const CODEX_MODE_FULL_AUTO_NO_SANDBOX = 'yoloNoSandbox';
const CODEX_MODE_LEGACY_AUTO = 'auto';
const CODEX_MODE_LEGACY_FULL_ACCESS = 'full-access';

export function normalizeCodexMode(mode?: string | null): string | undefined {
  if (!mode) return undefined;

  switch (mode) {
    case 'default':
    case CODEX_MODE_AUTO_EDIT:
    case CODEX_MODE_LEGACY_AUTO:
    case CODEX_MODE_NATIVE_DEFAULT:
      return CODEX_MODE_NATIVE_DEFAULT;
    case CODEX_MODE_FULL_AUTO:
    case CODEX_MODE_FULL_AUTO_NO_SANDBOX:
    case CODEX_MODE_LEGACY_FULL_ACCESS:
    case CODEX_MODE_NATIVE_FULL_ACCESS:
      return CODEX_MODE_NATIVE_FULL_ACCESS;
    case CODEX_MODE_READ_ONLY:
      return CODEX_MODE_READ_ONLY;
    default:
      return mode;
  }
}
