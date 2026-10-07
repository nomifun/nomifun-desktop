import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import * as arco from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useNavigate } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { conversation } from '@/common/adapter/ipcBridge';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type { PluginDetail, PluginSurfaceDescriptor } from '@/common/types/pluginPlatform';
import * as workspace from './PluginWorkspace';
import * as surface from './PluginSurfacePanel';
import PluginRunPage from './PluginRunPage';
import en from '../../services/i18n/locales/en-US/pluginPlatform.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { pluginPlatform: en } } } });
const changes = new Set<() => void>();
const details = new Map<string, PluginDetail>();
let nextSurface = 0;
const fixture = (id: string, overrides: Partial<PluginDetail['summary']> = {}): PluginDetail => ({
  summary: { plugin_id: id, package_id: `local.${id}`, display_name: id === 'plugin-a' ? 'Focus timer' : 'Quick notes', description: `Description for ${id}`,
    enabled: true, revision: 2, active: { artifact_digest: `${id}-bytes`, package_version: '1.0.0', data_generation: `${id}-data`, data_version: 1 },
    has_ui: true, has_service: false, action_count: 0, binding_count: 0, runtime: { state: 'stopped' }, updated_at_ms: 1, ...overrides },
  manifest: { schema: 'nomifun.plugin/v1', package_id: `local.${id}`, version: '1.0.0', name: id, description: '', host_api: '1',
    entrypoints: { ui: 'ui/index.html' }, actions: [], bindings: [], data_version: 1, config_schema: { type: 'object' }, secret_slots: [], permissions: [] },
  config: { schema: { type: 'object' }, values: { duration: 25 }, valid: true, validation_errors: [] }, credential_bindings: [], grants: [],
});
const descriptor = (id: string, session: string): PluginSurfaceDescriptor => ({ plugin_id: id, artifact_digest: `${id}-bytes`,
  surface_session_id: session, surface_generation: 1, entrypoint: 'ui/index.html', is_preview: false });
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(finish => { resolve = finish; });
  return { promise, resolve };
};

beforeEach(() => {
  (window as typeof window & { __backendPort?: number }).__backendPort = 11451;
  nextSurface = 0; changes.clear(); details.clear();
  details.set('plugin-a', fixture('plugin-a')); details.set('plugin-b', fixture('plugin-b'));
  spyOn(workspace, 'default').mockImplementation(({ children }) => <div>{children}</div>);
  // Keep actual settings fields and page persistence while removing animation
  // timers and focus management from this DOM-only integration test.
  const immediateModal = Object.assign((props: Parameters<typeof arco.Modal>[0]) => props.visible ? <section role='dialog'>
    <h2>{props.title}</h2>{props.children}<button onClick={() => props.onOk?.()}>{props.okText}</button>
    <button onClick={() => props.onCancel?.()}>{props.cancelText}</button>
  </section> : null, arco.Modal);
  spyOn(arco, 'Modal').mockImplementation(immediateModal);
  spyOn(surface, 'default').mockImplementation(({ descriptor }) => <div>App surface {descriptor.surface_session_id}</div>);
  spyOn(pluginPlatform.plugins.get, 'invoke').mockImplementation(async ({ plugin_id }) => details.get(plugin_id)!);
  spyOn(pluginPlatform.plugins.openSurface, 'invoke').mockImplementation(async ({ plugin_id }) => descriptor(plugin_id, `${plugin_id}-surface-${++nextSurface}`));
  spyOn(pluginPlatform.surface.close, 'invoke').mockResolvedValue(true);
  spyOn(pluginPlatform.libraryState.get, 'invoke').mockResolvedValue({ revision: 0, collections: [], items: [] });
  spyOn(pluginPlatform.credentials.list, 'invoke').mockResolvedValue([]);
  spyOn(conversation.reconnected, 'on').mockImplementation(() => () => {});
  spyOn(pluginPlatform.plugins.changed, 'on').mockImplementation(callback => {
    const refresh = () => callback(); changes.add(refresh);
    return () => { changes.delete(refresh); };
  });
});
afterEach(() => { cleanup(); mock.restore(); changes.clear(); delete (window as typeof window & { __backendPort?: number }).__backendPort; });

function mount(path = '/plugins/run/plugin-a') {
  const Navigate = () => {
    const navigate = useNavigate();
    return <nav><button onClick={() => navigate('/plugins/run/plugin-a')}>View timer</button><button onClick={() => navigate('/plugins/run/plugin-b')}>View notes</button></nav>;
  };
  return render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={[path]}><Navigate /><Routes>
    <Route path='/plugins/run/:id' element={<PluginRunPage />} />
  </Routes></MemoryRouter></I18nextProvider>);
}
async function manage(view: ReturnType<typeof mount>) {
  await view.findByRole('heading', { name: 'Focus timer' });
  fireEvent.click(view.getByRole('tab', { name: en.detail.tabs.manage }));
}
async function changed() { await act(async () => { changes.forEach(callback => callback()); }); }

test('a failed configuration save keeps the dialog and edited values ready to retry', async () => {
  const configure = spyOn(pluginPlatform.plugins.configure, 'invoke').mockRejectedValueOnce(new Error('Connection interrupted'));
  const view = mount();
  await manage(view);
  fireEvent.click(view.getByRole('button', { name: en.actions.configure }));
  await view.findByText(en.config.title);
  const input = view.getByRole('textbox', { name: en.config.values }) as HTMLTextAreaElement;
  fireEvent.input(input, { target: { value: '{"duration":45,"theme":"violet"}' } });
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.actions.save })); });
  await within(view.getByRole('dialog')).findByText(en.detail.operationFailed);
  expect(input.value).toBe('{"duration":45,"theme":"violet"}');
  expect(view.getByText(en.config.title)).toBeTruthy();
  configure.mockImplementation(async ({ request }) => {
    const updated = { ...details.get('plugin-a')!, config: { ...details.get('plugin-a')!.config, values: request.config } };
    details.set('plugin-a', updated); return updated;
  });
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.actions.save })); });
  await waitFor(() => expect(view.queryByText(en.config.title)).toBeNull());
  expect(configure).toHaveBeenCalledTimes(2);
  expect(configure.mock.calls[0][0].request.config).toEqual(configure.mock.calls[1][0].request.config);
});

test('fast plugin navigation ignores late details and releases a preview that opens after leaving', async () => {
  const firstDetail = deferred<PluginDetail>();
  const lateSurface = deferred<PluginSurfaceDescriptor>();
  let firstRead = true;
  const get = spyOn(pluginPlatform.plugins.get, 'invoke').mockImplementation(async ({ plugin_id }) => {
    if (plugin_id === 'plugin-a' && firstRead) { firstRead = false; return firstDetail.promise; }
    return details.get(plugin_id)!;
  });
  const open = spyOn(pluginPlatform.plugins.openSurface, 'invoke').mockImplementation(async ({ plugin_id }) =>
    plugin_id === 'plugin-a' ? lateSurface.promise : descriptor(plugin_id, `${plugin_id}-current`));
  const close = spyOn(pluginPlatform.surface.close, 'invoke');
  const view = mount();
  await waitFor(() => expect(get).toHaveBeenCalledTimes(1));
  fireEvent.click(view.getByRole('button', { name: 'View notes' }));
  await view.findByText('App surface plugin-b-current');
  await act(async () => { firstDetail.resolve(details.get('plugin-a')!); });
  expect(view.getByRole('heading', { name: 'Quick notes' })).toBeTruthy();
  expect(open.mock.calls.filter(call => call[0].plugin_id === 'plugin-a')).toHaveLength(0);
  fireEvent.click(view.getByRole('button', { name: 'View timer' }));
  await waitFor(() => expect(open.mock.calls.filter(call => call[0].plugin_id === 'plugin-a')).toHaveLength(1));
  fireEvent.click(view.getByRole('button', { name: 'View notes' }));
  await view.findByText('App surface plugin-b-current');
  await act(async () => { lateSurface.resolve(descriptor('plugin-a', 'late-timer')); });
  expect(view.getByRole('heading', { name: 'Quick notes' })).toBeTruthy();
  expect(view.queryByText('App surface late-timer')).toBeNull();
  expect(close.mock.calls.some(call => call[0].request.surface_session_id === 'late-timer')).toBe(true);
});

test('library organization refreshes preserve the app while plugin configuration and lifecycle changes reload it', async () => {
  const open = spyOn(pluginPlatform.plugins.openSurface, 'invoke');
  const close = spyOn(pluginPlatform.surface.close, 'invoke');
  const view = mount();
  await view.findByText('App surface plugin-a-surface-1');
  const original = details.get('plugin-a')!;
  await changed();
  expect(open).toHaveBeenCalledTimes(1); expect(close).not.toHaveBeenCalled();
  details.set('plugin-a', { ...original, summary: { ...original.summary, revision: 3, display_name: 'Updated focus timer' },
    config: { ...original.config, values: { duration: 50 } } });
  await changed();
  await view.findByRole('heading', { name: 'Updated focus timer' });
  await view.findByText('App surface plugin-a-surface-2');
  expect(open).toHaveBeenCalledTimes(2); expect(close).toHaveBeenCalledTimes(1);
  const update = details.get('plugin-a')!;
  details.set('plugin-a', { ...update, summary: { ...update.summary, revision: 4, active: { ...update.summary.active, artifact_digest: 'new-app-bytes' } } });
  await changed();
  await view.findByText('App surface plugin-a-surface-3');
  expect(close).toHaveBeenCalledTimes(2);
  const revised = details.get('plugin-a')!;
  details.set('plugin-a', { ...revised, summary: { ...revised.summary, revision: 5, enabled: false } });
  await changed();
  await view.findByRole('heading', { name: en.detail.disabledTitle });
  expect(open).toHaveBeenCalledTimes(3); expect(close).toHaveBeenCalledTimes(3);
  details.set('plugin-a', { ...revised, summary: { ...revised.summary, revision: 6, enabled: true } });
  await changed();
  await view.findByText('App surface plugin-a-surface-4');
  details.set('plugin-a', { ...revised, summary: { ...revised.summary, revision: 7, trashed_at_ms: 100 } });
  await changed();
  await view.findByRole('heading', { name: en.library.trashed });
  expect(open).toHaveBeenCalledTimes(4); expect(close).toHaveBeenCalledTimes(4);
});

test('restoring from trash uses the current identity without a version or restore-mode choice', async () => {
  details.set('plugin-a', fixture('plugin-a', { enabled: false, trashed_at_ms: 100 }));
  const restore = spyOn(pluginPlatform.plugins.restore, 'invoke').mockImplementation(async () => {
    const restored = fixture('plugin-a', { revision: 3 }); details.set('plugin-a', restored); return restored;
  });
  const open = spyOn(pluginPlatform.plugins.openSurface, 'invoke');
  const view = mount();
  await view.findByRole('heading', { name: en.library.trashed });
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.actions.restoreTrash })); });
  await view.findByText('App surface plugin-a-surface-1');
  expect(open).toHaveBeenCalledTimes(1);
  expect(restore.mock.calls[0][0]).toEqual({ plugin_id: 'plugin-a', request: { expected_revision: 2 } });
});

test('package and backup exports use a selected native folder and report the actual destination', async () => {
  const chooser = spyOn(ipcBridge.dialog.showOpen, 'invoke').mockResolvedValue(['C:/Plugin exports/']);
  const exportPackage = spyOn(pluginPlatform.plugins.exportPackage, 'invoke').mockImplementation(async ({ request }) => ({ destination_path: request.destination_path, digest: 'package', size_bytes: 42 }));
  const exportBackup = spyOn(pluginPlatform.plugins.exportBackup, 'invoke').mockImplementation(async ({ request }) => ({ destination_path: request.destination_path, digest: 'backup', size_bytes: 84 }));
  const view = mount();
  await manage(view);
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.actions.exportPackage })); });
  expect(chooser.mock.calls[0][0]).toEqual({ properties: ['openDirectory'] });
  const packageRequest = exportPackage.mock.calls[0][0];
  expect(packageRequest.plugin_id).toBe('plugin-a');
  expect(packageRequest.request).toMatchObject({ expected_revision: 2, include_source: true });
  expect(packageRequest.request.destination_path).toMatch(/^C:\/Plugin exports\/local.plugin-a-\d+\.zip$/);
  await view.findByText(i18n.t('pluginPlatform.detail.exported', { path: packageRequest.request.destination_path }));
  await act(async () => { fireEvent.click(view.getByRole('button', { name: en.actions.exportBackup })); });
  const backupRequest = exportBackup.mock.calls[0][0];
  expect(backupRequest.request.destination_path).toMatch(/^C:\/Plugin exports\/local.plugin-a-backup-\d+\.zip$/);
  expect(backupRequest.request.expected_revision).toBe(2);
  await view.findByText(i18n.t('pluginPlatform.detail.exported', { path: backupRequest.request.destination_path }));
});

test('direct plugin actions use named parameter fields and submit their actual values', async () => {
  const plugin = fixture('plugin-a', { has_ui: false, has_service: true });
  plugin.manifest.entrypoints = { service: 'service/main.mjs' };
  plugin.manifest.actions = [{ action_id: 'clean', stable_id: 'plugin-a/clean', name: 'Clean text', description: 'Clean the supplied text',
    input_schema: { type: 'object', properties: { text: { type: 'string', title: 'Text', default: 'Hello' }, uppercase: { type: 'boolean', title: 'Uppercase' } }, required: ['text'] },
    output_schema: { type: 'object' }, effect: 'read' }];
  plugin.manifest.bindings = [{ point: 'desktop.command', action_id: 'clean', optional: false, supported: true }];
  details.set('plugin-a', plugin);
  const invoke = spyOn(pluginPlatform.desktop.invoke, 'invoke').mockResolvedValue({ text: 'MILK' });
  const view = mount();
  await view.findByRole('heading', { name: 'Focus timer' });
  fireEvent.click(view.getByRole('button', { name: en.command.run }));
  const dialog = within(view.getByRole('dialog'));
  expect((dialog.getByRole('textbox', { name: 'Text' }) as HTMLInputElement).value).toBe('Hello');
  fireEvent.change(dialog.getByRole('textbox', { name: 'Text' }), { target: { value: 'milk' } });
  fireEvent.click(dialog.getByRole('checkbox', { name: 'Uppercase' }));
  await act(async () => { fireEvent.click(dialog.getByRole('button', { name: en.command.run })); });
  expect(invoke.mock.calls[0]?.[0]).toEqual({ action_id: 'plugin-a/clean', input: { text: 'milk', uppercase: true } });
  await view.findByRole('heading', { name: en.command.result });
});
