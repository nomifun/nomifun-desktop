import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { Message, Modal } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId } from '@/common/types/ids';
import * as historyContext from '@/renderer/hooks/context/ConversationHistoryContext';
import * as cron from '@/renderer/pages/cron';
import * as terminalSessions from '@/renderer/pages/terminal/useTerminalSessions';
import commonLocale from '@/renderer/services/i18n/locales/en-US/common.json';
import conversationLocale from '@/renderer/services/i18n/locales/en-US/conversation.json';
import sessionListLocale from '@/renderer/services/i18n/locales/en-US/sessionList.json';
import * as directoryModal from '@/renderer/components/settings/DirectorySelectionModal';
import * as conversationRow from './ConversationRow';
import * as sshGroup from './SshSessionGroup';
import * as capabilities from './hooks/useSessionCapabilities';
import * as branches from './hooks/useWorkpathBranches';
import * as knowledge from './hooks/useWorkpathKnowledge';
import { WORKPATH_EXPANSION_STORAGE_KEY } from './hooks/useWorkpathUiState';
import { WorkpathSessionList } from './index';
import type { ConversationRowProps } from './types';
import { DEFAULT_WORKPATH_KEY, workpathKey } from './utils/workpathKey';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: {
    common: commonLocale, conversation: conversationLocale, sessionList: sessionListLocale,
  } } },
  interpolation: { escapeValue: false },
});

const appId = '0190f5fe-7c00-7a00-8000-000000000931';
const sharedDirectory = 'C:/NomiFun/apps/shared-project';
const ownedOnlyDirectory = 'C:/NomiFun/apps/owned-only-project';
const ordinaryDirectory = 'C:/NomiFun/projects/ordinary-project';
function conversation(number: number, name: string, workspace = '', overrides: Partial<TChatConversation> = {}): TChatConversation {
  return {
    id: parseConversationId(`0190f5fe-7c00-7a00-8000-${String(number).padStart(12, '0')}`),
    name, type: 'nomi', created_at: number, modified_at: number,
    extra: { workspace, custom_workspace: Boolean(workspace) },
    model: {} as TChatConversation['model'],
    ...overrides,
  };
}
const ordinary = conversation(901, 'Ordinary conversation');
const sameTemplate = conversation(902, 'Ordinary app developer Agent', ordinaryDirectory, {
  agent_snapshot: { preset_name: 'app.developer', enabled_capabilities: [] } as unknown as TChatConversation['agent_snapshot'],
});
const sameDirectory = conversation(903, 'Ordinary conversation sharing the app directory', sharedDirectory);
const appOwned = conversation(904, 'App-owned project conversation', sharedDirectory, { app_project_id: appId });
const appOwnedOnly = conversation(905, 'App-owned isolated project conversation', ownedOnlyDirectory, { app_project_id: appId });
const ordinaryConversations = [ordinary, sameTemplate, sameDirectory];
const mixedConversations = [...ordinaryConversations, appOwned, appOwnedOnly];
const restores: Array<() => void> = [];

function restore(spy: { mockRestore: () => void }) {
  restores.push(() => spy.mockRestore());
  return spy;
}

beforeEach(() => {
  localStorage.clear();
  localStorage.setItem(WORKPATH_EXPANSION_STORAGE_KEY, JSON.stringify({
    [DEFAULT_WORKPATH_KEY]: true,
    [workpathKey(ordinaryDirectory)]: true,
    [workpathKey(sharedDirectory)]: true,
    [workpathKey(ownedOnlyDirectory)]: true,
  }));
  // The production list source already filters App ownership. Deliberately
  // inject mixed source data here to verify the actual sidebar wiring too.
  restore(spyOn(historyContext, 'useConversationHistoryContext').mockReturnValue({
    conversations: mixedConversations,
    isConversationGenerating: () => false,
    hasCompletionUnread: () => false,
    clearCompletionUnread: () => {},
    setActiveConversation: () => {},
  } as unknown as historyContext.ConversationHistoryContextValue));
  restore(spyOn(cron, 'useCronJobsMap').mockReturnValue({
    getJobStatus: () => 'none', markAsRead: () => {}, setActiveConversation: () => {},
  } as unknown as ReturnType<typeof cron.useCronJobsMap>));
  restore(spyOn(terminalSessions, 'useTerminalSessions').mockReturnValue({
    sessions: [], loading: false, refresh: async () => {}, removeSession: async () => {},
  }));
  restore(spyOn(capabilities, 'useSessionCapabilities').mockReturnValue({ autowork: new Map() }));
  restore(spyOn(branches, 'useWorkpathBranches').mockReturnValue(new Map()));
  restore(spyOn(knowledge, 'useWorkpathKnowledgeLit').mockReturnValue(false));
  restore(spyOn(sshGroup, 'default').mockImplementation(() => null));
  restore(spyOn(directoryModal, 'default').mockImplementation(() => null));
  restore(spyOn(conversationRow, 'default').mockImplementation((props: ConversationRowProps) => (
    <button
      type='button'
      data-testid={`conversation-${props.conversation.id}`}
      aria-pressed={props.checked}
      onClick={() => props.onConversationClick(props.conversation)}
    >{props.conversation.name}</button>
  )));
  restore(spyOn(ipcBridge.conversation.listChanged, 'on').mockImplementation(() => () => {}));
  restore(spyOn(ipcBridge.terminal.onCreated, 'on').mockImplementation(() => () => {}));
  restore(spyOn(ipcBridge.terminal.onRemoved, 'on').mockImplementation(() => () => {}));
  restore(spyOn(Message, 'success').mockImplementation(() => () => {}));
  restore(spyOn(Message, 'warning').mockImplementation(() => () => {}));
  restore(spyOn(Message, 'error').mockImplementation(() => () => {}));
});

afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach(revert => revert());
  localStorage.clear();
});

function fixture(collapsed = false) {
  const confirmations: Array<Parameters<typeof Modal.confirm>[0]> = [];
  restore(spyOn(Modal, 'confirm').mockImplementation(props => {
    confirmations.push(props);
    return { close: mock(), update: mock() };
  }));
  const remove = spyOn(ipcBridge.conversation.remove, 'invoke').mockResolvedValue(undefined);
  restore(remove);
  const listApps = spyOn(appHosting.list, 'invoke').mockResolvedValue([]);
  restore(listApps);
  const batchModeChanged = mock();
  const view = render(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter initialEntries={['/guid']}>
        <SWRConfig value={{ provider: () => new Map(), shouldRetryOnError: false }}>
          <WorkpathSessionList collapsed={collapsed} batchMode onBatchModeChange={batchModeChanged} />
        </SWRConfig>
      </MemoryRouter>
    </I18nextProvider>
  );
  return { view, confirmations, remove, listApps, batchModeChanged };
}

test('App ownership excludes project rows and counts while ordinary same-Agent and same-directory sessions remain visible', () => {
  const { view, listApps } = fixture();
  for (const row of ordinaryConversations) expect(view.getByTestId(`conversation-${row.id}`)).toBeTruthy();
  for (const row of [appOwned, appOwnedOnly]) expect(view.queryByTestId(`conversation-${row.id}`)).toBeNull();
  expect(view.getByTestId('workpath-section-toolbar').textContent).toBe(`${sessionListLocale.workpathSection}3`);
  expect(view.queryByText('owned-only-project')).toBeNull();
  const sharedDrawer = view.getByText('shared-project').closest('.workpath-drawer')!;
  expect(within(sharedDrawer as HTMLElement).getByTestId(`conversation-${sameDirectory.id}`)).toBeTruthy();
  expect(within(sharedDrawer as HTMLElement).getByText('(1)')).toBeTruthy();
  expect(listApps).not.toHaveBeenCalled();
});

test('real select-all and batch deletion include only ordinary sessions from a mixed source', async () => {
  const { view, confirmations, remove, batchModeChanged } = fixture();
  fireEvent.click(view.getByText(conversationLocale.history.selectAll));
  for (const row of ordinaryConversations) {
    expect(view.getByTestId(`conversation-${row.id}`).getAttribute('aria-pressed')).toBe('true');
  }
  expect(view.getByText('Delete (3)')).toBeTruthy();
  fireEvent.click(view.getByText(commonLocale.cancel));
  for (const row of ordinaryConversations) {
    expect(view.getByTestId(`conversation-${row.id}`).getAttribute('aria-pressed')).toBe('false');
  }
  expect(view.getByText('Delete (0)')).toBeTruthy();
  fireEvent.click(view.getByText(conversationLocale.history.selectAll));
  fireEvent.click(view.getByText('Delete (3)'));
  expect(confirmations).toHaveLength(1);
  expect(confirmations[0].content).toBe('Delete 3 selected topics?');
  expect(remove).not.toHaveBeenCalled();
  await act(async () => { await confirmations[0].onOk?.(); });
  expect(remove.mock.calls.map(([request]) => request.conversation_id)).toEqual(ordinaryConversations.map(row => row.id));
  expect(batchModeChanged).toHaveBeenCalledWith(false);
});

test('real project-group selection cannot capture an App session sharing the ordinary project directory', async () => {
  const { view, confirmations, remove } = fixture();
  const sharedDrawer = view.getByText('shared-project').closest('.workpath-drawer') as HTMLElement;
  const projectCheckbox = within(sharedDrawer).getAllByRole('checkbox')[0];
  fireEvent.click(projectCheckbox);
  expect(view.getByTestId(`conversation-${sameDirectory.id}`).getAttribute('aria-pressed')).toBe('true');
  expect(view.getByTestId(`conversation-${ordinary.id}`).getAttribute('aria-pressed')).toBe('false');
  expect(view.getByTestId(`conversation-${sameTemplate.id}`).getAttribute('aria-pressed')).toBe('false');
  fireEvent.click(view.getByText('Delete (1)'));
  expect(confirmations[0].content).toBe('Delete 1 selected topics?');
  await act(async () => { await confirmations[0].onOk?.(); });
  expect(remove.mock.calls.map(([request]) => request.conversation_id)).toEqual([sameDirectory.id]);
});

test('collapsed sidebar keeps the same App ownership boundary', () => {
  const { view } = fixture(true);
  for (const row of ordinaryConversations) expect(view.getByTestId(`conversation-${row.id}`)).toBeTruthy();
  for (const row of [appOwned, appOwnedOnly]) expect(view.queryByTestId(`conversation-${row.id}`)).toBeNull();
});
