/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Split a filesystem path into its parent directory ("head") and the final
 * segment with its leading separator ("tail"):
 *   `/a/b/c`      → { head: '/a/b',  tail: '/c'  }
 *   `C:\\a\\b`    → { head: 'C:\\a', tail: '\\b' }
 *   `project`     → { head: '',      tail: 'project' }
 *
 * Handles both POSIX `/` and Windows `\\` separators and strips trailing ones.
 * Powers middle-truncated path display, where the head collapses behind an
 * ellipsis while the tail (the distinguishing final folder) stays fully visible.
 */
export const splitPath = (path: string): { head: string; tail: string } => {
  if (!path) return { head: '', tail: '' };
  const normalized = path.replace(/[\\/]+$/, '');
  const idx = Math.max(normalized.lastIndexOf('/'), normalized.lastIndexOf('\\'));
  if (idx <= 0) return { head: '', tail: normalized };
  return { head: normalized.slice(0, idx), tail: normalized.slice(idx) };
};
