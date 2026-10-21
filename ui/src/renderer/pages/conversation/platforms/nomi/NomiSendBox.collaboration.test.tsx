import '../../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import { configService } from '@/common/config/configService';
import type { TChatConversation, TProviderWithModel } from '@/common/config/storage';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { parseConversationId, parseExecutionId, parseExecutionTemplateId, parseMessageId, parseProviderId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import { browserStorageKey, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import * as themeContext from '@/renderer/hooks/context/ThemeContext';
import * as drafts from '@/renderer/components/collaboration/useCollaborationDraft';
import * as selection from '@/renderer/components/chat/SessionCapabilityPicker/useSessionCapabilitySelection';
import * as modelCatalog from '../../execution/useExecutionModelPool';
import * as executionContext from '../../execution/ExecutionContext';
import { MessageListProvider } from '../../Messages/hooks';
import { PreviewProvider } from '../../Preview';
import type { NomiMessageRuntime } from './useNomiMessage';
import type { NomiModelSelection } from './useNomiModelSelection';
import NomiSendBox from './NomiSendBox';
import { inputDeliveryAttempt } from './inputDeliveryAttempt';
import enCollaboration from '@/renderer/services/i18n/locales/en-US/collaboration.json';
import { emitter } from '@/renderer/utils/emitter';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: { collaboration: enCollaboration } } } });
const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000921');
const model = { id: providerId, use_model: 'lead' } as TProviderWithModel;
const main = { provider_id: providerId, model: 'lead' };
const helper = { provider_id: providerId, model: 'helper' };
const modelSelection: NomiModelSelection = { current_model: model, providers: [], pickerDisabled: false,
  getAvailableModels: () => [], handleSelectModel: async () => {}, getDisplayModelName: name => name ?? '' };
const noop = () => {};
const activity = { running: false, hasHydratedRunningState: true, pauseNotice: null, tokenUsage: undefined,
  setActiveMsgId: noop, markTurnAccepted: noop, reconcilePublicDeliveryReplay: noop, reconcileAfterStreamTerminal: noop,
  setWaitingResponse: noop, resetState: noop, confirmStopped: noop, getTurnStartGeneration: () => 0, getTurnCompletionGeneration: () => 0,
} as unknown as NomiMessageRuntime;
let draft: ReturnType<typeof drafts.useCollaborationDraft>;
let apply: ReturnType<typeof mock<() => Promise<{ skill_names: string[]; mcp_server_ids: string[] }>>>;
let warning: ReturnType<typeof spyOn<typeof Message, 'warning'>>;

beforeEach(() => {
  configService.reset(); setBrowserStorageGeneration(uuidv7()); sessionStorage.clear();
  spyOn(ipcBridge.conversation.inputQueue, 'invoke').mockResolvedValue({ version: 0, items: [] });
  warning = spyOn(Message, 'warning').mockImplementation(() => undefined as never);
  draft = { mainModel: main, activeCollaborators: [helper], setCollaborators: noop,
    policy: { delegationPolicy: 'prefer_parallel', decisionPolicy: 'ask_user' }, setPolicy: noop, selectedTemplate: null,
    setTemplate: noop, config: { execution_model_pool: { mode: 'range', models: [main, helper] }, execution_template_id: null,
      delegation_policy: 'prefer_parallel', decision_policy: 'ask_user' }, ready: true, reset: noop, copyToConversation: noop };
  apply = mock(async () => ({ skill_names: ['selected-skill'], mcp_server_ids: [] }));
  spyOn(drafts, 'useCollaborationDraft').mockImplementation(() => draft);
  spyOn(selection, 'useSessionCapabilitySelection').mockReturnValue({
    state: { selection: { skill_names: ['selected-skill'], mcp_server_ids: [] }, binding_version: 1, editable: true },
    draft: { skillNames: ['selected-skill'], mcpServerIds: [] }, loading: false, saving: false, error: undefined,
    setDraft: noop, retry: noop, applyBeforeSend: apply,
  });
  spyOn(modelCatalog, 'useExecutionModelPool').mockReturnValue({ providers: [], getAvailableModels: () => [], formatModelLabel: (_provider, name) => name ?? '',
    isLoading: false, configuredPairs: [main, helper], allPairs: [main, helper], hasModels: true, buildModelPool: () => null });
  spyOn(executionContext, 'useExecutionSafe').mockReturnValue(null);
  spyOn(themeContext, 'useThemeContext').mockReturnValue({ theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {} });
  for (const event of [ipcBridge.mode.onProvidersChanged, ipcBridge.conversation.reconnected, ipcBridge.conversation.turnStarted,
    ipcBridge.conversation.turnCompleted, ipcBridge.agentPlatform.sessions.onAgentChanged, ipcBridge.agentPlatform.sessions.onCapabilitiesChanged,
    ipcBridge.conversation.responseStream, ipcBridge.conversation.userCreated, ipcBridge.knowledge.onTreeChanged, ipcBridge.knowledge.onEntryContentUpdated, ipcBridge.fileStream.contentUpdate]) {
    spyOn(event, 'on').mockImplementation(() => () => {});
  }
  spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockResolvedValue({ name: 'brief.txt', path: '/project/brief.txt', size: 0,
    type: 'text/plain', lastModified: 1, isDirectory: false });
  spyOn(ipcBridge.fs.getImageBase64, 'invoke').mockResolvedValue(null);
  spyOn(ipcBridge.mcpService.listServers, 'invoke').mockResolvedValue([]);
  spyOn(appHosting.mcpSources, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.agentExecutionTemplate.list, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  spyOn(ipcBridge.database.getUserConversations, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
});
afterEach(() => { cleanup(); mock.restore(); configService.reset(); sessionStorage.clear(); });

function mount(options: { compact?: boolean; busy?: boolean; linkedBusy?: boolean; paused?: boolean; waiting?: boolean;
  pauseReason?: string; knownTarget?: boolean } = {}) {
  const pausedTurnId = parseMessageId(uuidv7());
  const conversation = { id: parseConversationId(uuidv7()), name: 'General', type: 'nomi', session_purpose: 'conversation', status: 'finished',
    created_at: 1, modified_at: 1, model, extra: { workspace: '/project' } } as TChatConversation;
  spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async request => ({ ...conversation, id: request.conversation_id }));
  spyOn(ipcBridge.conversation.taskPlan, 'invoke').mockImplementation(async request => ({ conversation_id: request.conversation_id, sequence: 0, turn_id: null, turn_status: null, plan: null }));
  const send = spyOn(ipcBridge.conversation.sendMessage, 'invoke').mockResolvedValue({ msg_id: parseMessageId(uuidv7()), replayed: false });
  const create = spyOn(ipcBridge.agentExecution.create, 'invoke').mockResolvedValue({ execution_id: parseExecutionId(uuidv7()) } as never);
  const fromTemplate = spyOn(ipcBridge.agentExecutionTemplate.createExecution, 'invoke').mockResolvedValue({ execution_id: parseExecutionId(uuidv7()) } as never);
  if (options.linkedBusy) {
    const executionId = parseExecutionId(uuidv7());
    spyOn(executionContext, 'useExecutionSafe').mockReturnValue({ conversationId: conversation.id, executionId,
      detail: { execution: { execution_id: executionId, status: 'running' } } } as never);
  }
  const renderComposer = (busy = options.busy ?? options.waiting ?? false, id = conversation.id) => <MemoryRouter><I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [] }, shouldRetryOnError: false }}>
    <PreviewProvider persistNamespace={browserStorageKey('collaboration-test', 'conversation', id)} subscribeGlobalOpen={false}>
      <MessageListProvider><NomiSendBox conversation_id={id} modelSelection={modelSelection}
        turnActivity={{ ...activity, running: busy, activeTurnId: busy && options.knownTarget !== false ? pausedTurnId : undefined, executing: busy && !options.waiting, waitingNotice: options.waiting ? {
          turnId: pausedTurnId, outcome: { kind: 'waiting', wait_seq: 41, request_id: 'authorize', reason: 'user_input' },
        } : null, pauseNotice: options.paused
          ? { turnId: pausedTurnId, reason: options.pauseReason ?? 'EXECUTION_TOTAL_MODEL_BUDGET', cleanupProven: true, pausedAt: 123 } : null }}
        creationEnabled={false} collaborationEnabled compactProductComposer={options.compact} /></MessageListProvider>
    </PreviewProvider></SWRConfig></I18nextProvider></MemoryRouter>;
  const view = render(renderComposer());
  const setBusy = (busy: boolean) => view.rerender(renderComposer(busy));
  const input = view.getByTestId('sendbox-input') as HTMLTextAreaElement;
  const submit = async (text = 'Review this work') => {
    await waitFor(() => expect(input.disabled).toBe(false));
    fireEvent.change(input, { target: { value: text } });
    await waitFor(() => expect(input.value).toBe(text));
    await act(async () => { fireEvent.keyDown(input, { key: 'Enter' }); });
  };
  return { view, input, submit, setBusy, navigate: (id: typeof conversation.id) => view.rerender(renderComposer(false, id)), conversation, send, create, fromTemplate, pausedTurnId };
}

test('an existing conversation keeps the shared icon and submits its chosen pool and policies through the original Execution API', async () => {
  const composer = mount();
  const trigger = composer.view.getByTestId('guid-collaborator-selector') as HTMLButtonElement;
  expect(trigger.closest('[data-composer-tools]')).toBeTruthy();
  expect(trigger.disabled).toBe(false);
  fireEvent.click(trigger);
  await composer.view.findByTestId('collaboration-composer-panel');
  expect(composer.view.getByRole('radio', { name: 'Prefer parallel' }).hasAttribute('disabled')).toBe(false);
  fireEvent.keyDown(composer.input, { key: 'Escape' });
  await composer.submit();
  await waitFor(() => expect(composer.create).toHaveBeenCalledTimes(1));
  expect(composer.create.mock.calls[0][0]).toEqual({ goal: 'Review this work', work_dir: '/project',
    delegation_policy: 'prefer_parallel', decision_policy: 'ask_user', lead_conversation_id: composer.conversation.id,
    lead_model: main, model_pool: { mode: 'range', models: [main, helper] } });
  expect(apply).toHaveBeenCalledTimes(1);
  expect(composer.send).not.toHaveBeenCalled();
  await waitFor(() => expect(composer.input.value).toBe(''));
});

test('the single-model automatic default keeps ordinary chat admission', async () => {
  draft.config = { execution_model_pool: { mode: 'single', model: main }, execution_template_id: null, delegation_policy: 'automatic', decision_policy: 'automatic' };
  const composer = mount();
  await composer.submit('Hello');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.create).not.toHaveBeenCalled();
  expect(composer.send.mock.calls[0][0].inject_skills).toEqual(['selected-skill']);
});

test('a pending chat submission only shows stop until admission, then allows actions for a new draft', async () => {
  draft.config = { execution_model_pool: { mode: 'single', model: main }, execution_template_id: null, delegation_policy: 'automatic', decision_policy: 'automatic' };
  const composer = mount();
  let accept!: (value: Awaited<ReturnType<typeof ipcBridge.conversation.sendMessage.invoke>>) => void;
  composer.send.mockImplementation(() => new Promise(resolve => { accept = resolve; }));

  await composer.submit('Hello');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.input.value).toBe('Hello');
  expect(composer.view.getByTestId('sendbox-stop-btn')).toBeTruthy();
  expect(composer.view.queryByTestId('sendbox-steer-btn') === null).toBe(true);
  expect(composer.view.queryByTestId('sendbox-send-btn') === null).toBe(true);

  act(() => composer.setBusy(true));
  expect(composer.view.queryByTestId('sendbox-steer-btn') === null).toBe(true);
  expect(composer.view.queryByTestId('sendbox-send-btn') === null).toBe(true);
  await act(async () => accept({ msg_id: parseMessageId(uuidv7()), replayed: false }));
  await waitFor(() => expect(composer.input.value).toBe(''));
  expect(composer.view.getByTestId('sendbox-stop-btn')).toBeTruthy();
  expect(composer.view.queryByTestId('sendbox-steer-btn') === null).toBe(true);
  expect(composer.view.queryByTestId('sendbox-send-btn') === null).toBe(true);

  fireEvent.change(composer.input, { target: { value: 'A follow-up while the turn runs' } });
  await waitFor(() => expect(composer.view.getByTestId('sendbox-send-btn')).toBeTruthy());
  expect(composer.view.queryByTestId('sendbox-steer-btn')).toBeNull();
  expect(composer.view.getByTestId('sendbox-stop-btn')).toBeTruthy();
  expect(composer.view.getByTestId('sendbox-send-btn')).toBeTruthy();
  expect(composer.send).toHaveBeenCalledTimes(1);
});

test('a rejected chat submission restores send without exposing steer for its preserved draft', async () => {
  draft.config = { execution_model_pool: { mode: 'single', model: main }, execution_template_id: null, delegation_policy: 'automatic', decision_policy: 'automatic' };
  const composer = mount();
  let reject!: (reason: Error) => void;
  composer.send.mockImplementation(() => new Promise((_resolve, rejectSend) => { reject = rejectSend; }));

  await composer.submit('Keep this draft');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.view.getByTestId('sendbox-stop-btn')).toBeTruthy();
  expect(composer.view.queryByTestId('sendbox-steer-btn') === null).toBe(true);
  expect(composer.view.queryByTestId('sendbox-send-btn') === null).toBe(true);

  await act(async () => reject(new Error('Chat admission rejected')));
  await waitFor(() => expect(composer.view.getByTestId('sendbox-send-btn')).toBeTruthy());
  expect(composer.input.value).toBe('Keep this draft');
  expect(composer.view.queryByTestId('sendbox-stop-btn') === null).toBe(true);
  expect(composer.view.queryByTestId('sendbox-steer-btn') === null).toBe(true);
  expect(composer.send).toHaveBeenCalledTimes(1);
});

test.each([false, true])('repeated submit and steer shortcuts cannot resend or queue a pending draft when runtime busy is %s', async busy => {
  draft.config = { execution_model_pool: { mode: 'single', model: main }, execution_template_id: null, delegation_policy: 'automatic', decision_policy: 'automatic' };
  const composer = mount();
  let accept!: (value: Awaited<ReturnType<typeof ipcBridge.conversation.sendMessage.invoke>>) => void;
  composer.send.mockImplementation(() => new Promise(resolve => { accept = resolve; }));

  await composer.submit('Send this once');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  act(() => composer.setBusy(busy));
  await act(async () => {
    fireEvent.keyDown(composer.input, { key: 'Enter' });
    fireEvent.keyDown(composer.input, { key: 'Enter', metaKey: true });
    fireEvent.keyDown(composer.input, { key: 'Enter', ctrlKey: true });
  });
  expect(composer.send).toHaveBeenCalledTimes(1);
  expect(composer.input.value).toBe('Send this once');
  expect(composer.view.queryByTestId('session-input-queue')).toBeNull();

  await act(async () => accept({ msg_id: parseMessageId(uuidv7()), replayed: false }));
});

test('the selected plan uses the same template creation route as the welcome composer', async () => {
  const templateId = parseExecutionTemplateId('0190f5fe-7c00-7a00-8000-000000000922');
  draft.config!.execution_template_id = templateId;
  const composer = mount();
  await composer.submit();
  await waitFor(() => expect(composer.fromTemplate).toHaveBeenCalledTimes(1));
  expect(composer.fromTemplate.mock.calls[0][0]).toMatchObject({ execution_template_id: templateId,
    request: { lead_conversation_id: composer.conversation.id, goal: 'Review this work', decision_policy: 'ask_user' } });
  expect(composer.create).not.toHaveBeenCalled();
  expect(composer.send).not.toHaveBeenCalled();
});

test('an admission failure preserves the prompt and settings without a chat fallback or automatic retry', async () => {
  const composer = mount();
  composer.create.mockRejectedValue(new Error('Execution admission rejected'));
  await composer.submit();
  await waitFor(() => expect(composer.create).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(composer.input.disabled).toBe(false));
  expect(composer.input.value).toBe('Review this work');
  expect(draft.config!.delegation_policy).toBe('prefer_parallel');
  expect(composer.send).not.toHaveBeenCalled();
});

test('an active Turn accepts ordinary input into its existing owner instead of admitting or queueing collaboration', async () => {
  const composer = mount({ busy: true });
  const trigger = composer.view.getByTestId('guid-collaborator-selector') as HTMLButtonElement;
  expect(trigger.disabled).toBe(false);
  fireEvent.click(trigger);
  const radio = await composer.view.findByRole('radio', { name: 'Prefer parallel' });
  expect((radio as HTMLButtonElement).disabled).toBe(true);
  await composer.submit();
  await waitFor(() => expect(composer.input.value).toBe(''));
  expect(composer.create).not.toHaveBeenCalled();
  expect(composer.send).toHaveBeenCalledTimes(1);
  expect(composer.send.mock.calls[0][0].delivery).toEqual({ kind: 'queue' });
  expect(composer.view.queryByTestId('session-input-queue')).toBeNull();
});

test.each(['enter', 'mod-enter'] as const)('ordinary busy sends always queue with the existing %s shortcut', async sendKey => {
  configService.setLocal('chat.sendKey', sendKey);
  const composer = mount({ busy: true });
  fireEvent.change(composer.input, { target: { value: 'An independent queued instruction' } });
  expect(composer.view.getByTestId('sendbox-stop-btn')).toBeTruthy();
  fireEvent.keyDown(composer.input, { key: 'Enter', ctrlKey: sendKey === 'mod-enter' });
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.send.mock.calls[0][0].delivery).toEqual({ kind: 'queue' });
});

test('guiding is an explicit draft action beside Stop and ordinary Send', async () => {
  const composer = mount({ busy: true });
  expect(composer.view.queryByTestId('sendbox-guide-btn')).toBeNull();
  fireEvent.change(composer.input, { target: { value: 'A correction for the observed Turn' } });
  const guide = composer.view.getByTestId('sendbox-guide-btn');
  expect(guide.getAttribute('title')).toBe('conversation.sendbox.guideHint');
  expect(composer.view.getByTestId('sendbox-stop-btn')).toBeTruthy();
  expect(composer.view.getByTestId('sendbox-send-btn')).toBeTruthy();
  fireEvent.click(guide);
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.send.mock.calls[0][0].delivery).toEqual({ kind: 'prefer_current', turn_id: composer.pausedTurnId });
});

test('guiding is unavailable without a known active Turn and Ctrl+Enter remains an ordinary send', async () => {
  const composer = mount();
  draft.config = { execution_model_pool: { mode: 'single', model: main }, execution_template_id: null, delegation_policy: 'automatic', decision_policy: 'automatic' };
  fireEvent.change(composer.input, { target: { value: 'A normal independent request' } });
  expect(composer.view.queryByTestId('sendbox-guide-btn')).toBeNull();
  fireEvent.keyDown(composer.input, { key: 'Enter', ctrlKey: true });
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.send.mock.calls[0][0].delivery).toEqual({ kind: 'queue' });
});

test.each([false, true])('Stop names observed T even if successor A has already taken ownership (%s)', async successorStarted => {
  const composer = mount({ busy: true });
  const successor = parseMessageId(uuidv7());
  let serverActive: typeof successor | undefined = successorStarted ? successor : composer.pausedTurnId;
  const cancelled: Array<typeof successor> = [];
  // Model the exact-target server boundary: omission would select its newer
  // active Turn, so this checks the actual UI request rather than UI labels.
  const stop = spyOn(ipcBridge.conversation.stop, 'invoke').mockImplementation(async request => {
    const target = request.expected_turn_id ?? serverActive;
    if (target && target === serverActive) { cancelled.push(target); serverActive = undefined; }
  });
  fireEvent.click(composer.view.getByTestId('sendbox-stop-btn'));
  await waitFor(() => expect(stop).toHaveBeenCalledTimes(1));
  expect(stop.mock.calls[0][0]).toEqual({ conversation_id: composer.conversation.id, expected_turn_id: composer.pausedTurnId });
  expect(cancelled).toEqual(successorStarted ? [] : [composer.pausedTurnId]);
  expect(serverActive).toBe(successorStarted ? successor : undefined);
});

test('Stop without a known target reconciles authority without selecting the latest Turn', async () => {
  const reconcile = spyOn(activity, 'reconcileAfterStreamTerminal').mockImplementation(noop);
  const composer = mount({ busy: true, knownTarget: false });
  const stop = spyOn(ipcBridge.conversation.stop, 'invoke').mockResolvedValue();
  fireEvent.change(composer.input, { target: { value: 'Keep this independent draft' } });
  expect(composer.view.queryByTestId('sendbox-guide-btn')).toBeNull();
  fireEvent.click(composer.view.getByTestId('sendbox-stop-btn'));
  await waitFor(() => expect(reconcile).toHaveBeenCalled());
  expect(stop).not.toHaveBeenCalled();
  expect(warning).toHaveBeenCalledWith('conversation.stop.awaitingTarget');
  expect(composer.input.value).toBe('Keep this independent draft');
});

test('IME composition never submits either send behavior', async () => {
  const composer = mount({ busy: true });
  fireEvent.change(composer.input, { target: { value: '输入中的消息' } });
  fireEvent.compositionStart(composer.input);
  fireEvent.keyDown(composer.input, { key: 'Enter' });
  fireEvent.keyDown(composer.input, { key: 'Enter', ctrlKey: true });
  await act(async () => {});
  expect(composer.send).not.toHaveBeenCalled();
  expect(composer.input.value).toBe('输入中的消息');
});

test('a lost Guide acknowledgement retries its original target through ordinary Send', async () => {
  const composer = mount({ busy: true });
  composer.send.mockRejectedValueOnce(new Error('accepted response lost'));
  fireEvent.change(composer.input, { target: { value: 'This precise supplement' } });
  fireEvent.click(composer.view.getByTestId('sendbox-guide-btn'));
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  const original = composer.send.mock.calls[0][0];
  expect(original.delivery).toEqual({ kind: 'prefer_current', turn_id: composer.pausedTurnId });
  act(() => composer.setBusy(false));
  await composer.submit('This precise supplement');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(2));
  expect(composer.send.mock.calls[1][0]).toEqual(original);
});

test('a late accepted response cannot clear another Session draft or its attachments', async () => {
  const composer = mount({ busy: true });
  let accept!: (value: Awaited<ReturnType<typeof ipcBridge.conversation.sendMessage.invoke>>) => void;
  composer.send.mockImplementationOnce(() => new Promise(resolve => { accept = resolve; }));
  act(() => emitter.emit('nomi.selected.file', ['/project/brief.txt']));
  await composer.view.findByText('brief.txt');
  await composer.submit('Original Session request');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  act(() => composer.navigate(parseConversationId(uuidv7())));
  const nextInput = composer.view.getByTestId('sendbox-input') as HTMLTextAreaElement;
  fireEvent.change(nextInput, { target: { value: 'Keep the new Session draft' } });
  act(() => emitter.emit('nomi.selected.file', ['/project/brief.txt']));
  await composer.view.findByText('brief.txt');
  await act(async () => accept({ msg_id: parseMessageId(uuidv7()), replayed: false }));
  expect(nextInput.value).toBe('Keep the new Session draft');
  expect(composer.view.getByText('brief.txt')).toBeTruthy();
  expect(composer.send).toHaveBeenCalledTimes(1);
});

test('a technical pause accepts information through the same intake and retries a lost acknowledgement with its original payload and key', async () => {
  const composer = mount({ paused: true, pauseReason: 'EXECUTION_DEPENDENCY_BLOCKED' });
  composer.send.mockRejectedValueOnce(new Error('response lost')).mockResolvedValue({ msg_id: parseMessageId(uuidv7()), replayed: true });
  await composer.submit('The required application is now configured');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.input.value).toBe('The required application is now configured');
  act(() => composer.setBusy(true));
  await composer.submit('The required application is now configured');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(2));
  expect(composer.send.mock.calls[1][0]).toEqual(composer.send.mock.calls[0][0]);
  expect(composer.send.mock.calls[0][0]).not.toHaveProperty('target');
  expect(composer.send.mock.calls[0][0]).not.toHaveProperty('expected_wait');
  expect(composer.create).not.toHaveBeenCalled();
  await waitFor(() => expect(composer.input.value).toBe(''));
});

test.each(['idle', 'running', 'paused'] as const)('a local image capability refusal keeps the %s draft and attachments without acknowledging delivery', async state => {
  const composer = mount({ busy: state === 'running', paused: state === 'paused' });
  await waitFor(() => expect(composer.input.disabled).toBe(false));
  act(() => emitter.emit('nomi.selected.file', ['/project/photo.png']));
  await composer.view.findByText('photo.png');
  await composer.submit('Preserve this unsent explanation');
  await waitFor(() => expect(warning).toHaveBeenCalled());
  await act(async () => {});
  expect(composer.input.value).toBe('Preserve this unsent explanation');
  expect(composer.view.getByText('photo.png')).toBeTruthy();
  expect(composer.send).not.toHaveBeenCalled();
  expect(composer.create).not.toHaveBeenCalled();
});

test('unconfirmed delivery capacity keeps a new draft while an original retry retains its original payload and key', async () => {
  const composer = mount({ busy: true });
  const attempts = Array.from({ length: 16 }, (_, index) => inputDeliveryAttempt(composer.conversation.id, `Original ${index}`, [], ['original-skill']));
  await composer.submit('Another unsent explanation');
  await composer.view.findByText(/Too many messages have unconfirmed delivery/);
  expect(composer.input.value).toBe('Another unsent explanation');
  expect(composer.send).not.toHaveBeenCalled();
  act(() => composer.setBusy(false));
  await composer.submit('Original 0');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  const request = composer.send.mock.calls[0][0];
  expect(request.idempotency_key).toBe(attempts[0].key);
  expect(request.inject_skills).toEqual(['original-skill']);
  expect(apply).not.toHaveBeenCalled();
});

test('an explicit non-admission receipt releases only that attempt and keeps the draft for a later manual send', async () => {
  const composer = mount({ busy: true });
  composer.send.mockRejectedValueOnce(new BackendHttpError({ method: 'POST', path: '/api/agent-sessions/session/messages', status: 409,
    body: { code: 'INPUT_NOT_ACCEPTED', error: 'The message was not accepted' } }));
  await composer.submit('Keep this exact draft');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.input.value).toBe('Keep this exact draft');
  await composer.submit('Keep this exact draft');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(2));
  expect(composer.send.mock.calls[1][0].idempotency_key).not.toBe(composer.send.mock.calls[0][0].idempotency_key);
  await waitFor(() => expect(composer.input.value).toBe(''));
});

test('product composers keep their existing ordinary input behavior even if their Agent has collaboration', async () => {
  const composer = mount({ compact: true });
  expect(composer.view.queryByTestId('guid-collaborator-selector')).toBeNull();
  await composer.submit('Continue app development');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.create).not.toHaveBeenCalled();
});

test('attachments are retained when a collaboration launch is rejected before admission', async () => {
  const composer = mount();
  await waitFor(() => expect(composer.input.disabled).toBe(false));
  act(() => emitter.emit('nomi.selected.file', ['/project/brief.txt']));
  await composer.view.findByText('brief.txt');
  await composer.submit();
  expect(composer.create).not.toHaveBeenCalled();
  expect(composer.send).not.toHaveBeenCalled();
  expect(composer.input.value).toBe('Review this work');
  expect(composer.view.getByText('brief.txt')).toBeTruthy();
  expect(warning).toHaveBeenCalledWith(i18n.t('guid.collaboration.attachmentsUnsupported'));
});

test('repeated send input while admission is awaiting its receipt creates one Execution', async () => {
  const composer = mount();
  let complete!: (value: never) => void;
  composer.create.mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  await composer.submit();
  await waitFor(() => expect(composer.create).toHaveBeenCalledTimes(1));
  fireEvent.keyDown(composer.input, { key: 'Enter' });
  expect(composer.create).toHaveBeenCalledTimes(1);
  await act(async () => complete({ execution_id: parseExecutionId(uuidv7()) } as never));
  await waitFor(() => expect(composer.input.value).toBe(''));
  expect(composer.send).not.toHaveBeenCalled();
});

test('a technical pause keeps its separate cancellation action while accepting ordinary messages', async () => {
  const composer = mount({ paused: true });
  const cancel = composer.view.getByTestId('sendbox-paused-cancel-btn');
  expect(cancel.classList.contains('bg-animate')).toBe(false);
  expect(composer.view.queryByTestId('sendbox-stop-btn')).toBeNull();
  await waitFor(() => expect(composer.input.disabled).toBe(false));
  fireEvent.change(composer.input, { target: { value: 'Continue the unfinished work' } });
  await waitFor(() => expect(composer.input.value).toBe('Continue the unfinished work'));
  await act(async () => { fireEvent.keyDown(composer.input, { key: 'Enter' }); });
  await waitFor(() => expect(composer.input.value).toBe(''));
  expect(composer.send).toHaveBeenCalledTimes(1);
  expect(composer.create).not.toHaveBeenCalled();
  const stop = spyOn(ipcBridge.conversation.stop, 'invoke').mockResolvedValue(undefined);
  fireEvent.click(composer.view.getByTestId('sendbox-paused-cancel-btn'));
  await waitFor(() => expect(stop).toHaveBeenCalledTimes(1));
  expect(stop).toHaveBeenCalledWith({ conversation_id: composer.conversation.id, expected_turn_id: composer.pausedTurnId });
});

test('a normal input handoff has no End Turn button and sends nonempty input through ordinary intake', async () => {
  const composer = mount({ waiting: true });
  expect(composer.view.queryByTestId('sendbox-stop-btn')).toBeNull();
  expect(composer.view.queryByTestId('sendbox-paused-cancel-btn')).toBeNull();
  await composer.submit('The browser authorization is complete');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.send.mock.calls[0][0].conversation_id).toBe(composer.conversation.id);
  expect(composer.send.mock.calls[0][0]).not.toHaveProperty('expected_wait');
  expect(composer.create).not.toHaveBeenCalled();
  expect(composer.fromTemplate).not.toHaveBeenCalled();
  await waitFor(() => expect(composer.input.value).toBe(''));
});

test('a linked active collaboration cannot be replaced by a new admission from an idle chat', async () => {
  const composer = mount({ linkedBusy: true });
  await composer.submit();
  expect(composer.create).not.toHaveBeenCalled();
  expect(composer.send).not.toHaveBeenCalled();
  expect(composer.input.value).toBe('Review this work');
  expect(warning).toHaveBeenCalledWith(i18n.t('collaboration.session.busy'));
});

test('an edited input acknowledgement preserves a newer draft typed while the original revision was saving', async () => {
  draft.config = { execution_model_pool: { mode: 'single', model: main }, execution_template_id: null,
    delegation_policy: 'automatic', decision_policy: 'automatic' };
  const composer = mount();
  let accept!: (value: Awaited<ReturnType<typeof ipcBridge.conversation.sendMessage.invoke>>) => void;
  composer.send.mockImplementation(() => new Promise(resolve => { accept = resolve; }));
  await waitFor(() => expect(composer.input.disabled).toBe(false));
  act(() => emitter.emit('sendbox.edit', { msgId: parseMessageId(uuidv7()), createdAt: 1, content: 'Original revision' }));
  await composer.submit('Save this revision');
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  fireEvent.change(composer.input, { target: { value: 'Keep this newer draft' } });
  await act(async () => accept({ msg_id: parseMessageId(uuidv7()), replayed: false }));
  await waitFor(() => expect(composer.input.value).toBe('Keep this newer draft'));
  expect(composer.send.mock.calls[0][0].input).toBe('Save this revision');
  expect(composer.send).toHaveBeenCalledTimes(1);
});
