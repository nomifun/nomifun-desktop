import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useParams } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { agentPlatform, pluginRuntimes } from '@/common/adapter/ipcBridge';
import { asAgentPresetId, asDigestHex, createEmptyAgentPresetDocument,
  type AgentPresetSummary, type CreateAgentSessionResponse } from '@/common/types/agentPlatform';
import { parsePluginRuntimeId } from '@/common/types/ids';
import type { AgentPresetUiBinding, AgentUiContribution, PluginRuntimeSurfaceLaunchDescriptor } from '@/common/types/pluginRuntimePlatform';
import { agentUiChoiceKey } from '@/common/utils/agentUiChoice';
import * as platform from '@/renderer/utils/platform';
import { emitter } from '@/renderer/utils/emitter';
import { AgentSessionViewHost } from '../agentSession/AgentSessionPage';
import AgentPageSettings from './AgentPageSettings';
import { AgentUiAvailabilityContext } from '@/renderer/hooks/agent/useAgentUiAvailable';
import AgentPresetEditor from './AgentPresetEditor';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import pluginEn from '../../services/i18n/locales/en-US/pluginRuntime.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: {
  agentSettings: en, pluginRuntime: pluginEn,
} } }, interpolation: { escapeValue: false } });
const preset: AgentPresetSummary = {
  preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-0000000000a1'), display_name: 'My Agent',
  source: 'user', bound_target_count: 0, current_stable_revision: {
    preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-0000000000a1'), revision: 1, revision_digest: asDigestHex('c'.repeat(64)),
  },
};
const choice: AgentUiContribution = {
  plugin_id: parsePluginRuntimeId('0190f5fe-7c00-7a00-8000-0000000000b1'),
  capability: { id: 'my-agent-page', version: '1.0.0' }, expected_release_digest: 'a'.repeat(64),
  display_name: 'Custom page', description: '',
};
const sessionId = '0190f5fe-7c00-7a00-8000-0000000000c1';
const initial: AgentPresetUiBinding = { preset_id: preset.preset_id, display_name: preset.display_name,
  binding: { binding_version: 0, selection: null } };
const surface: PluginRuntimeSurfaceLaunchDescriptor = {
  plugin_id: choice.plugin_id, product_revision: 1, release_id: 'release', expected_release_digest: choice.expected_release_digest,
  active_release_epoch: 1, surface_session_id: 'surface', surface_generation: 1, surface_capability: 'bearer',
  ui_entrypoint: 'ui/index.html', kind: 'plugin',
};

afterEach(() => { cleanup(); mock.restore(); });

async function mount(options: {
  value?: AgentPresetUiBinding; choices?: AgentUiContribution[]; dirty?: boolean; stable?: boolean;
  load?: () => Promise<AgentPresetUiBinding>; editor?: boolean;
} = {}) {
  let stored = structuredClone(options.value ?? initial);
  const load = spyOn(agentPlatform.presetUiBinding, 'invoke').mockImplementation(options.load ?? (async () => stored));
  const catalog = spyOn(agentPlatform.agentUiContributions, 'invoke').mockResolvedValue(options.choices ?? [choice]);
  const save = spyOn(agentPlatform.putPresetUiBinding, 'invoke').mockImplementation(async ({ request }) => {
    stored = { ...stored, binding: { binding_version: request.expected_binding_version + 1, selection: request.selection } };
    return stored;
  });
  const sessionPreference = spyOn(agentPlatform.sessions.uiBinding, 'invoke').mockImplementation(async () => stored);
  const create = spyOn(agentPlatform.sessions.create, 'invoke').mockResolvedValue({ agent_session_id: sessionId } as CreateAgentSessionResponse);
  const turn = spyOn(agentPlatform.sessions.createTurn, 'invoke');
  const launch = spyOn(pluginRuntimes.openSurface, 'invoke').mockResolvedValue(surface);
  spyOn(pluginRuntimes.closeSurface, 'invoke').mockResolvedValue(true);
  const bridge = spyOn(pluginRuntimes.bridge, 'invoke');
  const history = spyOn(emitter, 'emit');
  spyOn(platform, 'resolveBackendAssetUrl').mockReturnValue('about:blank');
  const draftChanged = mock(() => {}), saveAgent = mock(() => {}), guid = mock(() => {});
  const Session = () => { const { id = '' } = useParams(); return <AgentSessionViewHost sessionId={id} key={id}>
    <div>Built-in session</div>
  </AgentSessionViewHost>; };
  const cache = new Map();
  const page = (current = preset, dirty = options.dirty ?? false) => {
    const actual = options.stable === false ? { ...current, current_stable_revision: undefined } : current;
    const draft = { preset_id: actual.preset_id, display_name: actual.display_name, document: createEmptyAgentPresetDocument() };
    return <AgentUiAvailabilityContext.Provider value={true}><I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => cache, dedupingInterval: 0, shouldRetryOnError: false }}>
      <MemoryRouter initialEntries={['/agent']}><Routes><Route path='/agent' element={options.editor
        ? <AgentPresetEditor key={actual.preset_id} editor={{ preset: actual, draft }} draft={draft}
          catalog={{ capabilities: [], skills: [], mcp_tools: [], roles: [] }} busyAction={null} dirty={dirty}
          onDraftChange={draftChanged} onSave={saveAgent} onStartConversation={guid} />
        : <AgentPageSettings key={actual.preset_id} preset={actual} busy={false} dirty={dirty} />} />
        <Route path='/agent-sessions/:id' element={<Session />} />
      </Routes></MemoryRouter>
    </SWRConfig></I18nextProvider></AgentUiAvailabilityContext.Provider>;
  };
  let result!: ReturnType<typeof render>;
  await act(async () => { result = render(page()); });
  const view = within(result.container);
  if (options.editor) await act(async () => { fireEvent.click(view.getByRole('tab', { name: en.page.title })); });
  if (!options.load) await view.findByRole('combobox', { name: en.page.default });
  const select = (value = choice) => fireEvent.change(view.getByRole('combobox', { name: en.page.default }), {
    target: { value: agentUiChoiceKey(value) },
  });
  return { ...result, view, page, select, stored: () => stored, load, catalog, save, create, turn,
    launch, bridge, history, sessionPreference, draftChanged, saveAgent, guid };
}

test('real editor saves page consent before any Session, then opens its page without sending a message', async () => {
  const v = await mount({ editor: true });
  await v.view.findByRole('option', { name: /Custom page/ });
  v.select();
  expect(v.view.getByText(en.page.savePageFirst)).toBeTruthy();
  fireEvent.click(v.view.getByRole('button', { name: en.page.open }));
  expect(v.create).not.toHaveBeenCalled();
  fireEvent.click(v.view.getByRole('button', { name: en.page.save }));
  await waitFor(() => expect(v.stored().binding.selection).toEqual(choice));
  expect(v.save).toHaveBeenCalledWith({ preset_id: preset.preset_id, request: { expected_binding_version: 0, selection: choice } });
  expect(v.create).not.toHaveBeenCalled();
  expect(v.draftChanged).not.toHaveBeenCalled();
  expect(v.saveAgent).not.toHaveBeenCalled();
  expect(v.launch).not.toHaveBeenCalled();
  fireEvent.click(v.view.getByRole('button', { name: en.page.open }));
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  expect(v.create).toHaveBeenCalledWith({ preset_id: preset.preset_id, title: preset.display_name });
  expect(v.sessionPreference).toHaveBeenCalledWith({ agent_session_id: sessionId });
  expect(v.launch).toHaveBeenCalledWith({ plugin_id: choice.plugin_id, agent_session: {
    agent_session_id: sessionId, expected_release_digest: choice.expected_release_digest, ui_capability: choice.capability,
  } });
  expect(v.turn).not.toHaveBeenCalled();
  expect(v.bridge).not.toHaveBeenCalled();
  expect(v.guid).not.toHaveBeenCalled();
  expect(v.history).toHaveBeenCalledWith('chat.history.refresh');
});

test('dirty Agent configuration does not block independent page saving, but blocks launching an old revision', async () => {
  const v = await mount({ dirty: true });
  await v.view.findByRole('option', { name: /Custom page/ });
  v.select(); fireEvent.click(v.view.getByRole('button', { name: en.page.save }));
  await waitFor(() => expect(v.save).toHaveBeenCalledTimes(1));
  expect(v.view.getByText(en.page.saveAgentFirst)).toBeTruthy();
  fireEvent.click(v.view.getByRole('button', { name: en.page.open }));
  expect(v.create).not.toHaveBeenCalled();
  v.rerender(v.page(preset, false));
  fireEvent.click(v.view.getByRole('button', { name: en.page.open }));
  await waitFor(() => expect(v.create).toHaveBeenCalledTimes(1));
});

test('a preset without a stable revision can configure presentation but cannot launch', async () => {
  const v = await mount({ stable: false });
  expect(v.view.getByText(en.page.saveAgentFirst)).toBeTruthy();
  fireEvent.click(v.view.getByRole('button', { name: en.page.open }));
  expect(v.create).not.toHaveBeenCalled();
});

test('withdrawn saved release stays visible; selecting builtin explicitly clears the independent binding', async () => {
  const v = await mount({ value: { ...initial, binding: { binding_version: 7, selection: choice } }, choices: [] });
  await v.view.findByText(en.view.defaultUnavailable);
  expect(v.save).not.toHaveBeenCalled();
  fireEvent.change(v.view.getByRole('combobox'), { target: { value: '' } });
  fireEvent.click(v.view.getByRole('button', { name: en.page.save }));
  await waitFor(() => expect(v.save).toHaveBeenCalledWith({ preset_id: preset.preset_id,
    request: { expected_binding_version: 7, selection: null } }));
  fireEvent.click(v.view.getByRole('button', { name: en.page.open }));
  await v.view.findByText('Built-in session');
  expect(v.launch).not.toHaveBeenCalled();
  expect(v.turn).not.toHaveBeenCalled();
});

test('save conflict retains the draft choice and requires explicit refresh before resubmitting', async () => {
  const v = await mount();
  await v.view.findByRole('option', { name: /Custom page/ });
  v.save.mockRejectedValueOnce(new Error('version conflict'));
  v.select(); fireEvent.click(v.view.getByRole('button', { name: en.page.save }));
  await v.view.findByText(en.view.defaultSaveFailed);
  fireEvent.click(v.view.getByRole('button', { name: en.page.save }));
  expect(v.save).toHaveBeenCalledTimes(1);
  v.load.mockResolvedValue({ ...initial, binding: { binding_version: 9, selection: null } });
  await act(async () => { fireEvent.click(v.view.getByRole('button', { name: en.actions.retry })); });
  await waitFor(() => expect(v.view.queryByText(en.view.defaultSaveFailed)).toBeNull());
  expect((v.view.getByRole('combobox') as HTMLSelectElement).value).toBe(agentUiChoiceKey(choice));
  fireEvent.click(v.view.getByRole('button', { name: en.page.save }));
  await waitFor(() => expect(v.save).toHaveBeenCalledTimes(2));
  expect(v.save.mock.calls[1][0].request.expected_binding_version).toBe(9);
});

test('late creation after switching presets does not navigate; duplicate clicks create only once', async () => {
  const v = await mount();
  let resolve!: (value: CreateAgentSessionResponse) => void;
  v.create.mockImplementation(() => new Promise(r => { resolve = r; }));
  const button = v.view.getByRole('button', { name: en.page.open });
  fireEvent.click(button); fireEvent.click(button);
  expect(v.create).toHaveBeenCalledTimes(1);
  const other = { ...preset, preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-0000000000a2'), display_name: 'Other Agent' };
  v.load.mockResolvedValue({ ...initial, preset_id: other.preset_id, display_name: other.display_name });
  v.rerender(v.page(other));
  await act(async () => { resolve({ agent_session_id: sessionId } as CreateAgentSessionResponse); });
  expect(v.view.getByRole('button', { name: en.page.open })).toBeTruthy();
  expect(v.sessionPreference).not.toHaveBeenCalled();
  expect(v.launch).not.toHaveBeenCalled();
  expect(v.turn).not.toHaveBeenCalled();
});

test('a failed create is disclosed and never retried by refresh or rendering', async () => {
  const v = await mount();
  v.create.mockRejectedValue(new Error('resource selection required'));
  fireEvent.click(v.view.getByRole('button', { name: en.page.open }));
  await v.view.findByText(en.page.openFailed);
  await act(async () => { fireEvent.click(v.view.getByRole('button', { name: en.actions.retry })); });
  await waitFor(() => expect(v.load).toHaveBeenCalledTimes(2));
  expect(v.create).toHaveBeenCalledTimes(1);
  expect(v.launch).not.toHaveBeenCalled();
  expect(v.turn).not.toHaveBeenCalled();
});

test('late save after switching presets cannot alter the new preset choice or create a Session', async () => {
  const v = await mount();
  await v.view.findByRole('option', { name: /Custom page/ });
  let resolve!: (value: AgentPresetUiBinding) => void;
  v.save.mockImplementation(() => new Promise(r => { resolve = r; }));
  v.select();
  const button = v.view.getByRole('button', { name: en.page.save });
  fireEvent.click(button); fireEvent.click(button);
  expect(v.save).toHaveBeenCalledTimes(1);
  const other = { ...preset, preset_id: asAgentPresetId('0190f5fe-7c00-7a00-8000-0000000000a2'), display_name: 'Other Agent' };
  v.load.mockResolvedValue({ ...initial, preset_id: other.preset_id, display_name: other.display_name });
  v.rerender(v.page(other));
  await act(async () => { resolve({ ...initial, binding: { binding_version: 1, selection: choice } }); });
  expect((v.view.getByRole('combobox') as HTMLSelectElement).value).toBe('');
  expect(v.create).not.toHaveBeenCalled();
  expect(v.launch).not.toHaveBeenCalled();
});

test('page preference load failure cannot overwrite an unknown default', async () => {
  const v = await mount({ load: async () => { throw new Error('unavailable'); } });
  await v.view.findByText(en.view.defaultFailed);
  fireEvent.click(v.view.getByRole('button', { name: en.page.save }));
  fireEvent.click(v.view.getByRole('button', { name: en.page.open }));
  expect(v.create).not.toHaveBeenCalled();
  expect(v.save).not.toHaveBeenCalled();
});
