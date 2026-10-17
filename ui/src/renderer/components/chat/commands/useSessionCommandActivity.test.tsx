import '../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { SWRConfig } from 'swr';
import type { ReactNode } from 'react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { useSessionCommandActivity, type SessionCommandActivity } from './useSessionCommandActivity';

const ID = parseConversationId('0190f5fe-7c00-7a00-8000-000000000321');
const OTHER = parseConversationId('0190f5fe-7c00-7a00-8000-000000000322');
const idle = { id: ID, type: 'nomi', status: 'finished' } as TChatConversation;
const paused = { ...idle, status: 'running', extra: { workspace: '' }, runtime: {
  state: 'paused', is_processing: false, can_send_message: false,
  active_turn_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000323'),
} } as TChatConversation;
beforeEach(() => {
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(idle);
  for (const event of [ipcBridge.conversation.listChanged, ipcBridge.conversation.turnStarted,
    ipcBridge.conversation.turnCompleted, ipcBridge.conversation.turnPaused, ipcBridge.conversation.reconnected]) {
    spyOn(event, 'on').mockImplementation(() => () => {});
  }
});
afterEach(() => { cleanup(); mock.restore(); });
function wrapper(cached?: TChatConversation) {
  const cache = new Map(cached ? [[`conversation/${ID}`, { data: cached }]] : []);
  return ({ children }: { children: ReactNode }) => <SWRConfig value={{ provider: () => cache }}>{children}</SWRConfig>;
}

test.each(['idle', 'busy', 'unknown'] as SessionCommandActivity[])('supplied %s authority avoids a second query or lifecycle subscription', async activity => {
  const read = spyOn(ipcBridge.conversation.get, 'invoke');
  const subscription = spyOn(ipcBridge.conversation.listChanged, 'on');
  const hook = renderHook(() => useSessionCommandActivity(ID, activity), { wrapper: wrapper() });
  await act(async () => {});
  expect(hook.result.current).toBe(activity);
  expect(read).not.toHaveBeenCalled();
  expect(subscription).not.toHaveBeenCalled();
});

test('cached idle does not replace the fresh authority check when a host starts observing', async () => {
  let resolve!: (snapshot: TChatConversation) => void;
  spyOn(ipcBridge.conversation.get, 'invoke').mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  const hook = renderHook(() => useSessionCommandActivity(ID), { wrapper: wrapper(idle) });
  expect(hook.result.current).toBe('unknown');
  await waitFor(() => expect(typeof resolve).toBe('function'));
  await act(async () => { resolve(paused); });
  await waitFor(() => expect(hook.result.current).toBe('busy'));
});

test('matching lifecycle events refresh canonical authority and read errors fail closed', async () => {
  const read = spyOn(ipcBridge.conversation.get, 'invoke');
  const hook = renderHook(() => useSessionCommandActivity(ID), { wrapper: wrapper() });
  await waitFor(() => expect(hook.result.current).toBe('idle'));
  const refresh = spyOn(ipcBridge.conversation.listChanged, 'on').mock.calls[0]![0];
  act(() => { refresh({ conversation_id: OTHER, action: 'updated' }); });
  expect(read).toHaveBeenCalledTimes(1);
  read.mockResolvedValue(paused);
  act(() => { refresh({ conversation_id: ID, action: 'updated' }); });
  await waitFor(() => expect(hook.result.current).toBe('busy'));
  read.mockRejectedValue(new Error('Canonical state unavailable'));
  act(() => { refresh({ conversation_id: ID, action: 'updated' }); });
  await waitFor(() => expect(read).toHaveBeenCalledTimes(3));
  expect(hook.result.current).toBe('unknown');
});
