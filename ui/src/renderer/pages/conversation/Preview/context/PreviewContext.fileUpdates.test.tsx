import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { useRef, useState } from 'react';
import type { PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import { previewFileKey, usePreviewFileRefresh } from './usePreviewFileRefresh';
import type { PreviewTab } from './PreviewContext';
import { LARGE_TEXT_PREVIEW_MAX_LENGTH, LARGE_TEXT_PREVIEW_THRESHOLD } from '../constants';

type FileEvent = { file_path: string; content?: string; workspace: string; relative_path: string; operation: 'write' | 'delete' };
let listener: ((event: FileEvent) => void) | undefined;
const readFile = mock(async (): Promise<string | null> => 'fresh disk content');
const fileMetadata = { name: 'report.txt', path: '/workspace/report.txt', size: 8, type: 'txt', lastModified: 1 };
const getMetadata = mock(async () => fileMetadata);
const writeFile = mock(async () => true);
const getImage = mock(async () => 'fresh image');
const { PreviewProvider, usePreviewContext } = await import('./PreviewContext');
let restoreSpies: Array<() => void> = [];

beforeEach(() => {
  readFile.mockReset().mockImplementation(async () => 'fresh disk content');
  getMetadata.mockReset().mockImplementation(async () => fileMetadata);
  writeFile.mockReset().mockImplementation(async () => true);
  getImage.mockReset().mockImplementation(async () => 'fresh image');
  const spies = [
    spyOn(ipcBridge.fs.readFile, 'invoke').mockImplementation(readFile),
    spyOn(ipcBridge.fs.writeFile, 'invoke').mockImplementation(writeFile),
    spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockImplementation(getMetadata),
    spyOn(ipcBridge.fs.getImageBase64, 'invoke').mockImplementation(getImage),
    spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation((handler) => {
      listener = handler;
      return () => { listener = undefined; };
    }),
    spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockReturnValue(() => {}),
    spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockReturnValue(() => {}),
  ];
  restoreSpies = spies.map((spy) => () => spy.mockRestore());
});
afterEach(() => { cleanup(); restoreSpies.forEach((restore) => restore()); localStorage.clear(); });
const wrapper = ({ children }: PropsWithChildren) => <PreviewProvider persistNamespace='fixture-preview' subscribeGlobalOpen={false}>{children}</PreviewProvider>;
const path = '/workspace/report.txt';
const metadata = { file_path: path, workspace: '/workspace' };
const open = () => {
  const hook = renderHook(usePreviewContext, { wrapper });
  act(() => hook.result.current.openPreview('original', 'code', metadata));
  return hook;
};
const changed = (content?: string, operation: FileEvent['operation'] = 'write', file_path = path) => {
  listener?.({ file_path, content, operation, workspace: '/workspace', relative_path: 'report.txt' });
};
// Exercise the real 500 ms event coalescing window, without changing production timing.
const deliverChange = async (content?: string) => act(async () => {
  changed(content);
  await new Promise((resolve) => setTimeout(resolve, 550));
});
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

test('a write notification without inline content rereads disk instead of blanking preview', async () => {
  const hook = renderHook(usePreviewContext, { wrapper });
  act(() => hook.result.current.openPreview('old content', 'code', { file_path: '/workspace/report.txt', workspace: '/workspace' }));
  await act(async () => {});
  await act(async () => {
    listener?.({ file_path: '/workspace/report.txt', workspace: '/workspace', relative_path: 'report.txt', operation: 'write' });
    // The production subscription deliberately coalesces changes for 500 ms.
    await new Promise((resolve) => setTimeout(resolve, 550));
  });
  expect(hook.result.current.activeTab?.content).toBe('fresh disk content');
  expect(readFile).toHaveBeenCalledWith({ path: '/workspace/report.txt', workspace: '/workspace' });
  expect(hook.result.current.activeTab?.isDirty).toBe(false);
});

test('edits made while a save is pending stay dirty after its acknowledgment', async () => {
  let resolve!: (value: boolean) => void;
  const pending = new Promise<boolean>((yes) => { resolve = yes; });
  writeFile.mockImplementation(() => pending);
  const hook = renderHook(usePreviewContext, { wrapper });
  act(() => hook.result.current.openPreview('original', 'code', { file_path: '/workspace/report.txt', workspace: '/workspace' }));
  act(() => hook.result.current.updateContent('sent to disk'));
  let saving!: Promise<boolean>;
  act(() => { saving = hook.result.current.saveContent(); });
  act(() => hook.result.current.updateContent('later unsaved edit'));
  await act(async () => { resolve(true); await saving; });
  expect(hook.result.current.activeTab?.content).toBe('later unsaved edit');
  expect(hook.result.current.activeTab?.originalContent).toBe('sent to disk');
  expect(hook.result.current.activeTab?.isDirty).toBe(true);
});

test('an explicit empty update is valid content', async () => {
  const hook = open();
  await deliverChange('');
  expect(hook.result.current.activeTab?.content).toBe('');
  expect(hook.result.current.activeTab?.originalContent).toBe('');
  expect(readFile).not.toHaveBeenCalled();
});

test('binary notifications use the image reader', async () => {
  const hook = renderHook(usePreviewContext, { wrapper });
  act(() => hook.result.current.openPreview('old image', 'image', metadata));
  await deliverChange();
  expect(hook.result.current.activeTab?.content).toBe('fresh image');
  expect(getImage).toHaveBeenCalledWith({ path, workspace: '/workspace' });
  expect(readFile).not.toHaveBeenCalled();
});

test('a failed reread preserves content, exposes the error, and can be retried', async () => {
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  try {
    const hook = open();
    readFile.mockRejectedValueOnce(new Error('access denied'));
    await act(async () => { await hook.result.current.refreshFile(); });
    expect(hook.result.current.activeTab?.content).toBe('original');
    expect(hook.result.current.activeTab?.fileReadError).toBe(true);
    expect(hook.result.current.activeTab?.fileRefreshing).toBe(false);
    await act(async () => { await hook.result.current.refreshFile(); });
    expect(hook.result.current.activeTab?.content).toBe('fresh disk content');
    expect(hook.result.current.activeTab?.fileReadError).toBe(false);
  } finally { errors.mockRestore(); }
});

test('missing read data is an error, not an empty file', async () => {
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  try {
    const hook = open();
    readFile.mockResolvedValueOnce(null);
    await act(async () => { await hook.result.current.refreshFile(); });
    expect(hook.result.current.activeTab?.content).toBe('original');
    expect(hook.result.current.activeTab?.fileReadError).toBe(true);
  } finally { errors.mockRestore(); }
});

test('an in-flight read cannot overwrite edits even after editing back to the original', async () => {
  const pending = deferred<string>();
  readFile.mockImplementationOnce(() => pending.promise);
  const hook = open();
  let reading!: Promise<void>;
  act(() => { reading = hook.result.current.refreshFile(); });
  act(() => hook.result.current.updateContent('local edit'));
  act(() => hook.result.current.updateContent('original'));
  await act(async () => { pending.resolve('stale read'); await reading; });
  expect(hook.result.current.activeTab?.content).toBe('original');
  expect(hook.result.current.activeTab?.fileRefreshing).toBe(false);
});

test('newer disk observations win when read replies arrive in reverse order', async () => {
  const old = deferred<string>();
  readFile.mockImplementationOnce(() => old.promise);
  const hook = open();
  let reading!: Promise<void>;
  act(() => { reading = hook.result.current.refreshFile(); });
  await act(async () => { await hook.result.current.refreshFile(); });
  await act(async () => { old.resolve('stale read'); await reading; });
  expect(hook.result.current.activeTab?.content).toBe('fresh disk content');
});

test('a new event cancels an old read before the coalescing window ends', async () => {
  const old = deferred<string>();
  readFile.mockImplementationOnce(() => old.promise);
  const hook = open();
  let reading!: Promise<void>;
  act(() => { reading = hook.result.current.refreshFile(); });
  act(() => changed('new event'));
  await act(async () => { old.resolve('stale read'); await reading; });
  expect(hook.result.current.activeTab?.content).toBe('original');
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 550)); });
  expect(hook.result.current.activeTab?.content).toBe('new event');
});

test('closed and reopened tabs reject a previous read of the same file', async () => {
  const old = deferred<string>();
  readFile.mockImplementationOnce(() => old.promise);
  const hook = open();
  let reading!: Promise<void>;
  act(() => { reading = hook.result.current.refreshFile(); });
  act(() => hook.result.current.closePreview());
  act(() => hook.result.current.openPreview('reopened', 'code', metadata));
  await act(async () => { old.resolve('stale read'); await reading; });
  expect(hook.result.current.tabs).toHaveLength(1);
  expect(hook.result.current.activeTab?.content).toBe('reopened');
});

test('unmount unsubscribes and discards a delayed rejection', async () => {
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  try {
    const old = deferred<string>();
    readFile.mockImplementationOnce(() => old.promise);
    const hook = open();
    let reading!: Promise<void>;
    act(() => { reading = hook.result.current.refreshFile(); });
    hook.unmount();
    expect(listener).toBeUndefined();
    await act(async () => { old.reject(new Error('late')); await reading; });
    expect(errors).not.toHaveBeenCalled();
  } finally { errors.mockRestore(); }
});

test('delete events close clean previews and preserve dirty drafts', () => {
  const hook = open();
  act(() => changed(undefined, 'delete'));
  expect(hook.result.current.tabs).toHaveLength(0);
  act(() => hook.result.current.openPreview('original', 'code', metadata));
  act(() => hook.result.current.updateContent('draft'));
  act(() => changed(undefined, 'delete'));
  expect(hook.result.current.activeTab?.content).toBe('draft');
  expect(hook.result.current.activeTab?.isDirty).toBe(true);
  expect(hook.result.current.activeTab?.fileReadError).toBe(true);
});

test('write events and manual rereads leave dirty drafts intact', async () => {
  const hook = open();
  act(() => hook.result.current.updateContent('draft'));
  await deliverChange('external');
  await act(async () => { await hook.result.current.refreshFile(); });
  expect(hook.result.current.activeTab?.content).toBe('draft');
  expect(hook.result.current.activeTab?.isDirty).toBe(true);
  expect(readFile).not.toHaveBeenCalled();
});

test('oversize code rereads remain truncated and cannot be saved as partial files', async () => {
  readFile.mockResolvedValueOnce('a'.repeat(LARGE_TEXT_PREVIEW_THRESHOLD + 1));
  const hook = open();
  await act(async () => { await hook.result.current.refreshFile(); });
  expect(hook.result.current.activeTab?.content).toHaveLength(LARGE_TEXT_PREVIEW_MAX_LENGTH);
  expect(hook.result.current.activeTab?.metadata?.truncated).toBe(true);
  expect(hook.result.current.activeTab?.metadata?.editable).toBe(false);
});

test('Windows native path spelling matches without collapsing case-sensitive names', async () => {
  expect(previewFileKey('c:\\work\\Report.txt')).toBe('C:/work/Report.txt');
  expect(previewFileKey('\\\\?\\C:\\work\\Report.txt')).toBe('C:/work/Report.txt');
  expect(previewFileKey('\\\\?\\UNC\\server\\share\\Report.txt')).toBe('//server/share/Report.txt');
  expect(previewFileKey('/work/a\\b')).toBe('/work/a\\b');
  expect(previewFileKey('C:/work/Report.txt')).not.toBe(previewFileKey('C:/work/report.txt'));
  const hook = renderHook(usePreviewContext, { wrapper });
  act(() => hook.result.current.openPreview('original', 'code', { file_path: 'C:/work/Report.txt', workspace: 'C:/work' }));
  await act(async () => {
    changed('native update', 'write', '\\\\?\\C:\\work\\Report.txt');
    await new Promise((resolve) => setTimeout(resolve, 550));
  });
  expect(hook.result.current.activeTab?.content).toBe('native update');
});

test('polling cannot starve an in-flight read or advance its mtime before success', async () => {
  const old = deferred<string>();
  const text = mock(() => old.promise);
  const metadataRead = mock(async () => ({ lastModified: 2 }));
  const io = { subscribe: () => () => {}, text, image: text, metadata: metadataRead };
  const hook = renderHook(() => {
    const [tabs, setTabs] = useState<PreviewTab[]>([{ id: 'tab', title: 'report', content: 'original', originalContent: 'original', content_type: 'code', metadata }]);
    const mtimes = useRef(new Map([[path, 1]]));
    const refresh = usePreviewFileRefresh({ tabs, setTabs, io, mtimes, saving: useRef(new Set<string>()), closeTab: useRef(() => {}) });
    return { tabs, mtimes, refresh };
  });
  let reading!: Promise<void>;
  await act(async () => { reading = hook.result.current.refresh.refresh('tab', true); });
  await act(async () => { await hook.result.current.refresh.refresh('tab', true); });
  expect(metadataRead).toHaveBeenCalledTimes(1);
  expect(text).toHaveBeenCalledTimes(1);
  expect(hook.result.current.mtimes.current.get(path)).toBe(1);
  await act(async () => { old.resolve('disk truth'); await reading; });
  expect(hook.result.current.tabs[0].content).toBe('disk truth');
  expect(hook.result.current.mtimes.current.get(path)).toBe(2);
});

test('a failed save releases the refresh indicator and keeps the previous read fenced', async () => {
  const old = deferred<string>();
  readFile.mockImplementationOnce(() => old.promise);
  writeFile.mockRejectedValueOnce(new Error('save denied'));
  const hook = open();
  let reading!: Promise<void>;
  act(() => { reading = hook.result.current.refreshFile(); });
  expect(hook.result.current.activeTab?.fileRefreshing).toBe(true);
  await act(async () => { await expect(hook.result.current.saveContent()).rejects.toThrow('save denied'); });
  await act(async () => { old.resolve('stale read'); await reading; });
  expect(hook.result.current.activeTab?.content).toBe('original');
  expect(hook.result.current.activeTab?.fileRefreshing).toBe(false);
});

test('a failed poll retains its mtime so the next poll can recover', async () => {
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  try {
    const text = mock(async () => 'recovered');
    text.mockRejectedValueOnce(new Error('read denied'));
    const io = { subscribe: () => () => {}, text, image: text, metadata: async () => ({ lastModified: 2 }) };
    const hook = renderHook(() => {
      const [tabs, setTabs] = useState<PreviewTab[]>([{ id: 'tab', title: 'report', content: 'original', originalContent: 'original', content_type: 'code', metadata }]);
      const mtimes = useRef(new Map([[path, 1]]));
      const refresh = usePreviewFileRefresh({ tabs, setTabs, io, mtimes, saving: useRef(new Set<string>()), closeTab: useRef(() => {}) });
      return { tabs, mtimes, refresh };
    });
    await act(async () => { await hook.result.current.refresh.refresh('tab', true); });
    expect(hook.result.current.tabs[0].content).toBe('original');
    expect(hook.result.current.tabs[0].fileReadError).toBe(true);
    expect(hook.result.current.mtimes.current.get(path)).toBe(1);
    await act(async () => { await hook.result.current.refresh.refresh('tab', true); });
    expect(hook.result.current.tabs[0].content).toBe('recovered');
    expect(hook.result.current.tabs[0].fileReadError).toBe(false);
    expect(hook.result.current.mtimes.current.get(path)).toBe(2);
  } finally { errors.mockRestore(); }
});
