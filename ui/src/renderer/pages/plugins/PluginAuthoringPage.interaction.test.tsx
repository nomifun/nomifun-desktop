import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { agentPlatform, conversation } from '@/common/adapter/ipcBridge';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type { IProvider, TChatConversation, TProviderWithModel } from '@/common/config/storage';
import { parseConversationId } from '@/common/types/ids';
import * as agentCatalog from '@/renderer/hooks/agent/useAgentPresets';
import * as modelCatalog from '@/renderer/hooks/agent/useModelsForTask';
import * as providerCatalog from '@/renderer/hooks/agent/useModelProviderList';
import * as chat from '@/renderer/pages/conversation/platforms/nomi/NomiChat';
import * as preview from '@/renderer/pages/conversation/Preview';
import * as modelSelector from '@/renderer/components/chat/ChatModelSelector';
import * as nomiModel from '@/renderer/pages/conversation/platforms/nomi/useNomiModelSelection';
import * as workspace from './PluginWorkspace';
import * as artifacts from './PluginAuthoringArtifacts';
import PluginAuthoringPage from './PluginAuthoringPage';
import en from '../../services/i18n/locales/en-US/pluginPlatform.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { pluginPlatform: en } } } });
const id = parseConversationId('0190f5fe-7c00-7a00-8000-000000000202');
const model = { id: 'test-provider', name: 'Test', platform: 'openai', models: [], use_model: 'test-model' } as unknown as TProviderWithModel & IProvider;
const projection = { id, type: 'nomi', name: 'Plugin task', extra: { workspace: '' }, model, created_at: 1, modified_at: 1 } as TChatConversation;

beforeEach(() => {
  (window as typeof window & { __backendPort?: number }).__backendPort = 11451;
  spyOn(agentCatalog, 'useAgentPresets').mockReturnValue({ library: undefined, presets: [], isLoading: false, error: undefined, refresh: async () => {} });
  spyOn(modelCatalog, 'useModelsForTask').mockReturnValue({ groups: [{ provider: model, models: [model.use_model] }], isLoading: false, refresh: () => {} });
  spyOn(workspace, 'default').mockImplementation(({ children }) => <div>{children}</div>);
  spyOn(chat, 'default').mockImplementation(({ conversation_id, hideSendBox }) => <div>Canonical messages {conversation_id} {String(hideSendBox)}</div>);
  spyOn(preview, 'PreviewProvider').mockImplementation(({ children }) => <div>{children}</div>);
  spyOn(modelSelector, 'default').mockImplementation(() => <div>Model selection</div>);
  spyOn(nomiModel, 'useNomiModelSelection').mockImplementation(({ initialModel, onSelectModel, readOnly }) => ({
    current_model: initialModel, providers: [model], pickerDisabled: Boolean(readOnly), getAvailableModels: () => [model.use_model],
    handleSelectModel: async (provider, name) => { await onSelectModel(provider, name); }, getDisplayModelName: name => name ?? '',
  }));
  spyOn(artifacts, 'default').mockImplementation(({ conversationId }) => <div>Plugin artifacts {conversationId}</div>);
  spyOn(conversation.turnStarted, 'on').mockImplementation(() => () => {});
  spyOn(conversation.turnPaused, 'on').mockImplementation(() => () => {});
  spyOn(conversation.turnCompleted, 'on').mockImplementation(() => () => {});
  spyOn(conversation.reconnected, 'on').mockImplementation(() => () => {});
  spyOn(pluginPlatform.authoring.preflight, 'invoke').mockResolvedValue({ status: 'ready', reason: '', owner_user_id: 'owner', selection: { kind: 'template', templateKey: 'assistant.general' } });
  spyOn(pluginPlatform.authoring.createSession, 'invoke').mockResolvedValue({ agent_session_id: id });
  spyOn(pluginPlatform.authoring.getSession, 'invoke').mockResolvedValue({ agent_session_id: id });
  spyOn(conversation.get, 'invoke').mockResolvedValue(projection);
  spyOn(agentPlatform.sessions.getExecution, 'invoke').mockResolvedValue(null);
  spyOn(conversation.sendMessage, 'invoke').mockResolvedValue({ msg_id: id, status: 'accepted' } as never);
});
afterEach(() => { cleanup(); mock.restore(); delete (window as typeof window & { __backendPort?: number }).__backendPort; });

function mount(path = '/plugins/create') {
  const Location = () => <output data-testid='location'>{useLocation().pathname}{useLocation().search}</output>;
  return render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={[path]}><Location /><Routes>
    <Route path='/plugins/create' element={<PluginAuthoringPage />} />
    <Route path='/plugins/authoring/:sessionId' element={<PluginAuthoringPage />} />
  </Routes></MemoryRouter></I18nextProvider>);
}

function realReasoningPicker() {
  spyOn(modelSelector, 'default').mockRestore();
  const provider = { ...model, models: [{
    model: model.use_model, provider_id: model.id, enabled: true, sort_order: 0, created_at: 1, updated_at: 1,
    capabilities: [{ task: 'chat', traits: [], protocol: 'openai.chat_text', connection_role: 'default',
      allow_cross_origin_credentials: false, provider_params: {}, created_at: 1, updated_at: 1 }],
  }] } as IProvider;
  spyOn(modelCatalog, 'useModelsForTask').mockReturnValue({ groups: [{ provider, models: [model.use_model] }], isLoading: false, refresh: () => {} });
  spyOn(providerCatalog, 'useProvidersQuery').mockReturnValue({ data: [provider], isLoading: false, isValidating: false, mutate: async () => [provider] } as ReturnType<typeof providerCatalog.useProvidersQuery>);
}

test('first requirement creates one plugin-owned canonical session and remains in the plugin workspace', async () => {
  const ordinaryCreate = spyOn(agentPlatform.sessions.create, 'invoke');
  const create = spyOn(pluginPlatform.authoring.createSession, 'invoke');
  const send = spyOn(conversation.sendMessage, 'invoke');
  const view = mount();
  fireEvent.input(view.getByRole('textbox', { name: en.authoring.requirement }), { target: { value: 'Create a todo app' } });
  await waitFor(() => expect((view.getByRole('button', { name: en.authoring.sendRequirement }) as HTMLButtonElement).disabled).toBe(false));
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.sendRequirement })); });
  await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
  expect(create).toHaveBeenCalledTimes(1);
  expect(create.mock.calls[0][0]).toMatchObject({ selection: { kind: 'template', templateKey: 'assistant.general' }, model: { provider_id: model.id, model: model.use_model } });
  expect(send.mock.calls[0][0]).toMatchObject({ conversation_id: id, input: 'Create a todo app', plugin_delivery: {} });
  expect(ordinaryCreate).not.toHaveBeenCalled();
  expect(view.getByTestId('location').textContent).toBe(`/plugins/authoring/${id}`);
  expect(view.getByText(`Canonical messages ${id} true`)).toBeTruthy();
  expect(view.getByText(`Plugin artifacts ${id}`)).toBeTruthy();
});

test('an explicitly disabled Agent cannot create a session or silently receive a grant', async () => {
  spyOn(pluginPlatform.authoring.preflight, 'invoke').mockResolvedValue({ status: 'configure_agent', reason: 'PLUGIN_MODULE_DISABLED', owner_user_id: 'owner', selection: { kind: 'template', templateKey: 'assistant.general' } });
  const create = spyOn(pluginPlatform.authoring.createSession, 'invoke');
  const view = mount();
  await view.findByText(en.authoring.moduleNeeded);
  fireEvent.input(view.getByRole('textbox', { name: en.authoring.requirement }), { target: { value: 'Build a timer' } });
  expect((view.getByRole('button', { name: en.authoring.sendRequirement }) as HTMLButtonElement).disabled).toBe(true);
  expect(create).not.toHaveBeenCalled();
  expect(view.getByTestId('location').textContent).toBe('/plugins/create');
});

test('draft continuation goes through the product command and retains the returned original session', async () => {
  const create = spyOn(pluginPlatform.authoring.createSession, 'invoke');
  const view = mount('/plugins/create?draft_id=existing-draft');
  await waitFor(() => expect((view.getByRole('button', { name: en.authoring.openDraft }) as HTMLButtonElement).disabled).toBe(false));
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.openDraft })); });
  await view.findByText(`Plugin artifacts ${id}`);
  expect(create.mock.calls[0][0]).toMatchObject({ draft_id: 'existing-draft' });
  expect(view.getByTestId('location').textContent).toBe(`/plugins/authoring/${id}?draft_id=existing-draft`);
});

test('an arbitrary ordinary session URL fails product ownership validation before loading history', async () => {
  spyOn(pluginPlatform.authoring.getSession, 'invoke').mockRejectedValue(new Error('PLUGIN_AUTHORING_SESSION_REQUIRED'));
  const get = spyOn(conversation.get, 'invoke');
  const view = mount(`/plugins/authoring/${id}`);
  await view.findByText('PLUGIN_AUTHORING_SESSION_REQUIRED');
  expect(get).not.toHaveBeenCalled();
  expect(view.queryByText(`Canonical messages ${id} true`) === null).toBe(true);
});

test('remote WebUI cannot create or resume an authoring session', async () => {
  delete (window as typeof window & { __backendPort?: number }).__backendPort;
  const create = spyOn(pluginPlatform.authoring.createSession, 'invoke');
  const get = spyOn(pluginPlatform.authoring.getSession, 'invoke');
  const view = mount();
  await view.findByText(en.readOnly.body);
  expect((view.getByRole('textbox', { name: en.authoring.requirement }) as HTMLTextAreaElement).disabled).toBe(true);
  expect(create).not.toHaveBeenCalled(); expect(get).not.toHaveBeenCalled();
});

test('a lost session creation response retains the same idempotency key for an unchanged retry', async () => {
  const create = spyOn(pluginPlatform.authoring.createSession, 'invoke').mockRejectedValueOnce(new Error('Response lost'));
  const view = mount();
  fireEvent.input(view.getByRole('textbox', { name: en.authoring.requirement }), { target: { value: 'Build a timer' } });
  await waitFor(() => expect((view.getByRole('button', { name: en.authoring.sendRequirement }) as HTMLButtonElement).disabled).toBe(false));
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.sendRequirement })); });
  await view.findByText('Response lost');
  create.mockResolvedValue({ agent_session_id: id });
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.sendRequirement })); });
  await waitFor(() => expect(create).toHaveBeenCalledTimes(2));
  expect(create.mock.calls[0][0].idempotency_key).toBe(create.mock.calls[1][0].idempotency_key);
});

test('a safely paused product task resumes its exact checkpoint with supplemental input', async () => {
  spyOn(agentPlatform.sessions.getExecution, 'invoke').mockResolvedValue({
    operation_id: 'original-operation', state: 'paused', checkpoint_revision: 7,
    checkpoint_digest: 'a'.repeat(64) as never, checkpoint_retained: true,
    pause: { revision: 3, reason: 'PLUGIN_VERIFICATION_REQUIRED', cleanup_proven: true },
  });
  const resume = spyOn(pluginPlatform.authoring.continueWithInput, 'invoke').mockResolvedValue({ operation_id: 'original-operation' } as never);
  const send = spyOn(conversation.sendMessage, 'invoke');
  const view = mount(`/plugins/authoring/${id}`);
  await view.findByText(`Canonical messages ${id} true`);
  fireEvent.input(view.getByRole('textbox', { name: en.authoring.requirement }), { target: { value: 'Keep the data and repair the delete button' } });
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.replyContinue })); });
  await waitFor(() => expect(resume).toHaveBeenCalledTimes(1));
  expect(resume.mock.calls[0][0]).toMatchObject({ agent_session_id: id, input: { content: 'Keep the data and repair the delete button' },
    request: { operation_id: 'original-operation', expected_pause_revision: 3, expected_checkpoint_revision: 7, expected_checkpoint_digest: 'a'.repeat(64) } });
  expect(send).not.toHaveBeenCalled();
});

test('saving source holds the page submission gate until its whole operation settles', async () => {
  let finish!: () => void;
  spyOn(artifacts, 'default').mockImplementation(({ onAuthoringOperation }) => <button onClick={() => void onAuthoringOperation(async () => {
    await new Promise<void>(resolve => { finish = resolve; });
  })}>Save source fixture</button>);
  const send = spyOn(conversation.sendMessage, 'invoke');
  const view = mount(`/plugins/authoring/${id}`);
  await view.findByText('Save source fixture');
  fireEvent.input(view.getByRole('textbox', { name: en.authoring.requirement }), { target: { value: 'Another requirement' } });
  fireEvent.click(view.getByRole('button', { name: 'Save source fixture' }));
  expect((view.getByRole('button', { name: en.authoring.sendRequirement }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(view.getByRole('button', { name: en.authoring.sendRequirement }));
  expect(send).not.toHaveBeenCalled();
  await act(async () => { finish(); });
  expect((view.getByRole('button', { name: en.authoring.sendRequirement }) as HTMLButtonElement).disabled).toBe(false);
});

test('a lost pause-resume response retries the retained checkpoint even after a refreshed state changes', async () => {
  const pause = {
    operation_id: 'original-operation', state: 'paused', checkpoint_revision: 7,
    checkpoint_digest: 'a'.repeat(64) as never, checkpoint_retained: true,
    pause: { revision: 3, reason: 'PLUGIN_VERIFICATION_REQUIRED', cleanup_proven: true },
  };
  const get = spyOn(agentPlatform.sessions.getExecution, 'invoke').mockResolvedValue(pause);
  const resume = spyOn(pluginPlatform.authoring.continueWithInput, 'invoke').mockRejectedValueOnce(new Error('Resume response lost'));
  const send = spyOn(conversation.sendMessage, 'invoke');
  const view = mount(`/plugins/authoring/${id}`);
  await view.findByText(`Canonical messages ${id} true`);
  fireEvent.input(view.getByRole('textbox', { name: en.authoring.requirement }), { target: { value: 'Repair existing work' } });
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.replyContinue })); });
  await view.findByText('Resume response lost');
  get.mockResolvedValue(null);
  resume.mockResolvedValue({ operation_id: 'original-operation' } as never);
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.replyContinue })); });
  await waitFor(() => expect(resume).toHaveBeenCalledTimes(2));
  expect(resume.mock.calls[0][0]).toEqual(resume.mock.calls[1][0]);
  expect(send).not.toHaveBeenCalled();
});

test('an edit submission retry retains its original prefixed input after the product session was created', async () => {
  const send = spyOn(conversation.sendMessage, 'invoke').mockRejectedValueOnce(new Error('Send response lost'));
  const view = mount('/plugins/create?plugin_id=existing-plugin&expected_plugin_revision=3');
  fireEvent.input(view.getByRole('textbox', { name: en.authoring.requirement }), { target: { value: 'Add a delete button' } });
  await waitFor(() => expect((view.getByRole('button', { name: en.authoring.sendRequirement }) as HTMLButtonElement).disabled).toBe(false));
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.sendRequirement })); });
  await view.findByText('Send response lost');
  send.mockResolvedValue({ msg_id: id, status: 'accepted' } as never);
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.sendRequirement })); });
  await waitFor(() => expect(send).toHaveBeenCalledTimes(2));
  expect(send.mock.calls[0][0]).toEqual(send.mock.calls[1][0]);
});

test('the real reasoning menu submits its selected medium effort in canonical product session creation', async () => {
  realReasoningPicker();
  const create = spyOn(pluginPlatform.authoring.createSession, 'invoke');
  const view = mount();
  await view.findByTestId('chat-model-selector-reasoning-trigger');
  fireEvent.click(view.getByTestId('chat-model-selector-reasoning-trigger'));
  fireEvent.click(await view.findByTestId('chat-model-selector-reasoning-medium'));
  await waitFor(() => expect(view.getByTestId('chat-model-selector-reasoning-value').textContent).toContain('medium'));
  fireEvent.input(view.getByRole('textbox', { name: en.authoring.requirement }), { target: { value: 'Create a todo app' } });
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.sendRequirement })); });
  await waitFor(() => expect(create).toHaveBeenCalledTimes(1));
  expect(create.mock.calls[0][0].reasoning_effort).toBe('medium');
});

test.each(['idle', 'running', 'paused', 'pending'] as const)('the real reasoning menu persists only editable idle sessions (%s)', async mode => {
  realReasoningPicker();
  if (mode === 'running' || mode === 'paused') {
    spyOn(agentPlatform.sessions.getExecution, 'invoke').mockResolvedValue({
      state: mode, operation_id: 'existing-operation', checkpoint_revision: 1, checkpoint_retained: true,
      checkpoint_digest: 'a'.repeat(64) as never,
      pause: mode === 'paused' ? { revision: 1, reason: 'PLUGIN_VERIFICATION_REQUIRED', cleanup_proven: true } : null,
    });
  }
  const update = spyOn(agentPlatform.sessions.updateReasoning, 'invoke').mockImplementation(async () => {
    spyOn(conversation.get, 'invoke').mockResolvedValue({ ...projection, reasoning_effort: 'medium' });
    return { reasoning_effort: 'medium' };
  });
  const view = mount(`/plugins/authoring/${id}`);
  await view.findByText(`Canonical messages ${id} true`);
  await view.findByTestId('chat-model-selector-reasoning-trigger');
  if (mode === 'pending') {
    spyOn(conversation.sendMessage, 'invoke').mockImplementation(() => new Promise(() => {}));
    fireEvent.input(view.getByRole('textbox', { name: en.authoring.requirement }), { target: { value: 'Create another output' } });
    fireEvent.click(view.getByRole('button', { name: en.authoring.sendRequirement }));
  }
  fireEvent.click(view.getByTestId('chat-model-selector-reasoning-trigger'));
  if (mode !== 'idle') {
    expect(view.queryByTestId('chat-model-selector-reasoning-medium') === null).toBe(true);
    expect(update).not.toHaveBeenCalled();
  } else {
    fireEvent.click(await view.findByTestId('chat-model-selector-reasoning-medium'));
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
    expect(update.mock.calls[0][0]).toEqual({ agent_session_id: id, reasoning_effort: 'medium' });
    await waitFor(() => expect(view.getByTestId('chat-model-selector-reasoning-value').textContent).toContain('medium'));
  }
});

test('an idea fills the composer and the keyboard shortcut creates the task', async () => {
  const send = spyOn(conversation.sendMessage, 'invoke');
  const view = mount();
  fireEvent.click(view.getByRole('button', { name: i18n.t('pluginPlatform.authoring.ideas.timer.title') }));
  const input = view.getByRole('textbox', { name: en.authoring.requirement }) as HTMLTextAreaElement;
  expect(input.value).toBe(i18n.t('pluginPlatform.authoring.ideas.timer.prompt'));
  expect(document.activeElement).toBe(input);
  await waitFor(() => expect((view.getByRole('button', { name: en.authoring.sendRequirement }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.keyDown(input, { key: 'Enter', ctrlKey: true });
  await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
  expect(send.mock.calls[0][0].input).toBe(i18n.t('pluginPlatform.authoring.ideas.timer.prompt'));
});

test('collapsing the app pane retains the task and composer input', async () => {
  const view = mount(`/plugins/authoring/${id}`);
  await view.findByText(`Plugin artifacts ${id}`);
  const input = view.getByRole('textbox', { name: en.authoring.requirement }) as HTMLTextAreaElement;
  fireEvent.input(input, { target: { value: 'Keep these unsent changes' } });
  fireEvent.click(view.getByRole('button', { name: i18n.t('pluginPlatform.authoring.hideArtifacts') }));
  const pane = view.container.querySelector('#plugin-authoring-artifacts') as HTMLElement;
  expect(pane.hidden).toBe(true);
  expect(view.getByText(`Canonical messages ${id} true`)).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: i18n.t('pluginPlatform.authoring.showArtifacts') }));
  expect(pane.hidden).toBe(false);
  expect(input.value).toBe('Keep these unsent changes');
});

test('a lost user-stop continuation response retains its checkpoint across retries', async () => {
  const paused = { operation_id: 'user-stopped-task', state: 'paused', checkpoint_revision: 5,
    checkpoint_digest: 'b'.repeat(64) as never, checkpoint_retained: true,
    pause: { revision: 2, reason: 'EXECUTION_USER_REQUESTED', cleanup_proven: true } };
  const get = spyOn(agentPlatform.sessions.getExecution, 'invoke').mockResolvedValue(paused);
  const resume = spyOn(pluginPlatform.authoring.continueWithInput, 'invoke').mockRejectedValueOnce(new Error('Continue response lost'));
  const view = mount(`/plugins/authoring/${id}`);
  await view.findByText(`Canonical messages ${id} true`);
  fireEvent.input(view.getByRole('textbox', { name: en.authoring.requirement }), { target: { value: 'Continue from where we stopped' } });
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.replyContinue })); });
  await view.findByText('Continue response lost');
  get.mockResolvedValue(null);
  resume.mockResolvedValue({ operation_id: 'user-stopped-task' } as never);
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.authoring.replyContinue })); });
  await waitFor(() => expect(resume).toHaveBeenCalledTimes(2));
  expect(resume.mock.calls[0][0]).toEqual(resume.mock.calls[1][0]);
  expect(resume.mock.calls[0][0].request).toMatchObject({ operation_id: 'user-stopped-task', expected_checkpoint_revision: 5, expected_pause_revision: 2 });
});
