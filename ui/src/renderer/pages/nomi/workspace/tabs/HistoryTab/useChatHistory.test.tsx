import { afterEach, expect, spyOn, test } from 'bun:test';
import { cleanup, renderHook, waitFor } from '@testing-library/react';
import { ipcBridge } from '@/common';
import { parseCompanionId, parseConversationId } from '@/common/types/ids';
import { useDayMessages, useHistoryDays } from './useChatHistory';

const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000041');
const conversationId = parseConversationId('019b0000-0000-7000-8000-000000000042');
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });

test('selected Session history is read without resolving or changing the desktop input Session', async () => {
  const active = spyOn(ipcBridge.companion.getCompanionSession, 'invoke');
  const ensure = spyOn(ipcBridge.companion.ensureCompanionSession, 'invoke');
  const days = spyOn(ipcBridge.companion.listHistoryDays, 'invoke').mockResolvedValue([{ day: '20261016', message_count: 2, has_digest: true }]);
  for (const spy of [active, ensure, days]) restores.push(() => spy.mockRestore());
  const view = renderHook(() => useHistoryDays(companionId, conversationId));
  await waitFor(() => expect(view.result.current.loading).toBe(false));
  expect(view.result.current.conversationId).toBe(conversationId);
  expect(days).toHaveBeenCalledWith({ companion_id: companionId, conversation_id: conversationId });
  expect(active).not.toHaveBeenCalled();
  expect(ensure).not.toHaveBeenCalled();
});

test('a rejected Session scope never falls back to desktop history', async () => {
  const active = spyOn(ipcBridge.companion.getCompanionSession, 'invoke');
  const days = spyOn(ipcBridge.companion.listHistoryDays, 'invoke').mockRejectedValue(new Error('wrong companion'));
  for (const spy of [active, days]) restores.push(() => spy.mockRestore());
  const view = renderHook(() => useHistoryDays(companionId, conversationId));
  await waitFor(() => expect(view.result.current.failed).toBe(true));
  expect(view.result.current.conversationId).toBeUndefined();
  expect(view.result.current.days).toEqual([]);
  expect(active).not.toHaveBeenCalled();
});

test('messages and day digests use the exact same Session scope', async () => {
  const messages = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  const digests = spyOn(ipcBridge.companion.listDayDigests, 'invoke').mockResolvedValue([]);
  for (const spy of [messages, digests]) restores.push(() => spy.mockRestore());
  const view = renderHook(() => useDayMessages(companionId, conversationId, '20261016'));
  await waitFor(() => expect(view.result.current.loading).toBe(false));
  expect(messages.mock.calls[0][0].conversation_id).toBe(conversationId);
  expect(digests.mock.calls[0][0]).toEqual({ companion_id: companionId, conversation_id: conversationId,
    since: '20261016', until: '20261016', limit: 20 });
});
