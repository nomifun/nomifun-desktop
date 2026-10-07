import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import type { IDirOrFile } from '@/common/adapter/ipcBridge';
import { conversationTarget, parseConversationId } from '@/common/types/ids';
import type { WorkspaceTreeSource } from '../types';
import { useWorkspaceTree } from './useWorkspaceTree';

afterEach(cleanup);

test('a failed first root read reports failure instead of an empty successful listing', async () => {
  const failure = new Error('fixture workspace unavailable');
  const treeSource = source(async () => { throw failure; }, async () => []);
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  try {
    let result: unknown;
    await act(async () => { result = await hook.result.current.refreshWorkspace(); });
    expect(result).toBeNull();
    expect(hook.result.current.hasLoadError).toBe(true);
    expect(hook.result.current.files).toEqual([]);
    expect(errors).toHaveBeenCalledWith('[useWorkspaceTree] loadWorkspace failed:', failure);
  } finally {
    errors.mockRestore();
  }
});

test('a failed refresh marks the retained snapshot as stale', async () => {
  let fail = false;
  const treeSource = source(async () => {
    if (fail) throw new Error('fixture workspace moved');
    return [directory('', [file('known.txt')])];
  }, async () => []);
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  const prior = hook.result.current.files;
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  try {
    fail = true;
    await act(async () => { await hook.result.current.refreshWorkspace(); });
    expect(hook.result.current.files).toBe(prior);
    expect(hook.result.current.hasLoadError).toBe(true);
  } finally {
    errors.mockRestore();
  }
});

test('an immediate successful retry clears the preceding root failure', async () => {
  let fail = true;
  const treeSource = source(async () => {
    if (fail) throw new Error('temporary read failure');
    return [directory('', [file('recovered.txt')])];
  }, async () => []);
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  try {
    await act(async () => {
      await hook.result.current.refreshWorkspace();
      fail = false;
      await hook.result.current.retryWorkspace();
    });
    expect(hook.result.current.hasLoadError).toBe(false);
    expect(hook.result.current.files[0].children).toEqual([file('recovered.txt')]);
  } finally {
    errors.mockRestore();
  }
});

test('a pending retry keeps the failure visible and preserves the requested search', async () => {
  let retry = false;
  const pending = deferred<IDirOrFile[]>();
  const queries: Array<string | undefined> = [];
  const treeSource = source(async (query) => {
    queries.push(query);
    if (!retry) throw new Error('read unavailable');
    return pending.promise;
  }, async () => []);
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  try {
    await act(async () => { await hook.result.current.loadWorkspace('', 'needle'); });
    retry = true;
    let request!: Promise<IDirOrFile[] | null>;
    act(() => { request = hook.result.current.retryWorkspace(); });
    expect(hook.result.current.hasLoadError).toBe(true);
    await act(async () => { pending.resolve([directory('', [file('needle.txt')])]); await request; });
    expect(queries).toEqual(['needle', 'needle']);
    expect(hook.result.current.hasLoadError).toBe(false);
  } finally {
    errors.mockRestore();
  }
});

test('an unrelated child success keeps the failed directory pending until it is re-read', async () => {
  let failed = true;
  const reads: string[] = [];
  const treeSource = source(async () => [directory('', [directory('first'), directory('second'), directory('unread')])], async (node) => {
    reads.push(node.relativePath);
    if (node.relativePath === 'first' && failed) throw new Error('first is unavailable');
    return [directory(node.relativePath, [file(`${node.relativePath}/current.txt`)])];
  });
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  try {
    await act(async () => { await hook.result.current.refreshWorkspace(); });
    await act(async () => { await hook.result.current.loadChildren(directory('first')); });
    await act(async () => { await hook.result.current.loadChildren(directory('second')); });
    expect(hook.result.current.hasLoadError).toBe(true);
    failed = false;
    await act(async () => { await hook.result.current.retryWorkspace(); });
    expect(reads).toEqual(['first', 'second', 'first', 'second']);
    expect(hook.result.current.files[0].children?.[0].children).toEqual([file('first/current.txt')]);
    expect(hook.result.current.hasLoadError).toBe(false);
  } finally {
    errors.mockRestore();
  }
});

test('an older root success cannot clear a newer directory failure', async () => {
  let delayRoot = false;
  const pending = deferred<IDirOrFile[]>();
  const rows = [directory('', [directory('nested')])];
  const treeSource = source(async () => delayRoot ? pending.promise : rows, async () => { throw new Error('new child failure'); });
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  try {
    await act(async () => { await hook.result.current.refreshWorkspace(); });
    delayRoot = true;
    let request!: Promise<IDirOrFile[] | null>;
    act(() => { request = hook.result.current.refreshWorkspace(); });
    await act(async () => { await hook.result.current.loadChildren(directory('nested')); });
    await act(async () => { pending.resolve(rows); await request; });
    expect(hook.result.current.hasLoadError).toBe(true);
  } finally {
    errors.mockRestore();
  }
});

function directory(path: string, children?: IDirOrFile[]): IDirOrFile {
  return { name: path || 'workspace', fullPath: `/workspace/${path}`, relativePath: path,
    isDir: true, isFile: false, ...(children === undefined ? {} : { children }) };
}

function file(path: string): IDirOrFile {
  return { name: path.split('/').at(-1)!, fullPath: `/workspace/${path}`, relativePath: path,
    isDir: false, isFile: true };
}

function source(listRoot: WorkspaceTreeSource['listRoot'], listChildren: WorkspaceTreeSource['listChildren']): WorkspaceTreeSource {
  return { key: 'workspace', target: conversationTarget(parseConversationId('0190f5fe-7c00-7a00-8abc-012345678901')),
    listRoot, listChildren };
}

test('a successful empty root refresh removes the last displayed file', async () => {
  let rows = [directory('', [file('last.txt')])];
  const treeSource = source(async () => rows, async () => []);
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  expect(hook.result.current.files[0].children).toHaveLength(1);
  rows = [directory('', [])];
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  expect(hook.result.current.files[0].children).toEqual([]);
});

test('root reconciliation re-reads a loaded directory instead of copying its old children', async () => {
  let children = [file('nested/old.txt')];
  let childReads = 0;
  const treeSource = source(async () => [directory('', [directory('nested')])], async () => {
    childReads += 1;
    return [directory('nested', children)];
  });
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  await act(async () => { await hook.result.current.loadChildren(directory('nested')); });
  children = [];
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  expect(hook.result.current.files[0].children?.[0].children).toEqual([]);
  expect(childReads).toBe(2);
});

test('an explicitly loaded empty directory clears earlier child rows', async () => {
  let children = [file('nested/old.txt')];
  const treeSource = source(async () => [directory('', [directory('nested')])], async () => [directory('nested', children)]);
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  await act(async () => { await hook.result.current.loadChildren(directory('nested')); });
  children = [];
  await act(async () => { await hook.result.current.loadChildren(directory('nested')); });
  expect(hook.result.current.files[0].children?.[0].children).toEqual([]);
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

test('a root refresh fences an older pending child response', async () => {
  let rows = [directory('', [directory('nested')])];
  const pending = deferred<IDirOrFile[]>();
  const treeSource = source(async () => rows, () => pending.promise);
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  let childRead!: Promise<void>;
  act(() => { childRead = hook.result.current.loadChildren(directory('nested')); });
  rows = [directory('', [directory('nested', [])])];
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  await act(async () => {
    pending.resolve([directory('nested', [file('nested/ghost.txt')])]);
    await childRead;
  });
  expect(hook.result.current.files[0].children?.[0].children).toEqual([]);
});

test('a superseded reconciliation cannot overwrite a newer empty root', async () => {
  let rows = [directory('', [directory('nested')])];
  const started = deferred<void>();
  const pending = deferred<IDirOrFile[]>();
  let reads = 0;
  const treeSource = source(async () => rows, () => {
    if (++reads === 1) return Promise.resolve([directory('nested', [file('nested/old.txt')])]);
    started.resolve();
    return pending.promise;
  });
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  await act(async () => { await hook.result.current.loadChildren(directory('nested')); });
  let previous!: Promise<IDirOrFile[] | null>;
  await act(async () => { previous = hook.result.current.refreshWorkspace(); await started.promise; });
  rows = [directory('', [])];
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  await act(async () => {
    pending.resolve([directory('nested', [file('nested/stale.txt')])]);
    await previous;
  });
  expect(hook.result.current.files[0].children).toEqual([]);
  expect(reads).toBe(2);
});

test('refresh discovers additions in a loaded empty folder without traversing unread branches', async () => {
  let children: IDirOrFile[] = [];
  const reads: string[] = [];
  const treeSource = source(async () => [directory('', [directory('nested'), directory('unread')])], async (node) => {
    reads.push(node.relativePath);
    return [directory(node.relativePath, children)];
  });
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  await act(async () => { await hook.result.current.loadChildren(directory('nested')); });
  act(() => { hook.result.current.setExpandedKeys(['', 'nested']); });
  children = [file('nested/new.txt')];
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  expect(hook.result.current.files[0].children?.[0].children).toEqual(children);
  expect(hook.result.current.files[0].children?.[1].children).toBeUndefined();
  expect(reads).toEqual(['nested', 'nested']);
  expect(hook.result.current.expandedKeys).toContain('nested');
});

test('switching sources rejects responses from the previous workspace', async () => {
  const previous = deferred<IDirOrFile[]>();
  const first = source(() => previous.promise, async () => []);
  const second = { ...source(async () => [directory('', [file('current.txt')])], async () => []), key: 'other-workspace' };
  const hook = renderHook(({ treeSource }) => useWorkspaceTree({ treeSource }), { initialProps: { treeSource: first } });
  let firstRead!: Promise<IDirOrFile[] | null>;
  act(() => { firstRead = hook.result.current.refreshWorkspace(); });
  hook.rerender({ treeSource: second });
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  await act(async () => { previous.resolve([directory('', [file('old-workspace.txt')])]); await firstRead; });
  expect(hook.result.current.files[0].children).toEqual([file('current.txt')]);
});

test('a failed descendant read does not publish a partial refresh or pretend the directory is empty', async () => {
  let fail = false;
  const failure = new Error('directory access denied');
  const treeSource = source(async () => [directory('', [directory('nested')])], async () => {
    if (fail) throw failure;
    return [directory('nested', [file('nested/known.txt')])];
  });
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  await act(async () => { await hook.result.current.loadChildren(directory('nested')); });
  const prior = hook.result.current.files;
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  try {
    fail = true;
    await act(async () => { await hook.result.current.refreshWorkspace(); });
    expect(hook.result.current.files).toBe(prior);
    expect(errors).toHaveBeenCalledWith('[useWorkspaceTree] loadWorkspace failed:', failure);
  } finally {
    errors.mockRestore();
  }
});

test('unmount prevents further reads in an unfinished reconciliation', async () => {
  const started = deferred<void>();
  const pending = deferred<IDirOrFile[]>();
  let refreshing = false;
  const reads: string[] = [];
  const treeSource = source(async () => [directory('', [directory('first'), directory('second')])], async (node) => {
    if (!refreshing) return [directory(node.relativePath, [])];
    reads.push(node.relativePath);
    started.resolve();
    return pending.promise;
  });
  const hook = renderHook(() => useWorkspaceTree({ treeSource }));
  await act(async () => { await hook.result.current.refreshWorkspace(); });
  await act(async () => {
    await hook.result.current.loadChildren(directory('first'));
    await hook.result.current.loadChildren(directory('second'));
  });
  refreshing = true;
  let refresh!: Promise<IDirOrFile[] | null>;
  await act(async () => { refresh = hook.result.current.refreshWorkspace(); await started.promise; });
  hook.unmount();
  pending.resolve([directory('first', [])]);
  await refresh;
  expect(reads).toEqual(['first']);
});
