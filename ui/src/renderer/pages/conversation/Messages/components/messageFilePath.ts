/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isAbsoluteLocalPath } from '@/common/utils/localPath';

const isAbsoluteMessageFilePath = (filePath: string): boolean =>
  isAbsoluteLocalPath(filePath) || /^[A-Za-z]:/.test(filePath);

export const resolveMessageFilePath = (
  filePath: string,
  workspace?: string,
  managedFilePaths?: Record<string, string>
): string => {
  // Only the exact mapping from the canonical Session projection can relocate
  // an absolute historical path. Do not infer ownership from a path prefix.
  if (managedFilePaths && Object.prototype.hasOwnProperty.call(managedFilePaths, filePath)) {
    const managedPath = managedFilePaths[filePath];
    if (typeof managedPath === 'string' && managedPath.length > 0) return managedPath;
  }
  if (!filePath || isAbsoluteMessageFilePath(filePath) || !workspace) return filePath;

  const normalizedWorkspace = workspace.replace(/[\\/]+$/, '').replace(/\\/g, '/');
  const normalizedFilePath = filePath.replace(/^\.?[\\/]+/, '').replace(/\\/g, '/');
  return `${normalizedWorkspace}/${normalizedFilePath}`.replace(/\/+/g, '/');
};
