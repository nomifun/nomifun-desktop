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
import type { PluginDraftSummary, PluginLibraryResponse, PluginSummary } from '@/common/types/pluginPlatform';
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
  spyOn(pluginPlatform.libraryState.get, 'invoke').mockResolvedValue({ revision: 0, collections: [], items: [] });
  const preflight = spyOn(pluginPlatform.authoring.preflight, 'invoke').mockResolvedValue({ status: 'ready', reason: '', owner_user_id: 'owner',
    selection: { kind: 'template', templateKey: 'assistant.general' } });
  spyOn(importDialog, 'default').mockImplementation(() => <></>);
  const BackToLibrary = () => { const navigate = useNavigate(); return <button onClick={() => navigate('/plugins')}>Return to library</button>; };
  const view = render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={['/plugins']}><Routes>
    <Route path='/plugins' element={<PluginLibraryPage />} />
    <Route path='/guid' element={<div><h1>Plugin conversation</h1><BackToLibrary /></div>} />
    <Route path='/conversation/:id' element={<div><h1>Source conversation</h1><BackToLibrary /></div>} />
    <Route path='/plugins/run/:id' element={<h1>Installed plugin</h1>} />
  </Routes></MemoryRouter></I18nextProvider>);
  return { ...view, list, preflight, changes, reconnects,
    setData: (nextPlugins: PluginSummary[], nextDrafts: PluginDraftSummary[]) => { plugins = nextPlugins; drafts = nextDrafts; },
    changed: () => { changes.forEach(callback => callback()); },
    reconnected: () => { reconnects.forEach(callback => callback()); },
  };
}

test('creation enters conversation and returning lists its draft and installed result with usable links', async () => {
  const v = mountLibrary();
  await v.findByText(en.library.emptyTitle);
  fireEvent.click(v.getByRole('button', { name: en.actions.create }));
  await v.findByRole('heading', { name: 'Plugin conversation' });
  expect(v.preflight.mock.calls[0][0]).toEqual({ selection: { kind: 'template', templateKey: 'assistant.general' } });
  v.setData([], [draft]);
  fireEvent.click(v.getByRole('button', { name: 'Return to library' }));
  await v.findByText(draft.display_name);
  fireEvent.click(v.getByRole('button', { name: en.library.continue }));
  await v.findByRole('heading', { name: 'Source conversation' });
  v.setData([plugin], [{ ...draft, plugin_id: plugin.plugin_id, base_plugin_revision: 1 }]);
  fireEvent.click(v.getByRole('button', { name: 'Return to library' }));
  await v.findByText(plugin.display_name);
  fireEvent.click(v.getByRole('button', { name: /^Saved text cleaner/ }));
  await v.findByRole('heading', { name: 'Installed plugin' });
});

test('conversation changes and reconnects update an already open library and its counts', async () => {
  const v = mountLibrary();
  await v.findByText(en.library.emptyTitle);
  v.setData([], [draft]);
  await act(async () => { v.changed(); });
  await v.findByText(draft.display_name);
  expect(v.getByRole('button', { name: `${en.workspace.views.all} 1` })).toBeTruthy();
  v.setData([plugin], []);
  await act(async () => { v.reconnected(); });
  await v.findByText(plugin.display_name);
  expect(v.queryByText(draft.display_name)).toBeNull();
  expect(v.getByRole('button', { name: `${en.workspace.views.enabled} 1` })).toBeTruthy();
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
