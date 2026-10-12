import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import type { PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import type { ConversationId } from '@/common/types/ids';
import { PreviewProvider, usePreviewContext } from '@/renderer/pages/conversation/Preview/context/PreviewContext';
import { useAutoPreviewOfficeFiles } from './useAutoPreviewOfficeFiles';

type AddedEvent = { file_path: string; workspace: string };
type WriteEvent = AddedEvent & { relative_path: string; operation: 'write' | 'delete' };
let added: ((event: AddedEvent) => void) | undefined;
const writes = new Set<(event: WriteEvent) => void>();
let restore: Array<() => void> = [];
const conversation = { conversation_id: '01900000-0000-7000-8000-000000000001' as ConversationId, workspace: '/workspace' };
const wrapper = ({ children }: PropsWithChildren) => <PreviewProvider persistNamespace='auto-office-fixture' subscribeGlobalOpen={false}>{children}</PreviewProvider>;
const open = () => renderHook(() => { useAutoPreviewOfficeFiles(conversation); return usePreviewContext(); }, { wrapper });
const sleep = (ms: number) => act(async () => { await new Promise((resolve) => setTimeout(resolve, ms)); });
const emitWrite = (operation: WriteEvent['operation'] = 'write') => act(() => {
  for (const handler of writes) handler({ file_path: '/workspace/new.pptx', workspace: '/workspace', relative_path: 'new.pptx', operation });
});

beforeEach(() => {
  writes.clear();
  restore = [
    spyOn(ipcBridge.workspaceOfficeWatch.start, 'invoke').mockResolvedValue(),
    spyOn(ipcBridge.workspaceOfficeWatch.stop, 'invoke').mockResolvedValue(),
    spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([{ name: 'existing.pptx', fullPath: '/workspace/existing.pptx', relativePath: 'existing.pptx' }]),
    spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockResolvedValue({ name: 'new.pptx', path: '/workspace/new.pptx', size: 20, type: 'pptx', lastModified: 1 }),
    spyOn(ipcBridge.workspaceOfficeWatch.fileAdded, 'on').mockImplementation((handler) => { added = handler; return () => { added = undefined; }; }),
    spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation((handler) => { writes.add(handler); return () => { writes.delete(handler); }; }),
    spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockReturnValue(() => {}),
    spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockReturnValue(() => {}),
  ].map((spy) => () => spy.mockRestore());
});
afterEach(() => { cleanup(); restore.forEach((run) => run()); localStorage.clear(); });

test('auto-open coalesces file creation and subsequent writes rather than waiting a fixed second', async () => {
  const hook = open();
  await act(async () => {});
  act(() => added?.({ file_path: '/workspace/new.pptx', workspace: '/workspace' }));
  await sleep(300);
  emitWrite();
  await sleep(300);
  expect(hook.result.current.tabs).toHaveLength(0);
  await sleep(250);
  expect(hook.result.current.tabs).toHaveLength(1);
  expect(hook.result.current.activeTab?.metadata?.file_path).toBe('/workspace/new.pptx');
  emitWrite();
  await sleep(550);
  expect(hook.result.current.tabs).toHaveLength(1);
});

test('baseline, other workspaces and non-Office files do not auto-open', async () => {
  const hook = open();
  await act(async () => {});
  act(() => {
    added?.({ file_path: '/workspace/existing.pptx', workspace: '/workspace' });
    added?.({ file_path: '/workspace/new.txt', workspace: '/workspace' });
    added?.({ file_path: '/elsewhere/new.pptx', workspace: '/elsewhere' });
  });
  await sleep(550);
  expect(hook.result.current.tabs).toHaveLength(0);
});

test('deletion and unmount cancel a pending auto-open and unsubscribe both event sources', async () => {
  const hook = open();
  await act(async () => {});
  act(() => added?.({ file_path: '/workspace/new.pptx', workspace: '/workspace' }));
  emitWrite('delete');
  await sleep(550);
  expect(hook.result.current.tabs).toHaveLength(0);
  act(() => added?.({ file_path: '/workspace/new.pptx', workspace: '/workspace' }));
  hook.unmount();
  expect(added).toBeUndefined();
  expect(writes.size).toBe(0);
  await sleep(550);
});
