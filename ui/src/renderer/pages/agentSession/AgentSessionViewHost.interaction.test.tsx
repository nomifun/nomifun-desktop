import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import { useEffect } from 'react';
import { agentPlatform, pluginRuntimes } from '@/common/adapter/ipcBridge';
import { pluginRuntimeProduct, type PluginRuntimeDraft } from '@/common/adapter/pluginRuntimeProductBridge';
import * as platform from '@/renderer/utils/platform';
import { parsePluginRuntimeId } from '@/common/types/ids';
import type { AgentPresetUiBinding, AgentUiContribution, PluginRuntimeSurfaceLaunchDescriptor } from '@/common/types/pluginRuntimePlatform';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import pluginEn from '../../services/i18n/locales/en-US/pluginRuntime.json';
import { AgentSessionViewHost } from './AgentSessionPage';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', fallbackLng: 'en-US', resources: {
  'en-US': { translation: { agentSettings: en, pluginRuntime: pluginEn } },
}, interpolation: { escapeValue: false } });

const choice: AgentUiContribution = {
  capability: { id: 'plugin.test.ui.agent-session', version: '1.0.0' },
  plugin_id: parsePluginRuntimeId('0190f5fe-7c00-7a00-8000-0000000000b1'),
  expected_release_digest: 'a'.repeat(64), display_name: 'My Agent', description: 'Custom view',
};
const surface: PluginRuntimeSurfaceLaunchDescriptor = {
  plugin_id: choice.plugin_id, product_revision: 4, release_id: 'release',
  expected_release_digest: choice.expected_release_digest, active_release_epoch: 1,
  surface_session_id: 'surface', surface_generation: 1, surface_capability: 'bearer',
  ui_entrypoint: 'ui/index.html', kind: 'plugin',
};

afterEach(() => { cleanup(); mock.restore(); });

const builtinPreference: AgentPresetUiBinding = {
  preset_id: 'preset-a', display_name: 'My preset', binding: { binding_version: 0, selection: null },
};

async function open(options: {
  preference?: () => Promise<AgentPresetUiBinding>;
  choices?: AgentUiContribution[];
} = {}) {
  const catalog = spyOn(agentPlatform.agentUiContributions, 'invoke').mockResolvedValue(options.choices ?? [choice]);
  let stored = structuredClone(builtinPreference);
  const preference = spyOn(agentPlatform.sessions.uiBinding, 'invoke').mockImplementation(options.preference ?? (async () => stored));
  const save = spyOn(agentPlatform.putPresetUiBinding, 'invoke').mockImplementation(async ({ request }) => {
    stored = { ...stored, binding: { binding_version: request.expected_binding_version + 1, selection: request.selection } };
    return stored;
  });
  const launch = spyOn(pluginRuntimes.openSurface, 'invoke').mockResolvedValue(surface);
  const close = spyOn(pluginRuntimes.closeSurface, 'invoke').mockResolvedValue(true);
  const bridge = spyOn(pluginRuntimes.bridge, 'invoke');
  spyOn(platform, 'resolveBackendAssetUrl').mockReturnValue('about:blank');
  const cache = new Map();
  const draftCreated = mock((_draftId: string) => {});
  let builtinMounts = 0;
  function BuiltinProbe() {
    useEffect(() => { builtinMounts++; }, []);
    return <div>Built-in session content</div>;
  }
  const page = (sessionId: string) => <SWRConfig value={{ provider: () => cache, dedupingInterval: 2000, shouldRetryOnError: false }}>
    <I18nextProvider i18n={i18n}><AgentSessionViewHost key={sessionId} sessionId={sessionId} onDraftCreated={draftCreated}>
      <BuiltinProbe />
    </AgentSessionViewHost></I18nextProvider>
  </SWRConfig>;
  let result!: ReturnType<typeof render>;
  await act(async () => { result = render(page('session-a')); });
  const view = within(result.container);
  if (options.choices?.length !== 0) await view.findByRole('option', { name: /My Agent/ });
  if (!options.preference) await view.findByText('Built-in session content');
  const select = () => fireEvent.change(view.getByRole('combobox'), {
    target: { value: (view.getByRole('option', { name: /My Agent/ }) as HTMLOptionElement).value },
  });
  const use = () => fireEvent.click(view.getByRole('button', { name: en.view.use }));
  return { ...result, view, catalog, preference, save, launch, close, bridge, select, use, page, draftCreated, builtinMounts: () => builtinMounts };
}

test('ordinary users can create, select, save and return to builtin without a host opt-in', async () => {
  const v = await open();
  const create = spyOn(pluginRuntimeProduct.agentSessionTemplate, 'invoke').mockResolvedValue({ id: 'draft-one' } as PluginRuntimeDraft);
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  expect((v.view.getByRole('combobox') as HTMLSelectElement).value).toBe('');
  expect(v.catalog).toHaveBeenCalledTimes(1);
  expect(v.preference).toHaveBeenCalledWith({ agent_session_id: 'session-a' });
  expect(v.launch).not.toHaveBeenCalled();
  expect(v.save).not.toHaveBeenCalled();
  expect(v.builtinMounts()).toBe(1);
  fireEvent.click(v.view.getByRole('button', { name: en.view.createTemplate }));
  await waitFor(() => expect(v.draftCreated).toHaveBeenCalledWith('draft-one'));
  expect(create).toHaveBeenCalledTimes(1);
  expect(v.launch).not.toHaveBeenCalled();
  v.select(); v.use();
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  fireEvent.click(v.view.getByRole('button', { name: en.view.remember }));
  await waitFor(() => expect(v.view.getByRole('button', { name: en.view.clearDefault }).hasAttribute('disabled')).toBe(false));
  expect(v.save).toHaveBeenCalledWith({ preset_id: 'preset-a', request: {
    expected_binding_version: 0, selection: choice,
  } });
  fireEvent.click(v.view.getByRole('button', { name: en.view.builtin }));
  await v.view.findByText('Built-in session content');
  await waitFor(() => expect(v.close).toHaveBeenCalledTimes(1));
  expect(v.launch).toHaveBeenCalledTimes(1);
  expect(v.bridge).not.toHaveBeenCalled();
});

test('empty catalog offers the first template without a published plugin', async () => {
  const v = await open({ choices: [] });
  expect(v.view.getByRole('button', { name: en.view.createTemplate })).toBeTruthy();
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  expect(v.launch).not.toHaveBeenCalled();
});

test('unavailable saved release opens builtin and allows explicit clearing', async () => {
  const v = await open({ choices: [], preference: async () => ({ ...builtinPreference,
    binding: { binding_version: 4, selection: choice } }) });
  await v.view.findByText(en.view.defaultUnavailable);
  await v.view.findByText('Built-in session content');
  expect(v.view.getByRole('button', { name: en.view.createTemplate })).toBeTruthy();
  expect(v.launch).not.toHaveBeenCalled();
  expect(v.save).not.toHaveBeenCalled();
  await act(async () => { fireEvent.click(v.view.getByRole('button', { name: en.view.clearDefault })); });
  await waitFor(() => expect(v.save).toHaveBeenCalledWith({ preset_id: 'preset-a', request: {
    expected_binding_version: 4, selection: null,
  } }));
  expect(v.bridge).not.toHaveBeenCalled();
});

test('remembered exact release opens a fresh grant for each Session; it is not a turn', async () => {
  const saved = { ...builtinPreference, binding: { binding_version: 2, selection: choice } };
  const v = await open({ preference: async () => saved });
  await waitFor(() => expect(v.launch).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  v.rerender(v.page('session-b'));
  await waitFor(() => expect(v.launch).toHaveBeenCalledTimes(2));
  expect(v.launch.mock.calls[1][0].agent_session?.agent_session_id).toBe('session-b');
  expect(v.close).toHaveBeenCalledTimes(1);
  expect(v.save).not.toHaveBeenCalled();
  expect(v.bridge).not.toHaveBeenCalled();
  expect(v.builtinMounts()).toBe(0);
});

test('saved selection resolves before builtin can mount or produce side effects', async () => {
  let resolve!: (value: AgentPresetUiBinding) => void;
  const v = await open({ preference: () => new Promise(r => { resolve = r; }) });
  expect(v.view.queryByText('Built-in session content')).toBeNull();
  expect(v.builtinMounts()).toBe(0);
  expect(v.launch).not.toHaveBeenCalled();
  await act(async () => { resolve({ ...builtinPreference, binding: { binding_version: 2, selection: choice } }); });
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  expect(v.builtinMounts()).toBe(0);
  expect(v.save).not.toHaveBeenCalled();
  expect(v.bridge).not.toHaveBeenCalled();
});

test('temporary builtin choice wins over late persisted consent', async () => {
  let resolve!: (value: AgentPresetUiBinding) => void;
  const v = await open({ preference: () => new Promise(r => { resolve = r; }) });
  expect(v.view.queryByText('Built-in session content')).toBeNull();
  fireEvent.click(v.view.getByRole('button', { name: en.view.builtin }));
  await act(async () => { resolve({ ...builtinPreference, binding: { binding_version: 2, selection: choice } }); });
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  expect(v.launch).not.toHaveBeenCalled();
  expect(v.save).not.toHaveBeenCalled();
});

test.each([
  { name: 'cleared', previous: choice, current: null },
  { name: 'selected', previous: null, current: choice },
])('returning to a cached Session uses the freshly $name default, not cached consent', async ({ previous, current }) => {
  const v = await open({ preference: async () => ({ ...builtinPreference,
    binding: { binding_version: 1, selection: previous } }) });
  if (previous) await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  else await v.view.findByText('Built-in session content');
  const initialLaunches = v.launch.mock.calls.length;
  v.preference.mockResolvedValue(builtinPreference);
  v.rerender(v.page('session-b'));
  await v.view.findByText('Built-in session content');
  let resolve!: (value: AgentPresetUiBinding) => void;
  v.preference.mockImplementation(() => new Promise(r => { resolve = r; }));
  v.rerender(v.page('session-a'));
  await waitFor(() => expect(v.preference.mock.calls.filter(([request]) => request.agent_session_id === 'session-a')).toHaveLength(2));
  expect(v.launch).toHaveBeenCalledTimes(initialLaunches);
  expect(v.view.queryByText('Built-in session content') === null).toBe(true);
  await act(async () => { resolve({ ...builtinPreference, binding: { binding_version: 2, selection: current } }); });
  if (current) {
    await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
    expect(v.launch).toHaveBeenCalledTimes(initialLaunches + 1);
  } else {
    await v.view.findByText('Built-in session content');
    expect(v.launch).toHaveBeenCalledTimes(initialLaunches);
  }
  expect(v.save).not.toHaveBeenCalled();
  expect(v.bridge).not.toHaveBeenCalled();
});

test('failed reread of a cached Session never authorizes its previously saved plugin', async () => {
  const v = await open({ preference: async () => ({ ...builtinPreference,
    binding: { binding_version: 1, selection: choice } }) });
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  v.preference.mockResolvedValue(builtinPreference);
  v.rerender(v.page('session-b'));
  await v.view.findByText('Built-in session content');
  v.preference.mockRejectedValue(new Error('default reread unavailable'));
  v.rerender(v.page('session-a'));
  await v.view.findByText(en.view.defaultFailed);
  await waitFor(() => expect(v.view.queryByText('Built-in session content') !== null).toBe(true));
  expect(v.launch).toHaveBeenCalledTimes(1);
  expect(v.save).not.toHaveBeenCalled();
  expect(v.bridge).not.toHaveBeenCalled();
});

test('saving exact default survives navigation; clearing persists null without a turn', async () => {
  const v = await open();
  v.select();
  fireEvent.click(v.view.getByRole('button', { name: en.view.remember }));
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  expect(v.save).toHaveBeenCalledWith({ preset_id: 'preset-a', request: { expected_binding_version: 0, selection: choice } });
  v.rerender(v.page('session-b'));
  await waitFor(() => expect(v.launch).toHaveBeenCalledTimes(2));
  fireEvent.click(v.view.getByRole('button', { name: en.view.clearDefault }));
  await v.view.findByText('Built-in session content');
  expect(v.save.mock.calls[1][0]).toEqual({ preset_id: 'preset-a', request: { expected_binding_version: 1, selection: null } });
  v.rerender(v.page('session-c'));
  await v.view.findByText('Built-in session content');
  expect(v.launch).toHaveBeenCalledTimes(2);
  expect(v.bridge).not.toHaveBeenCalled();
});

test('unavailable saved release stays recorded and never consents to an upgrade', async () => {
  const v = await open({ preference: async () => ({ ...builtinPreference, binding: { binding_version: 1, selection: choice } }),
    choices: [{ ...choice, expected_release_digest: 'b'.repeat(64) }] });
  await v.view.findByText(en.view.defaultUnavailable);
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  expect(v.launch).not.toHaveBeenCalled();
  expect(v.save).not.toHaveBeenCalled();
  v.select(); v.use();
  await waitFor(() => expect(v.launch).toHaveBeenCalledTimes(1));
  expect(v.launch.mock.calls[0][0].agent_session?.expected_release_digest).toBe('b'.repeat(64));
  expect(v.save).not.toHaveBeenCalled();
});

test('remembering the current page does not reopen its Surface or discard local presentation state', async () => {
  const v = await open();
  v.select(); v.use();
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  const frame = v.container.querySelector('iframe');
  fireEvent.click(v.view.getByRole('button', { name: en.view.remember }));
  await waitFor(() => expect(v.view.getByRole('button', { name: en.view.clearDefault }).hasAttribute('disabled')).toBe(false));
  expect(v.container.querySelector('iframe')).toBe(frame);
  expect(v.launch).toHaveBeenCalledTimes(1);
  expect(v.close).not.toHaveBeenCalled();
});

test('save failure does not retry or replace current view; explicit refresh permits a new CAS', async () => {
  const v = await open();
  v.save.mockRejectedValueOnce(new Error('version conflict'));
  v.select();
  fireEvent.click(v.view.getByRole('button', { name: en.view.remember }));
  await v.view.findByText(en.view.defaultSaveFailed);
  expect(v.save).toHaveBeenCalledTimes(1);
  expect(v.launch).not.toHaveBeenCalled();
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  v.preference.mockResolvedValue({ ...builtinPreference, binding: { binding_version: 9, selection: null } });
  fireEvent.click(v.view.getByRole('button', { name: en.actions.retry }));
  await waitFor(() => expect(v.preference).toHaveBeenCalledTimes(2));
  fireEvent.click(v.view.getByRole('button', { name: en.view.remember }));
  await waitFor(() => expect(v.save).toHaveBeenCalledTimes(2));
  expect(v.save.mock.calls[1][0].request.expected_binding_version).toBe(9);
});

test('late save cannot select or launch into a different Session, and duplicate saves are suppressed', async () => {
  const v = await open();
  let resolve!: (value: AgentPresetUiBinding) => void;
  v.save.mockImplementation(() => new Promise(r => { resolve = r; }));
  v.select();
  const button = v.view.getByRole('button', { name: en.view.remember });
  fireEvent.click(button); fireEvent.click(button);
  expect(v.save).toHaveBeenCalledTimes(1);
  v.rerender(v.page('session-b'));
  await act(async () => { resolve({ ...builtinPreference, binding: { binding_version: 1, selection: choice } }); });
  await v.view.findByText('Built-in session content');
  expect(v.launch).not.toHaveBeenCalled();
  expect(v.bridge).not.toHaveBeenCalled();
});

test('preference read failure keeps builtin recovery and temporary plugin use available', async () => {
  const v = await open({ preference: async () => { throw new Error('unavailable'); } });
  await v.view.findByText(en.view.defaultFailed);
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  v.select(); v.use();
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  expect(v.save).not.toHaveBeenCalled();
});

test('template creates a draft only, prevents double click and ignores a late navigation result', async () => {
  const v = await open();
  let resolve!: (value: PluginRuntimeDraft) => void;
  const create = spyOn(pluginRuntimeProduct.agentSessionTemplate, 'invoke').mockImplementation(() => new Promise(r => { resolve = r; }));
  const button = v.view.getByRole('button', { name: en.view.createTemplate });
  fireEvent.click(button); fireEvent.click(button);
  expect(create).toHaveBeenCalledTimes(1);
  await act(async () => { resolve({ id: 'draft-one' } as PluginRuntimeDraft); });
  expect(v.draftCreated).toHaveBeenCalledWith('draft-one');
  expect(v.launch).not.toHaveBeenCalled();
  expect(v.bridge).not.toHaveBeenCalled();
  fireEvent.click(button);
  v.rerender(v.page('session-b'));
  await act(async () => { resolve({ id: 'draft-two' } as PluginRuntimeDraft); });
  expect(v.draftCreated).toHaveBeenCalledTimes(1);
});

test('template failure keeps builtin available and never automatically retries', async () => {
  const v = await open();
  const create = spyOn(pluginRuntimeProduct.agentSessionTemplate, 'invoke').mockRejectedValue(new Error('unconfirmed'));
  fireEvent.click(v.view.getByRole('button', { name: en.view.createTemplate }));
  await v.view.findByText(en.view.templateFailed);
  expect(create).toHaveBeenCalledTimes(1);
  expect(v.draftCreated).not.toHaveBeenCalled();
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  expect(v.launch).not.toHaveBeenCalled();
});

test('explicit exact choice replaces builtin content; return closes only its Surface, never sends a turn', async () => {
  const v = await open();
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  v.select();
  expect(v.launch).not.toHaveBeenCalled();
  v.use();
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  expect(v.view.queryByText('Built-in session content')).toBeNull();
  expect(v.launch).toHaveBeenCalledWith({ plugin_id: choice.plugin_id, agent_session: {
    agent_session_id: 'session-a', expected_release_digest: choice.expected_release_digest, ui_capability: choice.capability,
  } });
  await act(async () => { await i18n.changeLanguage('en'); });
  expect(v.launch).toHaveBeenCalledTimes(1);
  fireEvent.click(v.view.getByRole('button', { name: en.view.builtin }));
  await waitFor(() => expect(v.close).toHaveBeenCalledWith({ plugin_id: choice.plugin_id,
    surface_session_id: surface.surface_session_id, surface_capability: surface.surface_capability }));
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  expect(v.container.querySelector('iframe')).toBeNull();
  expect(v.bridge).not.toHaveBeenCalled();
});

test('catalog withdrawal latches revocation; reappearing release requires a new explicit selection', async () => {
  const v = await open();
  v.select(); v.use();
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  v.catalog.mockResolvedValue([]);
  fireEvent.click(v.view.getByRole('button', { name: en.actions.retry }));
  await v.view.findByText(en.view.changed);
  await waitFor(() => expect(v.close).toHaveBeenCalledTimes(1));
  v.catalog.mockResolvedValue([choice]);
  fireEvent.click(v.view.getByRole('button', { name: en.actions.retry }));
  await v.view.findByRole('option', { name: /My Agent/ });
  expect(v.view.getByText(en.view.changed)).toBeTruthy();
  expect(v.launch).toHaveBeenCalledTimes(1);
  v.select(); v.use();
  await waitFor(() => expect(v.launch).toHaveBeenCalledTimes(2));
});

test('changed release is not consented automatically and an open failure retains builtin recovery', async () => {
  const v = await open();
  v.select(); v.use();
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  v.catalog.mockResolvedValue([{ ...choice, expected_release_digest: 'b'.repeat(64) }]);
  fireEvent.click(v.view.getByRole('button', { name: en.actions.retry }));
  await v.view.findByText(en.view.changed);
  expect(v.launch).toHaveBeenCalledTimes(1);
  v.launch.mockRejectedValue(new Error('release changed'));
  v.select(); v.use();
  await v.view.findByText(en.view.failed);
  expect(v.launch).toHaveBeenCalledTimes(2);
  fireEvent.click(v.view.getByRole('button', { name: en.view.builtin }));
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  expect(v.bridge).not.toHaveBeenCalled();
});

test('late open after Session navigation is closed and never mounted under the new Session', async () => {
  const v = await open();
  let resolve!: (value: PluginRuntimeSurfaceLaunchDescriptor) => void;
  v.launch.mockImplementation(() => new Promise(r => { resolve = r; }));
  v.select(); v.use();
  await waitFor(() => expect(v.launch).toHaveBeenCalledTimes(1));
  v.rerender(v.page('session-b'));
  await act(async () => { resolve(surface); });
  await waitFor(() => expect(v.close).toHaveBeenCalledTimes(1));
  expect(v.container.querySelector('iframe')).toBeNull();
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  expect(v.launch).toHaveBeenCalledTimes(1);
});

test('failed close is disclosed, not reported as successful server revocation', async () => {
  const v = await open();
  spyOn(console, 'warn').mockImplementation(() => {});
  v.close.mockRejectedValue(new Error('network unavailable'));
  v.select(); v.use();
  await waitFor(() => expect(v.container.querySelector('iframe')).not.toBeNull());
  fireEvent.click(v.view.getByRole('button', { name: en.view.builtin }));
  await v.view.findByText(en.view.closeFailed);
  expect(v.view.getByText('Built-in session content')).toBeTruthy();
  expect(v.container.querySelector('iframe')).toBeNull();
});
