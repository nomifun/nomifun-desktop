/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';

export class WorkspaceDirectoryUnavailableError extends Error {
  readonly workspacePath: string;
  readonly cause?: unknown;

  constructor(workspacePath: string, cause?: unknown) {
    super(`Workspace directory is unavailable: ${workspacePath}`);
    this.name = 'WorkspaceDirectoryUnavailableError';
    this.workspacePath = workspacePath;
    this.cause = cause;
  }
}

/**
 * Verify a user-selected project immediately before an executable submission.
 * The backend repeats this check at its persistence/runtime boundaries; this
 * client check exists to keep the user in the current form with an actionable
 * message instead of creating a task/session that can only fail later.
 */
export async function validateExistingWorkspaceDirectory(path: string): Promise<string> {
  const workspacePath = path.trim();
  if (!workspacePath) return '';

  try {
    const metadata = await ipcBridge.fs.getFileMetadata.invoke({
      path: workspacePath,
      workspace: workspacePath,
    });
    if (!metadata.isDirectory) {
      throw new WorkspaceDirectoryUnavailableError(workspacePath);
    }
    // Validation must not rewrite the user's durable path spelling (for
    // example macOS /var versus /private/var). The authoritative persistence
    // boundary canonicalizes where its identity contract requires it.
    return workspacePath;
  } catch (error) {
    if (error instanceof WorkspaceDirectoryUnavailableError) throw error;
    throw new WorkspaceDirectoryUnavailableError(workspacePath, error);
  }
}
