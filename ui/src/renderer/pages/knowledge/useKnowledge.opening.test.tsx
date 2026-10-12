import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, spyOn, test } from 'bun:test';

import { ipcBridge } from '@/common';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type {
  IKnowledgeBase,
  IKnowledgeFileEntry,
  IKnowledgeTreeEntry,
} from '@/common/adapter/ipcBridge';
import { parseKnowledgeBaseId } from '@/common/types/ids';
import {
  prefetchKnowledgeBaseOpen,
  useKnowledgeBase,
} from './useKnowledge';

const restore: Array<() => void> = [];

afterEach(() => {
  cleanup();
  restore.splice(0).reverse().forEach((dispose) => dispose());
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, resolve, reject };
}

const base = (): IKnowledgeBase => ({
  knowledge_base_id: parseKnowledgeBaseId(
    '0190f5fe-7c00-7a00-8000-000000000991'
  ),
  name: 'Product notes',
  description: '',
  root_path: '/knowledge/product-notes',
  managed: true,
  tree_access: 'editable',
  created_at: 1,
  updated_at: 2,
  file_count: 1,
  total_size: 12,
  root_exists: true,
  tags: [],
  kind: 'blank',
});

describe('knowledge-base opening pipeline', () => {
  test('the page retry reloads a file index whose first request failed', async () => {
    const openingBase = base();
    const files: IKnowledgeFileEntry[] = [{ rel_path: 'README.md', size: 12, modified_at: null }];
    const listTree = spyOn(ipcBridge.knowledge.listTree, 'invoke').mockResolvedValue([]);
    const getBase = spyOn(ipcBridge.knowledge.getBase, 'invoke').mockResolvedValue(openingBase);
    const listFiles = spyOn(ipcBridge.knowledge.listFiles, 'invoke')
      .mockRejectedValueOnce(new Error('first file index request failed'))
      .mockResolvedValueOnce(files);
    const onBaseUpdated = spyOn(ipcBridge.knowledge.onBaseUpdated, 'on').mockImplementation(() => () => undefined);
    const log = spyOn(console, 'error').mockImplementation(() => undefined);
    restore.push(
      () => listTree.mockRestore(),
      () => getBase.mockRestore(),
      () => listFiles.mockRestore(),
      () => onBaseUpdated.mockRestore(),
      () => log.mockRestore()
    );

    const hook = renderHook(() => useKnowledgeBase(openingBase.knowledge_base_id, openingBase));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    await act(async () => { await expect(hook.result.current.loadFiles()).rejects.toThrow(); });
    expect(hook.result.current.filesLoaded).toBe(false);
    expect(hook.result.current.error).not.toBeNull();

    await act(async () => { await hook.result.current.refresh(); });
    expect(listFiles).toHaveBeenCalledTimes(2);
    expect(hook.result.current.files).toEqual(files);
    expect(hook.result.current.error).toBeNull();
  });

  test('retains loaded files and structured diagnostics across a failed refresh, then clears the error on retry', async () => {
    const openingBase = base();
    const failure = new BackendHttpError({
      method: 'GET',
      path: `/api/knowledge/bases/${openingBase.knowledge_base_id}/files`,
      status: 500,
      body: { success: false, code: 'INTERNAL_ERROR', error: 'raw filesystem diagnostic' },
    });
    const allFiles: IKnowledgeFileEntry[] = [
      { rel_path: 'README.md', size: 12, modified_at: null },
    ];
    const listTree = spyOn(ipcBridge.knowledge.listTree, 'invoke').mockResolvedValue([]);
    const listFiles = spyOn(ipcBridge.knowledge.listFiles, 'invoke')
      .mockResolvedValueOnce(allFiles)
      .mockRejectedValueOnce(failure)
      .mockResolvedValueOnce(allFiles);
    const onBaseUpdated = spyOn(ipcBridge.knowledge.onBaseUpdated, 'on').mockImplementation(() => () => undefined);
    const log = spyOn(console, 'error').mockImplementation(() => undefined);
    restore.push(
      () => listTree.mockRestore(),
      () => listFiles.mockRestore(),
      () => onBaseUpdated.mockRestore(),
      () => log.mockRestore()
    );

    const hook = renderHook(() => useKnowledgeBase(openingBase.knowledge_base_id, openingBase));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    await act(async () => { await hook.result.current.loadFiles(); });
    await act(async () => { await expect(hook.result.current.loadFiles()).rejects.toBe(failure); });

    expect(hook.result.current.files).toEqual(allFiles);
    expect(hook.result.current.base).toEqual(openingBase);
    expect(hook.result.current.error?.type).toBe('knowledge-error');
    expect(hook.result.current.error?.context).toBe('load');
    expect(hook.result.current.error?.message).not.toContain('raw filesystem diagnostic');
    expect(hook.result.current.error?.technicalDetails).toContain('raw filesystem diagnostic');

    await act(async () => { await hook.result.current.loadFiles(); });
    expect(hook.result.current.error).toBeNull();
    expect(hook.result.current.files).toEqual(allFiles);
  });

  test('reuses the list snapshot and prefetched root before loading the recursive file index', async () => {
    const openingBase = base();
    const treeRequest = deferred<IKnowledgeTreeEntry[]>();
    const rootTree: IKnowledgeTreeEntry[] = [
      {
        name: 'README.md',
        rel_path: 'README.md',
        is_dir: false,
        is_file: true,
        size: 12,
        modified_at: null,
      },
    ];
    const allFiles: IKnowledgeFileEntry[] = [
      { rel_path: 'README.md', size: 12, modified_at: null },
    ];

    const listTree = spyOn(ipcBridge.knowledge.listTree, 'invoke').mockImplementation(
      () => treeRequest.promise
    );
    const getBase = spyOn(ipcBridge.knowledge.getBase, 'invoke').mockResolvedValue(
      openingBase
    );
    const listFiles = spyOn(
      ipcBridge.knowledge.listFiles,
      'invoke'
    ).mockResolvedValue(allFiles);
    const onBaseUpdated = spyOn(
      ipcBridge.knowledge.onBaseUpdated,
      'on'
    ).mockImplementation(() => () => undefined);
    restore.push(
      () => listTree.mockRestore(),
      () => getBase.mockRestore(),
      () => listFiles.mockRestore(),
      () => onBaseUpdated.mockRestore()
    );

    void prefetchKnowledgeBaseOpen(openingBase);
    const hook = renderHook(() =>
      useKnowledgeBase(openingBase.knowledge_base_id, openingBase)
    );

    expect(hook.result.current.base).toEqual(openingBase);
    expect(listTree).toHaveBeenCalledTimes(1);
    expect(getBase).not.toHaveBeenCalled();
    expect(listFiles).not.toHaveBeenCalled();

    await act(async () => treeRequest.resolve(rootTree));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    expect(hook.result.current.tree).toEqual(rootTree);
    expect(getBase).not.toHaveBeenCalled();
    expect(listFiles).not.toHaveBeenCalled();

    await act(async () => {
      await hook.result.current.loadFiles();
    });
    expect(listFiles).toHaveBeenCalledTimes(1);
    expect(hook.result.current.files).toEqual(allFiles);
    expect(hook.result.current.filesLoaded).toBe(true);
  });
});
