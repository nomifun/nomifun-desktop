import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import type { PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import { parseCompanionId, parseConversationId } from '@/common/types/ids';
import { useCompanionSessions } from './useCompanionSessions';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { BackendHttpError } from '@/common/adapter/httpBridge';

setBrowserStorageGeneration('019b0000-0000-7000-8000-000000000030');

const companion = parseCompanionId('019b0000-0000-7000-8000-000000000031');
const desktop = parseConversationId('019b0000-0000-7000-8000-000000000032');
const other = parseConversationId('019b0000-0000-7000-8000-000000000033');
const created = parseConversationId('019b0000-0000-7000-8000-000000000034');
const row = (conversation_id = desktop) => ({ conversation_id, companion_id: companion, role: 'companion' as const,
  title: 'Conversation', created_at: 1, updated_at: 1, archived: false });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); window.sessionStorage.clear(); });
const install = () => {
  const list = spyOn(ipcBridge.companion.listCompanionSessions, 'invoke').mockResolvedValue({ items: [row(), row(other)], total: 2, has_more: false });
  const active = spyOn(ipcBridge.companion.getCompanionSession, 'invoke').mockResolvedValue({ conversation_id: desktop });
  const ensure = spyOn(ipcBridge.companion.ensureCompanionSession, 'invoke').mockResolvedValue(row());
  const create = spyOn(ipcBridge.companion.createCompanionSession, 'invoke').mockResolvedValue(row(created));
  const route = spyOn(ipcBridge.companion.continueInCompanionWindow, 'invoke').mockResolvedValue({ conversation_id: other });
  const changed = spyOn(ipcBridge.conversation.listChanged, 'on').mockImplementation(() => () => {});
  const reconnected = spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(() => () => {});
  const config = spyOn(ipcBridge.companion.onConfigUpdated, 'on').mockImplementation(() => () => {});
  const update = spyOn(ipcBridge.conversation.update, 'invoke').mockResolvedValue(true);
  const remove = spyOn(ipcBridge.conversation.remove, 'invoke').mockResolvedValue(undefined);
  const clear = spyOn(ipcBridge.conversation.clearContext, 'invoke').mockResolvedValue(undefined);
  const get = spyOn(ipcBridge.conversation.get, 'invoke').mockRejectedValue(new BackendHttpError({ method: 'GET', path: '/projection',
    status: 404, body: { code: 'NOT_FOUND', error: 'not found' } }));
  for (const spy of [list, active, ensure, create, route, changed, reconnected, config, update, remove, clear, get]) restores.push(() => spy.mockRestore());
  return { list, active, ensure, create, route, update, remove, clear };
};
function mount(session = other, modelConfigured = true) {
  const wrapper = ({ children }: PropsWithChildren) => <SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}>
    <MemoryRouter initialEntries={[`/nomi?companion=${companion}&session=${session}&mode=cohabit`]}>{children}</MemoryRouter>
  </SWRConfig>;
  return renderHook(() => useCompanionSessions(companion, modelConfigured), { wrapper });
}

test('viewing and selecting conversations leaves the desktop input target unchanged', async () => {
  const api = install();
  const view = mount();
  await waitFor(() => expect(view.result.current.items).toHaveLength(2));
  expect(view.result.current.conversationId).toBe(other);
  expect(view.result.current.activeDesktopId).toBe(desktop);
  act(() => view.result.current.select(desktop));
  await waitFor(() => expect(view.result.current.conversationId).toBe(desktop));
  expect(api.route).not.toHaveBeenCalled();
  expect(api.ensure).not.toHaveBeenCalled();
  expect(api.create).not.toHaveBeenCalled();
});

test('only the explicit continue action changes desktop routing', async () => {
  const api = install();
  const view = mount();
  await waitFor(() => expect(view.result.current.items).toHaveLength(2));
  await act(async () => view.result.current.continueInWindow());
  expect(api.route).toHaveBeenCalledWith({ companion_id: companion, conversation_id: other });
  expect(api.create).not.toHaveBeenCalled();
});

test('an uncertain creation retries the same key and never changes desktop routing', async () => {
  const api = install();
  api.create.mockRejectedValueOnce(new Error('response unavailable'));
  const view = mount();
  await waitFor(() => expect(view.result.current.items).toHaveLength(2));
  await act(async () => view.result.current.create());
  expect(view.result.current.actionError).toBe(true);
  await act(async () => view.result.current.create());
  expect(api.create).toHaveBeenCalledTimes(2);
  expect(api.create.mock.calls[0][0].creation_key).toBe(api.create.mock.calls[1][0].creation_key);
  expect(view.result.current.conversationId).toBe(created);
  expect(api.route).not.toHaveBeenCalled();
  expect(view.result.current.activeDesktopId).toBe(desktop);
});

test('a late creation does not navigate over a newer conversation selection', async () => {
  const api = install();
  let resolve!: (value: ReturnType<typeof row>) => void;
  api.create.mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  const view = mount();
  await waitFor(() => expect(view.result.current.items).toHaveLength(2));
  let request!: Promise<void>;
  act(() => { request = view.result.current.create(); });
  act(() => view.result.current.select(desktop));
  await waitFor(() => expect(view.result.current.conversationId).toBe(desktop));
  await act(async () => { resolve(row(created)); await request; });
  expect(view.result.current.conversationId).toBe(desktop);
  expect(api.route).not.toHaveBeenCalled();
});

test('renaming and archiving annotate the exact Session without retargeting inputs or cancelling its work', async () => {
  const api = install();
  const stop = spyOn(ipcBridge.conversation.stop, 'invoke');
  restores.push(() => stop.mockRestore());
  const view = mount();
  await waitFor(() => expect(view.result.current.items).toHaveLength(2));
  await act(async () => { await view.result.current.rename(other, 'New title'); });
  await act(async () => { await view.result.current.setArchived(other, true); });
  expect(api.update.mock.calls.map(call => call[0])).toEqual([
    { conversation_id: other, updates: { name: 'New title' } },
    { conversation_id: other, updates: { archived: true } },
  ]);
  expect(view.result.current.conversationId).toBe(other);
  expect(view.result.current.activeDesktopId).toBe(desktop);
  expect(api.route).not.toHaveBeenCalled();
  expect(stop).not.toHaveBeenCalled();
});

test('deleting the selected receive Session leaves an explicit empty selection and never ensures or redirects a replacement', async () => {
  const api = install();
  api.remove.mockImplementation(async () => {
    api.active.mockResolvedValue({ conversation_id: null });
    api.list.mockResolvedValue({ items: [row(other)], total: 1, has_more: false });
  });
  const view = mount(desktop);
  await waitFor(() => expect(view.result.current.items).toHaveLength(2));
  await act(async () => { await view.result.current.remove(desktop); });
  await waitFor(() => expect(view.result.current.activeDesktopId).toBeNull());
  expect(view.result.current.selectionCleared).toBe(true);
  expect(view.result.current.conversationId).toBeNull();
  expect(api.remove).toHaveBeenCalledWith({ conversation_id: desktop });
  expect(api.ensure).not.toHaveBeenCalled();
  expect(api.create).not.toHaveBeenCalled();
  expect(api.route).not.toHaveBeenCalled();
});

test('a late deletion does not clear a newer explicit Session selection', async () => {
  const api = install();
  let resolve!: () => void;
  api.remove.mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  const view = mount();
  await waitFor(() => expect(view.result.current.items).toHaveLength(2));
  let request!: Promise<void>;
  act(() => { request = view.result.current.remove(other); });
  act(() => view.result.current.select(desktop));
  await waitFor(() => expect(view.result.current.conversationId).toBe(desktop));
  await act(async () => { resolve(); await request; });
  expect(view.result.current.conversationId).toBe(desktop);
  expect(view.result.current.selectionCleared).toBe(false);
  expect(api.route).not.toHaveBeenCalled();
});
