import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, render, renderHook } from '@testing-library/react';
import type { PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import type { IConversationTurnCompletedEvent } from '@/common/adapter/ipcBridge';
import type { TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId, type ConversationId } from '@/common/types/ids';
import { emitter } from '@/renderer/utils/emitter';
import {
  MessageListProvider, MessageListLoadingProvider, useMessageList,
  useMessageListLoading, useMessageLstCache,
} from './hooks';

type HistoryPage = Awaited<ReturnType<typeof ipcBridge.database.getConversationMessages.invoke>>;
type HistoryQuery = Parameters<typeof ipcBridge.database.getConversationMessages.invoke>[0];
const a = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');
const b = parseConversationId('0190f5fe-7c00-7a00-8000-000000000052');

function message(id: number, conversationId: ConversationId = a): TMessage {
  const messageId = parseMessageId('0190f5fe-7c00-7a00-8000-' + String(id).padStart(12, '0'));
  return {
    id: messageId, message_id: messageId, msg_id: messageId,
    conversation_id: conversationId, created_at: id,
    type: 'text', position: 'left', content: { content: 'message ' + id },
  };
}

const wrapper = ({ children }: PropsWithChildren) => (
  <MessageListProvider><MessageListLoadingProvider>{children}</MessageListLoadingProvider></MessageListProvider>
);

afterEach(() => { cleanup(); mock.restore(); });

function mockHistoryTransport() {
  const requests: Array<{
    query: HistoryQuery;
    resolve: (page: HistoryPage) => void;
    reject: (error: Error) => void;
  }> = [];
  spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockImplementation((query) =>
    new Promise<HistoryPage>((resolve, reject) => requests.push({ query, resolve, reject }))
  );
  let reconnect: (() => void) | undefined;
  spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation((listener) => {
    reconnect = () => listener();
    return () => { reconnect = undefined; };
  });
  let completed: ((event: IConversationTurnCompletedEvent) => void) | undefined;
  spyOn(ipcBridge.conversation.turnCompleted, 'on').mockImplementation((listener) => {
    completed = listener;
    return () => { completed = undefined; };
  });
  const reply = async (index: number, items: TMessage[], hasMore = true) => {
    expect(requests[index]).toBeDefined();
    await act(async () => requests[index].resolve({ items, has_more: hasMore, total: items.length }));
  };
  const refresh = () => { act(() => { reconnect?.(); }); };
  const complete = (event: IConversationTurnCompletedEvent) => { act(() => { completed?.(event); }); };
  return { requests, reply, refresh, complete };
}

function mountHistory(strict = false) {
  const transport = mockHistoryTransport();
  const hook = renderHook(({ id }) => ({
    ...useMessageLstCache(id),
    messages: useMessageList(), loading: useMessageListLoading(),
  }), { wrapper, reactStrictMode: strict, initialProps: { id: a } });
  const older = () => { act(() => { void hook.result.current.loadOlder(); }); };
  return { ...transport, hook, older };
}

test('an old page cannot re-enter after A to B to A navigation', async () => {
  const h = mountHistory();
  await h.reply(0, [message(10)]);
  h.older();
  h.hook.rerender({ id: b });
  await h.reply(2, [message(20, b)]);
  h.hook.rerender({ id: a });
  await h.reply(3, [message(30)]);
  await h.reply(1, [message(1)], false);
  expect(h.hook.result.current.messages.map((m) => m.created_at)).toEqual([30]);
  expect(h.hook.result.current.hasMore).toBe(true);
});

test('new conversation pagination does not inherit old loading ownership', async () => {
  const h = mountHistory();
  await h.reply(0, [message(20)]);
  h.older();
  h.hook.rerender({ id: b });
  await h.reply(2, [message(40, b)]);
  expect(h.hook.result.current.loadingOlder).toBe(false);
  h.older();
  expect(h.requests).toHaveLength(4);
  await h.reply(1, [message(10)]);
  expect(h.hook.result.current.loadingOlder).toBe(true);
  h.older();
  expect(h.requests).toHaveLength(4);
  await h.reply(3, [message(30, b)], false);
  expect(h.hook.result.current.messages.map((m) => m.created_at)).toEqual([30, 40]);
  expect(h.hook.result.current.loadingOlder).toBe(false);
});

test.each([false, true])('refresh invalidates an old page without settling the replacement (rejected=%p)', async (rejected) => {
  const h = mountHistory();
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  await h.reply(0, [message(20)]);
  h.older();
  h.refresh();
  await h.reply(2, [message(40)]);
  h.older();
  expect(h.requests).toHaveLength(4);
  if (rejected) await act(async () => h.requests[1].reject(new Error('superseded page failure')));
  else await h.reply(1, [message(10)], false);
  expect(errors).not.toHaveBeenCalled();
  expect(h.hook.result.current.loadingOlder).toBe(true);
  expect(h.hook.result.current.hasMore).toBe(true);
  await h.reply(3, [message(30)], false);
  expect(h.hook.result.current.messages.map((m) => m.created_at)).toEqual([20, 30, 40]);
});

test('pages after a latest-window refresh merge chronologically with already loaded pages', async () => {
  const h = mountHistory();
  await h.reply(0, [message(20)]);
  h.older();
  await h.reply(1, [message(10)], false);
  h.refresh();
  await h.reply(2, [message(40)]);
  h.older();
  expect(h.requests[3].query.cursor).toBe('40:' + message(40).message_id);
  await h.reply(3, [message(20), message(30)], false);
  expect(h.hook.result.current.messages.map((m) => m.created_at)).toEqual([10, 20, 30, 40]);
});

test('a superseded initial request cannot clear the newest request loading flag', async () => {
  const h = mountHistory();
  h.refresh();
  await h.reply(0, [message(10)]);
  expect(h.hook.result.current.messages).toEqual([]);
  expect(h.hook.result.current.loading).toBe(true);
  await h.reply(1, [message(20)], false);
  expect(h.hook.result.current.messages.map((m) => m.created_at)).toEqual([20]);
  expect(h.hook.result.current.loading).toBe(false);
});

test('pagination is single-flight and a failed current page can retry the same cursor', async () => {
  const h = mountHistory();
  spyOn(console, 'error').mockImplementation(() => {});
  await h.reply(0, [message(20)]);
  h.older(); h.older();
  expect(h.requests).toHaveLength(2);
  await act(async () => h.requests[1].reject(new Error('fixture page failure')));
  expect(h.hook.result.current.loadingOlder).toBe(false);
  h.older();
  expect(h.requests).toHaveLength(3);
  expect(h.requests[2].query).toEqual(h.requests[1].query);
  await h.reply(2, [message(10)], false);
  expect(h.hook.result.current.messages.map((m) => m.created_at)).toEqual([10, 20]);
});

test('pagination waits for refresh and can resume from its old cursor if refresh fails', async () => {
  const h = mountHistory();
  spyOn(console, 'error').mockImplementation(() => {});
  await h.reply(0, [message(20)]);
  h.refresh();
  h.older();
  expect(h.requests).toHaveLength(2);
  await act(async () => h.requests[1].reject(new Error('latest page failure')));
  expect(h.hook.result.current.loading).toBe(false);
  expect(h.hook.result.current.hasMore).toBe(true);
  h.older();
  expect(h.requests[2].query.cursor).toBe('20:' + message(20).message_id);
  await h.reply(2, [message(10)], false);
  expect(h.hook.result.current.messages.map((m) => m.created_at)).toEqual([10, 20]);
});

test('settle refresh is scoped to the mounted conversation and unsubscribes on unmount', async () => {
  const h = mountHistory();
  await h.reply(0, [message(20)]);
  act(() => { emitter.emit('conversation.turn.settled', b); });
  expect(h.requests).toHaveLength(1);
  act(() => { emitter.emit('conversation.turn.settled', a); });
  expect(h.requests).toHaveLength(2);
  await h.reply(1, [message(30)]);
  h.hook.unmount();
  act(() => { emitter.emit('conversation.turn.settled', a); });
  expect(h.requests).toHaveLength(2);
});

test('latest-page responses from an earlier conversation cannot replace the new transcript', async () => {
  const h = mountHistory();
  h.hook.rerender({ id: b });
  await h.reply(1, [message(20, b)], false);
  await h.reply(0, [message(10)]);
  expect(h.hook.result.current.messages.map((m) => m.created_at)).toEqual([20]);
  expect(h.hook.result.current.hasMore).toBe(false);
  expect(h.hook.result.current.loading).toBe(false);
});

test('StrictMode effect replay never admits the first abandoned request', async () => {
  const h = mountHistory(true);
  expect(h.requests).toHaveLength(2);
  await h.reply(0, [message(10)]);
  expect(h.hook.result.current.messages).toEqual([]);
  expect(h.hook.result.current.loading).toBe(true);
  await h.reply(1, [message(20)], false);
  expect(h.hook.result.current.messages.map((m) => m.created_at)).toEqual([20]);
  expect(h.hook.result.current.loading).toBe(false);
});

test('unmounting a history reader fences its pending page from a surviving message store', async () => {
  const h = mockHistoryTransport();
  let current: TMessage[] = [];
  let loadOlder: () => Promise<void> = async () => {};
  const Loader = () => { loadOlder = useMessageLstCache(a).loadOlder; return null; };
  const Observer = () => { current = useMessageList(); return null; };
  const View = ({ active }: { active: boolean }) => (
    <MessageListProvider><MessageListLoadingProvider>
      <Observer />{active && <Loader />}
    </MessageListLoadingProvider></MessageListProvider>
  );
  const view = render(<View active />);
  await h.reply(0, [message(20)]);
  act(() => { void loadOlder(); });
  view.rerender(<View active={false} />);
  await h.reply(1, [message(10)]);
  expect(current.map((m) => m.created_at)).toEqual([20]);
  h.refresh();
  expect(h.requests).toHaveLength(2);
});

test('terminal events refresh only this conversation with an explicitly idle runtime', async () => {
  const h = mountHistory();
  await h.reply(0, [message(20)]);
  const event: IConversationTurnCompletedEvent = {
    conversation_id: a, status: 'finished', state: 'ai_waiting_input', detail: '',
    can_send_message: true, workspace: '', model: { platform: '', name: '', use_model: '' },
    last_message: { content: '', created_at: 20 },
    runtime: { state: 'idle', can_send_message: true, has_runtime: true, is_processing: false },
  };
  h.complete({ ...event, conversation_id: b });
  h.complete({ ...event, runtime: { ...event.runtime, is_processing: true } });
  h.complete({ ...event, runtime: { ...event.runtime, active_turn_id: message(20).msg_id } });
  expect(h.requests).toHaveLength(1);
  h.complete(event);
  expect(h.requests).toHaveLength(2);
  await h.reply(1, [message(30)]);
  expect(h.hook.result.current.messages.map((m) => m.created_at)).toEqual([20, 30]);
});
