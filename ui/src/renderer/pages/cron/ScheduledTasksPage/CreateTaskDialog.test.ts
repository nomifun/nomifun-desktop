/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { afterEach, beforeAll, describe, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { createElement as h } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { ICronJob, ICronSchedule } from '@/common/adapter/ipcBridge';
import { parseCronJobId } from '@/common/types/ids';
import * as agents from '@/renderer/components/agent/useConversationAgentCatalog';
import * as capabilities from '@/renderer/components/agent/useAgentPresetCapabilities';
import * as providers from '@/renderer/hooks/agent/useModelProviderList';
import { configService } from '@/common/config/configService';
import { MemoryRouter } from 'react-router-dom';
import { parseProviderId, parseAgentPresetId, parseConversationId } from '@/common/types/ids';
import type { IProvider } from '@/common/config/storage';
import { asDigestHex, asResolvedSnapshotId } from '@/common/types/agentPlatform';
import agentSettings from '@/renderer/services/i18n/locales/en-US/agentSettings.json';
import guid from '@/renderer/services/i18n/locales/en-US/guid.json';
import * as models from '@/renderer/hooks/agent/useModelsForTask';
import * as conversations from '@/renderer/pages/conversation/SessionList/hooks/useConversationListSync';
import * as theme from '@/renderer/hooks/context/ThemeContext';
import cronStrings from '@/renderer/services/i18n/locales/en-US/cron.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import CreateTaskDialog from './CreateTaskDialog';

const source = readFileSync(new URL('./CreateTaskDialog.tsx', import.meta.url), 'utf8');

describe('CreateTaskDialog conversation id presentation', () => {
  test('formats and searches stable conversation UUIDs through shortSessionId without a # prefix', () => {
    expect(source.includes("import { shortSessionId } from '@renderer/utils/ui/shortId'")).toBe(true);
    expect(source.includes('const idLabel = shortSessionId(conv.id)')).toBe(true);
    expect(source.includes('const shortId = shortSessionId(conv.id).toLowerCase()')).toBe(true);
    expect(source.includes('`#${conv.id}`')).toBe(false);
  });
});

const locale = createInstance();
const restore: Array<() => void> = [];
beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { cron: cronStrings, common, agentSettings, guid } } } });
});
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach((dispose) => dispose()); });

const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000070');
const provider: IProvider = { id: providerId, name: 'Configured provider', platform: 'custom', base_url: 'https://example.invalid', auth_scheme: 'bearer', has_credentials: true, enabled: true,
  models: ['configured-model', 'other-model'].map(model => ({ model, provider_id: providerId, enabled: true, sort_order: 0, capabilities: [{ task: 'chat', traits: [], protocol: 'openai.chat_text', connection_role: 'default', allow_cross_origin_credentials: false, provider_params: {}, created_at: 1, updated_at: 1 }], created_at: 1, updated_at: 1 })) };
const templates = ['assistant.general', 'chat.minimal'].map(template_key => ({ template_key, seed: { enabled_capabilities: [], skill_bindings: [], required_resource_kinds: [], required_runtime_features: [] }, role_coverage: { required_capability_categories: [], required_capability_ids: [], required_runtime_features: [], required_resource_kinds: [] }, immutable: true, forkable: true })) as ReturnType<typeof agents.useConversationAgentCatalog>['officialTemplates'];

const cronJob = (schedule: ICronSchedule): ICronJob => ({
  cron_job_id: parseCronJobId('0190f5fe-7c00-7a00-8000-000000000074'),
  name: 'Fixture task', enabled: true, schedule, message: 'Fixture prompt', execution_mode: 'existing',
  metadata: { agent_selection: { kind: 'template', template_key: 'assistant.general' }, model: { provider_id: providerId, model: 'configured-model' }, clear_context_each_run: false, created_by: 'user', created_at: 1, updated_at: 1 },
  state: { run_count: 0 },
});
const dailySchedule: ICronSchedule = { kind: 'cron', expr: '0 0 9 * * ?', tz: 'Pacific/Honolulu', description: 'Daily fixture' };

function fixture() {
  const closed = mock(() => {});
  const requests: Array<{ resolve: (job: ICronJob) => void; reject: (error: unknown) => void }> = [];
  const reply = () => new Promise<ICronJob>((resolve, reject) => { requests.push({ resolve, reject }); });
  const adding = spyOn(ipcBridge.cron.addJob, 'invoke').mockImplementation(reply);
  const updating = spyOn(ipcBridge.cron.updateJob, 'invoke').mockImplementation(reply);
  const listing = spyOn(ipcBridge.cron.listJobs, 'invoke').mockResolvedValue([]);
  for (const event of [ipcBridge.cron.onJobCreated, ipcBridge.cron.onJobUpdated, ipcBridge.cron.onJobRemoved, ipcBridge.conversation.reconnected]) {
    const subscription = spyOn(event, 'on').mockImplementation(() => () => {});
    restore.push(() => subscription.mockRestore());
  }
  const identities = spyOn(agents, 'useConversationAgentCatalog').mockReturnValue({
    library: { official_templates: templates, official_presets: {}, user_presets: [], active_bindings: [], fresh_start: { data_generation: 6, legacy_data_imported: false, official_template_count: 2, user_preset_count: 0 } },
    presets: [], draftPresets: [], officialTemplates: templates, isLoading: false, error: undefined, refresh: async () => {},
  });
  const capabilityState = spyOn(capabilities, 'useAgentPresetCapabilities').mockReturnValue({
    capabilityIds: new Set(), actionIds: new Set(), requiredResourceKinds: new Set(),
    defaultModel: null, isLoading: false, error: undefined, refresh: () => {},
    idmm: {} as ReturnType<typeof capabilities.useAgentPresetCapabilities>['idmm'],
  });
  const defaults = spyOn(configService, 'get').mockImplementation(((key: Parameters<typeof configService.get>[0]) => key === 'nomi.defaultModel' ? { provider_id: providerId, model: 'configured-model' } : undefined) as typeof configService.get);
  const providerList = spyOn(providers, 'useProvidersQuery').mockReturnValue({ data: [provider], isLoading: false, error: undefined, mutate: async () => [provider] } as ReturnType<typeof providers.useProvidersQuery>);
  const modelList = spyOn(models, 'useModelsForTask').mockReturnValue({ groups: [{ provider, models: ['configured-model', 'other-model'] }], isLoading: false, refresh: () => {} });
  const conversationList = spyOn(conversations, 'useConversationListSync').mockReturnValue({
    conversations: [], sshConversations: [], isConversationGenerating: () => false,
    hasCompletionUnread: () => false, clearCompletionUnread: () => {}, setActiveConversation: () => {},
  });
  const themeValue = spyOn(theme, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  });
  const success = spyOn(Message, 'success').mockReturnValue(() => {});
  const error = spyOn(Message, 'error').mockReturnValue(() => {});
  const logging = spyOn(console, 'error').mockImplementation(() => {});
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockImplementation(async ({ path }) => ({
    name: path.split(/[\\/]/).pop() || path,
    path,
    size: 0,
    type: 'inode/directory',
    lastModified: 1,
    isDirectory: true,
  }));
  for (const spy of [adding, updating, listing, identities, capabilityState, defaults, providerList, modelList, conversationList, themeValue, success, error, logging, metadata]) {
    restore.push(() => spy.mockRestore());
  }
  const mount = async (job?: ICronJob, initialSpecifiedConversationId?: import('@/common/types/ids').ConversationId) => {
    const element = (visible = true, editJob = job) => h(I18nextProvider, { i18n: locale }, h(MemoryRouter, null, h(CreateTaskDialog, {
      visible, editJob, onClose: closed, initialSpecifiedConversationId,
    })));
    let view!: ReturnType<typeof render>;
    await act(async () => { view = render(element()); });
    const save = () => fireEvent.click(view.getByRole('button', { name: cronStrings.page.save }));
    const name = () => view.getByPlaceholderText(cronStrings.page.form.namePlaceholder) as HTMLInputElement;
    return { ...view, save, name, show: (visible: boolean, nextJob = job) => view.rerender(element(visible, nextJob)) };
  };
  return { mount, closed, requests, adding, updating, success, error, metadata, identities, capabilityState, modelList, conversationList };
}

describe('CreateTaskDialog actual form lifecycle', () => {
  test.each([
    { kind: 'at', at_ms: 2_000_000_000_000, description: 'Once fixture' },
    { kind: 'every', every_ms: 90_000, description: 'Interval fixture' },
    dailySchedule,
  ] as ICronSchedule[])('ordinary edit omits unchanged $kind schedule', async (schedule) => {
    const f = fixture(); const job = cronJob(schedule); const v = await f.mount(job);
    fireEvent.change(v.name(), { target: { value: 'Renamed task' } });
    await act(async () => { v.save(); });
    expect(f.updating).toHaveBeenCalledTimes(1);
    const updates = f.updating.mock.calls[0]![0].updates;
    await act(async () => { f.requests[0]!.resolve(job); });
    expect(updates.schedule).toBeUndefined();
    expect(updates.name).toBe('Renamed task');
  });

  test.each(['0 0 9 * * MON,WED', '0 */5 * * * MON-FRI', '0 0 9 * * * 2030'])(
    'non-preset expression stays a raw editable expression: %s', async (expr) => {
      const f = fixture(); const v = await f.mount(cronJob({ ...dailySchedule, kind: 'cron', expr }));
      expect(v.queryByDisplayValue(expr) !== null).toBe(true);
    }
  );

  test('changing the raw cron expression preserves the saved timezone', async () => {
    const f = fixture(); const job = cronJob({ ...dailySchedule, kind: 'cron', expr: '0 */5 * * * ?' });
    const v = await f.mount(job);
    fireEvent.change(v.getByDisplayValue('0 */5 * * * ?'), { target: { value: '0 */10 * * * ?' } });
    await act(async () => { v.save(); });
    const updates = f.updating.mock.calls[0]![0].updates;
    await act(async () => { f.requests[0]!.resolve(job); });
    expect(updates.schedule).toMatchObject({ expr: '0 */10 * * * ?', tz: 'Pacific/Honolulu' });
  });

  test('live job state updates do not reset an open draft', async () => {
    const f = fixture(); const job = cronJob(dailySchedule); const v = await f.mount(job);
    fireEvent.change(v.name(), { target: { value: 'Unsaved draft' } });
    v.show(true, { ...job, state: { ...job.state, run_count: 1 } });
    expect(v.name().value).toBe('Unsaved draft');
  });

  test('coalesces submits before asynchronous validation and allows retry after failure', async () => {
    const f = fixture(); const v = await f.mount();
    fireEvent.change(v.name(), { target: { value: 'Created task' } });
    fireEvent.change(v.getByPlaceholderText(cronStrings.page.form.promptPlaceholder), { target: { value: 'Fixture prompt' } });
    await act(async () => { v.save(); v.save(); });
    const count = f.requests.length;
    await act(async () => { for (const request of f.requests) request.reject(new Error('fixture offline')); });
    expect(count).toBe(1);
    await act(async () => { v.save(); });
    expect(f.requests).toHaveLength(2);
    await act(async () => { f.requests[1]!.resolve(cronJob(dailySchedule)); });
  });

  test('rejects a removed scheduled-task project before updating the job', async () => {
    const f = fixture();
    const job: ICronJob = {
      ...cronJob(dailySchedule),
      metadata: {
        ...cronJob(dailySchedule).metadata,
        agent_binding: { preset_revision_ref: { preset_id: parseAgentPresetId('0190f5fe-7c00-7a00-8000-000000000071'), revision: 1, revision_digest: asDigestHex('a'.repeat(64)) }, resolved_snapshot_ref: { snapshot_id: asResolvedSnapshotId('0190f5fe-7c00-7a00-8000-000000000072'), snapshot_digest: asDigestHex('b'.repeat(64)) }, binding_version: 1, typed_resource_bindings: [{ binding_id: 'workspace', resource_id: 'project', resource_kind: 'workspace', owner_id: 'owner', operations: ['read'], typed_parameters: { workspace_root: '/projects/removed' } }] },
      },
    };
    const v = await f.mount(job);
    f.metadata.mockRejectedValueOnce(new Error('directory not found'));

    await act(async () => { v.save(); });

    expect(f.updating).not.toHaveBeenCalled();
    expect(f.error).toHaveBeenCalledTimes(1);
    expect(String(f.error.mock.calls[0]?.[0])).toContain('removed');
  });

  test.each([false, true])('old save after closing/reopening cannot close or notify the new dialog (failure=%s)', async (failure) => {
    const f = fixture(); const job = cronJob(dailySchedule); const v = await f.mount(job);
    await act(async () => { v.save(); });
    v.show(false); v.show(true);
    fireEvent.change(v.name(), { target: { value: 'New draft' } });
    await act(async () => {
      if (failure) f.requests[0]!.reject(new Error('fixture offline'));
      else f.requests[0]!.resolve(job);
    });
    expect(f.closed).not.toHaveBeenCalled();
    expect(f.success).not.toHaveBeenCalled();
    expect(f.error).not.toHaveBeenCalled();
    expect(v.name().value).toBe('New draft');
  });
});


describe('workbench Agent task creation', () => {
  test('defaults to General and submits an unsaved official choice with the configured model', async () => {
    const f = fixture(); const v = await f.mount();
    expect(v.getByTestId('agent-selector').textContent).toContain(agentSettings.template.assistant.general.name);
    expect(v.getByTestId('chat-model-selector').textContent).toContain('configured-model');
    fireEvent.change(v.name(), { target: { value: 'Weather task' } });
    fireEvent.change(v.getByPlaceholderText(cronStrings.page.form.promptPlaceholder), { target: { value: 'Check Beijing weather' } });
    await act(async () => { v.save(); });
    expect(f.adding).toHaveBeenCalledTimes(1);
    expect(f.adding.mock.calls[0]?.[0]).toMatchObject({
      agent_selection: { kind: 'template', template_key: 'assistant.general' },
      model: { provider_id: providerId, model: 'configured-model' }, message: 'Check Beijing weather',
    });
    expect(f.adding.mock.calls[0]?.[0]).not.toHaveProperty('agent_config');
    expect(f.adding.mock.calls[0]?.[0]).not.toHaveProperty('agent_type');
    await act(async () => { f.requests[0]!.resolve(cronJob(dailySchedule)); });
  });

  test('the real shared Agent menu changes identity without hiding or clearing the independently chosen model', async () => {
    const f = fixture(); const v = await f.mount();
    const writeDefaults = spyOn(configService, 'set'); restore.push(() => writeDefaults.mockRestore());
    fireEvent.click(v.getByTestId('chat-model-selector'));
    const other = await waitFor(() => within(document.body).getByTestId('nomi-model-option-other-model'));
    await act(async () => { fireEvent.click(other); });
    expect(v.getByTestId('chat-model-selector').textContent).toContain('other-model');
    await act(async () => { fireEvent.click(v.getByTestId('agent-selector')); });
    const menu = within(document.body).getByRole('dialog', { name: guid.agentEntries.choose });
    await act(async () => { fireEvent.click(within(menu).getByRole('button', { name: agentSettings.template.chat.minimal.name })); });
    expect(v.getByTestId('agent-selector').textContent).toContain(agentSettings.template.chat.minimal.name);
    expect(v.getByTestId('chat-model-selector').textContent).toContain('other-model');
    fireEvent.change(v.name(), { target: { value: 'Changed Agent' } });
    fireEvent.change(v.getByPlaceholderText(cronStrings.page.form.promptPlaceholder), { target: { value: 'Run task' } });
    await act(async () => { v.save(); });
    expect(f.adding.mock.calls[0]?.[0]).toMatchObject({ agent_selection: { kind: 'template', template_key: 'chat.minimal' }, model: { provider_id: providerId, model: 'other-model' } });
    expect(writeDefaults).not.toHaveBeenCalled();
    await act(async () => { f.requests[0]!.resolve(cronJob(dailySchedule)); });
  });

  test('ordinary edits preserve an unavailable configured model instead of silently substituting another', async () => {
    const f = fixture(); const job = cronJob(dailySchedule);
    job.metadata.model = { provider_id: providerId, model: 'removed-model' };
    const v = await f.mount(job);
    expect(v.getByTestId('chat-model-selector').textContent).toContain('removed-model');
    expect(v.getByRole('alert').textContent).toContain(cronStrings.page.form.modelUnavailable);
    fireEvent.change(v.name(), { target: { value: 'Only renamed' } });
    await act(async () => { v.save(); });
    expect(f.updating.mock.calls[0]?.[0].updates).not.toHaveProperty('model');
    expect(f.updating.mock.calls[0]?.[0].updates).not.toHaveProperty('agent_selection');
    await act(async () => { f.requests[0]!.resolve(job); });
  });

  test('changing only the model does not refresh the frozen Agent or replace its resources', async () => {
    const f = fixture(); const job = cronJob(dailySchedule); const v = await f.mount(job);
    fireEvent.click(v.getByTestId('chat-model-selector'));
    const other = await waitFor(() => within(document.body).getByTestId('nomi-model-option-other-model'));
    await act(async () => { fireEvent.click(other); });
    await act(async () => { v.save(); });
    const updates = f.updating.mock.calls[0]?.[0].updates;
    expect(updates).toMatchObject({ model: { provider_id: providerId, model: 'other-model' } });
    for (const key of ['agent_selection', 'workspace', 'resource_selections']) expect(updates).not.toHaveProperty(key);
    await act(async () => { f.requests[0]!.resolve(job); });
  });

  test('explicitly reselecting the current Agent requests its current saved revision', async () => {
    const f = fixture(); const job = cronJob(dailySchedule); const v = await f.mount(job);
    await act(async () => { fireEvent.click(v.getByTestId('agent-selector')); });
    const menu = within(document.body).getByRole('dialog', { name: guid.agentEntries.choose });
    await act(async () => { fireEvent.click(within(menu).getByRole('button', { name: agentSettings.template.assistant.general.name })); });
    await act(async () => { v.save(); });
    expect(f.updating.mock.calls[0]?.[0].updates).toMatchObject({ agent_selection: { kind: 'template', template_key: 'assistant.general' } });
    expect(f.updating.mock.calls[0]?.[0].updates).not.toHaveProperty('model');
    await act(async () => { f.requests[0]!.resolve(job); });
  });

  test.each([undefined, { kind: 'preset' as const, preset_id: parseAgentPresetId('0190f5fe-7c00-7a00-8000-000000000079') }])(
    'migrated frozen tasks edit independently of missing author selections: %j', async (selection) => {
      const f = fixture(); const job = cronJob(dailySchedule);
      job.metadata.agent_selection = selection;
      job.metadata.agent_label = 'Frozen research agent';
      f.capabilityState.mockReturnValue({ ...f.capabilityState(undefined), error: new Error('authoring preset unavailable') });
      const v = await f.mount(job);
      expect(v.getByTestId('agent-selector').textContent).toContain('Frozen research agent');
      fireEvent.change(v.name(), { target: { value: 'Renamed frozen task' } });
      await act(async () => { v.save(); });
      const rename = f.updating.mock.calls[0]?.[0].updates;
      for (const key of ['agent_selection', 'model', 'workspace', 'resource_selections']) expect(rename).not.toHaveProperty(key);
      await act(async () => { f.requests[0]!.reject(new Error('keep the edit form open')); });
      fireEvent.click(v.getByTestId('chat-model-selector'));
      const other = await waitFor(() => within(document.body).getByTestId('nomi-model-option-other-model'));
      await act(async () => { fireEvent.click(other); });
      await act(async () => { v.save(); });
      const changed = f.updating.mock.calls[1]?.[0].updates;
      expect(changed).toMatchObject({ model: { provider_id: providerId, model: 'other-model' } });
      for (const key of ['agent_selection', 'workspace', 'resource_selections']) expect(changed).not.toHaveProperty(key);
      await act(async () => { f.requests[1]!.resolve(job); });
    }
  );

  test('rejects an incompatible task model before creating the task', async () => {
    const f = fixture();
    f.capabilityState.mockReturnValue({ ...f.capabilityState(undefined), actionIds: new Set(['tools/search']) });
    const unsupportedProvider = { ...provider, models: provider.models.map(model => ({ ...model, capabilities: model.capabilities.map(capability => ({ ...capability, health: { status: 'unknown' as const, unsupported_technical_capabilities: ['function_calling' as const] } })) })) };
    f.modelList.mockReturnValue({ groups: [{ provider: unsupportedProvider, models: ['configured-model'] }], isLoading: false, refresh: () => {} });
    const v = await f.mount();
    fireEvent.change(v.name(), { target: { value: 'Incompatible task' } });
    fireEvent.change(v.getByPlaceholderText(cronStrings.page.form.promptPlaceholder), { target: { value: 'Use a tool' } });
    await act(async () => { v.save(); });
    expect(f.adding).not.toHaveBeenCalled();
    expect(f.error).toHaveBeenCalledWith(cronStrings.page.form.modelIncompatible);
  });
});

test('bound continuing tasks retain Session-owned identity/model and omit execution fields on ordinary edits', async () => {
  const f = fixture(); const job = cronJob(dailySchedule);
  job.metadata.conversation_id = parseConversationId('0190f5fe-7c00-7a00-8000-000000000076');
  job.metadata.agent_binding = { preset_revision_ref: { preset_id: parseAgentPresetId('0190f5fe-7c00-7a00-8000-000000000071'), revision: 1, revision_digest: asDigestHex('a'.repeat(64)) }, resolved_snapshot_ref: { snapshot_id: asResolvedSnapshotId('0190f5fe-7c00-7a00-8000-000000000072'), snapshot_digest: asDigestHex('b'.repeat(64)) }, binding_version: 1, typed_resource_bindings: [{ binding_id: 'workspace', resource_id: 'project', resource_kind: 'workspace', owner_id: 'owner', operations: ['read'], typed_parameters: { workspace_root: '/projects/current' } }] };
  const v = await f.mount(job);
  expect(v.getByText('/projects/current')).toBeDefined();
  f.metadata.mockRejectedValue(new Error('directory is currently offline'));
  expect(v.getByTestId('agent-selector').hasAttribute('disabled')).toBe(true);
  expect(v.getByTestId('chat-model-selector').getAttribute('data-readonly')).toBe('true');
  fireEvent.change(v.name(), { target: { value: 'Rename bound task' } });
  await act(async () => { v.save(); });
  const updates = f.updating.mock.calls[0]?.[0].updates;
  for (const key of ['agent_selection', 'model', 'workspace', 'resource_selections']) expect(updates).not.toHaveProperty(key);
  expect(f.metadata).not.toHaveBeenCalled();
  await act(async () => { f.requests[0]!.resolve(job); });
});

test('specified conversation creation inherits its Agent/model without sending another execution configuration', async () => {
  const f = fixture(); const id = parseConversationId('0190f5fe-7c00-7a00-8000-000000000077');
  const v = await f.mount(undefined, id);
  expect(v.queryByTestId('agent-selector')).toBeNull();
  expect(v.queryByTestId('chat-model-selector')).toBeNull();
  fireEvent.change(v.name(), { target: { value: 'Conversation reminder' } });
  fireEvent.change(v.getByPlaceholderText(cronStrings.page.form.promptPlaceholder), { target: { value: 'Continue this work' } });
  await act(async () => { v.save(); });
  expect(f.adding.mock.calls[0]?.[0]).toMatchObject({ conversation_id: id, execution_mode: 'existing' });
  for (const key of ['agent_selection', 'model', 'workspace', 'resource_selections']) expect(f.adding.mock.calls[0]?.[0]).not.toHaveProperty(key);
  await act(async () => { f.requests[0]!.resolve(cronJob(dailySchedule)); });
});
