import { act, cleanup, render, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { useLayoutEffect, type PropsWithChildren } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { mcpService, type IResponseMessage, type ISendMessageResult } from '@/common/adapter/ipcBridge';
import type { TChatConversation, TProviderWithModel } from '@/common/config/storage';
import { configService } from '@/common/config/configService';
import { ThemeProvider } from '@/renderer/hooks/context/ThemeContext';
import { conversationTarget, parseConversationId, parseMessageId, parseProviderId } from '@/common/types/ids';
import type { AgentSessionCapabilitySelectionState } from '@/common/types/agentPlatform';
import { sessionStorageKey, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { PreviewProvider } from '../../Preview';
import { InitialMessageProvider, useInitialMessage } from '../../components/ConversationShell/InitialMessageContext';
import { persistInitialMessageDelivery, releaseInitialMessageDelivery } from '../initialMessageDelivery';
import conversationLocale from '@/renderer/services/i18n/locales/en-US/conversation.json';
import messagesLocale from '@/renderer/services/i18n/locales/en-US/messages.json';
import NomiChat from './NomiChat';
import type { NomiModelSelection } from './useNomiModelSelection';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000001401');
const otherId = parseConversationId('0190f5fe-7c00-7a00-8000-000000001402');
const messageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000001403');
setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000001400');
const storageKey = sessionStorageKey('initial-message-nomi', conversationTarget(conversationId));
const input = 'Explain the first request without changing pages';
const model = { id: parseProviderId('0190f5fe-7c00-7a00-8000-000000001404'),
  use_model: 'test-model', name: 'Test provider', platform: 'openai' } as TProviderWithModel;
const selection: NomiModelSelection = { current_model: model, providers: [], pickerDisabled: true,
  getAvailableModels: () => [], handleSelectModel: async () => {}, getDisplayModelName: name => name ?? '' };
const capabilitySelection: AgentSessionCapabilitySelectionState = {
  selection: { skill_names: [], mcp_server_ids: [] }, binding_version: 1, editable: true,
};
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: {
  'en-US': { translation: { conversation: conversationLocale, messages: messagesLocale } },
} });

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

beforeEach(() => {
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000001400');
  sessionStorage.clear();
  spyOn(configService, 'get').mockReturnValue(undefined);
  spyOn(configService, 'whenReady').mockResolvedValue(undefined);
  spyOn(configService, 'subscribe').mockImplementation(() => () => {});
  for (const name of ['turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'messageAnnotated', 'reconnected'] as const) {
    spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
  }
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.agentPlatform.sessions.onAgentChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.agentPlatform.sessions.onCapabilitiesChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]);
  spyOn(mcpService.listServers, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.workspaceOfficeWatch.start, 'invoke').mockResolvedValue(undefined);
  spyOn(ipcBridge.workspaceOfficeWatch.stop, 'invoke').mockResolvedValue(undefined);
  spyOn(ipcBridge.workspaceOfficeWatch.fileAdded, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.database.getUserConversations, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ id: conversationId, name: input,
    type: 'nomi', status: 'finished', extra: { workspace: '' },
    runtime: { state: 'idle', is_processing: false, can_send_message: true },
  } as TChatConversation);
  spyOn(ipcBridge.conversation.taskPlan, 'invoke').mockResolvedValue({
    conversation_id: conversationId, sequence: 0, turn_id: null, plan: null, turn_status: null,
  });
});
afterEach(() => {
  cleanup();
  mock.restore();
  releaseInitialMessageDelivery(storageKey);
  sessionStorage.clear();
});

function mountConversation(strict = false) {
  const delivery = persistInitialMessageDelivery(sessionStorage, storageKey, conversationId, input, []);
  let context!: ReturnType<typeof useInitialMessage>;
  function Start() {
    context = useInitialMessage();
    useLayoutEffect(() => { context.begin({ ...delivery, submittedAt: Date.now() }); }, []);
    return <NomiChat conversation_id={conversationId} workspace='' modelSelection={selection} creationEnabled={false} />;
  }
  const cache = new Map();
  const page = render(<SWRConfig value={strict ? { dedupingInterval: 0 } : { provider: () => cache, dedupingInterval: 0 }}>
    <MemoryRouter><I18nextProvider i18n={i18n}><ThemeProvider>
      <PreviewProvider persistNamespace='initial-message-test' subscribeGlobalOpen={false}>
        <InitialMessageProvider><Start /></InitialMessageProvider>
      </PreviewProvider>
    </ThemeProvider></I18nextProvider></MemoryRouter>
  </SWRConfig>, { reactStrictMode: strict });
  return { ...page, delivery, pending: () => context.pending };
}

test('slow preparation and acceptance keep the real transcript mounted without a skeleton or a duplicate first message', async () => {
  const capabilities = deferred<AgentSessionCapabilitySelectionState>();
  spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockReturnValue(capabilities.promise);
  const accepted = deferred<ISendMessageResult>();
  const send = spyOn(ipcBridge.conversation.sendMessage, 'invoke').mockReturnValue(accepted.promise);
  const warmup = spyOn(ipcBridge.conversation.warmup, 'invoke').mockResolvedValue(undefined);
  const streams = new Set<(event: IResponseMessage) => void>();
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation(listener => {
    streams.add(listener); return () => { streams.delete(listener); };
  });
  const page = mountConversation();
  const scroller = page.getByTestId('message-list-scroller');
  expect(page.getAllByTestId('message-text-content')).toHaveLength(1);
  expect(page.getByTestId('message-text-content').textContent).toBe(input);
  expect(page.getByTestId('initial-message-preparing')).toBeTruthy();
  expect(page.queryByTestId('message-list-skeleton')).toBeNull();
  expect(page.container.textContent).not.toContain('Worked for');
  expect(send).not.toHaveBeenCalled();
  // Exceed the removed 280ms timer while actual preparation is still unresolved.
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 320)); });
  expect(page.getByTestId('message-list-scroller')).toBe(scroller);
  expect(page.getByTestId('initial-message-preparing')).toBeTruthy();
  await act(async () => { capabilities.resolve(capabilitySelection); });
  await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
  expect(send.mock.calls[0][0]).toMatchObject({ conversation_id: conversationId, input,
    idempotency_key: page.delivery.idempotency_key, initial_only: true });
  expect(page.getByTestId('initial-message-preparing')).toBeTruthy();
  expect(page.queryByTestId('message-list-skeleton')).toBeNull();
  expect(warmup).not.toHaveBeenCalled();
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ id: conversationId, name: input,
    type: 'nomi', status: 'running', extra: { workspace: '' }, runtime: {
      state: 'running', is_processing: true, can_send_message: false, active_turn_id: messageId,
    },
  } as TChatConversation);
  await act(async () => { accepted.resolve({ msg_id: messageId, replayed: false, completed: false,
    result_ok: null, result_text: null, result_error: null, result_error_code: null, result_error_retryable: null }); });
  await waitFor(() => expect(page.pending()).toBeNull());
  expect(page.getByTestId('message-list-scroller')).toBe(scroller);
  expect(page.getAllByTestId('message-text-content')).toHaveLength(1);
  expect(page.queryByTestId('initial-message-preparing')).toBeNull();
  expect(sessionStorage.getItem(storageKey)).toBeNull();
  act(() => streams.forEach(listener => listener({ conversation_id: conversationId, msg_id: messageId,
    turn_id: messageId, type: 'thinking', created_at: Date.now(), data: { content: 'Inspecting the request', status: 'thinking' } })));
  await waitFor(() => expect(page.container.textContent).toContain(conversationLocale.thinking.label));
  expect(page.getByTestId('message-list-scroller')).toBe(scroller);
  expect(page.getAllByTestId('message-text-content')).toHaveLength(1);
  expect(page.queryByTestId('message-list-skeleton')).toBeNull();
  expect(send).toHaveBeenCalledTimes(1);
});

test('a rejected first send clears preparation and leaves the guarded delivery available for recovery', async () => {
  spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue(capabilitySelection);
  spyOn(ipcBridge.conversation.sendMessage, 'invoke').mockRejectedValue(new Error('Connection interrupted'));
  spyOn(console, 'error').mockImplementation(() => {});
  const page = mountConversation();
  await waitFor(() => expect(page.pending()).toBeNull());
  expect(page.queryByTestId('initial-message-preparing')).toBeNull();
  expect(page.container.querySelector('.message-error-note')).not.toBeNull();
  expect(JSON.parse(sessionStorage.getItem(storageKey)!)).toMatchObject({ idempotency_key: page.delivery.idempotency_key, input });
});

test.each([false, true])('a user-created event before the send receipt cannot leave an empty progress row (strict=%p)', async (strict) => {
  spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue(capabilitySelection);
  const accepted = deferred<ISendMessageResult>();
  const send = spyOn(ipcBridge.conversation.sendMessage, 'invoke').mockReturnValue(accepted.promise);
  let userCreated!: Parameters<typeof ipcBridge.conversation.userCreated.on>[0];
  spyOn(ipcBridge.conversation.userCreated, 'on').mockImplementation(listener => { userCreated = listener; return () => {}; });
  const streams = new Set<(event: IResponseMessage) => void>();
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation(listener => { streams.add(listener); return () => { streams.delete(listener); }; });
  const page = mountConversation(strict);
  const header = page.container.querySelector('.turn-process-disclosure__header');
  const scroller = page.getByTestId('message-list-scroller');
  await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
  act(() => userCreated({ conversation_id: conversationId, msg_id: messageId, content: input,
    position: 'right', status: 'finish', hidden: false, created_at: Date.now() }));
  await waitFor(() => expect(page.container.querySelector(`[data-message-business-id="${messageId}"]`)).not.toBeNull());
  expect(page.getByTestId('initial-message-preparing')).toBeTruthy();
  expect(page.container.querySelector('.turn-process-disclosure__header') === header).toBe(true);
  expect(page.container.textContent).not.toContain('Worked for');
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 320)); });
  expect(page.getByTestId('initial-message-preparing')).toBeTruthy();
  expect(page.getByTestId('message-list-scroller')).toBe(scroller);
  const runtime = deferred<TChatConversation>();
  spyOn(ipcBridge.conversation.get, 'invoke').mockReturnValue(runtime.promise);
  await act(async () => accepted.resolve({ msg_id: messageId, replayed: false, completed: false,
    result_ok: null, result_text: null, result_error: null, result_error_code: null, result_error_retryable: null }));
  await waitFor(() => expect(page.container.textContent).toContain('Worked for'));
  expect(page.container.querySelector('.turn-process-disclosure__header') === header).toBe(true);
  expect(page.getAllByTestId('message-text-content')).toHaveLength(1);
  const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000001405');
  await act(async () => runtime.resolve({ id: conversationId, name: input, type: 'nomi', status: 'running',
    extra: { workspace: '' }, runtime: { state: 'running', is_processing: true, can_send_message: false,
      active_turn_id: turnId, processing_started_at: Date.now() },
  } as TChatConversation));
  await waitFor(() => expect(page.container.querySelector(`#message-turn-disclosure-${turnId}`)).not.toBeNull());
  expect(page.container.querySelector('.turn-process-disclosure__header') === header).toBe(true);
  act(() => streams.forEach(listener => listener({ conversation_id: conversationId, msg_id: messageId,
    turn_id: turnId, type: 'thinking', created_at: Date.now(), data: { content: 'Inspecting the request', status: 'thinking' } })));
  await waitFor(() => expect(page.container.textContent).toContain(conversationLocale.thinking.label));
  expect(page.container.querySelector('.turn-process-disclosure__header') === header).toBe(true);
  expect(page.getByTestId('message-list-scroller')).toBe(scroller);
  expect(page.getAllByTestId('message-text-content')).toHaveLength(1);
  expect(send).toHaveBeenCalledTimes(1);
});

test('a late completion cannot clear another conversation or a replacement delivery', () => {
  const wrapper = ({ children }: PropsWithChildren) => <InitialMessageProvider>{children}</InitialMessageProvider>;
  const hook = renderHook(useInitialMessage, { wrapper });
  const delivery = persistInitialMessageDelivery(sessionStorage, storageKey, conversationId, input, []);
  const newer = { ...delivery, conversation_id: otherId, idempotency_key: 'newer-delivery', submittedAt: 2 };
  act(() => hook.result.current.begin({ ...delivery, submittedAt: 1 }));
  act(() => hook.result.current.begin(newer));
  act(() => hook.result.current.end(conversationId, delivery.idempotency_key));
  act(() => hook.result.current.end(otherId, delivery.idempotency_key));
  expect(hook.result.current.pending).toEqual(newer);
  act(() => hook.result.current.end(otherId, newer.idempotency_key));
  expect(hook.result.current.pending).toBeNull();
});
