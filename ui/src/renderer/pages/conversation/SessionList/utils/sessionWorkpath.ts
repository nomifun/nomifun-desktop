/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import { DEFAULT_WORKPATH_KEY, workpathKey } from './workpathKey';

/**
 * Terminal session → workpath key.
 * `is_default_workpath === true` → default, otherwise workpathKey(cwd).
 */
export function workpathKeyForTerminal(session: Pick<ITerminalSession, 'cwd' | 'is_default_workpath'>): string {
  return session.is_default_workpath ? DEFAULT_WORKPATH_KEY : workpathKey(session.cwd);
}
