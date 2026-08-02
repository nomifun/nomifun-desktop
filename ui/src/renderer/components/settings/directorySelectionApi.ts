/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest } from '@/common/adapter/httpBridge';

export interface DirectoryItem {
  name: string;
  path: string;
  isDirectory: boolean;
  isFile?: boolean;
}

export interface DirectoryData {
  items: DirectoryItem[];
  currentPath: string;
  canGoUp: boolean;
  parentPath?: string;
  truncated?: boolean;
  isRoot?: boolean;
}

/** Browse a host directory through the shared authenticated HTTP bridge. */
export function browseDirectory(path: string, showFiles: boolean): Promise<DirectoryData> {
  return httpRequest<DirectoryData>(
    'GET',
    `/api/fs/browse?path=${encodeURIComponent(path)}&showFiles=${showFiles ? 'true' : 'false'}`
  );
}

/** Create a direct child directory through the CSRF-protected HTTP bridge. */
export function createDirectory(parentPath: string, name: string): Promise<DirectoryItem> {
  return httpRequest<DirectoryItem>('POST', '/api/fs/directory', { parentPath, name });
}
