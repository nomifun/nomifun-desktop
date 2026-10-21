import '../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import type { ReactNode } from 'react';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import type { SessionInputQueueSnapshot } from '@/common/adapter/ipcBridge';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { useSessionInputQueue } from './useSessionInputQueue';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000981');
const messageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000982');
const snapshot: SessionInputQueueSnapshot = { version: 7, items: [{ message_id: messageId,
  input: { content: 'Canonical pending input', files: ['/work/reference.png'] }, delivery: { kind: 'queue' },
  input_delivery: { state: 'pending', placement: 'queued', event_seq: 7 }, held: true }] };

beforeEach(() => {
  for (const event of [ipcBridge.conversation.userCreated, ipcBridge.conversation.turnStarted,
    ipcBridge.conversation.turnCompleted, ipcBridge.conversation.reconnected]) spyOn(event, 'on').mockImplementation(() => () => {});
});
afterEach(() => { cleanup(); mock.restore(); });
const wrapper = ({ children }: { children: ReactNode }) => <SWRConfig value={{ provider: () => new Map(), shouldRetryOnError: false }}>{children}</SWRConfig>;

test('mount, refresh and remount only read the canonical queue and never send inputs', async () => {
  const read = spyOn(ipcBridge.conversation.inputQueue, 'invoke').mockResolvedValue(snapshot);
  const send = spyOn(ipcBridge.conversation.sendMessage, 'invoke');
  const mutate = spyOn(ipcBridge.conversation.updateInputQueue, 'invoke');
  const first = renderHook(() => useSessionInputQueue(conversationId), { wrapper });
  await waitFor(() => expect(first.result.current.snapshot).toEqual(snapshot));
  act(() => first.result.current.refresh());
  await waitFor(() => expect(read.mock.calls.length).toBeGreaterThan(1));
  first.unmount();
  const second = renderHook(() => useSessionInputQueue(conversationId), { wrapper });
  await waitFor(() => expect(second.result.current.snapshot.items[0].message_id).toBe(messageId));
  expect(send).not.toHaveBeenCalled(); expect(mutate).not.toHaveBeenCalled();
});

test('an unconfirmed mutation keeps the row and retries the original queue version and command key', async () => {
  spyOn(ipcBridge.conversation.inputQueue, 'invoke').mockResolvedValue(snapshot);
  const update = spyOn(ipcBridge.conversation.updateInputQueue, 'invoke')
    .mockRejectedValueOnce(new Error('acknowledgement lost'))
    .mockResolvedValue({ version: 8, items: [] });
  const hook = renderHook(() => useSessionInputQueue(conversationId), { wrapper });
  await waitFor(() => expect(hook.result.current.snapshot).toEqual(snapshot));
  await act(async () => { await expect(hook.result.current.change({ kind: 'remove', message_id: messageId })).rejects.toThrow('lost'); });
  expect(hook.result.current.snapshot.items).toHaveLength(1);
  await act(async () => { await hook.result.current.change({ kind: 'remove', message_id: messageId }); });
  expect(update.mock.calls[0][0]).toEqual(update.mock.calls[1][0]);
  expect(update.mock.calls[0][0]).toMatchObject({ expected_version: 7, action: { kind: 'remove', message_id: messageId } });
});

test('failed pending removal cannot turn into an active Turn stop', async () => {
  spyOn(ipcBridge.conversation.inputQueue, 'invoke').mockResolvedValue(snapshot);
  spyOn(ipcBridge.conversation.updateInputQueue, 'invoke').mockRejectedValue(new Error('already claimed'));
  const stop = spyOn(ipcBridge.conversation.stop, 'invoke');
  const hook = renderHook(() => useSessionInputQueue(conversationId), { wrapper });
  await waitFor(() => expect(hook.result.current.snapshot).toEqual(snapshot));
  await act(async () => { await expect(hook.result.current.change({ kind: 'remove', message_id: messageId })).rejects.toThrow('already claimed'); });
  expect(stop).not.toHaveBeenCalled();
  expect(hook.result.current.snapshot.items[0].input.files).toEqual(['/work/reference.png']);
});

test('a pending queue control in one Session cannot block or overwrite another Session', async () => {
  const other = parseConversationId('0190f5fe-7c00-7a00-8000-000000000983');
  const otherSnapshot = { version: 3, items: [] };
  spyOn(ipcBridge.conversation.inputQueue, 'invoke').mockImplementation(async request => request.conversation_id === conversationId ? snapshot : otherSnapshot);
  let finishFirst!: (value: SessionInputQueueSnapshot) => void;
  const update = spyOn(ipcBridge.conversation.updateInputQueue, 'invoke').mockImplementation(request => request.conversation_id === conversationId
    ? new Promise(resolve => { finishFirst = resolve; }) : Promise.resolve({ version: 4, items: [] }));
  const hook = renderHook(({ id }) => useSessionInputQueue(id), { wrapper, initialProps: { id: conversationId } });
  await waitFor(() => expect(hook.result.current.snapshot.version).toBe(7));
  let first!: Promise<unknown>;
  act(() => { first = hook.result.current.change({ kind: 'remove', message_id: messageId }); });
  expect(hook.result.current.changing).toBe(true);
  hook.rerender({ id: other });
  await waitFor(() => expect(hook.result.current.snapshot.version).toBe(3));
  expect(hook.result.current.changing).toBe(false);
  await act(async () => { await hook.result.current.change({ kind: 'resume' }); });
  expect(update).toHaveBeenCalledTimes(2);
  await act(async () => { finishFirst({ version: 8, items: [] }); await first; });
  expect(hook.result.current.snapshot.version).toBe(4);
  expect(hook.result.current.snapshot.items).toEqual([]);
});

test('an old queue read cannot restore a row after its newer canonical removal', async () => {
  let reads = 0;
  let finishOld!: (value: SessionInputQueueSnapshot) => void;
  spyOn(ipcBridge.conversation.inputQueue, 'invoke').mockImplementation(() => {
    reads++;
    if (reads === 2) return new Promise(resolve => { finishOld = resolve; });
    return Promise.resolve(reads === 1 ? snapshot : { version: 8, items: [] });
  });
  spyOn(ipcBridge.conversation.updateInputQueue, 'invoke').mockResolvedValue({ version: 8, items: [] });
  const hook = renderHook(() => useSessionInputQueue(conversationId), { wrapper });
  await waitFor(() => expect(hook.result.current.snapshot.version).toBe(7));
  act(() => hook.result.current.refresh());
  await waitFor(() => expect(reads).toBe(2));
  await act(async () => { await hook.result.current.change({ kind: 'remove', message_id: messageId }); });
  expect(hook.result.current.snapshot.version).toBe(8);
  await act(async () => finishOld(snapshot));
  expect(hook.result.current.snapshot.version).toBe(8);
  expect(hook.result.current.snapshot.items).toEqual([]);
});
