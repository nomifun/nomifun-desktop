import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { IAgentSessionObservation, IApiRobot } from '@/common/adapter/ipcBridge';
import type { TChatConversation } from '@/common/config/storage';
import type { AgentBindingValue, ProductAgentOptions } from '@/common/types/agentPlatform';
import { parseCompanionId, parseConversationId, parseProviderId } from '@/common/types/ids';
import CompanionRobotResourceSettings from './CompanionRobotResourceSettings';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'nomi.cohabit.robotResource.label': 'Conversation robot',
  'nomi.cohabit.robotResource.none': 'No robot control',
  'nomi.cohabit.robotResource.unavailable': 'Selected robot is no longer paired',
  'nomi.cohabit.robotResource.apply': 'Apply robot',
  'nomi.cohabit.robotResource.saved': 'Robot updated',
  'nomi.cohabit.robotResource.changed': 'Settings changed; review and apply again',
  'nomi.cohabit.robotResource.capabilitiesChanged': 'Apply the updated device capabilities',
  'agentSettings.productBinding.busy': 'Wait for the current reply',
  'agentSettings.productBinding.loadFailed': 'Load failed',
  'agentSettings.productBinding.saveFailed': 'Save failed',
  'agentSettings.productBinding.partialSave': 'Conversation changed; retry saving the default',
  'agentSettings.productBinding.selectionNeededHint': 'Choose an Agent first',
  'agentSettings.actions.retry': 'Retry',
} } } });
const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000022');
const conversationId = parseConversationId('019b0000-0000-7000-8000-000000000023');
const providerId = parseProviderId('019b0000-0000-7000-8000-000000000021');
const idle = { id: conversationId, type: 'nomi', status: 'pending', extra: { workspace: '', companion_id: companionId, companion_role: 'robot' },
  model: { id: providerId, use_model: 'model-a' },
  agent_snapshot: { enabled_capabilities: ['robot'] },
  runtime: { state: 'idle', is_processing: false, can_send_message: true, },
} as TChatConversation;
const options: ProductAgentOptions = { selection: { kind: 'template', template_key: 'companion.default' }, needs_model: false, options: [] };
const workspaceRoot = 'C:\\Companion\\existing-workspace';
const resources = [
  { resource_kind: 'companion', resource_id: companionId },
  { resource_kind: 'companion_memory', resource_id: companionId },
  { resource_kind: 'workspace', resource_id: 'selected-workspace-existing' },
  { resource_kind: 'project_memory', resource_id: 'project-workspace-memory:existing' },
  { resource_kind: 'knowledge_base', resource_id: 'knowledge-a' },
  { resource_kind: 'knowledge_base', resource_id: 'knowledge-b' },
  { resource_kind: 'mcp_server', resource_id: 'mcp-a' },
  { resource_kind: 'channel', resource_id: 'channel-a' },
];
const binding = (robotId?: string, version = 1, capabilityRevision = 0) => ({ binding_version: version, typed_resource_bindings:
  [...resources, ...(robotId ? [{ resource_kind: 'robot', resource_id: robotId }] : [])]
    .map((resource) => ({ ...resource, binding_id: resource.resource_id, owner_id: 'owner', operations: ['read'],
      typed_parameters: resource.resource_kind === 'robot' ? { capability_revision: String(capabilityRevision) }
        : resource.resource_kind === 'workspace' ? { workspace_root: workspaceRoot } : {} })),
} as AgentBindingValue);
const desktop = (robotId?: string, capabilities = ['robot']) => ({ ...idle,
  extra: { ...idle.extra, companion_role: 'companion' },
  agent_snapshot: { enabled_capabilities: capabilities, canonical_binding: binding(robotId) },
} as TChatConversation);
const robot = (robot_id: string, name: string, owner = companionId, capability_revision = 0) => ({ robot_id, name, companion_id: owner, capability_revision } as IApiRobot);
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });
const install = (initialRobot?: string) => {
  let currentBinding = binding(initialRobot);
  const get = spyOn(ipcBridge.agentPlatform.sessions.get, 'invoke').mockImplementation(async () => ({ session: { agent_binding: currentBinding } } as IAgentSessionObservation));
  const getOptions = spyOn(ipcBridge.agentPlatform.productBindingOptions, 'invoke').mockResolvedValue(options);
  let knownRobots = [robot('robot-a', 'Head A'), robot('robot-b', 'Head B'),
    robot('robot-foreign', 'Other companion', parseCompanionId('019b0000-0000-7000-8000-000000000024'))];
  const robots = spyOn(ipcBridge.robot.list, 'invoke').mockImplementation(async () => knownRobots);
  const permissions = spyOn(ipcBridge.robot.setPermissions, 'invoke').mockRejectedValue(new Error('Must not change permissions'));
  const status = spyOn(ipcBridge.robot.onStatus, 'on').mockImplementation(() => () => {});
  const save = spyOn(ipcBridge.agentPlatform.selectProductBinding, 'invoke').mockImplementation(async ({ request }) => {
    const selected = request.resource_selections?.find((resource) => resource.resource_kind === 'robot')?.resource_id;
    currentBinding = binding(selected, currentBinding.binding_version + 1, knownRobots.find((robot) => robot.robot_id === selected)?.capability_revision ?? 0);
    return { selection: options.selection!, needs_model: false };
  });
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  for (const spy of [get, getOptions, robots, permissions, status, save, success, error]) restores.push(() => spy.mockRestore());
  return { get, getOptions, robots, save, success, error, permissions, setBinding: (next: AgentBindingValue) => { currentBinding = next; },
    setRobots: (next: IApiRobot[]) => { knownRobots = next; } };
};
const mount = (conversation: TChatConversation = idle, onChanged?: () => void, onSavingChange?: (saving: boolean) => void) => render(
  <SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><I18nextProvider i18n={i18n}>
    <CompanionRobotResourceSettings companionId={companionId} conversation={conversation} onChanged={onChanged} onSavingChange={onSavingChange} />
  </I18nextProvider></SWRConfig>
);
const choose = async (view: ReturnType<typeof mount>, text: string) => {
  await waitFor(() => expect(view.container.querySelector('.arco-select')).not.toBeNull());
  fireEvent.click(view.container.querySelector('.arco-select')!);
  fireEvent.click(await view.findByText(text));
};

test('desktop companion Sessions without Robot authorization or an existing device do not load resources', async () => {
  const api = install();
  const view = mount(desktop(undefined, []));
  await act(async () => {});
  expect(view.container.textContent).toBe('');
  expect(api.get).not.toHaveBeenCalled();
  expect(api.getOptions).not.toHaveBeenCalled();
  expect(api.save).not.toHaveBeenCalled();
});

test.each(['robot', 'companion'] as const)('paired robots need an explicit apply and retain other resources for the %s input entrance', async (role) => {
  const { save, get, permissions } = install();
  let resolveSave!: () => void;
  save.mockImplementation(() => new Promise((resolve) => { resolveSave = () => resolve({ selection: options.selection!, needs_model: false }); }));
  const saving: boolean[] = [];
  const view = mount(role === 'robot' ? idle : desktop(), undefined, (value) => saving.push(value));
  await choose(view, 'Head B');
  expect(view.queryByText('Other companion')).toBeNull();
  expect(save).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button', { name: 'Apply robot' }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(get).toHaveBeenCalledTimes(2);
  expect(save.mock.calls[0][0]).toEqual({ target_kind: 'companion', target_id: companionId, request: {
    selection: options.selection, conversation_id: conversationId, model: { provider_id: providerId, model: 'model-a' },
    resource_selections: [...resources.map((resource) => resource.resource_kind === 'project_memory'
      ? { ...resource, resource_id: 'default-project-memory' }
      : resource.resource_kind === 'workspace' ? { ...resource, resource_id: 'default-workspace' } : resource),
    { resource_kind: 'robot', resource_id: 'robot-b' }],
  } });
  expect(view.container.querySelector('.arco-select-disabled')).not.toBeNull();
  expect(view.getByRole('button', { name: 'Apply robot' }).hasAttribute('disabled')).toBe(true);
  await act(async () => resolveSave());
  await waitFor(() => expect(saving).toEqual([true, false]));
  expect(permissions).not.toHaveBeenCalled();
});

test('explicitly removing a robot preserves other resources', async () => {
  const { save } = install('robot-a');
  const view = mount();
  await choose(view, 'No robot control');
  fireEvent.click(view.getByRole('button', { name: 'Apply robot' }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0][0].request.resource_selections).toHaveLength(resources.length);
  expect(save.mock.calls[0][0].request.resource_selections?.some((resource) => resource.resource_kind === 'robot')).toBe(false);
});

test('desktop companion can remove its existing device resource even after Robot authorization was removed', async () => {
  const { save, getOptions } = install('robot-a');
  const view = mount(desktop('robot-a', []));
  await choose(view, 'No robot control');
  expect(getOptions.mock.calls[0][0]).toEqual({ target_kind: 'companion', target_id: companionId, conversation_id: conversationId,
    model: { provider_id: providerId, model: 'model-a' } });
  fireEvent.click(view.getByRole('button', { name: 'Apply robot' }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0][0].target_kind).toBe('companion');
  expect(save.mock.calls[0][0].request.resource_selections).toHaveLength(resources.length);
  expect(save.mock.calls[0][0].request.resource_selections?.some((resource) => resource.resource_kind === 'robot')).toBe(false);
});

test('device configuration reads the exact device Session through the unified companion target', async () => {
  const { save, getOptions } = install();
  const view = mount();
  await waitFor(() => expect(view.container.querySelector('.arco-select')).not.toBeNull());
  expect(getOptions.mock.calls[0][0]).toEqual({ target_kind: 'companion', target_id: companionId, conversation_id: conversationId,
    model: { provider_id: providerId, model: 'model-a' } });
  expect(save).not.toHaveBeenCalled();
});

test.each(['robot', 'companion'] as const)('a capability revision can be reapplied for the %s input entrance without changing the device', async (role) => {
  const { save, setRobots } = install('robot-a');
  setRobots([robot('robot-a', 'Head A', companionId, 2)]);
  const view = mount(role === 'robot' ? idle : desktop('robot-a'));
  await waitFor(() => expect(view.getByRole('status').textContent).toBe('Apply the updated device capabilities'));
  const button = view.getByRole('button', { name: 'Apply robot' });
  expect(button.hasAttribute('disabled')).toBe(false);
  fireEvent.click(button);
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(save.mock.calls[0][0].target_kind).toBe('companion');
  expect(save.mock.calls[0][0].request.conversation_id).toBe(conversationId);
  expect(save.mock.calls[0][0].request.resource_selections?.find((item) => item.resource_kind === 'robot')?.resource_id).toBe('robot-a');
  await waitFor(() => expect(view.queryByRole('status')).toBeNull());
});

test('a newer capability revision during apply requires review before dispatching the transition', async () => {
  const { save, setRobots } = install('robot-a');
  setRobots([robot('robot-a', 'Head A', companionId, 1)]);
  const view = mount();
  await waitFor(() => expect(view.getByRole('status').textContent).toBe('Apply the updated device capabilities'));
  setRobots([robot('robot-a', 'Head A', companionId, 2)]);
  fireEvent.click(view.getByRole('button', { name: 'Apply robot' }));
  await waitFor(() => expect(view.getByRole('alert').textContent).toBe('Settings changed; review and apply again'));
  expect(save).not.toHaveBeenCalled();
});

test.each(['running', 'paused'])('active and paused turns block robot selection and apply (%s)', async (phase) => {
  const { save } = install();
  const conversation = { ...idle, status: 'running', extra: { ...idle.extra, },
    runtime: { ...idle.runtime, active_turn_id: 'turn', is_processing: phase === 'running' } } as TChatConversation;
  const view = mount(conversation);
  await waitFor(() => expect(view.container.querySelector('.arco-select-disabled')).not.toBeNull());
  expect(view.getByRole('status').textContent).toBe('Wait for the current reply');
  expect(view.getByRole('button', { name: 'Apply robot' }).hasAttribute('disabled')).toBe(true);
  expect(save).not.toHaveBeenCalled();
});

test('load failure stays disabled and can be retried without saving', async () => {
  const { get, save } = install();
  get.mockRejectedValueOnce(new Error('private read details'));
  const view = mount();
  await waitFor(() => expect(view.getByRole('alert').textContent).toContain('Load failed'));
  expect(view.getByRole('button', { name: 'Apply robot' }).hasAttribute('disabled')).toBe(true);
  expect(view.baseElement.textContent).not.toContain('private read details');
  fireEvent.click(view.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(view.queryByRole('alert')).toBeNull());
  expect(save).not.toHaveBeenCalled();
});

test('an unpaired current robot remains visible as unavailable and can be explicitly removed', async () => {
  const { save } = install('unpaired-robot');
  const view = mount();
  await waitFor(() => expect(view.container.textContent).toContain('Selected robot is no longer paired'));
  expect(save).not.toHaveBeenCalled();
  await choose(view, 'No robot control');
  fireEvent.click(view.getByRole('button', { name: 'Apply robot' }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
});

test('binding changes in another window require review before any PUT', async () => {
  const { save, setBinding } = install();
  let refreshed = 0;
  const view = mount(idle, () => { refreshed++; });
  await choose(view, 'Head A');
  setBinding(binding(undefined, 2));
  fireEvent.click(view.getByRole('button', { name: 'Apply robot' }));
  await waitFor(() => expect(view.getByRole('alert').textContent).toBe('Settings changed; review and apply again'));
  expect(save).not.toHaveBeenCalled();
  expect(refreshed).toBe(1);
});

test('missing editable Agent origin never guesses a selection or saves resources', async () => {
  const { getOptions, save } = install();
  getOptions.mockResolvedValue({ ...options, selection: null });
  const view = mount();
  await waitFor(() => expect(view.getByRole('status').textContent).toBe('Choose an Agent first'));
  expect(view.container.querySelector('.arco-select-disabled')).not.toBeNull();
  expect(save).not.toHaveBeenCalled();
});

test('a committed resource change with a default-save failure allows explicit retry', async () => {
  const { save, setBinding } = install();
  save.mockImplementationOnce(async () => {
    setBinding(binding('robot-a', 2));
    throw new BackendHttpError({ method: 'PUT', path: '/private', status: 500,
      body: { code: 'COMPANION_AGENT_DEFAULT_SAVE_FAILED', error: 'private storage details' } });
  });
  const view = mount();
  await choose(view, 'Head A');
  fireEvent.click(view.getByRole('button', { name: 'Apply robot' }));
  await waitFor(() => expect(view.getByRole('alert').textContent).toBe('Conversation changed; retry saving the default'));
  await waitFor(() => expect(view.getByRole('button', { name: 'Apply robot' }).hasAttribute('disabled')).toBe(false));
  fireEvent.click(view.getByRole('button', { name: 'Apply robot' }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(view.baseElement.textContent).not.toContain('private storage details');
});
