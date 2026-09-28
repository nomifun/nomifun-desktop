import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import type { IWorkspaceFlatFile } from '@/common/adapter/ipcBridge';
import { useWorkspaceMentionFiles } from './useWorkspaceMentionFiles';

afterEach(cleanup);

const initial = { workspace: '/first', sessionKey: 'session-a:0', enabled: true };
const file = (name: string): IWorkspaceFlatFile => ({ name, fullPath: `/first/${name}`, relativePath: name });
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test('a rejected read stays distinct from an empty successful inventory and supports retry', async () => {
  const failure = new Error('fixture rules unavailable');
  const pending = deferred<IWorkspaceFlatFile[]>();
  const read = mock(async (): Promise<IWorkspaceFlatFile[]> => { throw failure; });
  const warnings = spyOn(console, 'warn').mockImplementation(() => {});
  try {
    const hook = renderHook(() => useWorkspaceMentionFiles({ ...initial, listFiles: read }));
    await act(async () => {});
    expect(hook.result.current.hasError).toBe(true);
    expect(hook.result.current.loading).toBe(false);
    expect(hook.result.current.items).toEqual([]);
    expect(warnings).toHaveBeenCalledWith('[SendBox] Failed to load workspace file mentions:', failure);
    read.mockImplementation(() => pending.promise);
    act(() => hook.result.current.retry());
    expect(read).toHaveBeenCalledTimes(2);
    expect(read).toHaveBeenLastCalledWith('/first');
    expect(hook.result.current.loading).toBe(true);
    expect(hook.result.current.items).toEqual([]);
    await act(async () => { pending.resolve([file('recovered.txt')]); });
    expect(hook.result.current.hasError).toBe(false);
    expect(hook.result.current.loading).toBe(false);
    expect(hook.result.current.items).toEqual([{ path: '/first/recovered.txt', name: 'recovered.txt', relativePath: 'recovered.txt', isFile: true }]);
  } finally { warnings.mockRestore(); }
});

test('a successful empty inventory is ready rather than failed', async () => {
  const read = mock(async () => []);
  const hook = renderHook(() => useWorkspaceMentionFiles({ ...initial, listFiles: read }));
  await act(async () => {});
  expect(hook.result.current.items).toEqual([]);
  expect(hook.result.current.hasError).toBe(false);
  expect(hook.result.current.loading).toBe(false);
});

test('unchanged source rerenders do not refetch while the menu is open', async () => {
  const read = mock(async () => [file('known.txt')]);
  const hook = renderHook((props) => useWorkspaceMentionFiles({ ...props, listFiles: read }), { initialProps: initial });
  await act(async () => {});
  hook.rerender({ ...initial });
  expect(read).toHaveBeenCalledTimes(1);
  expect(hook.result.current.items[0]?.name).toBe('known.txt');
});

test('switching workspace hides old options until its own request completes', async () => {
  const second = deferred<IWorkspaceFlatFile[]>();
  const read = mock((root: string) => root === '/first' ? Promise.resolve([file('old.txt')]) : second.promise);
  const hook = renderHook((props) => useWorkspaceMentionFiles({ ...props, listFiles: read }), { initialProps: initial });
  await act(async () => {});
  expect(hook.result.current.items[0]?.name).toBe('old.txt');
  hook.rerender({ ...initial, workspace: '/second' });
  expect(hook.result.current.items).toEqual([]);
  expect(hook.result.current.loading).toBe(true);
  await act(async () => { second.resolve([{ name: 'new.txt', fullPath: '/second/new.txt', relativePath: 'new.txt' }]); });
  expect(hook.result.current.items[0]?.path).toBe('/second/new.txt');
});

test('a new session in the same workspace rejects the preceding late result', async () => {
  const old = deferred<IWorkspaceFlatFile[]>();
  const current = deferred<IWorkspaceFlatFile[]>();
  const read = mock(() => old.promise);
  const hook = renderHook((props) => useWorkspaceMentionFiles({ ...props, listFiles: read }), { initialProps: initial });
  read.mockImplementation(() => current.promise);
  hook.rerender({ ...initial, sessionKey: 'session-b:0' });
  await act(async () => { current.resolve([file('current.txt')]); });
  await act(async () => { old.resolve([file('obsolete.txt')]); });
  expect(hook.result.current.items.map((item) => item.name)).toEqual(['current.txt']);
  expect(hook.result.current.hasError).toBe(false);
});

test('closing and reopening the same menu starts a fresh read and ignores a late error', async () => {
  const old = deferred<IWorkspaceFlatFile[]>();
  const read = mock(() => old.promise);
  const warnings = spyOn(console, 'warn').mockImplementation(() => {});
  try {
    const hook = renderHook((props) => useWorkspaceMentionFiles({ ...props, listFiles: read }), { initialProps: initial });
    hook.rerender({ ...initial, enabled: false });
    expect(hook.result.current.loading).toBe(false);
    expect(hook.result.current.items).toEqual([]);
    read.mockImplementation(async () => [file('fresh.txt')]);
    hook.rerender(initial);
    await act(async () => {});
    await act(async () => { old.reject(new Error('obsolete failure')); });
    expect(read).toHaveBeenCalledTimes(2);
    expect(hook.result.current.items[0]?.name).toBe('fresh.txt');
    expect(hook.result.current.hasError).toBe(false);
    expect(warnings).not.toHaveBeenCalled();
  } finally { warnings.mockRestore(); }
});

test('an unmounted menu ignores its outstanding request failure', async () => {
  const pending = deferred<IWorkspaceFlatFile[]>();
  const read = mock(() => pending.promise);
  const warnings = spyOn(console, 'warn').mockImplementation(() => {});
  try {
    const hook = renderHook(() => useWorkspaceMentionFiles({ ...initial, listFiles: read }));
    hook.unmount();
    await act(async () => { pending.reject(new Error('late after unmount')); });
    expect(warnings).not.toHaveBeenCalled();
    expect(read).toHaveBeenCalledTimes(1);
  } finally { warnings.mockRestore(); }
});

test('disabled or unbound menus do not dispatch reads or retries', () => {
  const read = mock(async () => []);
  const hook = renderHook(() => useWorkspaceMentionFiles({ workspace: undefined, sessionKey: null, enabled: true, listFiles: read }));
  act(() => hook.result.current.retry());
  expect(read).not.toHaveBeenCalled();
  expect(hook.result.current.loading).toBe(false);
  expect(hook.result.current.hasError).toBe(false);
});
