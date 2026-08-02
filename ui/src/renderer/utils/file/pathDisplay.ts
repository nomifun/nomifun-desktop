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

export interface FileDisplayPathParts {
  /** Relative directory prefix including its trailing slash. */
  directoryPath: string;
  /** Final path segment. */
  fileName: string;
  /** Normalized relative path used by the row tooltip. */
  fullPath: string;
}

/**
 * Normalize and split a workspace-relative file path for inline display.
 * Directory separators stay with the muted directory prefix so the filename
 * can use the stronger foreground color without rendering the slash as part
 * of the filename.
 */
export const splitFileDisplayPath = (path: string, fallbackFileName: string): FileDisplayPathParts => {
  const normalized = path.trim().replace(/\\/g, '/').replace(/\/{2,}/g, '/').replace(/^\.\//, '');
  const fullPath = normalized || fallbackFileName;
  const separatorIndex = fullPath.lastIndexOf('/');

  if (separatorIndex < 0) {
    return { directoryPath: '', fileName: fullPath || fallbackFileName, fullPath };
  }

  return {
    directoryPath: fullPath.slice(0, separatorIndex + 1),
    fileName: fullPath.slice(separatorIndex + 1) || fallbackFileName,
    fullPath,
  };
};
