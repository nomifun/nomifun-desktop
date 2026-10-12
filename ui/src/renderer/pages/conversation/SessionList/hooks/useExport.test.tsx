import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { useState, type PropsWithChildren } from 'react';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId, parseMessageId, type ConversationId } from '@/common/types/ids';
import conversationLocale from '@/renderer/services/i18n/locales/en-US/conversation.json';
import { useExport } from './useExport';

const [a, b] = [1, 2].map((n) => parseConversationId(`0190f5fe-7c00-7a00-8000-${String(n).padStart(12, '0')}`));
const conversations = [a, b].map((id, i) => ({ id, name: `topic-${i}`, type: 'nomi', extra: { workspace: `D:/project-${i}` } }) as TChatConversation);
const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: { conversation: conversationLocale } } } });
const wrapper = ({ children }: PropsWithChildren) => <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
const restores: Array<() => void> = [];
afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach((restore) => restore());
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function fixture() {
  const desktop = spyOn(ipcBridge.application.getPath, 'invoke').mockResolvedValue('C:/Users/test/Desktop');
  const history = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockImplementation(async ({ conversation_id }) => ({
    items: [{ id: 'row', message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000003'), conversation_id,
      type: 'text', position: 'right', content: { content: 'full transcript' }, created_at: 1003 }],
    total: 1, has_more: false,
  }));
  const workspace = spyOn(ipcBridge.conversation.getWorkspace, 'invoke').mockResolvedValue([]);
  const inventory = spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockImplementation(async ({ root }) => [
    { name: 'photo.png', fullPath: `${root}/assets/photo.png`, relativePath: 'assets/photo.png' },
  ]);
  const zip = spyOn(ipcBridge.fs.createZip, 'invoke').mockResolvedValue(true);
  const cancel = spyOn(ipcBridge.fs.cancelZip, 'invoke').mockResolvedValue(true);
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke');
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  const warning = spyOn(Message, 'warning').mockImplementation(() => () => {});
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  const log = spyOn(console, 'error').mockImplementation(() => {});
  for (const spy of [desktop, history, workspace, inventory, zip, cancel, metadata, success, warning, error, log]) restores.push(() => spy.mockRestore());
  const batchChanged = mock();
  const hook = renderHook(() => {
    const [selectedConversationIds, setSelectedConversationIds] = useState(new Set<ConversationId>([a, b]));
    return { selectedConversationIds, ...useExport({ conversations, selectedConversationIds, setSelectedConversationIds, onBatchModeChange: batchChanged }) };
  }, { wrapper });
  const open = async (index = 0) => act(async () => { hook.result.current.handleExportConversation(conversations[index]); });
  const choose = () => act(() => hook.result.current.handleSelectExportDirectoryFromModal(['D:\\workplace\\data\\dev-temp']));
  const submit = async () => act(async () => { await hook.result.current.handleConfirmExport(); });
  return { hook, desktop, history, workspace, inventory, zip, cancel, metadata, success, warning, error, batchChanged, open, choose, submit };
}

test('exports to a changed destination with typed file sources and all recursive workspace entries', async () => {
  const f = fixture();
  await f.open();
  f.choose();
  await f.submit();
  expect(f.zip).toHaveBeenCalledTimes(1);
  const request = f.zip.mock.calls[0][0];
  expect(request.path).toMatch(/^D:\\workplace\\data\\dev-temp\\topic-0-\d{8}-\d{6}-[a-f0-9]{8}\.zip$/);
  expect(request.workspaces).toEqual(['D:/project-0']);
  expect(request.files.find((file) => file.name.endsWith('/workspace/assets/photo.png'))).toEqual({
    name: `topic-0__${a}/workspace/assets/photo.png`, file_path: 'D:/project-0/assets/photo.png',
  });
  const json = request.files.find((file) => file.name.endsWith('conversation.json'))!;
  expect(JSON.parse(json.content!).messages[0].content.content).toBe('full transcript');
  expect(f.inventory).toHaveBeenCalledWith({ root: 'D:/project-0', workspace: 'D:/project-0' });
  expect(f.metadata).not.toHaveBeenCalled();
  expect(f.success).toHaveBeenCalledTimes(1);
  expect(f.hook.result.current.exportModalVisible).toBe(false);
});

test('exports in WebUI contexts without the secure-context randomUUID API', async () => {
  const randomUUID = spyOn(globalThis.crypto, 'randomUUID').mockImplementation(() => {
    throw new Error('randomUUID is unavailable in an HTTP WebUI context');
  });
  restores.push(() => randomUUID.mockRestore());
  const f = fixture();
  await f.open(); f.choose(); await f.submit();
  expect(randomUUID).not.toHaveBeenCalled();
  expect(f.zip).toHaveBeenCalledTimes(1);
  expect(f.success).toHaveBeenCalledTimes(1);
});

test('batch export keeps each topic and explicit workspace source scope', async () => {
  const f = fixture();
  await act(async () => { f.hook.result.current.handleBatchExport(); });
  f.choose();
  await f.submit();
  const request = f.zip.mock.calls[0][0];
  expect(request.workspaces).toEqual(['D:/project-0', 'D:/project-1']);
  expect(request.files).toHaveLength(6);
  expect(f.hook.result.current.selectedConversationIds.size).toBe(0);
  expect(f.batchChanged).toHaveBeenCalledWith(false);
});

test('history and recursive inventory failures never produce incomplete successful archives', async () => {
  const f = fixture();
  f.history.mockRejectedValueOnce(new Error('canonical payload is unreadable'));
  await f.open(); f.choose(); await f.submit();
  expect(f.error.mock.calls[0][0]).toContain('canonical payload is unreadable');
  expect(f.zip).not.toHaveBeenCalled();
  expect(f.success).not.toHaveBeenCalled();
  expect(f.hook.result.current.exportModalVisible).toBe(true);
  expect(f.hook.result.current.exportModalLoading).toBe(false);

  f.inventory.mockRejectedValueOnce(new Error('workspace inventory is incomplete'));
  await f.submit();
  expect(f.error.mock.calls[1][0]).toContain('workspace inventory is incomplete');
  expect(f.zip).not.toHaveBeenCalled();
  expect(f.success).not.toHaveBeenCalled();
});

test('duplicate confirmation and closing/reopening cannot let an old export complete the new modal', async () => {
  const f = fixture();
  const oldZip = deferred<boolean>();
  const newZip = deferred<boolean>();
  f.zip.mockImplementationOnce(() => oldZip.promise).mockImplementationOnce(() => newZip.promise);
  await f.open(); f.choose();
  let oldCompletion!: Promise<void>;
  act(() => {
    oldCompletion = f.hook.result.current.handleConfirmExport();
    void f.hook.result.current.handleConfirmExport();
  });
  await waitFor(() => expect(f.zip).toHaveBeenCalledTimes(1));
  act(() => f.hook.result.current.closeExportModal());
  await f.open(1); f.choose();
  let newCompletion!: Promise<void>;
  act(() => { newCompletion = f.hook.result.current.handleConfirmExport(); });
  await waitFor(() => expect(f.zip).toHaveBeenCalledTimes(2));
  await act(async () => { oldZip.resolve(true); await oldCompletion; });
  expect(f.cancel).toHaveBeenCalledWith({ request_id: f.zip.mock.calls[0][0].request_id! });
  expect(f.success).not.toHaveBeenCalled();
  expect(f.hook.result.current.exportModalLoading).toBe(true);
  expect(f.hook.result.current.exportTask?.mode === 'single' && f.hook.result.current.exportTask.conversation.id).toBe(b);
  await act(async () => { newZip.resolve(true); await newCompletion; });
  expect(f.success).toHaveBeenCalledTimes(1);
  expect(f.hook.result.current.exportModalLoading).toBe(false);
});

test('late default-directory lookup does not overwrite the user selected destination', async () => {
  const f = fixture();
  const desktop = deferred<string>();
  f.desktop.mockImplementationOnce(() => desktop.promise);
  act(() => f.hook.result.current.handleExportConversation(conversations[0]));
  f.choose();
  await act(async () => { desktop.resolve('C:/Users/test/Desktop'); });
  expect(f.hook.result.current.exportTargetPath).toBe('D:\\workplace\\data\\dev-temp');
});
