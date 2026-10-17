import { parseMessageId } from '@/common/types/ids';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { IProvider, TChatConversation } from '@/common/config/storage';
import type { ProductAgentOptions, ProductAgentSelectionResult } from '@/common/types/agentPlatform';
import { parseCompanionId, parseConversationId, parseProviderId } from '@/common/types/ids';
import * as taskModels from '@/renderer/hooks/agent/useModelsForTask';
import CompanionAgentModelQuickSettings from './CompanionAgentModelQuickSettings';
import type { CompanionHandle } from './types';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'nomi.cohabit.quickAgent': 'Agent', 'nomi.cohabit.quickModel': 'Model',
  'nomi.cohabit.modelDefaultSaveFailed': 'Session switched; retry saving the default.',
  'agentSettings.template.companion.default.name': 'Companion',
  'agentSettings.template.companion.robot.name': 'Robot companion',
  'agentSettings.productBinding.label': 'Agent setup', 'agentSettings.productBinding.busy': 'Wait for the current reply to finish',
  'agentSettings.productBinding.saveFailed': 'Save failed', 'agentSettings.actions.retry': 'Retry',
  'agent.model.switchSuccess': 'Model switched',
  'nomi.cohabit.inheritForFuture': 'Inherit Companion model for future conversations',
  'nomi.cohabit.robotInherited': 'Future Robot conversations inherit the Companion model',
} } } });
const providerId = parseProviderId('019b0000-0000-7000-8000-000000000021');
const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000022');
const conversationId = parseConversationId('019b0000-0000-7000-8000-000000000023');
const provider = { id: providerId, name: 'Provider', platform: 'openai', models: [
  { model: 'model-a', display_name: 'Model A' }, { model: 'model-b', display_name: 'Model B' },
] } as IProvider;
const profile = { companion_id: companionId, model: { provider_id: providerId, model: 'model-a' },
  robot: { model: { provider_id: providerId, model: 'model-b' }, skills: { enabled: [], disabled_auto: [] }, mcp_servers: [] },
} as unknown as NonNullable<CompanionHandle['profile']>;
const idle: TChatConversation = { id: conversationId, type: 'nomi', name: 'Companion', created_at: 0, modified_at: 0, status: 'pending',
  model: { ...provider, use_model: 'model-a' }, extra: { workspace: '' }, runtime: { state: 'idle', is_processing: false, can_send_message: true, },
};
const agentOptions: ProductAgentOptions = { selection: { kind: 'template', template_key: 'companion.default' }, needs_model: false,
  options: [{ selection: { kind: 'template', template_key: 'companion.default' }, display_name: '', available: true, reason: null },
    { selection: { kind: 'template', template_key: 'companion.robot' }, display_name: '', available: true, reason: null },
    { selection: { kind: 'preset', preset_id: 'my-companion' }, display_name: 'Personal companion', available: true, reason: null }],
};
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });
const install = () => {
  const catalog = spyOn(taskModels, 'useModelsForTask').mockReturnValue({ groups: [{ provider, models: ['model-a', 'model-b'] }], isLoading: false, refresh: () => {} });
  const options = spyOn(ipcBridge.agentPlatform.productBindingOptions, 'invoke').mockImplementation(async ({ target_kind }) => ({
    ...agentOptions, selection: { kind: 'template', template_key: target_kind === 'robot' ? 'companion.robot' : 'companion.default' },
    options: agentOptions.options.filter(option => option.selection.kind !== 'template'
      || option.selection.template_key === (target_kind === 'robot' ? 'companion.robot' : 'companion.default')),
  }));
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  for (const spy of [catalog, options, success, error]) restores.push(() => spy.mockRestore());
  return { success, error };
};
const mount = (patchCompanion: CompanionHandle['patchCompanion'], conversation: TChatConversation | null = idle, onSavingChange?: (saving: boolean) => void, onChanged?: () => void, role: 'companion' | 'robot' = 'companion') => render(
  <SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <CompanionAgentModelQuickSettings companion={{ profile, patchCompanion } as CompanionHandle} role={role} conversation={conversation} onSavingChange={onSavingChange} onChanged={onChanged} />
  </I18nextProvider></MemoryRouter></SWRConfig>
);

test('the docked model dropdown switches the existing Session and locks the Agent selector while saving', async () => {
  install();
  let resolveSwitch!: (saved: boolean) => void;
  const switched = spyOn(ipcBridge.conversation.switchModel, 'invoke').mockImplementation(() => new Promise((resolve) => { resolveSwitch = resolve; }));
  restores.push(() => switched.mockRestore());
  const profilePatches: unknown[] = [];
  const saving: boolean[] = [];
  const view = mount(async (patch) => { profilePatches.push(patch); return { ...profile, model: patch.model! }; }, idle, (value) => saving.push(value));
  await waitFor(() => expect(view.container.querySelectorAll('.arco-select')).toHaveLength(2));
  fireEvent.click(view.container.querySelectorAll('.arco-select')[1]);
  fireEvent.click(await view.findByText('Model B'));
  await waitFor(() => expect(switched).toHaveBeenCalledTimes(1));
  expect(profilePatches).toHaveLength(0);
  expect(view.container.querySelectorAll('.arco-select-disabled')).toHaveLength(2);
  expect(view.queryByRole('link')).toBeNull();
  await act(async () => resolveSwitch(true));
  await waitFor(() => expect(profilePatches).toEqual([{ model: { provider_id: providerId, model: 'model-b' } }]));
  expect(switched.mock.calls[0][0].conversation_id).toBe(conversationId);
  await waitFor(() => expect(saving).toEqual([true, false]));
  expect(view.container.querySelectorAll('.arco-select')[1].textContent).toContain('Model B');
});

test('the independent robot Agent default does not switch the current companion Session', async () => {
  install();
  const selected = spyOn(ipcBridge.agentPlatform.selectProductBinding, 'invoke').mockResolvedValue({
    selection: { kind: 'preset', preset_id: 'my-companion' }, needs_model: false,
  });
  const switchModel = spyOn(ipcBridge.conversation.switchModel, 'invoke').mockResolvedValue(true);
  const permissions = spyOn(ipcBridge.robot.setPermissions, 'invoke').mockRejectedValue(new Error('Must not change permissions'));
  for (const spy of [selected, switchModel, permissions]) restores.push(() => spy.mockRestore());
  const patches: unknown[] = [];
  const view = mount(async (patch) => { patches.push(patch); return profile; }, null, undefined, undefined, 'robot');
  await waitFor(() => expect(view.container.querySelectorAll('.arco-select')).toHaveLength(2));
  expect(selected).not.toHaveBeenCalled();
  expect(view.container.querySelectorAll('.arco-select')[0].textContent).toContain('Robot companion');
  fireEvent.click(view.container.querySelectorAll('.arco-select')[0]);
  fireEvent.click(await view.findByText('Personal companion'));
  await waitFor(() => expect(selected).toHaveBeenCalledTimes(1));
  expect(selected.mock.calls[0][0]).toEqual({ target_kind: 'robot', target_id: companionId,
    request: { selection: { kind: 'preset', preset_id: 'my-companion' },
      model: { provider_id: providerId, model: 'model-b' } },
  });
  expect(switchModel).not.toHaveBeenCalled();
  expect(permissions).not.toHaveBeenCalled();
  expect(patches).toEqual([]);
});

test('a busy Session refusal preserves the current model and does not patch the default', async () => {
  const { error, success } = install();
  const switched = spyOn(ipcBridge.conversation.switchModel, 'invoke').mockRejectedValue(new BackendHttpError({ method: 'PUT', path: '/private', status: 409,
    body: { code: 'AGENT_SESSION_EFFECTS_UNSETTLED', error: 'private receipt identity' } }));
  restores.push(() => switched.mockRestore());
  let patched = false;
  const view = mount(async () => { patched = true; return profile; });
  await waitFor(() => expect(view.container.querySelectorAll('.arco-select')).toHaveLength(2));
  fireEvent.click(view.container.querySelectorAll('.arco-select')[1]);
  fireEvent.click(await view.findByText('Model B'));
  await waitFor(() => expect(error).toHaveBeenCalledWith('Wait for the current reply to finish'));
  expect(patched).toBe(false);
  expect(success).not.toHaveBeenCalled();
  expect(view.container.querySelectorAll('.arco-select')[1].textContent).toContain('Model A');
  expect(view.baseElement.textContent).not.toContain('private receipt identity');
});

test('clearing the robot override restores inheritance without switching the current Session', async () => {
  install();
  const switched = spyOn(ipcBridge.conversation.switchModel, 'invoke');
  restores.push(() => switched.mockRestore());
  const patches: unknown[] = [];
  const view = mount(async patch => { patches.push(patch); return profile; }, null, undefined, undefined, 'robot');
  await waitFor(() => expect(view.container.querySelectorAll('.arco-select')).toHaveLength(2));
  fireEvent.click(view.getByRole('button', { name: 'Inherit Companion model for future conversations' }));
  await waitFor(() => expect(patches).toEqual([{ robot: { model: null } }]));
  expect(switched).not.toHaveBeenCalled();
});

test('both roles remain visible and an unset robot model displays the inherited Companion model', async () => {
  install();
  const inheritedProfile = { ...profile, robot: { ...profile.robot!, model: null } };
  const patchCompanion: CompanionHandle['patchCompanion'] = async () => inheritedProfile;
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}>
    <CompanionAgentModelQuickSettings companion={{ profile: inheritedProfile, patchCompanion } as CompanionHandle} role='companion' />
    <CompanionAgentModelQuickSettings companion={{ profile: inheritedProfile, patchCompanion } as CompanionHandle} role='robot' />
  </I18nextProvider></MemoryRouter></SWRConfig>);
  await waitFor(() => expect(view.container.querySelectorAll('.arco-select')).toHaveLength(4));
  expect(view.container.querySelectorAll('.arco-select')[0].textContent).toContain('Companion');
  expect(view.container.querySelectorAll('.arco-select')[2].textContent).toContain('Robot companion');
  expect(view.container.querySelectorAll('.arco-select')[3].textContent).toContain('Model A');
  expect(view.getByText('Future Robot conversations inherit the Companion model')).toBeTruthy();
  expect(view.container.querySelectorAll('.arco-select-disabled')).toHaveLength(0);
});

test('partial default save keeps the committed model and retries without another binding transition', async () => {
  const { success } = install();
  const switched = spyOn(ipcBridge.conversation.switchModel, 'invoke').mockResolvedValue(true);
  restores.push(() => switched.mockRestore());
  let patches = 0;
  const view = mount(async () => { if (++patches === 1) throw new Error('profile save failed'); return { ...profile, model: { provider_id: providerId, model: 'model-b' } }; });
  await waitFor(() => expect(view.container.querySelectorAll('.arco-select')).toHaveLength(2));
  fireEvent.click(view.container.querySelectorAll('.arco-select')[1]);
  fireEvent.click(await view.findByText('Model B'));
  await waitFor(() => expect(view.getByRole('alert').textContent).toContain('Session switched; retry saving the default.'));
  await waitFor(() => expect(view.container.querySelectorAll('.arco-select')).toHaveLength(2));
  expect(view.container.querySelectorAll('.arco-select')[1].textContent).toContain('Model B');
  expect(success).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(patches).toBe(2));
  expect(switched).toHaveBeenCalledTimes(1);
  await waitFor(() => expect(success).toHaveBeenCalledTimes(1));
});

test.each(['running', 'paused'])('active or paused Turns disable Agent and model quick editing (%s)', async (phase) => {
  install();
  const conversation = { ...idle, status: 'running' as const,
    runtime: { ...idle.runtime!, active_turn_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000081'), is_processing: phase === 'running' },
    extra: { ...idle.extra, },
  } as TChatConversation;
  const view = mount(async () => profile, conversation);
  await waitFor(() => expect(view.container.querySelectorAll('.arco-select')).toHaveLength(2));
  expect(view.container.querySelectorAll('.arco-select-disabled')).toHaveLength(2);
  expect(view.getByRole('status').textContent).toBe('Wait for the current reply to finish');
});

test('closing the sidebar during a model save releases its parent lock and refreshes the committed Session', async () => {
  install();
  let resolveSwitch!: (saved: boolean) => void;
  const switched = spyOn(ipcBridge.conversation.switchModel, 'invoke').mockImplementation(() => new Promise((resolve) => { resolveSwitch = resolve; }));
  restores.push(() => switched.mockRestore());
  const saving: boolean[] = [];
  let refreshes = 0;
  const view = mount(async () => profile, idle, (value) => saving.push(value), () => { refreshes++; });
  await waitFor(() => expect(view.container.querySelectorAll('.arco-select')).toHaveLength(2));
  fireEvent.click(view.container.querySelectorAll('.arco-select')[1]);
  fireEvent.click(await view.findByText('Model B'));
  await waitFor(() => expect(switched).toHaveBeenCalledTimes(1));
  view.unmount();
  await act(async () => resolveSwitch(true));
  await waitFor(() => expect(saving).toEqual([true, false]));
  expect(refreshes).toBe(1);
});

test('closing the sidebar during an Agent save releases its parent lock', async () => {
  install();
  let resolveSelect!: (saved: ProductAgentSelectionResult) => void;
  const selected = spyOn(ipcBridge.agentPlatform.selectProductBinding, 'invoke').mockImplementation(() => new Promise((resolve) => { resolveSelect = resolve; }));
  restores.push(() => selected.mockRestore());
  const saving: boolean[] = [];
  let refreshes = 0;
  const view = mount(async () => profile, idle, (value) => saving.push(value), () => { refreshes++; });
  await waitFor(() => expect(view.container.querySelectorAll('.arco-select')).toHaveLength(2));
  fireEvent.click(view.container.querySelectorAll('.arco-select')[0]);
  fireEvent.click(await view.findByText('Personal companion'));
  await waitFor(() => expect(selected).toHaveBeenCalledTimes(1));
  expect(selected.mock.calls[0][0].request.update_default).toBe(true);
  view.unmount();
  await act(async () => resolveSelect({ selection: { kind: 'preset', preset_id: 'my-companion' }, needs_model: false }));
  await waitFor(() => expect(saving).toEqual([true, false]));
  expect(refreshes).toBe(1);
});
