import { act, cleanup, fireEvent, render, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, jest, mock, spyOn, test } from 'bun:test';
import { useLayoutEffect, type PropsWithChildren } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import { mcpService, type IResponseMessage, type IAcceptMessageResult } from '@/common/adapter/ipcBridge';
import type { TChatConversation, TProviderWithModel } from '@/common/config/storage';
import { configService } from '@/common/config/configService';
import { ThemeProvider } from '@/renderer/hooks/context/ThemeContext';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import SendBox from '@/renderer/components/chat/SendBox';
import { conversationTarget, parseEntityId, parseConversationId, parseMessageId, parseProviderId } from '@/common/types/ids';
import type { IMessageTips, TMessage } from '@/common/chat/chatLib';
import type { AgentSessionCapabilitySelectionState } from '@/common/types/agentPlatform';
import { sessionStorageKey, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { PreviewProvider } from '../../Preview';
import { MessageListProvider } from '../../Messages/hooks';
import { InitialMessageProvider, useInitialMessage } from '../../components/ConversationShell/InitialMessageContext';
import { persistInitialMessageDelivery, releaseInitialMessageDelivery } from '../initialMessageDelivery';
import conversationLocale from '@/renderer/services/i18n/locales/en-US/conversation.json';
import messagesLocale from '@/renderer/services/i18n/locales/en-US/messages.json';
import agentSettingsLocale from '@/renderer/services/i18n/locales/en-US/agentSettings.json';
import zhConversationLocale from '@/renderer/services/i18n/locales/zh-CN/conversation.json';
import zhAgentSettingsLocale from '@/renderer/services/i18n/locales/zh-CN/agentSettings.json';
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
const conversation: TChatConversation = { id: conversationId, name: input, type: 'nomi',
  model, extra: { workspace: '' } } as TChatConversation;
const capabilitySelection: AgentSessionCapabilitySelectionState = {
  selection: { skill_names: [], mcp_server_ids: [] }, binding_version: 1, editable: true,
};
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: {
  'en-US': { translation: { conversation: conversationLocale, messages: messagesLocale, agentSettings: agentSettingsLocale } },
  'zh-CN': { translation: { conversation: zhConversationLocale, messages: messagesLocale, agentSettings: zhAgentSettingsLocale } },
} });

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

beforeEach(() => {
  spyOn(ipcBridge.conversation.inputQueue, 'invoke').mockResolvedValue({ version: 0, items: [] });
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
  spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue(capabilitySelection);
  spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]);
  spyOn(mcpService.listServers, 'invoke').mockResolvedValue([]);
  spyOn(appHosting.mcpSources, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.workspaceOfficeWatch.start, 'invoke').mockResolvedValue(undefined);
  spyOn(ipcBridge.workspaceOfficeWatch.stop, 'invoke').mockResolvedValue(undefined);
  spyOn(ipcBridge.workspaceOfficeWatch.fileAdded, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.database.getUserConversations, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ ...conversation, id: conversationId, name: input,
    type: 'nomi', status: 'finished', extra: { workspace: '' },
    runtime: { state: 'idle', is_processing: false, can_send_message: true },
  } as TChatConversation);
  spyOn(ipcBridge.conversation.turnFiles, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.conversation.taskPlan, 'invoke').mockResolvedValue({
    conversation_id: conversationId, sequence: 0, turn_id: null, plan: null, turn_status: null,
  });
});
afterEach(() => {
  cleanup();
  jest.useRealTimers();
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
    return <NomiChat conversation={conversation} modelSelection={selection} creationEnabled={false} />;
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

test.each([false, true])('shared chat renders Agent switches immediately with localized history and an intact draft (companion=%p)', async (companion) => {
  const sourcePreset = parseEntityId('agent-preset', '0190f5fe-7c00-7a00-8000-000000001420');
  const nextPreset = parseEntityId('agent-preset', '0190f5fe-7c00-7a00-8000-000000001421');
  const templateKey = companion ? 'companion.default' : 'assistant.general';
  const englishLabel = companion ? 'Companion' : 'General';
  const chineseLabel = companion ? '伙伴' : '通用';
  const personalLabel = companion ? '我的研究伙伴' : 'My Research Agent';
  let authority = { ...conversation, preset_id: sourcePreset,
    extra: { workspace: '', official_template_key: templateKey, ...(companion ? { companion_session: true } : {}) },
    agent_snapshot: { preset_id: sourcePreset, preset_name: templateKey, preset_revision: 1,
      instructions: '', included_skills: [], excluded_auto_skills: [], enabled_capabilities: [],
      enabled_capability_actions: {}, required_resource_kinds: [], warnings: [],
      knowledge_policy: { enabled: false, writeback: false, grounded: false }, resolved_agent_backend: 'nomifun.nomi' },
  } as TChatConversation;
  spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async () => authority);
  const oldBoundary: IMessageTips = {
    id: 'old-agent-boundary', message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000001422'),
    msg_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000001422'),
    conversation_id: conversationId, type: 'tips', position: 'center', created_at: 20,
    content: { type: 'success', content: '', agent_transition: {
      transition_id: '0190f5fe-7c00-7a00-8000-000000001422',
      previous_agent_label: templateKey, next_agent_label: templateKey,
      previous_preset_id: sourcePreset, next_preset_id: sourcePreset,
      previous_template_key: templateKey, next_template_key: templateKey,
      effective_from: 'next_turn', handoff_mode: 'context_only', completion_gate_inherited: false,
    } },
  };
  const answer: TMessage = { id: 'prior-answer', conversation_id: conversationId, type: 'text', position: 'left',
    message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000001424'), msg_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000001424'),
    created_at: 10, content: { content: 'The previous answer stays visible.' } };
  const history = spyOn(ipcBridge.database.getConversationMessages, 'invoke')
    .mockResolvedValue({ items: [oldBoundary, answer], total: 2, has_more: false });
  const listeners = new Set<Parameters<typeof ipcBridge.agentPlatform.sessions.onAgentChanged.on>[0]>();
  spyOn(ipcBridge.agentPlatform.sessions.onAgentChanged, 'on').mockImplementation(listener => {
    listeners.add(listener); return () => { listeners.delete(listener); };
  });
  const cache = new Map();
  const view = () => <SWRConfig value={{ provider: () => cache, dedupingInterval: 0 }}>
    <MemoryRouter><I18nextProvider i18n={i18n}><ThemeProvider>
      <PreviewProvider persistNamespace={`agent-switch-${companion}`} subscribeGlobalOpen={false}>
        <InitialMessageProvider><NomiChat conversation={authority} modelSelection={selection}
          agent_name={companion ? '天天' : undefined} creationEnabled={false} compactProductComposer={companion} />
        </InitialMessageProvider>
      </PreviewProvider>
    </ThemeProvider></I18nextProvider></MemoryRouter>
  </SWRConfig>;
  const page = render(view());
  await page.findByText(`${englishLabel} → ${englishLabel} · effective from the next message`);
  const scroller = page.getByTestId('message-list-scroller');
  const editor = page.getByTestId('sendbox-input') as HTMLTextAreaElement;
  fireEvent.change(editor, { target: { value: '未发送的草稿' } });
  const newBoundary: IMessageTips = { ...oldBoundary, id: 'new-agent-boundary', created_at: 30,
    message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000001423'), msg_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000001423'),
    content: { ...oldBoundary.content, agent_transition: { ...oldBoundary.content.agent_transition!,
      transition_id: '0190f5fe-7c00-7a00-8000-000000001423',
      // An internal receipt label must resolve through the exact current Agent,
      // including on a Companion surface whose composer uses the persona name.
      next_preset_id: nextPreset, next_template_key: undefined, next_agent_label: templateKey,
    } },
  };
  history.mockResolvedValue({ items: [newBoundary, oldBoundary, answer], total: 3, has_more: false });
  authority = { ...authority, preset_id: nextPreset, extra: { workspace: '', ...(companion ? { companion_session: true } : {}) },
    agent_snapshot: { ...authority.agent_snapshot!, preset_id: nextPreset, preset_name: personalLabel },
  };
  const publish = () => listeners.forEach(listener => listener({
    agent_session_id: parseEntityId('agent-session', conversationId), transition_id: newBoundary.content.agent_transition!.transition_id,
    previous_agent_label: templateKey, current_agent_label: personalLabel, binding_version: 2, effective_from: 'next_turn',
  }));
  act(publish);
  page.rerender(view());
  await page.findByText(`${englishLabel} → ${personalLabel} · effective from the next message`);
  expect(page.getByTestId('message-list-scroller')).toBe(scroller);
  expect(page.getByTestId('sendbox-input')).toBe(editor);
  expect(editor.value).toBe('未发送的草稿');
  expect(page.container.textContent).not.toContain(templateKey);
  act(publish);
  await waitFor(() => expect(history).toHaveBeenCalledTimes(3));
  expect(page.container.querySelectorAll('.agent-transition-boundary')).toHaveLength(2);
  await act(async () => { await i18n.changeLanguage('zh-CN'); });
  try {
    expect(page.getByText(`${chineseLabel} → ${personalLabel} · 下一条消息起生效`)).toBeDefined();
    expect(page.getByText(`${chineseLabel} → ${chineseLabel} · 下一条消息起生效`)).toBeDefined();
    expect(editor.value).toBe('未发送的草稿');
  } finally {
    await act(async () => { await i18n.changeLanguage('en-US'); });
  }
});

test('slow preparation and acceptance keep the real transcript mounted without a skeleton or a duplicate first message', async () => {
  const capabilities = deferred<AgentSessionCapabilitySelectionState>();
  spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockReturnValue(capabilities.promise);
  const accepted = deferred<IAcceptMessageResult>();
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
  expect(warmup).not.toHaveBeenCalled();
  await act(async () => { capabilities.resolve(capabilitySelection); });
  await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
  expect(send.mock.calls[0][0]).toMatchObject({ conversation_id: conversationId, input,
    idempotency_key: page.delivery.idempotency_key, initial_only: true });
  expect(page.getByTestId('initial-message-preparing')).toBeTruthy();
  expect(page.queryByTestId('message-list-skeleton')).toBeNull();
  expect(warmup).not.toHaveBeenCalled();
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ ...conversation, id: conversationId, name: input,
    type: 'nomi', status: 'running', extra: { workspace: '' }, runtime: {
      state: 'running', is_processing: true, can_send_message: false, active_turn_id: messageId,
    },
  } as TChatConversation);
  await act(async () => { accepted.resolve({ msg_id: messageId, replayed: false }); });
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

test('a Pending initial delivery prepares once through its owner and focus cannot compete with the accepted send', async () => {
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ ...conversation, status: 'pending',
    runtime: { state: 'idle', is_processing: false, can_send_message: true },
  });
  const prepared = deferred<void>();
  const warmup = spyOn(ipcBridge.conversation.warmup, 'invoke').mockReturnValue(prepared.promise);
  const accepted = deferred<IAcceptMessageResult>();
  const send = spyOn(ipcBridge.conversation.sendMessage, 'invoke').mockReturnValue(accepted.promise);
  let page!: ReturnType<typeof mountConversation>;
  await act(async () => { page = mountConversation(); });
  expect(warmup).toHaveBeenCalledTimes(1);
  expect(warmup.mock.calls[0][0]).toEqual({ conversation_id: conversationId });
  expect(send).not.toHaveBeenCalled();
  await act(async () => { prepared.resolve(); });
  expect(send).toHaveBeenCalledTimes(1);
  expect(send.mock.calls[0][0]).toMatchObject({ conversation_id: conversationId, initial_only: true,
    idempotency_key: page.delivery.idempotency_key });
  // This full Host-facing component also owns SWR reads and background
  // reconciliation. Keep its clock real; the focused SendBox tests below
  // isolate the debounce with fake time. Verify the same actual 300ms window.
  fireEvent.focus(page.getByTestId('sendbox-input'));
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 350)); });
  expect(warmup).toHaveBeenCalledTimes(1);
  expect(send).toHaveBeenCalledTimes(1);
  await act(async () => { accepted.resolve({ msg_id: messageId, replayed: false }); });
});

test.each(['unmount', 'conversation-change'] as const)('shared composer cancels the previous focus warmup on %s', async (transition) => {
  jest.useFakeTimers();
  const warmup = spyOn(ipcBridge.conversation.warmup, 'invoke').mockResolvedValue(undefined);
  const view = (id: typeof conversationId) => <MemoryRouter><I18nextProvider i18n={i18n}><ThemeProvider>
    <PreviewProvider persistNamespace='focus-warmup-lifecycle' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: id, type: 'nomi', workspace: '' }}>
        <MessageListProvider><SendBox onSend={async () => {}} /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </ThemeProvider></I18nextProvider></MemoryRouter>;
  const page = render(view(conversationId));
  fireEvent.focus(page.getByTestId('sendbox-input'));
  act(() => { jest.advanceTimersByTime(299); });
  expect(warmup).not.toHaveBeenCalled();
  if (transition === 'unmount') page.unmount();
  else page.rerender(view(otherId));
  await act(async () => { jest.advanceTimersByTime(1); });
  expect(warmup).not.toHaveBeenCalled();

  // The current mounted composer still supports explicit focus preparation.
  const current = transition === 'unmount' ? render(view(otherId)) : page;
  fireEvent.focus(current.getByTestId('sendbox-input'));
  await act(async () => { jest.advanceTimersByTime(300); });
  expect(warmup).toHaveBeenCalledTimes(1);
  expect(warmup.mock.calls[0][0]).toEqual({ conversation_id: otherId });
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
  const accepted = deferred<IAcceptMessageResult>();
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
  await act(async () => accepted.resolve({ msg_id: messageId, replayed: false }));
  await waitFor(() => expect(page.pending()).toBeNull());
  expect(page.container.textContent).not.toContain('Worked for');
  expect(page.getAllByTestId('message-text-content')).toHaveLength(1);
  const turnId = messageId;
  await act(async () => runtime.resolve({ id: conversationId, name: input, type: 'nomi', status: 'running',
    extra: { workspace: '' }, runtime: { state: 'running', is_processing: true, can_send_message: false,
      active_turn_id: turnId, processing_started_at: Date.now() },
  } as TChatConversation));
  await waitFor(() => expect(page.container.querySelector(`#message-turn-disclosure-${turnId}`)).not.toBeNull());
  expect(page.container.textContent).toContain('Worked for');
  act(() => streams.forEach(listener => listener({ conversation_id: conversationId, msg_id: messageId,
    turn_id: turnId, type: 'thinking', created_at: Date.now(), data: { content: 'Inspecting the request', status: 'thinking' } })));
  await waitFor(() => expect(page.container.textContent).toContain(conversationLocale.thinking.label));
  expect(page.getByTestId('message-list-scroller')).toBe(scroller);
  expect(page.getAllByTestId('message-text-content')).toHaveLength(1);
  expect(send).toHaveBeenCalledTimes(1);
});

test.each([false, true])('Turn completion preserves an active composer and IME draft while capabilities refresh (compact=%p)', async (compact) => {
  const sessionId = parseConversationId(compact
    ? '0190f5fe-7c00-7a00-8000-000000001411'
    : '0190f5fe-7c00-7a00-8000-000000001410');
  const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000001412');
  const startedAt = Date.now() - 1000;
  let authority = { id: sessionId, name: 'Active conversation', type: 'nomi', status: 'running',
    extra: { workspace: '' }, runtime: { state: 'running', is_processing: true,
      can_send_message: false, active_turn_id: turnId, processing_started_at: startedAt },
  } as TChatConversation;
  spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async () => authority);
  spyOn(ipcBridge.conversation.taskPlan, 'invoke').mockResolvedValue({
    conversation_id: sessionId, sequence: 0, turn_id: null, plan: null, turn_status: null,
  });
  const refresh = deferred<AgentSessionCapabilitySelectionState>();
  const capabilities = spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke')
    .mockResolvedValueOnce(capabilitySelection).mockReturnValue(refresh.promise);
  const accepted = deferred<IAcceptMessageResult>();
  const send = spyOn(ipcBridge.conversation.sendMessage, 'invoke').mockReturnValue(accepted.promise);
  const completions = new Set<Parameters<typeof ipcBridge.conversation.turnCompleted.on>[0]>();
  spyOn(ipcBridge.conversation.turnCompleted, 'on').mockImplementation(listener => {
    completions.add(listener); return () => { completions.delete(listener); };
  });
  let userCreated!: Parameters<typeof ipcBridge.conversation.userCreated.on>[0];
  spyOn(ipcBridge.conversation.userCreated, 'on').mockImplementation(listener => {
    userCreated = listener; return () => {};
  });
  const cache = new Map();
  const page = render(<SWRConfig value={{ provider: () => cache, dedupingInterval: 0 }}>
    <MemoryRouter><I18nextProvider i18n={i18n}><ThemeProvider>
      <PreviewProvider persistNamespace={`composer-completion-${compact}`} subscribeGlobalOpen={false}>
        <InitialMessageProvider><NomiChat conversation={authority} modelSelection={selection}
          creationEnabled={false} compactProductComposer={compact} /></InitialMessageProvider>
      </PreviewProvider>
    </ThemeProvider></I18nextProvider></MemoryRouter>
  </SWRConfig>);
  await waitFor(() => expect(page.queryByTestId('sendbox-stop-btn')).not.toBeNull());
  const editor = page.getByTestId('sendbox-input') as HTMLTextAreaElement;
  await waitFor(() => expect(editor.disabled).toBe(false));
  act(() => userCreated({ conversation_id: sessionId, msg_id: turnId, content: 'Current task',
    position: 'right', status: 'finish', hidden: false, created_at: startedAt }));
  await waitFor(() => expect(page.getByTestId('message-text-content').textContent).toBe('Current task'));
  const scroller = page.getByTestId('message-list-scroller');

  const committedPrefix = '已经写好的草稿';
  const imeBuffer = `${committedPrefix}fa'guo`;
  act(() => editor.focus());
  fireEvent.change(editor, { target: { value: committedPrefix } });
  fireEvent.compositionStart(editor);
  fireEvent.compositionUpdate(editor, { data: "fa'guo" });
  fireEvent.change(editor, { target: { value: imeBuffer } });
  editor.setSelectionRange(4, 7);
  expect(document.activeElement).toBe(editor);
  expect(editor.value).toBe(imeBuffer);

  authority = { ...authority, status: 'finished', runtime: { state: 'idle', is_processing: false,
    can_send_message: true } };
  act(() => completions.forEach(listener => listener({ conversation_id: sessionId, turn_id: turnId,
    status: 'finished', state: 'ai_waiting_input', detail: '', can_send_message: true,
    runtime: { state: 'idle', is_processing: false, can_send_message: true },
    workspace: '', model: { platform: 'openai', name: 'Test provider', use_model: 'test-model' },
    last_message: { content: 'Task finished', created_at: Date.now() },
  })));
  await waitFor(() => expect(capabilities).toHaveBeenCalledTimes(2));
  expect(page.getByTestId('sendbox-input')).toBe(editor);
  expect(page.getByTestId('message-list-scroller')).toBe(scroller);
  expect(page.queryByTestId('message-list-skeleton')).toBeNull();
  expect(editor.disabled).toBe(false);
  expect(document.activeElement).toBe(editor);
  expect(editor.value).toBe(imeBuffer);
  expect([editor.selectionStart, editor.selectionEnd]).toEqual([4, 7]);
  fireEvent.keyDown(editor, { key: 'Enter' });
  expect(send).not.toHaveBeenCalled();

  const committedDraft = `${committedPrefix}法国`;
  fireEvent.compositionEnd(editor, { data: '法国', target: { value: committedDraft } });
  await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
  expect(editor.value).toBe(committedDraft);
  const sendButton = page.getByTestId('sendbox-send-btn') as HTMLButtonElement;
  expect(sendButton.disabled).toBe(false);
  fireEvent.click(sendButton);
  await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
  expect(send.mock.calls[0][0]).toMatchObject({ conversation_id: sessionId, input: committedDraft });
  expect(editor.value).toBe(committedDraft);
  await act(async () => refresh.resolve({ ...capabilitySelection, binding_version: 2 }));
  expect(page.getByTestId('sendbox-input')).toBe(editor);
  expect(document.activeElement).toBe(editor);
  expect(send).toHaveBeenCalledTimes(1);
  await act(async () => accepted.resolve({ msg_id: messageId, replayed: false }));
  await waitFor(() => expect(editor.value).toBe(''));
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
