/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

export type FileChangeOperation = 'create' | 'modify' | 'delete';

/** A single file's change status */
export type FileChangeInfo = {
  file_path: string;
  relativePath: string;
  operation: FileChangeOperation;
};

/** Comparison result with staged/unstaged separation (git-repo mode) */
export type CompareResult = {
  staged: FileChangeInfo[];
  unstaged: FileChangeInfo[];
};

/** Snapshot metadata returned by init and getInfo */
export type SnapshotInfo = {
  mode: 'git-repo' | 'snapshot' | 'disabled';
  branch: string | null;
  /**
   * Present only for `mode === 'disabled'`: why snapshot tracking was refused
   * (drive root, well-known system dir, or too large to safely snapshot).
   * `null`/absent for the active `git-repo` / `snapshot` modes.
   */
  reason?: string | null;
};
