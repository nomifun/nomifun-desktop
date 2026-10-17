/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';
import { cleanup, renderHook } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import { parseConversationId } from '@/common/types/ids';
import { useCompanionConversationUpdates } from './useCompanionConversationUpdates';

const sources = [ipcBridge.conversation.turnStarted, ipcBridge.conversation.turnPaused,
  ipcBridge.conversation.turnCompleted, ipcBridge.conversation.listChanged,
  ipcBridge.agentPlatform.sessions.onAgentChanged, ipcBridge.conversation.reconnected];
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });
const id = parseConversationId('019f0000-0000-7000-8000-000000000001');
const otherId = parseConversationId('019f0000-0000-7000-8000-000000000002');

function subscriptions() {
  const handlers: Array<((event: any) => void) | undefined> = [];
  const detached = Array.from({ length: sources.length }, () => mock());
  sources.forEach((source, index) => {
    const spy = spyOn(source, 'on').mockImplementation((handler: any) => {
      handlers[index] = handler;
      return detached[index];
    });
    restores.push(() => spy.mockRestore());
  });
  return { handlers, detached };
}

test('canonical lifecycle events refresh only this companion Session and reconnect refreshes its authority', () => {
  const { handlers, detached } = subscriptions();
  const refresh = mock(() => Promise.resolve());
  const view = renderHook(() => useCompanionConversationUpdates(id, refresh));
  for (const handler of handlers.slice(0, 3)) {
    handler!({ conversation_id: otherId });
    expect(refresh).not.toHaveBeenCalled();
  }
  for (const handler of handlers.slice(0, 3)) handler!({ conversation_id: id });
  expect(refresh).toHaveBeenCalledTimes(3);
  handlers[3]!({ conversation_id: id, action: 'updated' });
  handlers[4]!({ agent_session_id: id });
  handlers[5]!(undefined);
  expect(refresh).toHaveBeenCalledTimes(6);
  handlers[3]!({ conversation_id: id, action: 'deleted' });
  expect(refresh).toHaveBeenCalledTimes(7);
  view.unmount();
  detached.forEach((unsubscribe) => expect(unsubscribe).toHaveBeenCalledTimes(1));
});

test('a changed Session releases every old subscription before listening to the next one', () => {
  const { handlers, detached } = subscriptions();
  const refresh = mock(() => Promise.resolve());
  const view = renderHook(({ conversationId }) => useCompanionConversationUpdates(conversationId, refresh),
    { initialProps: { conversationId: id } });
  view.rerender({ conversationId: otherId });
  detached.forEach((unsubscribe) => expect(unsubscribe).toHaveBeenCalledTimes(1));
  handlers[0]!({ conversation_id: id });
  expect(refresh).not.toHaveBeenCalled();
  handlers[0]!({ conversation_id: otherId });
  expect(refresh).toHaveBeenCalledTimes(1);
});
