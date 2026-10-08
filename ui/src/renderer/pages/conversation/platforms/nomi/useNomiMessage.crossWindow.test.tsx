/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { createElement, type PropsWithChildren } from 'react';
import { ipcBridge } from '@/common';
import type { IConversationTurnStartedEvent, IResponseMessage, IUserMessageCreatedEvent } from '@/common/adapter/ipcBridge';
import type { TChatConversation } from '@/common/config/storage';
import type { IMessageText } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId, type ConversationId } from '@/common/types/ids';
import { MessageListProvider, useAddOrUpdateMessage, useMessageList } from '../../Messages/hooks';
import { useNomiMessage } from './useNomiMessage';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000991');
const otherConversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000992');
const userMessageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000993');
const assistantMessageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000994');
const turnId = userMessageId;
const hiddenMessageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000996');
const otherMessageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000997');
const wrapper = ({ children }: PropsWithChildren) => createElement(MessageListProvider, { initialValue: [] }, children);

afterEach(() => { cleanup(); mock.restore(); });

function mockConversationBroadcasts() {
  const users = new Set<(event: IUserMessageCreatedEvent) => void>();
  const streams = new Set<(event: IResponseMessage) => void>();
  const starts = new Set<(event: IConversationTurnStartedEvent) => void>();
  spyOn(ipcBridge.conversation.userCreated, 'on').mockImplementation(listener => {
    users.add(listener);
    return () => { users.delete(listener); };
  });
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation(listener => {
    streams.add(listener);
    return () => { streams.delete(listener); };
  });
  spyOn(ipcBridge.conversation.turnStarted, 'on').mockImplementation(listener => {
    starts.add(listener);
    return () => { starts.delete(listener); };
  });
  for (const name of ['turnCompleted', 'turnPaused', 'messageAnnotated', 'reconnected'] as const) {
    spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
  }

  const running = new Set<ConversationId>();
  spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async ({ conversation_id }) => ({
    id: conversation_id,
    type: 'nomi',
    status: running.has(conversation_id) ? 'running' : 'finished',
    runtime: running.has(conversation_id) ? {
      state: 'running', is_processing: true, can_send_message: false, has_runtime: true,
      active_turn_id: turnId, processing_started_at: 1000,
    } : {
      state: 'idle', is_processing: false, can_send_message: true, has_runtime: false,
    },
  } as TChatConversation));

  return {
    user: (event: IUserMessageCreatedEvent) => { for (const listener of users) listener(event); },
    stream: (event: IResponseMessage) => { for (const listener of streams) listener(event); },
    start: (id: ConversationId) => {
      running.add(id);
      for (const listener of starts) listener({
        conversation_id: id, turn_id: turnId, status: 'running', state: 'ai_generating', detail: '',
        can_send_message: false,
        runtime: { state: 'running', is_processing: true, can_send_message: false, has_runtime: true,
          active_turn_id: turnId, processing_started_at: 1000 },
      });
    },
  };
}

const acceptedUser = (overrides: Partial<IUserMessageCreatedEvent> = {}): IUserMessageCreatedEvent => ({
  conversation_id: conversationId, msg_id: userMessageId, content: '从桌面伙伴发送的请求',
  position: 'right', status: 'finish', created_at: 1000, origin: null,
  ...overrides,
});

const assistantChunk = (content: string): IResponseMessage => ({
  conversation_id: conversationId, msg_id: assistantMessageId, turn_id: turnId,
  type: 'content', data: content, created_at: 1001,
});

const useConversationMessages = (id: ConversationId) => ({
  activity: useNomiMessage(id), messages: useMessageList(), addMessage: useAddOrUpdateMessage(),
});

test('another window accepted input is visible before model output or Turn completion without a local POST or history reload', async () => {
  const broadcasts = mockConversationBroadcasts();
  const send = spyOn(ipcBridge.conversation.sendMessage, 'invoke').mockRejectedValue(new Error('unexpected local POST'));
  const history = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockRejectedValue(new Error('unexpected history reload'));
  const getMessage = spyOn(ipcBridge.database.getConversationMessage, 'invoke').mockRejectedValue(new Error('unexpected message fetch'));
  const hook = renderHook(() => useConversationMessages(conversationId), { wrapper });
  await waitFor(() => expect(hook.result.current.activity.hasHydratedRunningState).toBe(true));

  act(() => broadcasts.user(acceptedUser()));
  await waitFor(() => expect(hook.result.current.messages).toHaveLength(1));
  expect(hook.result.current.messages[0]).toMatchObject({
    msg_id: userMessageId, conversation_id: conversationId, position: 'right',
    created_at: 1000, content: { content: '从桌面伙伴发送的请求' },
  });
  expect(hook.result.current.activity.getTurnCompletionGeneration()).toBe(0);

  act(() => broadcasts.start(conversationId));
  await waitFor(() => expect(hook.result.current.activity.activeTurnId).toBe(turnId));
  act(() => broadcasts.stream(assistantChunk('先解释')));
  await waitFor(() => expect(hook.result.current.messages).toHaveLength(2));
  act(() => broadcasts.stream(assistantChunk('第一步。')));
  await waitFor(() => expect(hook.result.current.messages[1]).toMatchObject({
    msg_id: assistantMessageId, turn_id: turnId, position: 'left', content: { content: '先解释第一步。' },
  }));
  expect(hook.result.current.messages[0]).toMatchObject({ position: 'right', content: { content: '从桌面伙伴发送的请求' } });
  expect(hook.result.current.activity.running).toBe(true);
  expect(hook.result.current.activity.getTurnCompletionGeneration()).toBe(0);
  expect(send).not.toHaveBeenCalled();
  expect(history).not.toHaveBeenCalled();
  expect(getMessage).not.toHaveBeenCalled();
});

test.each(['notification first', 'HTTP echo first'] as const)('%s: user notifications and local HTTP echoes reconcile once even after assistant output', async (order) => {
  const broadcasts = mockConversationBroadcasts();
  const hook = renderHook(() => useConversationMessages(conversationId), { wrapper });
  await waitFor(() => expect(hook.result.current.activity.hasHydratedRunningState).toBe(true));
  // This is the complete user row NomiSendBox adds after its HTTP receipt.
  const localEcho: IMessageText = { id: 'local-http-echo', msg_id: userMessageId,
    type: 'text', position: 'right', conversation_id: conversationId, created_at: 1002,
    content: { content: '从桌面伙伴发送的请求' } };
  const notify = () => broadcasts.user(acceptedUser());
  const echo = () => hook.result.current.addMessage({ ...localEcho });
  act(order === 'notification first' ? notify : echo);
  await waitFor(() => expect(hook.result.current.messages).toHaveLength(1));
  const originalRowId = hook.result.current.messages[0].id;

  act(() => broadcasts.stream(assistantChunk('模型正在回复。')));
  await waitFor(() => expect(hook.result.current.messages).toHaveLength(2));
  act(() => {
    (order === 'notification first' ? echo : notify)();
    notify();
    echo();
  });
  // Wait for the batched message updates to drain through the actual Provider.
  act(() => broadcasts.stream(assistantChunk('继续。')));
  await waitFor(() => expect(hook.result.current.messages[1]).toMatchObject({
    position: 'left', content: { content: '模型正在回复。继续。' },
  }));
  expect(hook.result.current.messages).toHaveLength(2);
  expect(hook.result.current.messages.filter(row => row.msg_id === userMessageId)).toHaveLength(1);
  expect(hook.result.current.messages[0]).toMatchObject({
    id: originalRowId, msg_id: userMessageId, position: 'right', content: { content: '从桌面伙伴发送的请求' },
  });
});

test('shared cross-window broadcasts keep each Session list isolated and never display hidden accepted input', async () => {
  const broadcasts = mockConversationBroadcasts();
  const first = renderHook(() => useConversationMessages(conversationId), { wrapper });
  const second = renderHook(() => useConversationMessages(otherConversationId), { wrapper });
  await waitFor(() => expect(first.result.current.activity.hasHydratedRunningState).toBe(true));
  await waitFor(() => expect(second.result.current.activity.hasHydratedRunningState).toBe(true));

  act(() => {
    broadcasts.user(acceptedUser({ msg_id: hiddenMessageId, content: '内部输入', hidden: true }));
    broadcasts.user(acceptedUser());
    broadcasts.user(acceptedUser({ conversation_id: otherConversationId, msg_id: otherMessageId, content: '另一个伙伴的请求' }));
    broadcasts.stream(assistantChunk('当前伙伴的回复。'));
  });
  await waitFor(() => expect(first.result.current.messages).toHaveLength(2));
  await waitFor(() => expect(second.result.current.messages).toHaveLength(1));
  expect(first.result.current.messages.map(row => row.msg_id)).toEqual([userMessageId, assistantMessageId]);
  expect(first.result.current.messages.every(row => row.conversation_id === conversationId)).toBe(true);
  expect(second.result.current.messages[0]).toMatchObject({
    msg_id: otherMessageId, conversation_id: otherConversationId, position: 'right', content: { content: '另一个伙伴的请求' },
  });
  expect([...first.result.current.messages, ...second.result.current.messages].some(row => row.msg_id === hiddenMessageId)).toBe(false);
});
