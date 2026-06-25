/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import { DEFAULT_WORKPATH_KEY, workpathKey } from './workpathKey';

/**
 * Resolve the workpath key a session belongs to — the unit a knowledge binding
 * is now scoped to (spec §7: every session under a workpath shares one binding).
 *
 * These pure functions mirror the membership rules in {@link buildWorkpathTree}
 * exactly. They are split per source shape so each input is the minimal known
 * field set (easy to unit-test, no full session object required).
 */

/**
 * Interactive (conversation) session → workpath key.
 * `custom_workspace === true && typeof workspace === 'string'` →
 * workpathKey(workspace), otherwise the default workpath.
 * Accepts the raw `extra` bag (shape varies per conversation type).
 */
export function workpathKeyForConversation(extra: Record<string, unknown> | undefined | null): string {
  const e = extra ?? {};
  return e.custom_workspace === true && typeof e.workspace === 'string' ? workpathKey(e.workspace) : DEFAULT_WORKPATH_KEY;
}

/**
 * Terminal session → workpath key.
 * `is_default_workpath === true` → default, otherwise workpathKey(cwd).
 */
export function workpathKeyForTerminal(session: Pick<ITerminalSession, 'cwd' | 'is_default_workpath'>): string {
  return session.is_default_workpath ? DEFAULT_WORKPATH_KEY : workpathKey(session.cwd);
}

/**
 * Draft (pre-creation) selection → workpath key.
 * An empty/unset directory means the user has not chosen a custom workspace yet,
 * so the draft maps to the default workpath.
 */
export function workpathKeyForDraftDir(dir: string | undefined | null): string {
  const trimmed = (dir ?? '').trim();
  return trimmed ? workpathKey(trimmed) : DEFAULT_WORKPATH_KEY;
}
