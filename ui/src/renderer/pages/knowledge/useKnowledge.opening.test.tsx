import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, spyOn, test } from 'bun:test';

import { ipcBridge } from '@/common';
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
