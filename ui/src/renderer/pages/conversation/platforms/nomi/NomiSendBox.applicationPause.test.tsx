import '../../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { TChatConversation, TProviderWithModel } from '@/common/config/storage';
import type { AgentSessionCapabilitySelection } from '@/common/types/agentPlatform';
import { configService } from '@/common/config/configService';
import { conversationTarget, parseConversationId, parseMessageId, parseProviderId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import { browserStorageKey, sessionStorageKey, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import * as themeContext from '@/renderer/hooks/context/ThemeContext';
import { MessageListProvider } from '../../Messages/hooks';
import { PreviewProvider } from '../../Preview';
import { persistInitialMessageDelivery } from '../initialMessageDelivery';
import { createQueuedCommandItem } from '../useConversationCommandQueue';
import type { NomiMessageRuntime } from './useNomiMessage';
import type { NomiModelSelection } from './useNomiModelSelection';
import NomiSendBox from './NomiSendBox';
import * as capabilities from '@/renderer/components/chat/SessionCapabilityPicker/useSessionCapabilitySelection';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: {} } } });
const model = { id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000421'), use_model: 'configured-model' } as TProviderWithModel;
const modelSelection: NomiModelSelection = { current_model: model, providers: [], pickerDisabled: false,
  getAvailableModels: () => [], handleSelectModel: async () => {}, getDisplayModelName: name => name ?? '' };
const noop = () => {};
const activity = { running: false, hasHydratedRunningState: true, pauseNotice: null, tokenUsage: undefined,
  setActiveMsgId: noop, markTurnAccepted: noop, reconcilePublicDeliveryReplay: noop, reconcileAfterStreamTerminal: noop,
  setWaitingResponse: noop, resetState: noop, confirmStopped: noop, getTurnStartGeneration: () => 0, getTurnCompletionGeneration: () => 0,
} as unknown as NomiMessageRuntime;

beforeEach(() => {
  configService.reset(); setBrowserStorageGeneration(uuidv7()); sessionStorage.clear();
  spyOn(themeContext, 'useThemeContext').mockReturnValue({ theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {} });
  for (const event of [ipcBridge.mode.onProvidersChanged, ipcBridge.conversation.reconnected, ipcBridge.conversation.turnStarted,
    ipcBridge.conversation.turnCompleted, ipcBridge.agentPlatform.sessions.onAgentChanged, ipcBridge.agentPlatform.sessions.onCapabilitiesChanged,
    ipcBridge.conversation.responseStream, ipcBridge.knowledge.onTreeChanged, ipcBridge.knowledge.onEntryContentUpdated,
    ipcBridge.fileStream.contentUpdate]) spyOn(event, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.mcpService.listServers, 'invoke').mockResolvedValue([]);
  spyOn(appHosting.mcpSources, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.agentPlatform.sessions.getCapabilitySelection, 'invoke').mockResolvedValue({
    selection: { skill_names: [], mcp_server_ids: [] }, binding_version: 1, editable: true,
  });
});
afterEach(() => { cleanup(); mock.restore(); configService.reset(); sessionStorage.clear(); });

function mountPausedComposer(conversation: TChatConversation, initialPaused = false, onBeforeSubmit?: () => Promise<void>) {
  let pause!: (disabled: boolean) => void;
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(conversation);
  spyOn(ipcBridge.conversation.taskPlan, 'invoke').mockResolvedValue({ conversation_id: conversation.id, sequence: 0, turn_id: null, turn_status: null, plan: null });
  const send = spyOn(ipcBridge.conversation.sendMessage, 'invoke').mockResolvedValue({
    msg_id: parseMessageId(uuidv7()), replayed: false, completed: false,
    result_ok: true, result_text: null, result_error: null, result_error_code: null, result_error_retryable: null,
  });
  function Composer() {
    const [paused, setPaused] = useState(initialPaused); pause = setPaused;
    return <NomiSendBox conversation_id={conversation.id} modelSelection={modelSelection} turnActivity={activity}
      creationEnabled={false} compactProductComposer submissionDisabled={paused} onBeforeSubmit={onBeforeSubmit} />;
  }
  const view = render(<MemoryRouter><I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(),
    fallback: { providers: [] }, shouldRetryOnError: false,
  }}><PreviewProvider persistNamespace={browserStorageKey('app-pause-test', 'conversation', conversation.id)} subscribeGlobalOpen={false}>
    <MessageListProvider><Composer /></MessageListProvider>
  </PreviewProvider></SWRConfig></I18nextProvider></MemoryRouter>);
  return { view, send, pause: (value: boolean) => act(() => pause(value)) };
}

function freshConversation(): TChatConversation {
  return { id: parseConversationId(uuidv7()), name: 'App development', type: 'nomi', session_purpose: 'conversation', status: 'finished',
    created_at: 1, modified_at: 1, model, extra: { workspace: 'C:/NomiFun/apps/test', official_template_key: 'app.developer' } as TChatConversation['extra'] };
}

test.each([false, true])('manual file work cancels an old authority read and resumes the same initial key (released before receipt=%s)', async releaseBeforeReceipt => {
  const conversation = freshConversation();
  const key = sessionStorageKey('initial-message-nomi', conversationTarget(conversation.id));
  const original = persistInitialMessageDelivery(sessionStorage, key, conversation.id, 'Develop the imported app', ['C:/brief.txt']);
  let finishRead!: (result: { items: []; total: number; has_more: boolean }) => void;
  let reads = 0;
  const transcript = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockImplementation(() => {
    reads++;
    return reads === 1 ? new Promise(resolve => { finishRead = resolve; }) : Promise.resolve({ items: [], total: 0, has_more: false });
  });
  const composer = mountPausedComposer(conversation);
  await waitFor(() => expect(transcript).toHaveBeenCalledTimes(1));
  composer.pause(true);
  expect(composer.send).not.toHaveBeenCalled();
  expect(JSON.parse(sessionStorage.getItem(key)!)).toEqual(original);
  if (releaseBeforeReceipt) composer.pause(false);
  await act(async () => { finishRead({ items: [], total: 0, has_more: false }); });
  if (!releaseBeforeReceipt) {
    expect(composer.send).not.toHaveBeenCalled();
    expect(JSON.parse(sessionStorage.getItem(key)!)).toEqual(original);
    composer.pause(false);
  }
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(transcript).toHaveBeenCalledTimes(2);
  expect(composer.send.mock.calls[0][0]).toMatchObject({ conversation_id: conversation.id, idempotency_key: original.idempotency_key,
    files: original.files, initial_only: true });
  expect(sessionStorage.getItem(key)).toBeNull();
});

test('quick manual pause during capability admission cancels the old handoff before its POST', async () => {
  const conversation = freshConversation();
  const key = sessionStorageKey('initial-message-nomi', conversationTarget(conversation.id));
  const original = persistInitialMessageDelivery(sessionStorage, key, conversation.id, 'Continue the app after import', []);
  let finishAdmission!: (value: { skill_names: string[]; mcp_server_ids: string[] }) => void;
  const apply = mock((): Promise<AgentSessionCapabilitySelection> => Promise.resolve({ skill_names: [], mcp_server_ids: [] }));
  apply.mockImplementationOnce(() => new Promise(resolve => { finishAdmission = resolve; }));
  const state = { selection: { skill_names: [], mcp_server_ids: [] }, binding_version: 1, editable: true };
  const selection = { state, draft: { skillNames: [], mcpServerIds: [] }, loading: false, saving: false, error: undefined,
    setDraft: noop, retry: noop, applyBeforeSend: apply };
  spyOn(capabilities, 'useSessionCapabilitySelection').mockReturnValue(selection);
  const transcript = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  const composer = mountPausedComposer(conversation);
  await waitFor(() => expect(apply).toHaveBeenCalledTimes(1));
  const readsBeforePause = transcript.mock.calls.length;
  composer.pause(true); composer.pause(false);
  expect(composer.send).not.toHaveBeenCalled();
  expect(JSON.parse(sessionStorage.getItem(key)!)).toEqual(original);
  await act(async () => { finishAdmission(state.selection); });
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(transcript.mock.calls.length).toBeGreaterThan(readsBeforePause);
  expect(apply).toHaveBeenCalledTimes(2);
  expect(composer.send.mock.calls[0][0]).toMatchObject({ idempotency_key: original.idempotency_key, initial_only: true });
});

test('a fresh application prompt awaits product preparation before ordinary admission', async () => {
  const conversation = freshConversation();
  spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  let finishPreparation!: () => void;
  const prepare = mock(() => new Promise<void>(resolve => { finishPreparation = resolve; }));
  const composer = mountPausedComposer(conversation, false, prepare);
  const input = composer.view.getByTestId('sendbox-input') as HTMLTextAreaElement;
  await waitFor(() => expect(input.disabled).toBe(false));
  fireEvent.change(input, { target: { value: 'Change the Snake game' } });
  fireEvent.keyDown(input, { key: 'Enter' });
  await waitFor(() => expect(prepare).toHaveBeenCalledTimes(1));
  expect(composer.send).not.toHaveBeenCalled();
  await act(async () => finishPreparation());
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.send.mock.calls[0][0]).toMatchObject({ conversation_id: conversation.id, initial_only: false });
});

test('a rejected product preparation preserves the unsent prompt and does not retry admission', async () => {
  const conversation = freshConversation();
  spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  const prepare = mock(async () => { throw new Error('Application workflow refresh failed'); });
  const composer = mountPausedComposer(conversation, false, prepare);
  const input = composer.view.getByTestId('sendbox-input') as HTMLTextAreaElement;
  await waitFor(() => expect(input.disabled).toBe(false));
  fireEvent.change(input, { target: { value: 'Keep this unsent application change' } });
  fireEvent.keyDown(input, { key: 'Enter' });
  await waitFor(() => expect(prepare).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(input.value).toBe('Keep this unsent application change'));
  expect(composer.send).not.toHaveBeenCalled();
});

test.each(['initial', 'queued'] as const)('restored %s delivery preserves its authority without product preparation', async kind => {
  const conversation = freshConversation();
  if (kind === 'initial') {
    const key = sessionStorageKey('initial-message-nomi', conversationTarget(conversation.id));
    persistInitialMessageDelivery(sessionStorage, key, conversation.id, 'The admitted application request', []);
  } else {
    const key = sessionStorageKey('command-queue', conversationTarget(conversation.id));
    sessionStorage.setItem(key, JSON.stringify({ items: [createQueuedCommandItem({ input: 'A queued application change', files: [] })], isPaused: false }));
  }
  spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  const prepare = mock(async () => {});
  const composer = mountPausedComposer(conversation, kind === 'queued', prepare);
  if (kind === 'queued') {
    await composer.view.findAllByText('A queued application change');
    composer.pause(false);
  }
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(prepare).not.toHaveBeenCalled();
});

test('manual file work pauses the existing command queue until ordinary runtime reconciliation releases it', async () => {
  const conversation = freshConversation();
  const queued = createQueuedCommandItem({ input: 'Keep this queued app change', files: [] });
  const key = sessionStorageKey('command-queue', conversationTarget(conversation.id));
  sessionStorage.setItem(key, JSON.stringify({ items: [queued], isPaused: false }));
  spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  const composer = mountPausedComposer(conversation, true);
  await composer.view.findAllByText(queued.input);
  expect(composer.send).not.toHaveBeenCalled();
  expect(JSON.parse(sessionStorage.getItem(key)!)).toMatchObject({ items: [queued] });
  composer.pause(false);
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.send.mock.calls[0][0]).toMatchObject({ conversation_id: conversation.id, idempotency_key: queued.id, initial_only: false });
});

test('a pause after the POST begins preserves unknown-outcome handoff without automatically dispatching again', async () => {
  const conversation = freshConversation();
  const key = sessionStorageKey('initial-message-nomi', conversationTarget(conversation.id));
  const original = persistInitialMessageDelivery(sessionStorage, key, conversation.id, 'Keep the original delivery identity', []);
  const transcript = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  const composer = mountPausedComposer(conversation);
  let loseResponse!: (error: Error) => void;
  composer.send.mockImplementationOnce(() => new Promise((_, reject) => { loseResponse = reject; }));
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  const readsBeforePause = transcript.mock.calls.length;
  composer.pause(true); composer.pause(false);
  await act(async () => { loseResponse(new Error('Accepted response lost')); });
  expect(composer.send).toHaveBeenCalledTimes(1);
  expect(transcript).toHaveBeenCalledTimes(readsBeforePause);
  expect(JSON.parse(sessionStorage.getItem(key)!)).toEqual(original);
});

test('an explicit app prompt stays editable when manual work begins during capability admission', async () => {
  const conversation = freshConversation();
  let finishAdmission!: (value: AgentSessionCapabilitySelection) => void;
  const apply = mock((): Promise<AgentSessionCapabilitySelection> => Promise.resolve({ skill_names: [], mcp_server_ids: [] }));
  apply.mockImplementationOnce(() => new Promise(resolve => { finishAdmission = resolve; }));
  const state = { selection: { skill_names: [], mcp_server_ids: [] }, binding_version: 1, editable: true };
  spyOn(capabilities, 'useSessionCapabilitySelection').mockReturnValue({ state, draft: { skillNames: [], mcpServerIds: [] },
    loading: false, saving: false, error: undefined, setDraft: noop, retry: noop, applyBeforeSend: apply });
  const composer = mountPausedComposer(conversation);
  const input = composer.view.getByTestId('sendbox-input') as HTMLTextAreaElement;
  fireEvent.change(input, { target: { value: 'Keep my pending application request' } });
  await waitFor(() => expect(input.value).toBe('Keep my pending application request'));
  fireEvent.keyDown(input, { key: 'Enter' });
  await waitFor(() => expect(apply).toHaveBeenCalledTimes(1));
  composer.pause(true);
  await act(async () => { finishAdmission(state.selection); });
  expect(composer.send).not.toHaveBeenCalled();
  expect(input.value).toBe('Keep my pending application request');
  composer.pause(false);
  fireEvent.keyDown(input, { key: 'Enter' });
  await waitFor(() => expect(composer.send).toHaveBeenCalledTimes(1));
  expect(composer.send.mock.calls[0][0]).toMatchObject({ conversation_id: conversation.id, initial_only: false });
});
