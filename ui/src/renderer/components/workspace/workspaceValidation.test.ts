/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import {
  WorkspaceDirectoryUnavailableError,
  validateExistingWorkspaceDirectory,
} from './workspaceValidation';

const restore: Array<() => void> = [];
afterEach(() => restore.splice(0).reverse().forEach((dispose) => dispose()));

describe('workspace submission validation', () => {
  test('validates through backend metadata without rewriting the selected spelling', async () => {
    const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockResolvedValue({
      name: 'project',
      path: '/private/tmp/project',
      size: 0,
      type: 'inode/directory',
      lastModified: 1,
      isDirectory: true,
    });
    restore.push(() => metadata.mockRestore());

    await expect(validateExistingWorkspaceDirectory('/tmp/project')).resolves.toBe('/tmp/project');
    expect(metadata).toHaveBeenCalledWith({ path: '/tmp/project', workspace: '/tmp/project' });
  });

  test('rejects missing paths and files with the selected path attached', async () => {
    const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke')
      .mockRejectedValueOnce(new Error('not found'))
      .mockResolvedValueOnce({
        name: 'project.txt',
        path: '/tmp/project.txt',
        size: 1,
        type: 'text/plain',
        lastModified: 1,
        isDirectory: false,
      });
    restore.push(() => metadata.mockRestore());

    for (const path of ['/tmp/missing', '/tmp/project.txt']) {
      try {
        await validateExistingWorkspaceDirectory(path);
        throw new Error('expected workspace validation to fail');
      } catch (error) {
        expect(error).toBeInstanceOf(WorkspaceDirectoryUnavailableError);
        expect((error as WorkspaceDirectoryUnavailableError).workspacePath).toBe(path);
      }
    }
  });

  test('allows an empty selection to use the managed default workspace', async () => {
    const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke');
    restore.push(() => metadata.mockRestore());

    await expect(validateExistingWorkspaceDirectory('   ')).resolves.toBe('');
    expect(metadata).not.toHaveBeenCalled();
  });
});
