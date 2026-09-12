/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { afterEach, beforeAll, describe, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { createElement as h } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { ICronJob, ICronSchedule } from '@/common/adapter/ipcBridge';
import { parseConversationId, parseCronJobId } from '@/common/types/ids';
import * as agents from '@/renderer/pages/conversation/hooks/useConversationAgents';
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
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { cron: cronStrings, common } } } });
});
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach((dispose) => dispose()); });

const cronJob = (schedule: ICronSchedule): ICronJob => ({
  cron_job_id: parseCronJobId('0190f5fe-7c00-7a00-8000-000000000074'),
  name: 'Fixture task', enabled: true, schedule, message: 'Fixture prompt', execution_mode: 'existing',
  metadata: { agent_type: 'claude', created_by: 'user', created_at: 1, updated_at: 1 },
  state: { run_count: 0, retry_count: 0, max_retries: 0 },
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
  const identities = spyOn(agents, 'useConversationAgents').mockReturnValue({
    cliAgents: [], agentPresets: [], isLoading: false, refresh: async () => {},
  });
  const modelList = spyOn(models, 'useModelsForTask').mockReturnValue({ groups: [], isLoading: false, refresh: () => {} });
  const conversationList = spyOn(conversations, 'useConversationListSync').mockReturnValue({
    conversations: [], sshConversations: [], robotConversations: [], isConversationGenerating: () => false,
    hasCompletionUnread: () => false, clearCompletionUnread: () => {}, setActiveConversation: () => {},
  });
  const themeValue = spyOn(theme, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  });
  const success = spyOn(Message, 'success').mockReturnValue(() => {});
  const error = spyOn(Message, 'error').mockReturnValue(() => {});
  const logging = spyOn(console, 'error').mockImplementation(() => {});
  for (const spy of [adding, updating, listing, identities, modelList, conversationList, themeValue, success, error, logging]) {
    restore.push(() => spy.mockRestore());
  }
  const mount = async (job?: ICronJob) => {
    const element = (visible = true, editJob = job) => h(I18nextProvider, { i18n: locale }, h(CreateTaskDialog, {
      visible, editJob, onClose: closed,
      initialSpecifiedConversationId: parseConversationId('0190f5fe-7c00-7a00-8000-000000000075'),
    }));
    let view!: ReturnType<typeof render>;
    await act(async () => { view = render(element()); });
    const save = () => fireEvent.click(view.getByRole('button', { name: cronStrings.page.save }));
    const name = () => view.getByPlaceholderText(cronStrings.page.form.namePlaceholder) as HTMLInputElement;
    return { ...view, save, name, show: (visible: boolean, nextJob = job) => view.rerender(element(visible, nextJob)) };
  };
  return { mount, closed, requests, adding, updating, success, error };
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

describe('CreateTaskDialog preset identity presentation', () => {
  test('uses the safe identity component instead of rendering avatar values as text', () => {
    expect(source.includes('<CronPresetOptionIdentity')).toBe(true);
    expect(source.includes("!preset.avatar.endsWith('.svg')")).toBe(false);
    expect(source.includes('<span>{preset.avatar}</span>')).toBe(false);
  });

  test('prevents presets that the backend cannot resolve for scheduled tasks from being selected', () => {
    expect(source.includes('const supportsCron = Boolean(preset.current_stable_revision)')).toBe(true);
    expect(source.includes('disabled={!supportsCron}')).toBe(true);
    expect(source.includes('if (!supportsCron)')).toBe(true);
    expect(source.includes("aria-disabled={!supportsCron || undefined}")).toBe(true);
  });
});

describe('CreateTaskDialog Agent identity contract', () => {
  test('keys direct Agent options by stable AgentRegistry ID and persists that identity', () => {
    expect(source.includes('getCronAgentOptionValue(agent.agent_id)')).toBe(true);
    expect(source.includes('custom_agent_id: agent.agent_id')).toBe(true);
    expect(source.includes('value={`cli:${agentKey}`}')).toBe(false);
  });

  test('locks unsupported cross-runtime changes and preserves unchanged frozen preset snapshots', () => {
    expect(source.includes('disabled={isEditMode}')).toBe(true);
    expect(source.includes('hasCronAgentConfigurationChanged(editJob!, cliAgents')).toBe(true);
    expect(source.includes('...(agentConfigChanged ? { agent_config } : {})')).toBe(true);
  });
});
