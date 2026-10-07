import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useNavigate } from 'react-router-dom';
import { conversation } from '@/common/adapter/ipcBridge';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import { configService } from '@/common/config/configService';
import type { PluginDraftSummary, PluginLibraryResponse, PluginLibraryState, PluginSummary } from '@/common/types/pluginPlatform';
import type { PluginAuthoringSessionSummary } from '@/common/types/pluginDevelopment';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { uuidv7 } from '@/common/utils';
import * as importDialog from './PluginImportDialog';
import PluginLibraryPage from './PluginLibraryPage';
import en from '../../services/i18n/locales/en-US/pluginPlatform.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { pluginPlatform: en } } } });
const draft: PluginDraftSummary = { draft_id: 'draft-library', source_conversation_id: 'source-conversation',
  revision: 1, display_name: 'My text cleaner', description: 'Created in conversation', status: 'ready', updated_at_ms: 20 };
const plugin: PluginSummary = { plugin_id: 'plugin-library', package_id: 'local.text-cleaner', display_name: 'Saved text cleaner',
  description: 'Created in conversation', enabled: true, revision: 1,
  active: { artifact_digest: 'a'.repeat(64), package_version: '1.0.0', data_generation: 'generation', data_version: 1 },
  has_ui: false, has_service: true, action_count: 1, binding_count: 1, runtime: { state: 'stopped' }, updated_at_ms: 30 };

beforeEach(() => {
  configService.reset();
  setBrowserStorageGeneration(uuidv7());
  (window as typeof window & { __backendPort?: number }).__backendPort = 11451;
});
afterEach(() => {
  cleanup();
  mock.restore();
  configService.reset();
  sessionStorage.clear();
  delete (window as typeof window & { __backendPort?: number }).__backendPort;
});

function mountLibrary() {
  let plugins: PluginSummary[] = [], drafts: PluginDraftSummary[] = [];
  const changes = new Set<() => void>(), reconnects = new Set<() => void>();
  spyOn(pluginPlatform.plugins.changed, 'on').mockImplementation(callback => {
    const refresh = () => callback(); changes.add(refresh);
    return () => { changes.delete(refresh); };
  });
  spyOn(conversation.reconnected, 'on').mockImplementation(callback => {
    const refresh = () => callback(); reconnects.add(refresh);
    return () => { reconnects.delete(refresh); };
  });
  const list = spyOn(pluginPlatform.plugins.list, 'invoke').mockImplementation(async () => ({ revision: 1, plugins }));
  spyOn(pluginPlatform.drafts.list, 'invoke').mockImplementation(async () => ({ drafts }));
  let sessions: PluginAuthoringSessionSummary[] = [];
  spyOn(pluginPlatform.authoring.listSessions, 'invoke').mockImplementation(async () => ({ sessions }));
  let organization: PluginLibraryState = { revision: 0, collections: [], items: [] };
  spyOn(pluginPlatform.libraryState.get, 'invoke').mockImplementation(async () => organization);
  const updateState = spyOn(pluginPlatform.libraryState.update, 'invoke').mockImplementation(async request => {
    organization = { revision: organization.revision + 1, collections: request.collections, items: request.items };
    return organization;
  });
  const preflight = spyOn(pluginPlatform.authoring.preflight, 'invoke').mockResolvedValue({ status: 'ready', reason: '', owner_user_id: 'owner',
    selection: { kind: 'template', templateKey: 'assistant.general' } });
  spyOn(importDialog, 'default').mockImplementation(() => <></>);
  const BackToLibrary = () => { const navigate = useNavigate(); return <button onClick={() => navigate('/plugins')}>Return to library</button>; };
  const view = render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={['/plugins']}><Routes>
    <Route path='/plugins' element={<PluginLibraryPage />} />
    <Route path='/plugins/create' element={<div><h1>Plugin workspace</h1><BackToLibrary /></div>} />
    <Route path='/plugins/authoring/:sessionId' element={<div><h1>Authoring task</h1><BackToLibrary /></div>} />
    <Route path='/plugins/run/:id' element={<h1>Installed plugin</h1>} />
  </Routes></MemoryRouter></I18nextProvider>);
  return { ...view, list, preflight, changes, reconnects, updateState,
    setOrganization: (state: PluginLibraryState) => { organization = state; },
    setSessions: (values: PluginAuthoringSessionSummary[]) => { sessions = values; },
    setData: (nextPlugins: PluginSummary[], nextDrafts: PluginDraftSummary[]) => { plugins = nextPlugins; drafts = nextDrafts; },
    changed: () => { changes.forEach(callback => callback()); },
    reconnected: () => { reconnects.forEach(callback => callback()); },
  };
}

test('creation records keep drafts discoverable and the library opens saved apps', async () => {
  const v = mountLibrary();
  await v.findByText(en.library.emptyTitle);
  fireEvent.click(v.getByRole('button', { name: en.actions.create }));
  await v.findByRole('heading', { name: 'Plugin workspace' });
  expect(v.preflight).not.toHaveBeenCalled();
  v.setData([], [draft]);
  fireEvent.click(v.getByRole('button', { name: 'Return to library' }));
  await v.findByText(en.library.emptyTitle);
  fireEvent.click(v.getByRole('button', { name: en.workspace.views.drafts + ' 1' }));
  await v.findByText(draft.display_name);
  fireEvent.click(v.getByRole('button', { name: en.library.continue }));
  await v.findByRole('heading', { name: 'Plugin workspace' });
  v.setData([plugin], [{ ...draft, plugin_id: plugin.plugin_id, base_plugin_revision: 1 }]);
  fireEvent.click(v.getByRole('button', { name: 'Return to library' }));
  await v.findByText(plugin.display_name);
  fireEvent.click(v.getByRole('button', { name: /^Saved text cleaner/ }));
  await v.findByRole('heading', { name: 'Installed plugin' });
});

test('a product session with no draft remains discoverable without inflating plugin and draft counts', async () => {
  const view = mountLibrary();
  await view.findByText(en.library.emptyTitle);
  view.setSessions([{ conversation_id: '0190f5fe-7c00-7a00-8000-000000000202', name: 'Timer creation', created_at: 1, modified_at: 2 }]);
  await act(async () => { view.changed(); });
  fireEvent.click(view.getByRole('button', { name: en.workspace.views.drafts + ' 1' }));
  await view.findByText('Timer creation');
  expect(view.getByRole('button', { name: `${en.workspace.views.all} 0` })).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: en.library.continue }));
  await view.findByRole('heading', { name: 'Authoring task' });
});

test('conversation changes and reconnects update an already open library and its counts', async () => {
  const v = mountLibrary();
  await v.findByText(en.library.emptyTitle);
  v.setData([], [draft]);
  await act(async () => { v.changed(); });
  fireEvent.click(v.getByRole('button', { name: en.workspace.views.drafts + ' 1' }));
  await v.findByText(draft.display_name);
  expect(v.getByRole('button', { name: `${en.workspace.views.all} 0` })).toBeTruthy();
  v.setData([plugin], []);
  await act(async () => { v.reconnected(); });
  fireEvent.click(v.getByRole('button', { name: en.workspace.views.all + ' 1' }));
  await v.findByText(plugin.display_name);
  expect(v.queryByText(draft.display_name)).toBeNull();
  expect(v.getAllByRole('button', { name: `${en.workspace.views.all} 1` }).length).toBe(1);
  v.unmount();
  expect(v.changes.size).toBe(0);
  expect(v.reconnects.size).toBe(0);
});

test('an older library response cannot erase a plugin loaded after a conversation change', async () => {
  const v = mountLibrary();
  await v.findByText(en.library.emptyTitle);
  let finishOld!: (value: PluginLibraryResponse) => void;
  v.list.mockImplementationOnce(() => new Promise(resolve => { finishOld = resolve; }));
  act(() => { v.changed(); });
  await waitFor(() => expect(finishOld).toBeDefined());
  v.setData([plugin], []);
  await act(async () => { v.changed(); });
  await v.findByText(plugin.display_name);
  await act(async () => { finishOld({ revision: 0, plugins: [] }); });
  expect(v.getByText(plugin.display_name)).toBeTruthy();
  expect(v.queryByText(en.library.emptyTitle)).toBeNull();
});

test('personal names, categories and favorites participate in library search and filtering', async () => {
  const v = mountLibrary();
  await v.findByText(en.library.emptyTitle);
  v.setData([plugin, { ...plugin, plugin_id: 'other', display_name: 'Other app', has_ui: true, has_service: false }], []);
  v.setOrganization({ revision: 1, collections: ['Writing'], items: [
    { plugin_id: plugin.plugin_id, pinned: true, collection_id: 'Writing', custom_name: 'Daily helper' },
    { plugin_id: 'other', pinned: false },
  ] });
  await act(async () => { v.changed(); });
  await v.findByText('Daily helper');
  fireEvent.change(v.getByRole('textbox', { name: en.library.search }), { target: { value: 'Writing' } });
  expect(v.getByText('Daily helper')).toBeTruthy();
  expect(v.queryByText('Other app')).toBeNull();
  fireEvent.change(v.getByRole('textbox', { name: en.library.search }), { target: { value: '' } });
  fireEvent.click(v.getByRole('button', { name: 'Writing' }));
  expect(v.queryByText('Other app')).toBeNull();
  fireEvent.click(v.getByRole('button', { name: en.library.allCategories }));
  expect(v.getByText('Other app')).toBeTruthy();
});

test('a failed organization save keeps the users name and category available for retry', async () => {
  spyOn(console, 'error').mockImplementation(() => undefined);
  const v = mountLibrary();
  await v.findByText(en.library.emptyTitle);
  v.setData([plugin], []);
  v.setOrganization({ revision: 1, collections: [], items: [{ plugin_id: plugin.plugin_id, pinned: false }] });
  await act(async () => { v.changed(); });
  await v.findByText(plugin.display_name);
  fireEvent.click(v.getByRole('button', { name: en.library.moreActions.replace('{{name}}', plugin.display_name) }));
  fireEvent.click(await v.findByText(en.library.organize));
  const dialog = await v.findByRole('dialog');
  const fields = dialog.querySelectorAll('input');
  fireEvent.change(fields[0]!, { target: { value: 'Personal name' } });
  fireEvent.change(fields[1]!, { target: { value: 'Writing' } });
  v.updateState.mockRejectedValueOnce(new Error('connection dropped'));
  await act(async () => { fireEvent.click(v.getByRole('button', { name: en.actions.save })); });
  await v.findAllByText(en.library.mutationFailed);
  expect(v.getByRole('dialog')).toBeTruthy();
  expect((fields[0] as HTMLInputElement).value).toBe('Personal name');
  expect((fields[1] as HTMLInputElement).value).toBe('Writing');
  await waitFor(() => expect(v.getByRole('button', { name: en.actions.save }).className).not.toContain('arco-btn-loading'));
  await act(async () => { fireEvent.click(v.getByRole('button', { name: en.actions.save })); });
  await v.findByText('Personal name');
  expect(v.updateState.mock.calls.at(-1)?.[0].items[0]).toMatchObject({ custom_name: 'Personal name', collection_id: 'Writing' });
});

test('creation history groups a session and its files once and keeps delivered work reopenable', async () => {
  const v = mountLibrary();
  await v.findByText(en.library.emptyTitle);
  v.setData([plugin], [{ ...draft, plugin_id: plugin.plugin_id, delivered_artifact_digest: plugin.active.artifact_digest }]);
  v.setSessions([{ conversation_id: draft.source_conversation_id!, name: 'Cleaner creation', created_at: 10, modified_at: 40 }]);
  await act(async () => { v.changed(); });
  fireEvent.click(v.getByRole('button', { name: en.workspace.views.drafts + ' 1' }));
  await v.findByText('Cleaner creation');
  expect(v.getAllByRole('button', { name: en.library.continue })).toHaveLength(1);
  fireEvent.click(v.getByRole('button', { name: en.library.continue }));
  await v.findByRole('heading', { name: 'Authoring task' });
});
