import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { Message, Modal } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { useState, type PropsWithChildren } from 'react';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter, useLocation, useNavigate } from 'react-router-dom';
import * as Router from 'react-router-dom';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId, parseTerminalId, type ConversationId, type TerminalId } from '@/common/types/ids';
import conversationLocale from '@/renderer/services/i18n/locales/en-US/conversation.json';
import { emitter } from '@/renderer/utils/emitter';
import { parseSessionRoute } from '@/renderer/utils/routes/sessionRoute';
import { useBatchSelection } from './useBatchSelection';
import { useTerminalBatchSelection } from './useTerminalBatchSelection';
import { useSessionBatchDelete } from './useSessionBatchDelete';

const [a, b, c] = [1, 2, 3].map((n) => parseConversationId(`0190f5fe-7c00-7a00-8000-${String(n).padStart(12, '0')}`));
const [terminalA, terminalB] = [4, 5].map((n) => parseTerminalId(`0190f5fe-7c00-7a00-8000-${String(n).padStart(12, '0')}`));
const conversations = [a, b, c].map((id) => ({ id }) as TChatConversation);
const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: { conversation: conversationLocale } } } });

const restores: Array<() => void> = [];
afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach((restore) => restore());
});

function fixture(activeId: ConversationId | TerminalId = a, activeKind: 'conversation' | 'terminal' = 'conversation') {
  const requests: Array<{
    kind: 'conversation' | 'terminal'; id: ConversationId | TerminalId;
    resolve: () => void; reject: (error: Error) => void;
  }> = [];
  const removeConversation = spyOn(ipcBridge.conversation.remove, 'invoke').mockImplementation(({ conversation_id }) =>
    new Promise<void>((resolve, reject) => requests.push({ kind: 'conversation', id: conversation_id, resolve, reject }))
  );
  const removeTerminal = spyOn(ipcBridge.terminal.remove, 'invoke').mockImplementation(({ terminal_id }) =>
    new Promise<void>((resolve, reject) => requests.push({ kind: 'terminal', id: terminal_id, resolve, reject }))
  );
  const listChanged = new Set<Parameters<typeof ipcBridge.conversation.listChanged.on>[0]>();
  const listEvents = spyOn(ipcBridge.conversation.listChanged, 'on').mockImplementation((listener) => {
    listChanged.add(listener);
    return () => { listChanged.delete(listener); };
  });
  const terminalRemoved = new Set<Parameters<typeof ipcBridge.terminal.onRemoved.on>[0]>();
  const terminalEvents = spyOn(ipcBridge.terminal.onRemoved, 'on').mockImplementation((listener) => {
    terminalRemoved.add(listener);
    return () => { terminalRemoved.delete(listener); };
  });
  const confirmations: Array<Parameters<typeof Modal.confirm>[0]> = [];
  const confirm = spyOn(Modal, 'confirm').mockImplementation((props) => {
    confirmations.push(props);
    return { close: mock(), update: mock() };
  });
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  const warning = spyOn(Message, 'warning').mockImplementation(() => () => {});
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  const log = spyOn(console, 'error').mockImplementation(() => {});
  const emit = spyOn(emitter, 'emit');
  for (const spy of [removeConversation, removeTerminal, listEvents, terminalEvents, confirm, success, warning, error, log, emit]) {
    restores.push(() => spy.mockRestore());
  }
  const changed = mock();
  const wrapper = ({ children }: PropsWithChildren) => (
    <I18nextProvider i18n={i18n}>
      <MemoryRouter initialEntries={[`/${activeKind}/${activeId}`]}>{children}</MemoryRouter>
    </I18nextProvider>
  );
  const hook = renderHook(({ rows }) => {
    const [batchMode, setBatchMode] = useState(true);
    const selection = useBatchSelection(batchMode, rows);
    const terminalSelection = useTerminalBatchSelection(batchMode);
    const location = useLocation();
    const route = parseSessionRoute(location.pathname);
    const deletion = useSessionBatchDelete({
      ...selection, ...terminalSelection,
      activeConversationId: route?.kind === 'conversation' ? route.id : null,
      activeTerminalId: route?.kind === 'terminal' ? route.id : null,
      onBatchModeChange: (value) => { changed(value); setBatchMode(value); },
    });
    return { ...selection, ...terminalSelection, ...deletion, batchMode, pathname: location.pathname, navigate: useNavigate() };
  }, { wrapper, initialProps: { rows: conversations } });
  const selectAll = (terminals: TerminalId[] = []) => act(() => {
    hook.result.current.handleToggleSelectAll();
    hook.result.current.setSelectedTerminalIds(new Set(terminals));
  });
  const open = () => act(() => { hook.result.current.handleBatchDelete(); });
  const submit = () => {
    let promise: Promise<void> | void;
    act(() => { promise = confirmations.at(-1)!.onOk!(); });
    return promise!;
  };
  const settle = async (index: number, failed = false) => act(async () => {
    if (failed) requests[index].reject(new Error('cleanup blocked'));
    else requests[index].resolve();
  });
  return { hook, requests, confirmations, selectAll, open, submit, settle, success, warning, error, changed, emit,
    deletedEvent: (id: ConversationId) => act(() => {
      listChanged.forEach((listener) => listener({ action: 'deleted', conversation_id: id }));
    }),
    terminalRemoved: (id: TerminalId) => act(() => {
      terminalRemoved.forEach((listener) => listener({ terminal_id: id }));
    }),
  };
}

test('full selection waits for each deletion receipt across both session kinds and prevents duplicate submissions', async () => {
  const f = fixture(terminalA, 'terminal');
  f.selectAll([terminalA, terminalB]);
  f.open();
  f.open();
  expect(f.confirmations).toHaveLength(1);
  expect(f.confirmations[0].content).toBe('Delete 5 selected topics?');
  const completion = f.submit();
  expect(f.submit()).toBe(completion);
  expect(f.hook.result.current.isDeleting).toBe(true);
  expect(f.requests.map((request) => request.id)).toEqual([a]);
  for (let index = 0; index < 5; index++) {
    expect(f.requests).toHaveLength(index + 1);
    expect(f.changed).not.toHaveBeenCalled();
    await f.settle(index);
  }
  await act(async () => { await completion; });
  expect(f.requests.map((request) => [request.kind, request.id])).toEqual([
    ['conversation', a], ['conversation', b], ['conversation', c], ['terminal', terminalA], ['terminal', terminalB],
  ]);
  expect(f.hook.result.current.selectedConversationIds.size).toBe(0);
  expect(f.hook.result.current.selectedTerminalIds.size).toBe(0);
  expect(f.hook.result.current.isDeleting).toBe(false);
  expect(f.hook.result.current.pathname).toBe('/guid');
  expect(f.changed).toHaveBeenCalledWith(false);
  expect(f.success).toHaveBeenCalledWith('5 topics deleted');
  expect(f.error).not.toHaveBeenCalled();
  expect(f.warning).not.toHaveBeenCalled();
  expect(f.emit).toHaveBeenCalledWith('chat.history.refresh');
  expect(f.emit).toHaveBeenCalledWith('terminal.list.refresh');
});

test('partial failure continues the whole snapshot and retains hidden fenced sessions for retry', async () => {
  const f = fixture(b);
  f.selectAll([terminalA, terminalB]);
  f.open();
  const completion = f.submit();
  // A is deleted and B is fenced: the ordinary live-list refresh omits both.
  f.hook.rerender({ rows: [conversations[2]] });
  await f.settle(0);
  await f.settle(1, true);
  await f.settle(2);
  await f.settle(3, true);
  await f.settle(4);
  await act(async () => { await completion; });
  expect(f.hook.result.current.selectedConversationIds).toEqual(new Set([b]));
  expect(f.hook.result.current.selectedTerminalIds).toEqual(new Set([terminalA]));
  expect(f.hook.result.current.batchMode).toBe(true);
  expect(f.hook.result.current.pathname).toBe(`/conversation/${b}`);
  expect(f.success).not.toHaveBeenCalled();
  expect(f.warning).toHaveBeenCalledWith('Deleted 3 sessions; 2 failed. Failed sessions remain selected for retry.');
  expect(f.emit).not.toHaveBeenCalledWith('conversation.deleted', b);
  expect(f.changed).not.toHaveBeenCalled();

  f.open();
  expect(f.confirmations[1].content).toBe('Delete 2 selected topics?');
  const retry = f.submit();
  await f.settle(5);
  await f.settle(6);
  await act(async () => { await retry; });
  expect(f.requests.slice(5).map((request) => request.id)).toEqual([b, terminalA]);
  expect(f.success).toHaveBeenCalledWith('2 topics deleted');
  expect(f.hook.result.current.pathname).toBe('/guid');
  expect(f.hook.result.current.batchMode).toBe(false);
});

test('all failures keep selections and report failure without leaving batch mode', async () => {
  const f = fixture();
  f.selectAll();
  f.open();
  const completion = f.submit();
  for (let index = 0; index < 3; index++) await f.settle(index, true);
  await act(async () => { await completion; });
  expect(f.hook.result.current.selectedConversationIds).toEqual(new Set([a, b, c]));
  expect(f.hook.result.current.batchMode).toBe(true);
  expect(f.hook.result.current.pathname).toBe(`/conversation/${a}`);
  expect(f.error).toHaveBeenCalledWith('Failed to delete 3 sessions. They remain selected for retry.');
  expect(f.changed).not.toHaveBeenCalled();
  expect(f.success).not.toHaveBeenCalled();
});

test('authoritative deletion receipts settle successes even when the HTTP response fails', async () => {
  const f = fixture();
  f.selectAll([terminalA, terminalB]);
  f.open();
  // Another deletion completes while the confirmation is open.
  f.deletedEvent(b);
  const completion = f.submit();
  f.deletedEvent(a);
  await f.settle(0, true);
  expect(f.requests[1].id).toBe(c); // B already has a definitive receipt.
  await f.settle(1);
  f.terminalRemoved(terminalA);
  await f.settle(2, true);
  await f.settle(3);
  await act(async () => { await completion; });
  expect(f.requests.map((request) => request.id)).toEqual([a, c, terminalA, terminalB]);
  expect(f.success).toHaveBeenCalledWith('5 topics deleted');
  expect(f.warning).not.toHaveBeenCalled();
  expect(f.error).not.toHaveBeenCalled();
  expect(f.hook.result.current.selectedConversationIds.size).toBe(0);
  expect(f.hook.result.current.selectedTerminalIds.size).toBe(0);
  expect(f.hook.result.current.batchMode).toBe(false);
  expect(f.hook.result.current.pathname).toBe('/guid');
});

test('cancel permits another confirmation and empty selection never deletes', () => {
  const f = fixture();
  f.open();
  expect(f.confirmations).toHaveLength(0);
  expect(f.warning).toHaveBeenCalledWith('Please select topics first');
  f.selectAll();
  f.open();
  act(() => { f.confirmations[0].onCancel?.(); });
  f.open();
  expect(f.confirmations).toHaveLength(2);
  expect(f.requests).toHaveLength(0);
  expect(f.hook.result.current.selectedConversationIds.size).toBe(3);
});

test('closing the running confirmation and navigating away cannot redirect a later page', async () => {
  const f = fixture();
  f.selectAll();
  f.open();
  const completion = f.submit();
  act(() => {
    f.confirmations[0].onCancel?.();
    void f.hook.result.current.navigate('/settings');
  });
  f.open();
  expect(f.confirmations).toHaveLength(1); // Closing does not submit a second batch.
  for (let index = 0; index < 3; index++) await f.settle(index);
  await act(async () => { await completion; });
  expect(f.hook.result.current.pathname).toBe('/settings');
  expect(f.success).toHaveBeenCalledWith('3 topics deleted');
});

test('unmount completes the confirmed batch and retains receipts without navigating or updating the old sidebar', async () => {
  const navigate = mock(() => {});
  const navigation = spyOn(Router, 'useNavigate').mockReturnValue(navigate);
  restores.push(() => navigation.mockRestore());
  const f = fixture();
  f.selectAll([terminalA]);
  f.open();
  const completion = f.submit();
  f.hook.unmount();
  f.deletedEvent(a);
  await f.settle(0, true);
  await f.settle(1);
  await f.settle(2);
  f.terminalRemoved(terminalA);
  await f.settle(3, true);
  await completion;
  expect(f.requests.map((request) => request.id)).toEqual([a, b, c, terminalA]);
  expect(navigate).not.toHaveBeenCalled();
  expect(f.changed).not.toHaveBeenCalled();
  expect(f.success).toHaveBeenCalledWith('4 topics deleted');
  expect(f.error).not.toHaveBeenCalled();
  expect(f.warning).not.toHaveBeenCalled();
});

test('selection removes only authoritative deletions and tests select-all by membership', () => {
  const f = fixture();
  f.selectAll();
  f.hook.rerender({ rows: [conversations[1], conversations[2]] });
  expect(f.hook.result.current.selectedConversationIds).toEqual(new Set([a, b, c]));
  expect(f.hook.result.current.allSelected).toBe(true);
  f.deletedEvent(b);
  expect(f.hook.result.current.selectedConversationIds).toEqual(new Set([a, c]));
  expect(f.hook.result.current.allSelected).toBe(false);
  act(() => { f.hook.result.current.handleToggleSelectAll(); });
  expect(f.hook.result.current.selectedConversationIds).toEqual(new Set([a, b, c]));
  act(() => { f.hook.result.current.handleToggleSelectAll(); });
  expect(f.hook.result.current.selectedConversationIds.size).toBe(0);
});

test('an empty live list cannot clear fenced retry targets through select-all', () => {
  const f = fixture();
  f.selectAll([terminalA]);
  f.hook.rerender({ rows: [] });
  act(() => { f.hook.result.current.handleToggleSelectAll(); });
  expect(f.hook.result.current.selectedConversationIds).toEqual(new Set([a, b, c]));
  expect(f.hook.result.current.selectedTerminalIds).toEqual(new Set([terminalA]));
  f.terminalRemoved(terminalA);
  expect(f.hook.result.current.selectedTerminalIds.size).toBe(0);
});
