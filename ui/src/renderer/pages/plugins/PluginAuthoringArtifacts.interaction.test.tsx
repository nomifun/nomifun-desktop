import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type { PluginDetail, PluginDraftDetail } from '@/common/types/pluginPlatform';
import * as surface from './PluginSurfacePanel';
import PluginAuthoringArtifacts from './PluginAuthoringArtifacts';
import en from '../../services/i18n/locales/en-US/pluginPlatform.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { pluginPlatform: en } } } });
const id = '0190f5fe-7c00-7a00-8000-000000000202';
let draft: PluginDraftDetail;
const installed = { summary: { plugin_id: 'focus-timer', revision: 12, has_ui: true },
  config: { values: { theme: 'cobalt' } }, credential_bindings: [{ slot: 'calendar', credential_id: 'credential-reference', status: 'bound' }] } as unknown as PluginDetail;
const label = (key: string) => i18n.t(`pluginPlatform.authoring.${key}`);

beforeEach(() => {
  (window as typeof window & { __backendPort?: number }).__backendPort = 11451;
  draft = { summary: { draft_id: 'timer-draft', source_conversation_id: id, revision: 2, plugin_id: 'focus-timer',
    delivered_artifact_digest: 'original-digest', display_name: 'Focus timer', description: 'Stay focused', status: 'ready', updated_at_ms: 1 },
    files: [
      { path: 'ui/index.html', text: '<h1>Focus</h1>', media_type: 'text/html', digest: 'one', size_bytes: 14 },
      { path: 'ui/styles.css', text: 'body { color: blue; }', media_type: 'text/css', digest: 'two', size_bytes: 20 },
    ] };
  spyOn(pluginPlatform.drafts.list, 'invoke').mockImplementation(async () => ({ drafts: [draft.summary] }));
  spyOn(pluginPlatform.authoring.details, 'invoke').mockImplementation(async () => ({ draft, verification: { delivery: { old: 'do not expose verification details' } }, commands: [] }));
  spyOn(pluginPlatform.authoring.changed, 'on').mockImplementation(() => () => {});
  spyOn(pluginPlatform.plugins.get, 'invoke').mockResolvedValue(installed);
  spyOn(pluginPlatform.surface.close, 'invoke').mockResolvedValue(true);
  spyOn(pluginPlatform.drafts.replaceFile, 'invoke').mockImplementation(async ({ request }) => {
    const text = new TextDecoder().decode(Uint8Array.from(atob(request.content_base64), character => character.charCodeAt(0)));
    draft = { ...draft, summary: { ...draft.summary, revision: draft.summary.revision + 1, delivered_artifact_digest: undefined },
      files: draft.files.map(file => file.path === request.path ? { ...file, text } : file) };
    return draft;
  });
  spyOn(pluginPlatform.drafts.save, 'invoke').mockImplementation(async () => {
    draft = { ...draft, summary: { ...draft.summary, delivered_artifact_digest: 'saved-digest' } };
    return { draft: draft.summary, result: { outcome: 'installed', plugin: installed } };
  });
  spyOn(surface, 'default').mockImplementation(({ descriptor }) => <div>Preview {descriptor.surface_session_id}</div>);
});
afterEach(() => { cleanup(); mock.restore(); delete (window as typeof window & { __backendPort?: number }).__backendPort; });

function mount(operationDisabled = false) {
  const Location = () => <output data-testid='location'>{useLocation().pathname}</output>;
  return render(<I18nextProvider i18n={i18n}><MemoryRouter><Location /><PluginAuthoringArtifacts
    conversationId={id} operationDisabled={operationDisabled} onAuthoringOperation={async operation => { await operation(); }}
  /></MemoryRouter></I18nextProvider>);
}
async function source(view: ReturnType<typeof mount>) {
  await view.findByRole('heading', { name: 'Focus timer' });
  fireEvent.click(view.getByRole('tab', { name: label('files') }));
  return view.getByRole('textbox', { name: 'ui/index.html' });
}

test('source edits save in sequence, support UTF-8, and preserve existing configuration and credential references', async () => {
  const replace = spyOn(pluginPlatform.drafts.replaceFile, 'invoke');
  const save = spyOn(pluginPlatform.drafts.save, 'invoke');
  const view = mount();
  fireEvent.change(await source(view), { target: { value: '<h1>专注 ✨</h1>' } });
  fireEvent.change(view.getByRole('combobox', { name: label('files') }), { target: { value: 'ui/styles.css' } });
  fireEvent.change(view.getByRole('textbox', { name: 'ui/styles.css' }), { target: { value: 'body { color: purple; }' } });
  await act(async () => { fireEvent.click(view.getByRole('button', { name: label('saveAndUse') })); });
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(replace.mock.calls.map(call => [call[0].request.path, call[0].request.expected_revision])).toEqual([['ui/index.html', 2], ['ui/styles.css', 3]]);
  expect(draft.files[0].text).toBe('<h1>专注 ✨</h1>');
  expect(save.mock.calls[0][0].request).toEqual({ expected_revision: 4, expected_plugin_revision: 12,
    config: { theme: 'cobalt' }, credential_bindings: { calendar: 'credential-reference' } });
  expect(view.queryByText('do not expose verification details')).toBeNull();
  await view.findByText(label('sourceSaved'));
});

test('a failed install can be retried after source files have already been saved', async () => {
  const replace = spyOn(pluginPlatform.drafts.replaceFile, 'invoke');
  const save = spyOn(pluginPlatform.drafts.save, 'invoke').mockRejectedValueOnce(new Error('Save interrupted'));
  const view = mount();
  fireEvent.change(await source(view), { target: { value: '<h1>Updated timer</h1>' } });
  await act(async () => { fireEvent.click(view.getByRole('button', { name: label('saveAndUse') })); });
  await view.findByText('Save interrupted');
  const retry = view.getByRole('button', { name: label('saveAndUse') }) as HTMLButtonElement;
  expect(retry.disabled).toBe(false);
  await act(async () => { fireEvent.click(retry); });
  await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(replace).toHaveBeenCalledTimes(1);
  expect(save.mock.calls.map(call => call[0].request.expected_revision)).toEqual([3, 3]);
  await view.findByText(label('sourceSaved'));
});

test('source remains readable while the Agent is writing files', async () => {
  const view = mount(true);
  const editor = await source(view) as HTMLTextAreaElement;
  expect(editor.readOnly).toBe(true);
  expect((view.getByRole('button', { name: label('saveAndUse') }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(view.getByRole('button', { name: en.actions.open }));
  expect(view.getByTestId('location').textContent).toBe('/plugins/run/focus-timer');
});

test('a generated app opens its actual installed preview and closes the owned surface on leaving', async () => {
  const open = spyOn(pluginPlatform.plugins.openSurface, 'invoke').mockResolvedValue({ plugin_id: 'focus-timer', artifact_digest: 'saved-digest',
    surface_session_id: 'timer-preview', surface_generation: 4, entrypoint: 'ui/index.html', is_preview: false });
  const close = spyOn(pluginPlatform.surface.close, 'invoke');
  const view = mount();
  await view.findByRole('heading', { name: 'Focus timer' });
  await act(async () => { fireEvent.click(await view.findByRole('button', { name: label('preview') })); });
  await view.findByText('Preview timer-preview');
  expect(open.mock.calls[0][0]).toEqual({ plugin_id: 'focus-timer', request: { expected_revision: 12 } });
  view.unmount();
  expect(close.mock.calls.at(-1)?.[0]).toEqual({ plugin_id: 'focus-timer', is_preview: false,
    request: { surface_session_id: 'timer-preview', surface_generation: 4 } });
});

test('an empty task keeps a clear app placeholder', async () => {
  spyOn(pluginPlatform.drafts.list, 'invoke').mockResolvedValue({ drafts: [] });
  const view = mount();
  await view.findByRole('heading', { name: label('artifactEmptyTitle') });
  expect(view.queryByRole('button', { name: label('saveAndUse') })).toBeNull();
});

test('a lost save response reconciles the retained source and installed app instead of installing twice', async () => {
  const replace = spyOn(pluginPlatform.drafts.replaceFile, 'invoke');
  const save = spyOn(pluginPlatform.drafts.save, 'invoke').mockImplementation(async () => {
    draft = { ...draft, summary: { ...draft.summary, delivered_artifact_digest: 'saved-before-response-lost' } };
    throw new Error('Save response lost');
  });
  const view = mount();
  fireEvent.change(await source(view), { target: { value: '<h1>Saved once</h1>' } });
  await act(async () => { fireEvent.click(view.getByRole('button', { name: label('saveAndUse') })); });
  await view.findByText(label('sourceSaved'));
  expect(view.queryByText('Save response lost')).toBeNull();
  expect(replace).toHaveBeenCalledTimes(1);
  expect(save).toHaveBeenCalledTimes(1);
  expect((view.getByRole('button', { name: label('saveAndUse') }) as HTMLButtonElement).disabled).toBe(true);
});

test('an installed preview that resolves after leaving the workspace is released', async () => {
  let finish!: (descriptor: Awaited<ReturnType<typeof pluginPlatform.plugins.openSurface.invoke>>) => void;
  const open = spyOn(pluginPlatform.plugins.openSurface, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const close = spyOn(pluginPlatform.surface.close, 'invoke');
  const view = mount();
  await view.findByRole('heading', { name: 'Focus timer' });
  fireEvent.click(await view.findByRole('button', { name: label('preview') }));
  await waitFor(() => expect(open).toHaveBeenCalledTimes(1));
  view.unmount();
  await act(async () => { finish({ plugin_id: 'focus-timer', artifact_digest: 'saved-digest', surface_session_id: 'late-preview',
    surface_generation: 9, entrypoint: 'ui/index.html', is_preview: false }); });
  expect(close.mock.calls.at(-1)?.[0]).toEqual({ plugin_id: 'focus-timer', draft_id: undefined, is_preview: false,
    request: { surface_session_id: 'late-preview', surface_generation: 9 } });
});

test('switching apps preserves unsaved source edits and releases the previous preview', async () => {
  const first = { ...draft, summary: { ...draft.summary, draft_id: 'first-draft', display_name: 'First app', delivered_artifact_digest: undefined } };
  const second = { ...draft, summary: { ...draft.summary, draft_id: 'second-draft', display_name: 'Second app' } };
  const descriptor = { draft_id: 'first-draft', artifact_digest: 'first-bytes', surface_session_id: 'first-preview',
    surface_generation: 1, entrypoint: 'ui/index.html', is_preview: true };
  spyOn(pluginPlatform.drafts.list, 'invoke').mockResolvedValue({ drafts: [first.summary, second.summary] });
  spyOn(pluginPlatform.authoring.details, 'invoke').mockImplementation(async ({ draft_id }) => ({
    draft: draft_id === 'first-draft' ? first : second, verification: draft_id === 'first-draft' ? { surface: descriptor } : {}, commands: [],
  }));
  const close = spyOn(pluginPlatform.surface.close, 'invoke');
  const view = mount();
  await view.findByRole('tab', { name: 'First app' });
  fireEvent.click(view.getByRole('tab', { name: 'First app' }));
  await view.findByText('Preview first-preview');
  fireEvent.click(view.getByRole('tab', { name: label('files') }));
  fireEvent.change(view.getByRole('textbox', { name: 'ui/index.html' }), { target: { value: '<h1>My unsaved idea</h1>' } });
  fireEvent.click(view.getByRole('tab', { name: 'Second app' }));
  await waitFor(() => expect(close).toHaveBeenCalledTimes(1));
  expect(close.mock.calls[0][0]).toEqual({ plugin_id: undefined, draft_id: 'first-draft', is_preview: true,
    request: { surface_session_id: 'first-preview', surface_generation: 1 } });
  fireEvent.click(view.getByRole('tab', { name: 'First app' }));
  expect((view.getByRole('textbox', { name: 'ui/index.html' }) as HTMLTextAreaElement).value).toBe('<h1>My unsaved idea</h1>');
  fireEvent.click(view.getByRole('tab', { name: label('preview') }));
  expect(view.queryByText('Preview first-preview')).toBeNull();
});

test('an editing draft previews with the installed app configuration', async () => {
  draft = { ...draft, summary: { ...draft.summary, delivered_artifact_digest: undefined } };
  const preview = spyOn(pluginPlatform.drafts.preview, 'invoke').mockResolvedValue({ draft_revision: 2,
    descriptor: { draft_id: draft.summary.draft_id, artifact_digest: 'editing-bytes', surface_session_id: 'editing-preview',
      surface_generation: 1, entrypoint: 'ui/index.html', is_preview: true } });
  const view = mount();
  await view.findByRole('heading', { name: 'Focus timer' });
  await act(async () => { fireEvent.click(await view.findByRole('button', { name: label('preview') })); });
  await view.findByText('Preview editing-preview');
  expect(preview.mock.calls[0][0]).toEqual({ draft_id: 'timer-draft', request: { expected_revision: 2, config: { theme: 'cobalt' } } });
});
