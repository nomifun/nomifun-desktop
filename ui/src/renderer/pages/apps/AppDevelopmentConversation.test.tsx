import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance, type TFunction } from 'i18next';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { StrictMode, type ComponentProps, type Dispatch, type SetStateAction } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useLocation, useNavigate, type NavigateFunction } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { TChatConversation, TProviderWithModel } from '@/common/config/storage';
import type { HostedApp } from '@/common/types/appHosting';
import type { OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { conversationTarget, parseConversationId, parseMessageId, parseProviderId } from '@/common/types/ids';
import { sessionStorageKey, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { uuidv7 } from '@/common/utils';
import * as GuidModule from '@/renderer/pages/guid/GuidPage';
import * as ChatModule from '@/renderer/pages/conversation/components/ChatConversation';
import * as FilePanelModule from './AppWorkspaceFiles';
import { useGuidDraftState } from '@/renderer/pages/guid/hooks/useGuidDraftState';
import { useGuidSend, type GuidSendDeps, type GuidSendResult } from '@/renderer/pages/guid/hooks/useGuidSend';
import { useGuidCreation } from '@/renderer/creation/useGuidCreation';
import { creationDraftStorageKey, emptyCreationDraft } from '@/renderer/creation/useCreationDraft';
import type { useGuidAgentSelection } from '@/renderer/pages/guid/hooks/useGuidAgentSelection';
import { CreationComposerContext, type CreationComposerValue } from '@/renderer/creation/CreationComposerContext';
import { ComposerSceneHeader } from '@/renderer/creation/ComposerSceneSelector';
import locale from '../../services/i18n/locales/en-US/appHosting.json';
import AppDevelopmentConversation, { matchesAppDevelopmentConversation } from './AppDevelopmentConversation';
import AppWorkspaceRoute from './AppWorkspaceRoute';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { appHosting: locale } } }, interpolation: { escapeValue: false } });
const SESSION = parseConversationId('0190f5fe-7c00-7a00-8000-000000000301');
const OTHER_SESSION = parseConversationId('0190f5fe-7c00-7a00-8000-000000000302');
const PROVIDER = parseProviderId('0190f5fe-7c00-7a00-8000-000000000303');
const model: TProviderWithModel = { id: PROVIDER, name: 'Configured provider', platform: 'stepfun', base_url: 'https://example.invalid/v1', auth_scheme: 'bearer', has_credentials: true, enabled: true, use_model: 'chat-model' };
const app: HostedApp = { id: 'owned-app', name: 'Reminder', description: '', tags: [], development_session_id: SESSION,
  directory: 'C:/NomiFun/apps/owned-app', import_kind: 'source', settings: { project_directory: '.', environment: {} },
  state: 'stopped', operation: null, last_error: null, output: '', pid: null, preview: false, prepared: false };
const conversation: TChatConversation = { id: SESSION, name: 'A normal development conversation', type: 'nomi', session_purpose: 'conversation', created_at: 1, modified_at: 1, model,
  extra: { workspace: app.directory, official_template_key: 'app.developer' } as TChatConversation['extra'] };
let entry: GuidModule.GuidProductEntry | undefined;
let renderedChat: ComponentProps<typeof ChatModule.default> | undefined;
let setScopedDraft: Dispatch<SetStateAction<string>>;
let scopedSend: GuidSendResult;
const wrapper = ({ children }: { children: React.ReactNode }) => <MemoryRouter><I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0, shouldRetryOnError: false }}>{children}</SWRConfig></I18nextProvider></MemoryRouter>;

beforeEach(() => {
  entry = undefined; renderedChat = undefined;
  setBrowserStorageGeneration(uuidv7()); sessionStorage.clear();
  // These component spies only capture the existing composition interface.
  // Session admission and its initial-message queue use the real hook below.
  spyOn(GuidModule, 'default').mockImplementation(({ productEntry }) => {
    entry = productEntry;
    const [draft, setDraft] = useGuidDraftState('input', '');
    setScopedDraft = setDraft;
    const noop = () => {};
    scopedSend = useGuidSend({ input: draft, setInput: setDraft, files: [], setFiles: noop, dir: '', setDir: noop, loading: productEntry?.submissionDisabled ?? false,
      setLoading: noop, selection: { kind: 'template', templateKey: 'app.developer' }, selectedPreset: undefined, selectedTemplate: { template_key: 'app.developer' } as OfficialPresetTemplate,
      current_model: model, autoWork: { enabled: false }, workspaceEnabled: false, resourceResolutionReady: true, resourceSelections: [],
      setMentionOpen: noop, setMentionQuery: noop, setMentionSelectorOpen: noop, setMentionActiveIndex: noop, navigate: mock(() => {}) as NavigateFunction, t: i18n.t as TFunction,
      createConversation: productEntry?.createConversation, onConversationCreated: productEntry?.onConversationCreated });
    return <><textarea aria-label='Scoped application prompt' value={draft} onChange={(event) => setDraft(event.target.value)} />
      <button disabled={scopedSend.isButtonDisabled} onClick={scopedSend.sendMessageHandler}>Send scoped application prompt</button></>;
  });
  spyOn(ChatModule, 'default').mockImplementation((props) => {
    renderedChat = props; return <div data-testid='ordinary-conversation'>{props.conversation?.id}</div>;
  });
  spyOn(ipcBridge.conversation.listChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.turnStarted, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.turnCompleted, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.turnPaused, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(() => () => {});
  spyOn(appHosting.developmentSessions, 'invoke').mockResolvedValue({ items: [{ conversation, editable_in_app: true, archived: false }], total: 1, has_more: false, next_cursor: null, project_directory: app.directory, current_session_id: SESSION });
  spyOn(ipcBridge.database.getUserConversations, 'invoke').mockResolvedValue({ items: [], total: 0, has_more: false });
  spyOn(ipcBridge.agentPlatform.library, 'invoke').mockResolvedValue({ user_presets: [], official_presets: {}, official_templates: [], active_bindings: [], fresh_start: { data_generation: 6, legacy_data_imported: false, user_preset_count: 0, official_template_count: 0 } });
});
afterEach(() => { cleanup(); mock.restore(); sessionStorage.clear(); });

test('development identity requires the exact owner Session, host-verified template and workspace', () => {
  expect(matchesAppDevelopmentConversation(app, conversation)).toBe(true);
  expect(matchesAppDevelopmentConversation(app, { ...conversation, id: OTHER_SESSION })).toBe(false);
  expect(matchesAppDevelopmentConversation(app, { ...conversation, extra: { workspace: 'C:/another-app', official_template_key: 'app.developer' } as TChatConversation['extra'] })).toBe(false);
  expect(matchesAppDevelopmentConversation(app, { ...conversation, name: 'Built-in app', extra: { workspace: app.directory } })).toBe(false);
  expect(matchesAppDevelopmentConversation(app, { ...conversation, extra: { workspace: app.directory, official_template_key: 'coding.codex' } as TChatConversation['extra'] })).toBe(false);
  expect(matchesAppDevelopmentConversation(app, { ...conversation, extra: { workspace: 'C:\\NomiFun\\apps\\owned-app\\', official_template_key: 'app.developer' } as TChatConversation['extra'] })).toBe(true);
});

test('the app draft is isolated from the ordinary Guid prompt and survives closing its surface', () => {
  const ordinary = renderHook(() => useGuidDraftState('input', ''));
  act(() => ordinary.result.current[1]('Keep my ordinary conversation prompt'));
  const view = render(<AppDevelopmentConversation app={{ ...app, development_session_id: null }} mode='create' onAppChanged={() => {}} />, { wrapper });
  const input = view.getByRole('textbox', { name: 'Scoped application prompt' }) as HTMLTextAreaElement;
  expect(input.value).toBe('');
  act(() => setScopedDraft('Build my reminder app'));
  expect(ordinary.result.current[0]).toBe('Keep my ordinary conversation prompt');
  view.unmount();
  const reopened = render(<AppDevelopmentConversation app={{ ...app, development_session_id: null }} mode='edit' onAppChanged={() => {}} />, { wrapper });
  expect((reopened.getByRole('textbox') as HTMLTextAreaElement).value).toBe('Build my reminder app');
  expect(entry?.templateKey).toBe('app.developer');
  expect(entry?.workspace).toBe(app.directory);
});

test('first app message uses the owner admission API and existing ordinary initial-message queue', async () => {
  const createApp = spyOn(appHosting.create, 'invoke').mockResolvedValue({ ...app, development_session_id: null });
  const open = spyOn(appHosting.developmentSession, 'invoke').mockResolvedValue({ app, conversation });
  const genericCreate = spyOn(ipcBridge.agentPlatform.sessions.create, 'invoke');
  const deleteSession = spyOn(ipcBridge.agentPlatform.sessions.delete, 'invoke');
  const deleteApp = spyOn(appHosting.delete, 'invoke');
  let accepted: HostedApp | undefined;
  const view = render(<AppDevelopmentConversation mode='import' onAppChanged={(value) => { accepted = value; }} />, { wrapper });
  expect(createApp).not.toHaveBeenCalled(); expect(open).not.toHaveBeenCalled();
  const noop = () => {};
  const request: GuidSendDeps = {
    input: 'Help me develop a NomiFun reminder app', files: [], dir: '', current_model: model, selection: { kind: 'template', templateKey: 'app.developer' }, selectedPreset: undefined,
    selectedTemplate: { template_key: 'app.developer' } as OfficialPresetTemplate, autoWork: { enabled: false }, loading: false,
    workspaceEnabled: false, resourceResolutionReady: true, resourceSelections: [], reasoningEffort: 'high',
    setInput: noop, setFiles: noop, setDir: noop, setLoading: noop, setMentionOpen: noop, setMentionQuery: noop, setMentionSelectorOpen: noop, setMentionActiveIndex: noop,
    navigate: mock(() => {}) as NavigateFunction, t: i18n.t as TFunction,
    createConversation: entry!.createConversation, onConversationCreated: entry!.onConversationCreated,
  };
  const send = renderHook(() => useGuidSend(request));
  await act(async () => { await send.result.current.handleSend(); });
  expect(createApp).toHaveBeenCalledTimes(1);
  expect(open.mock.calls[0][0]).toEqual({ id: app.id, model: { provider_id: PROVIDER, model: 'chat-model' }, reasoning_effort: 'high', refresh_workflow: true });
  expect(genericCreate).not.toHaveBeenCalled();
  expect(accepted).toEqual(app);
  const queued = JSON.parse(sessionStorage.getItem(sessionStorageKey('initial-message-nomi', conversationTarget(SESSION)))!);
  expect(queued).toMatchObject({ conversation_id: SESSION, initial_admission_epoch: 0, input: request.input });
  expect(queued.idempotency_key).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  expect(request.navigate).not.toHaveBeenCalled();
  view.unmount();
  expect(deleteSession).not.toHaveBeenCalled(); expect(deleteApp).not.toHaveBeenCalled();
});

test('existing development Session renders the ordinary chat embedded and keeps model controls', async () => {
  const get = spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(conversation);
  const view = render(<AppDevelopmentConversation app={app} mode='edit' onAppChanged={() => {}} />, { wrapper });
  await view.findByTestId('ordinary-conversation');
  await waitFor(() => expect(get).toHaveBeenCalled());
  expect(get.mock.calls[0][0]).toEqual({ conversation_id: SESSION });
  expect(renderedChat?.conversation).toEqual(conversation);
  expect(renderedChat?.embedded).toBe(true);
  expect(renderedChat?.fixedAgentLabel).toBe(locale.workspace.fixedAgent);
  expect(entry).toBeUndefined();
});

test('a retained Session with the wrong owner workspace is rejected without launching another one', async () => {
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ ...conversation, extra: { workspace: 'C:/other', official_template_key: 'app.developer' } as TChatConversation['extra'] });
  const open = spyOn(appHosting.developmentSession, 'invoke');
  const view = render(<AppDevelopmentConversation app={app} mode='edit' onAppChanged={() => {}} />, { wrapper });
  await view.findByText(locale.workspace.sessionMismatch);
  expect(renderedChat).toBeUndefined(); expect(entry).toBeUndefined(); expect(open).not.toHaveBeenCalled();
});

test('disabled App Guid media handling never reads or rewrites the ordinary media draft', async () => {
  const key = creationDraftStorageKey('guid');
  const original = { ...emptyCreationDraft(), mode: 'image', pendingPrompt: 'Keep my unfinished illustration', pendingFiles: ['C:/references/art.png'],
    models: { image: { providerId: PROVIDER, model: 'configured-image-model' }, video: null, music: null },
    references: [{ asset_id: 'retained-image', kind: 'image', title: 'My reference', role: 'reference' }] };
  const bytes = JSON.stringify(original, null, 2);
  sessionStorage.setItem(key, bytes);
  const get = spyOn(sessionStorage, 'getItem');
  const createSession = spyOn(ipcBridge.agentPlatform.sessions.create, 'invoke');
  const agent: ReturnType<typeof useGuidAgentSelection> = { selection: { kind: 'template', templateKey: 'app.developer' }, selectedPreset: undefined, selectedTemplate: undefined,
    presets: [], draftPresets: [], officialTemplates: [], isLoading: false, isLoaded: true, loadError: undefined, setSelection: mock(() => {}), refreshPresets: async () => {} };
  const hookWrapper = ({ children }: { children: React.ReactNode }) => <MemoryRouter initialEntries={['/apps/owned-app?creation=video']}><SWRConfig value={{ provider: () => new Map(), fallback: { providers: [] }, revalidateOnMount: false, shouldRetryOnError: false }}>{children}</SWRConfig></MemoryRouter>;
  const hook = renderHook(() => ({ controller: useGuidCreation(agent, 'Build an application', [], app.directory, undefined, undefined, undefined, false), location: useLocation() }), { wrapper: hookWrapper });
  expect(hook.result.current.controller.draft.mode).toBeNull();
  expect(hook.result.current.controller.draft.pendingPrompt).toBeUndefined();
  expect(hook.result.current.controller.draft.pendingFiles).toBeUndefined();
  await act(async () => {
    const controller = hook.result.current.controller;
    controller.selectMode('video'); controller.setMode('music'); controller.exit();
    controller.update((draft) => ({ ...draft, references: [], pendingPrompt: 'Wrong draft' }));
    await controller.send();
  });
  expect(get.mock.calls.filter(([storedKey]) => storedKey === key)).toHaveLength(0);
  expect(agent.setSelection).not.toHaveBeenCalled(); expect(createSession).not.toHaveBeenCalled();
  expect(hook.result.current.location.search).toBe('?creation=video');
  get.mockRestore();
  expect(sessionStorage.getItem(key)).toBe(bytes);
});

test('known app creation survives Session admission failure and explicit retry reuses its owner', async () => {
  const createApp = spyOn(appHosting.create, 'invoke').mockResolvedValue({ ...app, development_session_id: null });
  const open = spyOn(appHosting.developmentSession, 'invoke').mockRejectedValueOnce(new Error('Session admission failed')).mockResolvedValueOnce({ app, conversation });
  let accepted: HostedApp | undefined;
  render(<AppDevelopmentConversation mode='import' onAppChanged={(value) => { accepted = value; }} />, { wrapper });
  const request = { title: 'Reminder', model: { provider_id: PROVIDER, model: 'chat-model' } };
  await expect(entry!.createConversation(request)).rejects.toThrow('Session admission failed');
  expect(createApp).toHaveBeenCalledTimes(1); expect(accepted).toBeUndefined();
  await act(async () => {
    const opened = await entry!.createConversation(request);
    entry!.onConversationCreated(opened);
  });
  expect(createApp).toHaveBeenCalledTimes(1);
  expect(open.mock.calls.map(([body]) => body.id)).toEqual([app.id, app.id]);
  expect(accepted).toEqual(app);
});

test('an unconfirmed app creation never invents a Session owner or calls Session admission', async () => {
  spyOn(appHosting.create, 'invoke').mockRejectedValue(new Error('App creation response unavailable'));
  const open = spyOn(appHosting.developmentSession, 'invoke');
  const changed = mock(() => {});
  render(<AppDevelopmentConversation mode='import' onAppChanged={changed} />, { wrapper });
  await expect(entry!.createConversation({ title: 'Reminder', model: { provider_id: PROVIDER, model: 'chat-model' } })).rejects.toThrow('App creation response unavailable');
  expect(open).not.toHaveBeenCalled(); expect(changed).not.toHaveBeenCalled();
});

test('app ingress keeps its fixed Agent header without exposing the ordinary media scenes', () => {
  const noop = () => {};
  const ordinaryCreation: CreationComposerValue = { draft: emptyCreationDraft(), update: noop, setMode: noop, selectMode: noop, exit: noop };
  const fixedAgent = <span>{locale.workspace.fixedAgent}</span>;
  const view = render(<CreationComposerContext.Provider value={ordinaryCreation}><ComposerSceneHeader agent={fixedAgent} sceneSelectionEnabled={false} /></CreationComposerContext.Provider>, { wrapper });
  expect(view.getByText(locale.workspace.fixedAgent)).toBeTruthy();
  expect(view.queryByTestId('composer-scene-selector')).toBeNull();
  view.rerender(<CreationComposerContext.Provider value={null}><ComposerSceneHeader agent={fixedAgent} /></CreationComposerContext.Provider>);
  expect(view.queryByTestId('composer-scene-selector')).toBeNull();
  view.rerender(<CreationComposerContext.Provider value={ordinaryCreation}><ComposerSceneHeader agent={fixedAgent} /></CreationComposerContext.Provider>);
  expect(view.getByTestId('composer-scene-selector').querySelectorAll('button')).toHaveLength(4);
  // This is the shared welcome composer: enforce the product boundary at its
  // ingress rather than globally disabling the ordinary scene controls.
  const source = readFileSync(new URL('../guid/GuidPage.tsx', import.meta.url), 'utf8');
  expect(source).toMatch(/CreationComposerContext\.Provider\s+value=\{productEntry\s*\?\s*null\s*:\s*creation\}/);
  expect(source).toMatch(/ComposerSceneHeader\s+sceneSelectionEnabled=\{!productEntry\}/);
});

test.each([false, true])('manual import blocks first admission and navigates its real owner with draft retained (StrictMode=%s)', async (strict) => {
  let finish!: (app: HostedApp) => void;
  spyOn(appHosting.import, 'invoke').mockImplementation(() => new Promise<HostedApp>((resolve) => { finish = resolve; }));
  const admission = spyOn(appHosting.developmentSession, 'invoke');
  const create = spyOn(appHosting.create, 'invoke');
  const imported = { ...app, development_session_id: null };
  const getApp = spyOn(appHosting.get, 'invoke').mockResolvedValue(imported);
  const ordinary = renderHook(() => useGuidDraftState('input', ''));
  act(() => ordinary.result.current[1]('Keep my ordinary Guid prompt'));
  const Location = () => <span data-testid='app-route-path'>{useLocation().pathname}</span>;
  const route = <MemoryRouter initialEntries={['/apps/import']}><I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0, shouldRetryOnError: false }}><Location /><Routes>
    <Route path='/apps/import' element={<AppWorkspaceRoute />} /><Route path='/apps/:id' element={<AppWorkspaceRoute />} />
  </Routes></SWRConfig></I18nextProvider></MemoryRouter>;
  const view = render(strict ? <StrictMode>{route}</StrictMode> : route);
  expect(view.getByTestId('app-route-path').textContent).toBe('/apps/import');
  expect(getApp).not.toHaveBeenCalled();
  act(() => setScopedDraft('Keep this application request while importing'));
  fireEvent.input(view.getByRole('textbox', { name: locale.import.hostPath }), { target: { value: 'C:/source/app' } });
  fireEvent.click(view.getByRole('button', { name: locale.import.action }));
  await waitFor(() => expect(entry?.submissionDisabled).toBe(true));
  act(() => scopedSend.sendMessageHandler());
  expect(create).not.toHaveBeenCalled(); expect(admission).not.toHaveBeenCalled();
  expect((view.getByRole('textbox', { name: 'Scoped application prompt' }) as HTMLTextAreaElement).value).toBe('Keep this application request while importing');
  await act(async () => { finish(imported); });
  await waitFor(() => expect(view.getByTestId('app-route-path').textContent).toBe(`/apps/${imported.id}`));
  await waitFor(() => expect(entry?.workspace).toBe(imported.directory));
  expect((view.getByRole('textbox', { name: 'Scoped application prompt' }) as HTMLTextAreaElement).value).toBe('Keep this application request while importing');
  expect(ordinary.result.current[0]).toBe('Keep my ordinary Guid prompt');
  expect(entry?.submissionDisabled).toBe(false);
  view.unmount();
  const nextImport = render(<AppDevelopmentConversation mode='import' onAppChanged={() => {}} />, { wrapper });
  expect((nextImport.getByRole('textbox', { name: 'Scoped application prompt' }) as HTMLTextAreaElement).value).toBe('');
});

test('existing chat receives manual replacement admission blocking and releases it without losing its Session', async () => {
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(conversation);
  const view = render(<AppDevelopmentConversation app={app} mode='edit' submissionDisabled onAppChanged={() => {}} />, { wrapper });
  await view.findByTestId('ordinary-conversation');
  expect(renderedChat?.submissionDisabled).toBe(true);
  view.rerender(<AppDevelopmentConversation app={app} mode='edit' submissionDisabled={false} onAppChanged={() => {}} />);
  expect(renderedChat?.submissionDisabled).toBe(false);
  expect(renderedChat?.conversation?.id).toBe(SESSION);
});

test('existing application refreshes its normal workflow only at fresh submission and validates the same owner', async () => {
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(conversation);
  const open = spyOn(appHosting.developmentSession, 'invoke').mockResolvedValue({ app, conversation });
  const changed = mock(() => {}), preparing = mock(() => {});
  const view = render(<AppDevelopmentConversation app={app} mode='edit' onAppChanged={changed} onSubmittingChange={preparing} />, { wrapper });
  await view.findByTestId('ordinary-conversation');
  expect(open).not.toHaveBeenCalled();
  await act(async () => renderedChat!.onBeforeSubmit!());
  expect(open.mock.calls).toEqual([[{ id: app.id, refresh_workflow: true }]]);
  expect(changed).toHaveBeenCalledWith(app);
  expect(preparing.mock.calls).toEqual([[true], [false]]);
  open.mockResolvedValue({ app, conversation: { ...conversation, id: OTHER_SESSION } });
  await expect(renderedChat!.onBeforeSubmit!()).rejects.toThrow(locale.workspace.sessionMismatch);
  expect(changed).toHaveBeenCalledTimes(1);
});

test('a new project-session draft retries an unknown creation with the same key and rotates only after admission', async () => {
  const open = spyOn(appHosting.developmentSession, 'invoke').mockRejectedValueOnce(new Error('Creation response unknown'))
    .mockResolvedValue({ app: { ...app, development_session_id: OTHER_SESSION }, conversation: { ...conversation, id: OTHER_SESSION } });
  const request = { title: 'Next iteration', model: { provider_id: PROVIDER, model: 'chat-model' } };
  const props = { app, mode: 'edit' as const, selectedSessionId: null, onAppChanged: mock(() => {}), onSessionCreated: mock(() => {}) };
  const first = render(<AppDevelopmentConversation {...props} />, { wrapper });
  await expect(entry!.createConversation(request)).rejects.toThrow('Creation response unknown');
  const creationKey = open.mock.calls[0][0].creation_key;
  expect(creationKey).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  first.unmount();
  const retry = render(<AppDevelopmentConversation {...props} />, { wrapper });
  await act(async () => { const created = await entry!.createConversation(request); entry!.onConversationCreated(created); });
  expect(open.mock.calls[1][0]).toMatchObject({ id: app.id, new_session: true, creation_key: creationKey });
  expect(props.onSessionCreated).toHaveBeenCalledWith(OTHER_SESSION);
  retry.unmount();
  render(<AppDevelopmentConversation {...props} />, { wrapper });
  await act(async () => { await entry!.createConversation(request); });
  expect(open.mock.calls[2][0].creation_key).not.toBe(creationKey);
});

test('an empty application retains the new-draft creation key after an unknown response changes the polled navigation reference', async () => {
  const opened = { ...app, development_session_id: SESSION };
  const open = spyOn(appHosting.developmentSession, 'invoke').mockRejectedValueOnce(new Error('Creation response unknown')).mockResolvedValue({ app: opened, conversation });
  const props = { mode: 'create' as const, selectedSessionId: null, onAppChanged: mock(() => {}) };
  const view = render(<AppDevelopmentConversation {...props} app={{ ...app, development_session_id: null }} />, { wrapper });
  act(() => setScopedDraft('Keep this first application request'));
  const request = { title: 'First iteration', model: { provider_id: PROVIDER, model: 'chat-model' } };
  await act(async () => { await expect(entry!.createConversation(request)).rejects.toThrow('Creation response unknown'); });
  const creationKey = open.mock.calls[0][0].creation_key;
  view.rerender(<AppDevelopmentConversation {...props} app={opened} />);
  expect((view.getByRole('textbox', { name: 'Scoped application prompt' }) as HTMLTextAreaElement).value).toBe('Keep this first application request');
  await act(async () => { await entry!.createConversation(request); });
  expect(open.mock.calls[1][0]).toMatchObject({ id: app.id, new_session: true, creation_key: creationKey });
});

test('an explicitly selected older project Session preserves its identity at fresh submission', async () => {
  const latest = { ...app, development_session_id: OTHER_SESSION };
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(conversation);
  const open = spyOn(appHosting.developmentSession, 'invoke').mockResolvedValue({ app: latest, conversation });
  const view = render(<AppDevelopmentConversation app={latest} mode='edit' selectedSessionId={SESSION} onAppChanged={() => {}} />, { wrapper });
  await view.findByTestId('ordinary-conversation');
  expect(open).not.toHaveBeenCalled();
  expect(renderedChat?.conversation?.id).toBe(SESSION);
  await act(async () => { await renderedChat!.onBeforeSubmit!(); });
  expect(open.mock.calls[0][0]).toEqual({ id: app.id, session_id: SESSION, refresh_workflow: true });
});

test('project history switches ordinary Sessions while retaining settings and the actual preview', async () => {
  spyOn(FilePanelModule, 'default').mockImplementation(() => <div data-testid='application-files' />);
  const second = { ...conversation, id: OTHER_SESSION, name: 'Second project iteration' };
  let requests = 0;
  const server = createServer((_request, response) => { requests++; response.writeHead(200, { 'Content-Type': 'text/html' }); response.end('<!doctype html><title>Shared application</title>'); });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const running = { ...app, state: 'running' as const, preview: true, pid: 51,
    settings: { ...app.settings, start_command: 'node server.cjs', web_url: `http://127.0.0.1:${(server.address() as AddressInfo).port}` } };
  const owner = spyOn(appHosting.get, 'invoke').mockResolvedValue(running);
  spyOn(appHosting.developmentSessions, 'invoke').mockResolvedValue({ items: [{ conversation: second, editable_in_app: true, archived: false }, { conversation, editable_in_app: true, archived: false }],
    total: 2, has_more: false, next_cursor: null, project_directory: app.directory, current_session_id: OTHER_SESSION });
  spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async ({ conversation_id }) => conversation_id === SESSION ? conversation : second);
  const select = spyOn(appHosting.developmentSession, 'invoke').mockResolvedValue({ app: { ...running, development_session_id: OTHER_SESSION }, conversation: second });
  const Location = () => <output data-testid='app-project-location'>{useLocation().search}</output>;
  const view = render(<MemoryRouter initialEntries={['/apps/owned-app']}><I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0, shouldRetryOnError: false }}><Location /><Routes>
    <Route path='/apps/:id' element={<AppWorkspaceRoute />} />
  </Routes></SWRConfig></I18nextProvider></MemoryRouter>);
  try {
    await view.findByTestId('ordinary-conversation');
    await view.findByRole('button', { name: /Second project iteration/ });
    fireEvent.click(view.getByText(locale.actions.edit, { selector: 'summary' }));
    const description = view.getByRole('textbox', { name: locale.description }) as HTMLTextAreaElement;
    fireEvent.input(description, { target: { value: 'Retain my project note' } });
    const frame = view.container.querySelector('iframe')!;
    await waitFor(() => expect(requests).toBe(1));
    // A changed navigation pointer delivered by another window does not steal the mounted Session.
    owner.mockResolvedValue({ ...running, development_session_id: OTHER_SESSION });
    fireEvent.click(view.getByRole('button', { name: locale.actions.refresh }));
    await waitFor(() => expect(owner).toHaveBeenCalledTimes(2));
    expect(renderedChat?.conversation?.id).toBe(SESSION);
    fireEvent.click(view.getByRole('button', { name: /Second project iteration/ }));
    await waitFor(() => expect(renderedChat?.conversation?.id).toBe(OTHER_SESSION));
    expect(select.mock.calls[0][0]).toEqual({ id: app.id, session_id: OTHER_SESSION });
    expect(view.getByTestId('app-project-location').textContent).toBe(`?session=${OTHER_SESSION}`);
    expect(view.getByRole('textbox', { name: locale.description })).toBe(description);
    expect(description.value).toBe('Retain my project note');
    expect(view.container.querySelector('iframe')).toBe(frame);
    fireEvent.click(view.getByRole('button', { name: locale.project.newSession }));
    await waitFor(() => expect(view.getByTestId('app-project-location').textContent).toBe('?session=new'));
    expect(entry?.templateKey).toBe('app.developer');
    expect(view.getByRole('textbox', { name: locale.description })).toBe(description);
    expect(view.container.querySelector('iframe')).toBe(frame);
    expect(requests).toBe(1);
  } finally { view.unmount(); server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())); }
});

function projectSelectionRace(initialEntries?: string[]) {
  const thirdId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000306');
  const sessions = [conversation, { ...conversation, id: OTHER_SESSION, name: 'Selection B' }, { ...conversation, id: thirdId, name: 'Current iteration' }];
  spyOn(FilePanelModule, 'default').mockImplementation(() => <div />);
  spyOn(appHosting.get, 'invoke').mockResolvedValue({ ...app, development_session_id: thirdId });
  spyOn(appHosting.developmentSessions, 'invoke').mockResolvedValue({ items: sessions.map(item => ({ conversation: item, editable_in_app: true, archived: false })),
    total: sessions.length, has_more: false, next_cursor: null, project_directory: app.directory, current_session_id: thirdId });
  spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(async ({ conversation_id }) => sessions.find(item => item.id === conversation_id)!);
  const pending = new Map<string, { resolve: (value: { app: HostedApp; conversation: TChatConversation }) => void; reject: (error: Error) => void }>();
  const select = spyOn(appHosting.developmentSession, 'invoke').mockImplementation(({ session_id }) => new Promise((resolve, reject) => pending.set(session_id!, { resolve, reject })));
  const Location = () => {
    const location = useLocation(), navigate = useNavigate();
    return <><output data-testid='selection-race-location'>{location.pathname}{location.search}</output><button onClick={() => void navigate(-1)}>Browser back</button></>;
  };
  const entries = initialEntries ?? [`/apps/${app.id}?session=${thirdId}`];
  const view = render(<MemoryRouter initialEntries={entries} initialIndex={entries.length - 1}><I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0, shouldRetryOnError: false }}><Location /><Routes>
    <Route path='/apps/:id' element={<AppWorkspaceRoute />} />
  </Routes></SWRConfig></I18nextProvider></MemoryRouter>);
  const choose = (name: string) => {
    fireEvent.click(view.container.querySelector('summary[aria-label]')!);
    fireEvent.click(view.getByRole('button', { name: new RegExp(name) }));
  };
  const finish = (id: string, name: string) => pending.get(id)!.resolve({ app: { ...app, name, development_session_id: id }, conversation: sessions.find(item => String(item.id) === id)! });
  return { view, select, pending, thirdId, choose, finish };
}

test.each([true, false])('the latest project selection wins when A and B responses arrive out of order (A first=%s)', async aFirst => {
  const race = projectSelectionRace();
  await race.view.findByTestId('ordinary-conversation');
  await race.view.findByRole('button', { name: /Selection B/ });
  race.choose(conversation.name); race.choose('Selection B');
  expect(race.select.mock.calls.map(([request]) => request.session_id)).toEqual([SESSION, OTHER_SESSION]);
  if (aFirst) {
    await act(async () => { race.finish(SESSION, 'Stale owner A'); });
    expect(race.view.getByTestId('selection-race-location').textContent).toBe(`/apps/${app.id}?session=${race.thirdId}`);
    expect(race.view.queryByRole('heading', { name: 'Stale owner A' })).toBeNull();
  }
  await act(async () => { race.finish(OTHER_SESSION, 'Latest owner B'); });
  await waitFor(() => expect(renderedChat?.conversation?.id).toBe(OTHER_SESSION));
  if (!aFirst) await act(async () => { race.finish(SESSION, 'Stale owner A'); });
  expect(race.view.getByTestId('selection-race-location').textContent).toBe(`/apps/${app.id}?session=${OTHER_SESSION}`);
  expect(race.view.getByRole('heading', { name: 'Latest owner B' })).toBeTruthy();
  expect(race.view.queryByRole('heading', { name: 'Stale owner A' })).toBeNull();
});

test.each([true, false])('a pending history selection cannot replace a new-session draft or report a stale error (late success=%s)', async success => {
  const race = projectSelectionRace();
  await race.view.findByTestId('ordinary-conversation');
  await race.view.findByRole('button', { name: /Selection B/ });
  race.choose(conversation.name);
  fireEvent.click(race.view.getByRole('button', { name: locale.project.newSession }));
  await race.view.findByRole('textbox', { name: 'Scoped application prompt' });
  act(() => setScopedDraft('Keep this newer draft'));
  await act(async () => { if (success) race.finish(SESSION, 'Stale selected owner'); else race.pending.get(SESSION)!.reject(new Error('Stale selection failure')); });
  expect(race.view.getByTestId('selection-race-location').textContent).toBe(`/apps/${app.id}?session=new`);
  expect((race.view.getByRole('textbox', { name: 'Scoped application prompt' }) as HTMLTextAreaElement).value).toBe('Keep this newer draft');
  expect(race.view.queryByRole('heading', { name: 'Stale selected owner' })).toBeNull();
  expect(race.view.queryByText('Stale selection failure')).toBeNull();
});

test('browser back on the same App pathname invalidates a pending project selection', async () => {
  const thirdId = '0190f5fe-7c00-7a00-8000-000000000306';
  const race = projectSelectionRace([`/apps/${app.id}?session=${OTHER_SESSION}`, `/apps/${app.id}?session=${thirdId}`]);
  await race.view.findByTestId('ordinary-conversation');
  await race.view.findByRole('button', { name: /Selection B/ });
  race.choose(conversation.name);
  fireEvent.click(race.view.getByRole('button', { name: 'Browser back' }));
  await waitFor(() => expect(renderedChat?.conversation?.id).toBe(OTHER_SESSION));
  await act(async () => { race.finish(SESSION, 'Stale selected owner'); });
  expect(race.view.getByTestId('selection-race-location').textContent).toBe(`/apps/${app.id}?session=${OTHER_SESSION}`);
  expect(race.view.queryByRole('heading', { name: 'Stale selected owner' })).toBeNull();
});

test.each([true, false])('an active sibling project Session blocks manual files and a new draft while the selected Session is idle (processing=%s)', async processing => {
  spyOn(FilePanelModule, 'default').mockImplementation(() => <div />);
  spyOn(appHosting.get, 'invoke').mockResolvedValue(app);
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(conversation);
  const sibling = { ...conversation, id: OTHER_SESSION, name: 'Working sibling', status: 'running' as const,
    extra: { ...conversation.extra, ...(processing ? {} : { execution_phase: 'paused' }) },
    runtime: { state: processing ? 'running' as const : 'idle' as const, is_processing: processing, can_send_message: false, has_runtime: true, active_turn_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000305') } };
  spyOn(appHosting.developmentSessions, 'invoke').mockResolvedValue({ items: [{ conversation: sibling, editable_in_app: true, archived: false }, { conversation, editable_in_app: true, archived: false }],
    total: 2, has_more: false, next_cursor: null, project_directory: app.directory, current_session_id: SESSION });
  const view = render(<MemoryRouter initialEntries={[`/apps/owned-app?session=${SESSION}`]}><I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), shouldRetryOnError: false }}><Routes>
    <Route path='/apps/:id' element={<AppWorkspaceRoute />} />
  </Routes></SWRConfig></I18nextProvider></MemoryRouter>);
  await view.findByTestId('ordinary-conversation');
  await waitFor(() => expect((view.getByRole('button', { name: locale.project.newSession }) as HTMLButtonElement).disabled).toBe(true));
  expect((view.getByRole('button', { name: locale.overwrite.action }) as HTMLButtonElement).disabled).toBe(true);
  expect((view.getByRole('button', { name: locale.actions.delete }) as HTMLButtonElement).disabled).toBe(true);
  expect(renderedChat?.submissionDisabled).toBe(true);
  expect(view.getByText(locale.workspace.workspaceBusy)).toBeTruthy();
});

test('a failed owner refresh keeps the mounted conversation, preview and unsaved settings until a successful refresh', async () => {
  // The filesystem sidebar has its own network contracts. This regression
  // exercises the actual Route, SWR owner cache, settings form and HTTP frame.
  spyOn(FilePanelModule, 'default').mockImplementation(() => <div data-testid='application-files' />);
  let webpageRequests = 0;
  const server = createServer((_request, response) => { webpageRequests++; response.writeHead(200, { 'Content-Type': 'text/html' }); response.end('<!doctype html><title>Application preview</title>'); });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const running = { ...app, state: 'running' as const, preview: true, pid: 51,
    settings: { ...app.settings, start_command: 'node server.cjs', web_url: `http://127.0.0.1:${(server.address() as AddressInfo).port}` } };
  const get = spyOn(appHosting.get, 'invoke').mockResolvedValue(running);
  const create = spyOn(appHosting.create, 'invoke');
  const prepare = spyOn(appHosting.developmentSession, 'invoke');
  const send = spyOn(ipcBridge.conversation.sendMessage, 'invoke');
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(conversation);
  const view = render(<MemoryRouter initialEntries={['/apps/owned-app']}><I18nextProvider i18n={i18n}><SWRConfig value={{
    provider: () => new Map(), dedupingInterval: 0, shouldRetryOnError: false,
  }}><Routes><Route path='/apps/:id' element={<AppWorkspaceRoute />} /></Routes></SWRConfig></I18nextProvider></MemoryRouter>);
  try {
    const chat = await view.findByTestId('ordinary-conversation');
    const advanced = view.getByText(locale.actions.edit, { selector: 'summary' });
    fireEvent.click(advanced);
    const description = view.getByRole('textbox', { name: locale.description }) as HTMLTextAreaElement;
    fireEvent.input(description, { target: { value: 'Keep my unsaved note' } });
    const frame = view.container.querySelector('iframe')!;
    await waitFor(() => expect(webpageRequests).toBe(1));
    expect(frame).not.toBeNull();
    get.mockRejectedValue(new Error('Failed to fetch application'));
    fireEvent.click(view.getByRole('button', { name: locale.actions.refresh }));
    await view.findByText('Failed to fetch application');
    expect(view.getByTestId('ordinary-conversation')).toBe(chat);
    expect(view.getByRole('textbox', { name: locale.description })).toBe(description);
    expect(description.value).toBe('Keep my unsaved note');
    expect(view.container.querySelector('iframe')).toBe(frame);
    expect(webpageRequests).toBe(1);
    get.mockResolvedValue({ ...running, description: 'The latest saved description' });
    fireEvent.click(view.getByRole('button', { name: locale.actions.refresh }));
    await waitFor(() => expect(view.queryByText('Failed to fetch application')).toBeNull());
    expect(view.getByTestId('ordinary-conversation')).toBe(chat);
    expect(view.getByRole('textbox', { name: locale.description })).toBe(description);
    expect(description.value).toBe('Keep my unsaved note');
    expect(view.container.querySelector('iframe')).toBe(frame);
    expect(create).not.toHaveBeenCalled(); expect(prepare).not.toHaveBeenCalled(); expect(send).not.toHaveBeenCalled();
  } finally {
    view.unmount(); server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve()));
  }
});

test.each(['create', 'read'] as const)('an unconfirmed %s with no cached record remains fatal without another admission', async operation => {
  const create = spyOn(appHosting.create, 'invoke').mockRejectedValue(new Error('Unconfirmed creation'));
  spyOn(appHosting.get, 'invoke').mockRejectedValue(new Error('Application unavailable'));
  const prepare = spyOn(appHosting.developmentSession, 'invoke');
  const view = render(<MemoryRouter initialEntries={[operation === 'create' ? '/apps/new' : '/apps/owned-app']}>
    <I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), shouldRetryOnError: false }}><Routes>
      <Route path='/apps/new' element={<AppWorkspaceRoute />} /><Route path='/apps/:id' element={<AppWorkspaceRoute />} />
    </Routes></SWRConfig></I18nextProvider></MemoryRouter>);
  await view.findByText(operation === 'create' ? locale.workspace.createFailed : 'Application unavailable');
  expect(view.queryByTestId('ordinary-conversation')).toBeNull();
  expect(view.queryByRole('textbox', { name: locale.description })).toBeNull();
  expect(prepare).not.toHaveBeenCalled();
  expect(create).toHaveBeenCalledTimes(operation === 'create' ? 1 : 0);
});
