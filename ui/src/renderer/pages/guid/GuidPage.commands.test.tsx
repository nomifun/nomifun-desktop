import '../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { uuidv7 } from '@/common/utils';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import * as agentSelection from './hooks/useGuidAgentSelection';
import * as modelSelection from './hooks/useGuidModelSelection';
import * as modelCatalog from '@/renderer/hooks/agent/useModelsForTask';
import * as capabilityCatalog from '@/renderer/components/chat/SessionCapabilityPicker/useSessionCapabilityCatalog';
import * as collaboration from '@/renderer/components/collaboration/useCollaborationDraft';
import * as companionShowcase from './components/GuidCompanionShowcase';
import * as feedbackModal from '@/renderer/components/settings/SettingsModal/contents/FeedbackReportModal';
import * as resourcePicker from '@/renderer/components/agent/AgentResourcePicker';
import * as focusRing from '@/renderer/hooks/chat/useInputFocusRing';
import { GuidDraftScopeContext } from './hooks/useGuidDraftState';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import GuidPage, { type GuidProductEntry } from './GuidPage';

const i18n = createInstance();
await i18n.init({ lng: 'en-US', resources: { 'en-US': { translation: { messages, conversation, common } } } });
const noop = () => {};
beforeEach(() => {
  setBrowserStorageGeneration(uuidv7());
  spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation(() => noop);
  spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(() => noop);
  spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockResolvedValue({ name: 'reference.txt', path: '/workspace/reference.txt',
    size: 16, type: 'file', lastModified: 0, isDirectory: false });
  spyOn(focusRing, 'useInputFocusRing').mockReturnValue({ activeBorderColor: '#777', inactiveBorderColor: '#ccc', activeShadow: 'none' });
  spyOn(agentSelection, 'useGuidAgentSelection').mockReturnValue({
    selection: { kind: 'template', templateKey: 'chat.minimal' }, selectedPreset: undefined,
    selectedTemplate: undefined, presets: [], draftPresets: [], officialTemplates: [],
    isLoading: false, isLoaded: true, loadError: undefined, setSelection: noop, refreshPresets: async () => {},
  });
  spyOn(modelSelection, 'useGuidModelSelection').mockReturnValue({ modelList: [], current_model: undefined,
    currentModelTraits: [], getAvailableModels: () => [], formatGeminiModelLabel: (_provider, name) => name ?? '',
    setCurrentModel: async () => {},
  });
  spyOn(modelCatalog, 'useModelsForTask').mockReturnValue({ groups: [], isLoading: false, refresh: noop });
  spyOn(capabilityCatalog, 'useSessionCapabilityCatalog').mockReturnValue({
    catalog: { skills: [], autoSkillNames: new Set(), mcpServers: [] }, loading: false, error: undefined, retry: noop,
  });
  spyOn(collaboration, 'useCollaborationDraft').mockReturnValue({ mainModel: null, activeCollaborators: [],
    setCollaborators: noop, policy: { delegationPolicy: 'automatic', decisionPolicy: 'automatic' }, setPolicy: noop,
    selectedTemplate: null, setTemplate: noop, config: undefined, ready: true, reset: noop, copyToConversation: noop,
  });
  // Unrelated page chrome is outside the command/Composer interaction under test.
  spyOn(companionShowcase, 'default').mockImplementation(() => <></>);
  spyOn(feedbackModal, 'default').mockImplementation(() => <></>);
  spyOn(resourcePicker, 'default').mockImplementation(() => <></>);
});
afterEach(() => { cleanup(); mock.restore(); sessionStorage.clear(); });

function fixture(kind: 'home' | 'application') {
  const create = spyOn(ipcBridge.agentPlatform.sessions.create, 'invoke');
  const open = spyOn(ipcBridge.dialog.showOpen, 'invoke').mockResolvedValue(['/workspace/reference.txt']);
  const warning = spyOn(Message, 'warning').mockImplementation(() => () => {});
  const productCreate = mock<NonNullable<GuidProductEntry['createConversation']>>();
  const productEntry: GuidProductEntry | undefined = kind === 'application' ? {
    templateKey: 'app.developer', agentLabel: 'Application Agent', placeholder: 'Describe an application',
    createConversation: productCreate, onConversationCreated: noop,
  } : undefined;
  const content = (scope = kind === 'application' ? 'app-workspace:test:new' : undefined) => <MemoryRouter><I18nextProvider i18n={i18n}>
    <GuidDraftScopeContext.Provider value={scope}>
      <GuidPage productEntry={productEntry} />
    </GuidDraftScopeContext.Provider>
  </I18nextProvider></MemoryRouter>;
  const page = render(content());
  const input = page.getByTestId('guid-input') as HTMLTextAreaElement;
  const button = page.getByTestId('guid-send-btn');
  return { page, input, button, create, open, productCreate, warning,
    change: (value: string) => fireEvent.change(input, { target: { value } }),
    scope: (scope: string) => page.rerender(content(scope)),
  };
}

test('switching application drafts cancels stale file selection and routes the next open to the new scope', async () => {
  const f = fixture('application');
  let resolveOld!: (paths: string[]) => void;
  f.open.mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve; }));
  f.change('/open');
  fireEvent.click(f.button);
  await waitFor(() => expect(f.open).toHaveBeenCalledTimes(1));
  await act(async () => { f.scope('app-workspace:other:new'); });
  expect(f.input.value).toBe('');
  f.open.mockResolvedValueOnce(['/workspace/new-reference.txt']);
  f.change('/open');
  fireEvent.click(f.button);
  await waitFor(() => expect(f.page.getByText('new-reference.txt')).toBeTruthy());
  await act(async () => { resolveOld(['/workspace/stale-reference.txt']); });
  expect(f.page.queryByText('stale-reference.txt')).toBeNull();
  expect(f.page.getByText('new-reference.txt')).toBeTruthy();
  expect(f.input.value).toBe('');
  await act(async () => { f.scope('app-workspace:test:new'); });
  expect(f.page.queryByText('stale-reference.txt')).toBeNull();
  expect(f.page.queryByText('new-reference.txt')).toBeNull();
  expect(f.create).not.toHaveBeenCalled();
  expect(f.productCreate).not.toHaveBeenCalled();
});

for (const kind of ['home', 'application'] as const) {
  test.each(['button', 'keyboard', 'menu'])(`${kind} opens attachments through shared commands without model/Session creation for %s`, async gesture => {
    const f = fixture(kind);
    expect((f.button as HTMLButtonElement).disabled).toBe(true);
    f.change('/open');
    const option = await f.page.findByRole('option', { name: /\/open/ });
    expect((f.button as HTMLButtonElement).disabled).toBe(false);
    if (gesture === 'button') fireEvent.click(f.button);
    else if (gesture === 'keyboard') fireEvent.keyDown(f.input, { key: 'Enter' });
    else fireEvent.click(option);
    await waitFor(() => expect(f.open).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(f.input.value).toBe(''));
    expect(f.page.getByText('reference.txt')).toBeTruthy();
    expect(f.create).not.toHaveBeenCalled();
    expect(f.productCreate).not.toHaveBeenCalled();
  });

  test(`${kind} consumes unavailable Session commands and invalid arguments without creating a Session`, async () => {
    const f = fixture(kind);
    f.change('/clear');
    expect((await f.page.findByRole('option', { name: /\/clear/ })).getAttribute('aria-disabled')).toBe('true');
    fireEvent.click(f.button);
    expect(f.warning).toHaveBeenCalledTimes(1);
    expect(f.input.value).toBe('/clear');
    f.change('/open unexpected arguments');
    fireEvent.keyDown(f.input, { key: 'Enter' });
    expect(f.warning).toHaveBeenCalledTimes(2);
    expect(f.open).not.toHaveBeenCalled();
    expect(f.create).not.toHaveBeenCalled();
    expect(f.productCreate).not.toHaveBeenCalled();
  });
}
