import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { useState } from 'react';
import { Modal } from '@arco-design/web-react';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { HostedApp } from '@/common/types/appHosting';
import { asAgentPresetId, type AgentPresetSummary, type OfficialPresetTemplate } from '@/common/types/agentPlatform';
import locale from '../../services/i18n/locales/en-US/appHosting.json';
import common from '../../services/i18n/locales/en-US/common.json';
import agentSettings from '../../services/i18n/locales/en-US/agentSettings.json';
import AppsPage from './index';
import AppSettingsForm from './AppSettingsForm';
import AppWorkspace from './AppWorkspace';
import AppImportForm from './AppImportForm';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { appHosting: locale, common, agentSettings } } }, interpolation: { escapeValue: false } });
const app = (id: string, name: string): HostedApp => ({ id, name, description: '', tags: [], development_session_id: null, directory: `C:/host/apps/${id}`, import_kind: 'source',
  settings: { project_directory: '.', environment: {}, install_command: 'bun install', start_command: 'bun run start' },
  state: 'stopped', operation: null, last_error: null, output: '', pid: null, preview: false, prepared: false });
const wrapper = ({ children }: { children: React.ReactNode }) => <I18nextProvider i18n={i18n}>
  <MemoryRouter><SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}>{children}</SWRConfig></MemoryRouter></I18nextProvider>;
beforeEach(() => {
  spyOn(agentPlatform.library, 'invoke').mockResolvedValue({ user_presets: [], official_presets: {}, official_templates: [], active_bindings: [],
    fresh_start: { data_generation: 6, legacy_data_imported: false, user_preset_count: 0, official_template_count: 0 } });
});
afterEach(() => { cleanup(); mock.restore(); });

test('application deletion confirms its complete project Session count and preserves the ordinary conversation boundary', async () => {
  const confirmation = spyOn(Modal, 'confirm').mockReturnValue({ close: mock(() => {}), update: mock(() => {}) });
  const remove = spyOn(appHosting.delete, 'invoke').mockResolvedValue(undefined);
  const deleted = mock(() => {});
  const value = app('counted-app', 'Counted app');
  const view = render(<AppWorkspace app={value} mode='edit' projectSessionCount={31} conversation={<p>Project history</p>}
    onAppChanged={() => {}} onRefresh={() => {}} onDeleted={deleted} />, { wrapper });
  fireEvent.click(view.getByRole('button', { name: locale.actions.delete }));
  const dialog = confirmation.mock.calls[0][0];
  expect(dialog.content).toBe(locale.delete.hintWithSessions.replace('{{count}}', '31'));
  expect(String(dialog.content)).toContain('including archived sessions');
  expect(String(dialog.content)).toContain('Ordinary conversations');
  await act(async () => { await dialog.onOk?.(); });
  expect(remove).toHaveBeenCalledWith({ id: value.id }); expect(deleted).toHaveBeenCalledTimes(1);
});

// Run controls live in the app inspector. Each real workspace owns its UI
// operation state while the Host owns the independent application processes.
function Workspaces({ initial }: { initial: HostedApp[] }) {
  const [apps, setApps] = useState(initial);
  const accept = (value: HostedApp) => setApps(current => current.map(existing => existing.id === value.id ? value : existing));
  const refresh = () => { void appHosting.list.invoke().then(setApps); };
  return <>{apps.map(value => <article key={value.id}><AppWorkspace app={value} mode='edit' conversation={<p>Ordinary conversation</p>}
    onAppChanged={accept} onRefresh={refresh} onDeleted={() => {}} /></article>)}</>;
}

test('a pending app operation does not block another app, and errors remain visible', async () => {
  const a = app('a', 'Slow app'), b = app('b', 'Other app');
  spyOn(appHosting.list, 'invoke').mockResolvedValue([a, b]);
  let rejectA!: (e: Error) => void;
  const operate = spyOn(appHosting.operate, 'invoke').mockImplementation(({ id }) => id === 'a'
    ? new Promise<HostedApp>((_, reject) => { rejectA = reject; }) : Promise.resolve({ ...b, state: 'running', pid: 22 }));
  const view = render(<Workspaces initial={[a, b]} />, { wrapper });
  const aCard = (await view.findByRole('heading', { name: a.name })).closest('article')!;
  const bCard = view.getByRole('heading', { name: b.name }).closest('article')!;
  fireEvent.click(within(aCard).getByRole('button', { name: locale.actions.use }));
  fireEvent.click(within(aCard).getByRole('button', { name: locale.actions.use }));
  expect(operate.mock.calls.filter(([request]) => request.id === 'a')).toHaveLength(1);
  expect((within(bCard).getByRole('button', { name: locale.actions.use }) as HTMLButtonElement).disabled).toBe(false);
  fireEvent.click(within(bCard).getByRole('button', { name: locale.actions.use }));
  await within(bCard).findByRole('button', { name: locale.actions.stop });
  await act(async () => rejectA(new Error('bun was not found')));
  await view.findByText('bun was not found');
});

test('apps without webpage settings retain a preview action and can still start directly', async () => {
  const background = app('background', 'Background service');
  spyOn(appHosting.list, 'invoke').mockResolvedValue([background]);
  const view = render(<Workspaces initial={[background]} />, { wrapper });
  await view.findByRole('heading', { name: 'Background service' });
  expect(view.getByRole('button', { name: locale.actions.preview })).toBeTruthy();
  expect(view.queryByRole('button', { name: locale.actions.open })).toBeNull();
  expect(view.getByRole('button', { name: locale.actions.use })).toBeTruthy();
  expect(view.getByText(locale.workspace.noWebpage)).toBeTruthy();
});

test('a generated app with no run settings previews the same owner and loads only its returned webpage', async () => {
  const generated = { ...app('generated', 'New application'), development_session_id: 'ordinary-session', settings: { project_directory: '.', environment: {} } };
  const server = createServer((_request, response) => { response.writeHead(200, { 'Content-Type': 'text/html' }); response.end('<!doctype html><title>Generated game</title>'); });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const url = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  try {
    let finish!: (value: HostedApp) => void;
    const operate = spyOn(appHosting.operate, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const create = spyOn(appHosting.create, 'invoke');
    const imported = spyOn(appHosting.import, 'invoke');
    const development = spyOn(appHosting.developmentSession, 'invoke');
    const view = render(<Workspaces initial={[generated]} />, { wrapper });
    expect(view.container.querySelector('iframe')).toBeNull();
    const preview = view.getByRole('button', { name: locale.actions.preview });
    fireEvent.click(preview); fireEvent.click(preview);
    expect(operate.mock.calls).toEqual([[{ id: generated.id, operation: 'preview' }]]);
    expect(view.container.querySelector('iframe')).toBeNull();
    await act(async () => finish({ ...generated, state: 'running', preview: true, pid: 42, prepared: true,
      settings: { ...generated.settings, preview_command: 'node .nomifun-static-server.cjs', web_url: url } }));
    await waitFor(() => expect(view.container.querySelector('iframe')?.getAttribute('src')).toBe(url));
    expect(view.getByRole('dialog').contains(view.container.querySelector('iframe'))).toBe(true);
    expect(view.getByText(locale.state.running)).toBeTruthy();
    expect(create).not.toHaveBeenCalled(); expect(imported).not.toHaveBeenCalled(); expect(development).not.toHaveBeenCalled();
    await (window as unknown as { happyDOM: { waitUntilComplete: () => Promise<void> } }).happyDOM.waitUntilComplete();
  } finally {
    cleanup(); server.closeAllConnections(); await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});

test('preview failures remain visible and do not turn a model success claim into a running owner', async () => {
  const generated = { ...app('failed-preview', 'New application'), settings: { project_directory: '.', environment: {} } };
  const operate = spyOn(appHosting.operate, 'invoke').mockRejectedValue(new Error('No single webpage project was found'));
  const refresh = mock(() => {});
  const view = render(<AppWorkspace app={generated} mode='edit' conversation={<p>Created and running successfully.</p>}
    onAppChanged={() => {}} onRefresh={refresh} onDeleted={() => {}} />, { wrapper });
  fireEvent.click(view.getByRole('button', { name: locale.actions.preview }));
  await view.findByText('No single webpage project was found');
  expect(operate.mock.calls).toEqual([[{ id: generated.id, operation: 'preview' }]]);
  expect(refresh).toHaveBeenCalledTimes(1);
  expect(view.getByText(locale.state.stopped)).toBeTruthy();
  expect(view.container.querySelector('iframe')).toBeNull();
  expect(view.queryByRole('button', { name: locale.workspace.openWindow })).toBeNull();
  expect(view.getByRole('button', { name: locale.actions.preview })).toBeTruthy();
});

test('a running owner without a webpage still has the preview action, disabled while files are being generated', async () => {
  const generated = { ...app('generating', 'Generating app'), state: 'running' as const, pid: 55, settings: { project_directory: '.', environment: {} } };
  const operate = spyOn(appHosting.operate, 'invoke');
  const view = await act(async () => render(<AppWorkspace app={generated} mode='edit' developmentBusy conversation={<p>Generating files</p>}
    onAppChanged={() => {}} onRefresh={() => {}} onDeleted={() => {}} />, { wrapper }));
  const preview = view.getByRole('button', { name: locale.actions.preview }) as HTMLButtonElement;
  expect(preview.disabled).toBe(true);
  fireEvent.click(preview); expect(operate).not.toHaveBeenCalled();
  view.rerender(<AppWorkspace app={generated} mode='edit' conversation={<p>Generation finished</p>}
    onAppChanged={() => {}} onRefresh={() => {}} onDeleted={() => {}} />);
  expect((view.getByRole('button', { name: locale.actions.preview }) as HTMLButtonElement).disabled).toBe(false);
});

test.each(['response', 'error'] as const)('Stop supersedes preparation and ignores its late %s without blocking other apps', async (late) => {
  let a = app('a', 'Preparing app');
  let b = app('b', 'Other app');
  spyOn(appHosting.list, 'invoke').mockImplementation(async () => [a, b]);
  let resolvePrepare!: (app: HostedApp) => void;
  let rejectPrepare!: (error: Error) => void;
  const operate = spyOn(appHosting.operate, 'invoke').mockImplementation(({ id, operation }) => {
    if (id === 'a' && operation === 'use') {
      a = { ...a, operation: 'installing' };
      return new Promise<HostedApp>((resolve, reject) => { resolvePrepare = resolve; rejectPrepare = reject; });
    }
    if (id === 'a' && operation === 'stop') {
      a = { ...a, operation: null, state: 'stopped', pid: null, last_error: null };
      return Promise.resolve(a);
    }
    b = { ...b, operation: null, state: 'running', pid: 22 };
    return Promise.resolve(b);
  });
  const view = render(<Workspaces initial={[a, b]} />, { wrapper });
  const aCard = (await view.findByRole('heading', { name: a.name })).closest('article')!;
  const bCard = view.getByRole('heading', { name: b.name }).closest('article')!;
  fireEvent.click(within(aCard).getByRole('button', { name: locale.actions.use }));
  fireEvent.click(within(aCard).getByRole('button', { name: locale.actions.refresh }));
  const stop = await within(aCard).findByRole('button', { name: locale.actions.stop });
  expect((stop as HTMLButtonElement).disabled).toBe(false);
  fireEvent.click(stop); fireEvent.click(stop);
  await waitFor(() => expect(operate.mock.calls.filter(([request]) => request.id === 'a' && request.operation === 'stop')).toHaveLength(1));
  await waitFor(() => expect((within(aCard).getByRole('button', { name: locale.actions.use }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(within(bCard).getByRole('button', { name: locale.actions.use }));
  await within(bCard).findByRole('button', { name: locale.actions.stop });
  await act(async () => {
    if (late === 'response') resolvePrepare({ ...a, state: 'running', pid: 99 });
    else rejectPrepare(new Error('superseded install failed'));
  });
  expect(within(aCard).queryByRole('button', { name: locale.actions.stop })).toBeNull();
  expect(view.queryByText('superseded install failed')).toBeNull();
  expect(within(aCard).getByText(locale.state.stopped)).toBeTruthy();
});

test('settings polling cannot overwrite unsaved commands and saving uses the current draft', async () => {
  const original = app('a', 'My app');
  const save = spyOn(appHosting.update, 'invoke').mockResolvedValue(original);
  const view = render(<AppSettingsForm app={original} onSaved={() => {}} />, { wrapper });
  const command = view.getByRole('textbox', { name: locale.settings.start_command });
  fireEvent.input(command, { target: { value: 'node custom-server.js' } });
  view.rerender(<AppSettingsForm app={{ ...original, settings: { ...original.settings, start_command: 'different command' } }} onSaved={() => {}} />);
  expect((command as HTMLInputElement).value).toBe('node custom-server.js');
  fireEvent.click(view.getByRole('button', { name: common.save }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0][0]).toEqual({ id: original.id, settings: { start_command: 'node custom-server.js' } });
});

test('compact metadata edits survive folding and owner refresh while commands remain a separate section', async () => {
  const original = app('compact', 'My app');
  const save = spyOn(appHosting.update, 'invoke').mockResolvedValue(original);
  const view = render(<AppSettingsForm app={original} compact onSaved={() => {}} />, { wrapper });
  const edit = view.getByText(locale.actions.edit, { selector: 'summary' });
  const metadata = edit.closest('details')!;
  const advanced = view.getByText(locale.workspace.advanced, { selector: 'summary' }).closest('details')!;
  expect(metadata.open).toBe(false);
  expect(advanced.parentElement).toBe(metadata.parentElement);
  fireEvent.click(edit);
  fireEvent.input(view.getByRole('textbox', { name: locale.name }), { target: { value: 'Renamed application' } });
  fireEvent.input(view.getByRole('textbox', { name: locale.description }), { target: { value: 'Keep my description' } });
  fireEvent.input(view.getByRole('textbox', { name: locale.tags }), { target: { value: 'Life, Tools' } });
  fireEvent.click(edit);
  view.rerender(<AppSettingsForm app={{ ...original, name: 'Polled name', description: 'Polled description' }} compact onSaved={() => {}} />);
  fireEvent.click(edit);
  expect((view.getByRole('textbox', { name: locale.name }) as HTMLInputElement).value).toBe('Renamed application');
  fireEvent.click(view.getByRole('button', { name: common.save }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0][0]).toEqual({ id: original.id, name: 'Renamed application', description: 'Keep my description', tags: ['Life', 'Tools'] });
});

test('Agent updates fill untouched fields during a manual draft and saving emits only the edited fields', async () => {
  const original = { ...app('linked-settings', 'New application'), settings: { project_directory: '.', environment: {} } };
  const configured = { ...original, name: 'Snake game', description: 'A keyboard game', settings: {
    project_directory: 'snake-game', environment: { PORT: '4317' }, preview_command: 'node .nomifun-static-server.cjs',
    start_command: 'node .nomifun-static-server.cjs', web_url: 'http://127.0.0.1:4317',
  } };
  const save = spyOn(appHosting.update, 'invoke').mockResolvedValue({ ...configured, description: 'My description' });
  const view = render(<AppSettingsForm app={original} onSaved={() => {}} />, { wrapper });
  fireEvent.input(view.getByRole('textbox', { name: locale.description }), { target: { value: 'My description' } });
  view.rerender(<AppSettingsForm app={configured} onSaved={() => {}} />);
  expect((view.getByRole('textbox', { name: locale.name }) as HTMLInputElement).value).toBe('Snake game');
  expect((view.getByRole('textbox', { name: locale.description }) as HTMLInputElement).value).toBe('My description');
  expect((view.getByRole('textbox', { name: locale.settings.project_directory }) as HTMLInputElement).value).toBe('snake-game');
  expect((view.getByRole('textbox', { name: locale.settings.preview_command }) as HTMLInputElement).value).toBe(configured.settings.preview_command);
  expect((view.getByRole('textbox', { name: locale.settings.web_url }) as HTMLInputElement).value).toBe(configured.settings.web_url);
  expect(view.getByText(locale.settings.unsaved)).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: common.save }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls).toEqual([[{ id: original.id, description: 'My description' }]]);
});

test('manual configuration detection fills the same owner without starting it or discarding drafts', async () => {
  const original = { ...app('detect-settings', 'My application'), settings: { project_directory: '.', environment: {} } };
  const configured = { ...original, settings: { project_directory: 'web', environment: {}, start_command: 'node .nomifun-static-server.cjs' } };
  const update = spyOn(appHosting.update, 'invoke').mockResolvedValue(configured);
  const operate = spyOn(appHosting.operate, 'invoke');
  let accepted: HostedApp | undefined;
  const view = render(<AppSettingsForm app={original} onSaved={value => { accepted = value; }} />, { wrapper });
  fireEvent.input(view.getByRole('textbox', { name: locale.name }), { target: { value: 'My unsaved name' } });
  fireEvent.click(view.getByRole('button', { name: locale.settings.detect }));
  await waitFor(() => expect(accepted).toBe(configured));
  view.rerender(<AppSettingsForm app={configured} onSaved={() => {}} />);
  expect((view.getByRole('textbox', { name: locale.name }) as HTMLInputElement).value).toBe('My unsaved name');
  expect((view.getByRole('textbox', { name: locale.settings.start_command }) as HTMLInputElement).value).toBe(configured.settings.start_command);
  expect(update.mock.calls).toEqual([[{ id: original.id, detect_settings: true }]]);
  expect(operate).not.toHaveBeenCalled();
});

test('running apps expose a stop-first hint and disable runtime Agent rebinding', async () => {
  const view = await act(async () => render(<AppSettingsForm app={{ ...app('running', 'Running app'), state: 'running', pid: 33 }} onSaved={() => {}} />, { wrapper }));
  expect(view.getByText(locale.workspace.stopToBindAgent)).toBeTruthy();
  const picker = view.getByRole('combobox', { name: locale.settings.agent_id }) as HTMLElement;
  expect(picker.getAttribute('aria-disabled')).toBe('true');
});

test('app runtime Agent selection includes saved official configurations alongside personal Agents', async () => {
  const official: AgentPresetSummary = { preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000101'),
    source: 'official', display_name: 'chat.minimal', bound_target_count: 0 };
  const personal: AgentPresetSummary = { ...official, preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000102'),
    source: 'user', display_name: 'My text Agent' };
  const companion: AgentPresetSummary = { ...official, preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000103'),
    display_name: 'companion.default' };
  spyOn(agentPlatform.library, 'invoke').mockResolvedValue({
    official_templates: [{ template_key: 'chat.minimal' }, { template_key: 'companion.default' }] as OfficialPresetTemplate[],
    official_presets: { 'chat.minimal': official, 'companion.default': companion }, user_presets: [personal], active_bindings: [],
    fresh_start: { data_generation: 6, legacy_data_imported: false, user_preset_count: 1, official_template_count: 2 },
  });
  const original = app('official-runtime', 'Keyboard');
  const update = spyOn(appHosting.update, 'invoke').mockResolvedValue({ ...original, settings: { ...original.settings, agent_id: official.preset_id } });
  const view = render(<AppSettingsForm app={original} onSaved={() => {}} />, { wrapper });
  const picker = view.getByRole('combobox', { name: locale.settings.agent_id });
  // Arco renders a native input internally. An enclosing HTML label forwards
  // another click to it in real browsers and immediately closes the popup.
  expect(picker.closest('label')).toBeNull();
  fireEvent.click(picker);
  const option = await view.findByRole('option', { name: agentSettings.template.chat.minimal.name });
  expect(view.getByRole('option', { name: personal.display_name })).toBeTruthy();
  expect(view.queryByRole('option', { name: agentSettings.template.companion.default.name })).toBeNull();
  fireEvent.click(option);
  fireEvent.click(view.getByRole('button', { name: common.save }));
  await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
  expect(update.mock.calls[0][0]).toEqual({ id: original.id, settings: { agent_id: official.preset_id } });
});

test('an already selected official Agent remains available without creating a personal copy', async () => {
  const id = asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000104');
  spyOn(agentPlatform.library, 'invoke').mockResolvedValue({ official_templates: [{ template_key: 'assistant.general' }] as OfficialPresetTemplate[],
    official_presets: { 'assistant.general': { preset_id: id, source: 'official', display_name: 'assistant.general', bound_target_count: 0 } },
    user_presets: [], active_bindings: [], fresh_start: { data_generation: 6, legacy_data_imported: false, user_preset_count: 0, official_template_count: 1 } });
  const create = spyOn(agentPlatform.createFromTemplate, 'invoke');
  const value = app('official-saved', 'Keyboard');
  const view = render(<AppSettingsForm app={{ ...value, settings: { ...value.settings, agent_id: id } }} onSaved={() => {}} />, { wrapper });
  await view.findByText(agentSettings.template.assistant.general.name);
  expect(view.queryByText(locale.settings.unavailableAgent)).toBeNull();
  expect(create).not.toHaveBeenCalled();
});

test('the unified start command is visibly used for preview while null install and build steps remain explicit skips', async () => {
  const unified = { ...app('unified-command', 'Static application'), settings: { project_directory: '.', environment: {},
    start_command: 'node .nomifun-static-server.cjs', preview_command: null, install_command: null, build_command: null } };
  const update = spyOn(appHosting.update, 'invoke').mockResolvedValue(unified);
  const view = render(<AppSettingsForm app={unified} onSaved={() => {}} />, { wrapper });
  const preview = view.getByRole('textbox', { name: locale.settings.preview_command }) as HTMLInputElement;
  expect(preview.value).toBe('');
  expect(preview.placeholder).toBe(locale.settings.previewFallback);
  expect(view.getByText(locale.settings.effectivePreview.replace('{{command}}', unified.settings.start_command))).toBeTruthy();
  expect((view.getByRole('textbox', { name: locale.settings.install_command }) as HTMLInputElement).placeholder).toBe(locale.settings.skippedStep);
  expect((view.getByRole('textbox', { name: locale.settings.build_command }) as HTMLInputElement).placeholder).toBe(locale.settings.skippedStep);
  expect(update).not.toHaveBeenCalled();
  fireEvent.input(view.getByRole('textbox', { name: locale.description }), { target: { value: 'A static webpage' } });
  fireEvent.click(view.getByRole('button', { name: common.save }));
  await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
  expect(update.mock.calls[0][0]).toEqual({ id: unified.id, description: 'A static webpage' });
  fireEvent.input(preview, { target: { value: 'bun run preview' } });
  expect(view.queryByText(locale.settings.effectivePreview.replace('{{command}}', unified.settings.start_command))).toBeNull();
});

test('home exposes create and import routes, filters tags and searches metadata', async () => {
  const operate = spyOn(appHosting.operate, 'invoke');
  spyOn(appHosting.list, 'invoke').mockResolvedValue([
    { ...app('a', 'Todo'), description: 'Task reminders', tags: ['Productivity'] },
    { ...app('b', 'Music'), description: 'A playlist', tags: ['Life'] },
  ]);
  const Location = () => <span data-testid='location'>{useLocation().pathname}</span>;
  const view = render(<><AppsPage /><Location /></>, { wrapper });
  const card = (await view.findByRole('heading', { name: 'Todo' })).closest('article')!;
  expect(within(card).getAllByRole('button')).toHaveLength(1);
  fireEvent.click(view.getByRole('button', { name: 'Life' }));
  expect(view.queryByRole('heading', { name: 'Todo' })).toBeNull();
  expect(view.getByRole('heading', { name: 'Music' })).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: locale.home.all }));
  fireEvent.input(view.getByRole('textbox', { name: locale.home.search }), { target: { value: 'reminders' } });
  expect(view.getByRole('heading', { name: 'Todo' })).toBeTruthy();
  expect(view.queryByRole('heading', { name: 'Music' })).toBeNull();
  fireEvent.click(view.getByRole('button', { name: locale.create.action }));
  expect(view.getByTestId('location').textContent).toBe('/apps/new');
  fireEvent.click(view.getByRole('button', { name: locale.import.action }));
  expect(view.getByTestId('location').textContent).toBe('/apps/import');
  fireEvent.click(view.getByRole('button', { name: 'Edit Todo' }));
  expect(view.getByTestId('location').textContent).toBe('/apps/a');
  expect(operate).not.toHaveBeenCalled();
});

test('manual import works without a model or development Session', async () => {
  const imported = app('imported', 'Imported app');
  const request = spyOn(appHosting.import, 'invoke').mockResolvedValue(imported);
  const session = spyOn(appHosting.developmentSession, 'invoke');
  let accepted: HostedApp | undefined;
  const view = render(<AppWorkspace mode='import' conversation={<p>Ordinary conversation</p>} onAppChanged={(value) => { accepted = value; }} onRefresh={() => {}} onDeleted={() => {}} />, { wrapper });
  fireEvent.input(view.getByRole('textbox', { name: locale.name }), { target: { value: 'Imported app' } });
  fireEvent.input(view.getByRole('textbox', { name: locale.import.hostPath }), { target: { value: 'C:/work/app' } });
  fireEvent.click(view.getByRole('radio', { name: locale.import.artifact }));
  fireEvent.click(view.getByRole('button', { name: locale.import.action }));
  await waitFor(() => expect(accepted?.id).toBe('imported'));
  expect(request.mock.calls[0][0]).toEqual({ source: 'C:/work/app', name: 'Imported app', import_kind: 'artifact' });
  expect(session).not.toHaveBeenCalled();
});

test('application grid pages nine records and filtering resets the page', async () => {
  spyOn(appHosting.list, 'invoke').mockResolvedValue(Array.from({ length: 12 }, (_, index) => ({
    ...app(`app-${index}`, `Application ${index + 1}`), tags: index < 2 ? ['Focus'] : [],
  })));
  const view = render(<AppsPage />, { wrapper });
  await view.findByRole('heading', { name: 'Application 1' });
  expect(view.container.querySelectorAll('article')).toHaveLength(9);
  fireEvent.click(view.getByText('2', { selector: '.arco-pagination-item' }));
  expect(view.container.querySelectorAll('article')).toHaveLength(3);
  expect(view.getByRole('heading', { name: 'Application 10' })).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: 'Focus' }));
  expect(view.container.querySelectorAll('article')).toHaveLength(2);
  expect(view.getByRole('heading', { name: 'Application 1' })).toBeTruthy();
});

test('importing into an existing app preserves its identity and uses detection owner', async () => {
  const existing = app('same-app', 'My app');
  const request = spyOn(appHosting.importInto, 'invoke').mockResolvedValue(existing);
  const oldImport = spyOn(appHosting.import, 'invoke');
  const view = render(<AppWorkspace app={existing} mode='import' conversation={<p>Ordinary conversation</p>} onAppChanged={() => {}} onRefresh={() => {}} onDeleted={() => {}} />, { wrapper });
  fireEvent.input(view.getByRole('textbox', { name: locale.import.hostPath }), { target: { value: 'C:/existing/code' } });
  fireEvent.click(view.getByRole('button', { name: locale.import.action }));
  await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
  expect(request.mock.calls[0][0]).toEqual({ id: 'same-app', source: 'C:/existing/code', name: 'My app', import_kind: 'source' });
  expect(oldImport).not.toHaveBeenCalled();
});

test('active development disables manual workspace replacement without locking metadata edits', async () => {
  const existing = app('busy-development', 'Busy app');
  const request = spyOn(appHosting.importInto, 'invoke');
  const view = await act(async () => render(<AppWorkspace app={existing} mode='import' developmentBusy conversation={<p>Ordinary running conversation</p>} onAppChanged={() => {}} onRefresh={() => {}} onDeleted={() => {}} />, { wrapper }));
  const source = view.getByRole('textbox', { name: locale.import.hostPath }) as HTMLInputElement;
  fireEvent.input(source, { target: { value: 'C:/ready-to-import' } });
  expect(source.disabled).toBe(false);
  const importButton = view.getByRole('button', { name: locale.import.action }) as HTMLButtonElement;
  expect(importButton.disabled).toBe(true);
  fireEvent.click(importButton);
  expect(request).not.toHaveBeenCalled();
  expect((view.getByRole('button', { name: locale.overwrite.action }) as HTMLButtonElement).disabled).toBe(true);
  expect((view.getByRole('button', { name: locale.actions.delete }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.input(view.getByRole('textbox', { name: locale.description }) as HTMLElement, { target: { value: 'Editable metadata' } });
  expect((view.getByRole('button', { name: common.save }) as HTMLButtonElement).disabled).toBe(false);
});

test('late file-import completion cannot release another mounted app operation or navigate after unmount', async () => {
  let finishOld!: (app: HostedApp) => void;
  let finishNew!: (app: HostedApp) => void;
  spyOn(appHosting.importInto, 'invoke').mockImplementation(({ id }) => new Promise<HostedApp>((resolve) => {
    if (id === 'old-app') finishOld = resolve; else finishNew = resolve;
  }));
  const oldStates: boolean[] = [], newStates: boolean[] = [];
  const oldAccepted = mock(() => {}), newAccepted = mock(() => {});
  const previous = render(<AppImportForm app={app('old-app', 'Old')} onFileOperationChange={(value) => oldStates.push(value)} onImported={oldAccepted} />, { wrapper });
  fireEvent.input(previous.getByRole('textbox', { name: locale.import.hostPath }), { target: { value: 'C:/old/source' } });
  fireEvent.click(previous.getByRole('button', { name: locale.import.action }));
  expect(oldStates).toEqual([true]);
  previous.unmount();
  expect(oldStates).toEqual([true, false]);
  const current = render(<AppImportForm app={app('new-app', 'New')} onFileOperationChange={(value) => newStates.push(value)} onImported={newAccepted} />, { wrapper });
  fireEvent.input(current.getByRole('textbox', { name: locale.import.hostPath }), { target: { value: 'C:/new/source' } });
  fireEvent.click(current.getByRole('button', { name: locale.import.action }));
  expect(newStates).toEqual([true]);
  await act(async () => finishOld(app('old-app', 'Old')));
  expect(oldAccepted).not.toHaveBeenCalled(); expect(oldStates).toEqual([true, false]); expect(newStates).toEqual([true]);
  await act(async () => finishNew(app('new-app', 'New')));
  expect(newAccepted).toHaveBeenCalledTimes(1); expect(newStates).toEqual([true, false]);
});
