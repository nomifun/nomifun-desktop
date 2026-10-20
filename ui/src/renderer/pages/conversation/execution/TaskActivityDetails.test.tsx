import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { agentActivities, agentExecution, conversation } from '@/common/adapter/ipcBridge';
import { parseExecutionAttemptId, parseExecutionId, parseExecutionStepId } from '@/common/types/ids';
import type { AgentActivity } from '@/common/types/agentExecution/agentActivity';
import type { TAgentExecutionChangedEvent } from '@/common/types/agentExecution/agentExecutionEvents';
import agentExecutionMessages from '@/renderer/services/i18n/locales/en-US/agentExecution.json';
import cron from '@/renderer/services/i18n/locales/en-US/cron.json';
import TaskActivityDetails from './TaskActivityDetails';

const instance = createInstance();
await instance.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentExecution: agentExecutionMessages, cron } } }, interpolation: { escapeValue: false } });
const executionId = parseExecutionId('0190f5fe-7c00-7a00-8000-000000000801');
const stepId = parseExecutionStepId('0190f5fe-7c00-7a00-8000-000000000802');
const attemptId = parseExecutionAttemptId('0190f5fe-7c00-7a00-8000-000000000803');
const successor = parseExecutionAttemptId('0190f5fe-7c00-7a00-8000-000000000804');
const target = { kind: 'execution_task', execution_id: executionId, step_id: stepId, attempt_id: attemptId, expected_execution_version: 2, expected_step_version: 3 } as const;
const activity = (overrides: Partial<AgentActivity> = {}): AgentActivity => ({ key: `execution:${attemptId}`, title: 'Original task', kind: 'delegated_task', state: 'stopping',
  agent_session_id: null, target, continuation: 'none', waiting_reason: null, stop_accepted: true, execution_stopped: false,
  resources: [{ kind: 'runtime', state: 'releasing', detail: null }, { kind: 'browser', state: 'unconfirmed', detail: null }],
  can_stop: false, can_retry_cleanup: true, updated_at_ms: null, ...overrides });
afterEach(() => { cleanup(); mock.restore(); });
const view = (selected = attemptId) => <I18nextProvider i18n={instance}><TaskActivityDetails executionId={executionId} stepId={stepId} attemptId={selected} /></I18nextProvider>;

function events() {
  let changed: (event: TAgentExecutionChangedEvent) => void = () => {};
  let reconnect: () => void = () => {};
  let active = true;
  spyOn(agentExecution.events.changed, 'on').mockImplementation(listener => { changed = listener; return () => { active = false; }; });
  spyOn(conversation.reconnected, 'on').mockImplementation(listener => { reconnect = listener; return () => {}; });
  return { changed: async () => { if (active) await act(async () => { changed({ execution_id: executionId } as TAgentExecutionChangedEvent); }); },
    reconnect: async () => { await act(async () => reconnect()); } };
}

test('exact task activity keeps Stop acceptance, execution stop and resource release separate', async () => {
  const event = events();let row = activity();
  const read = spyOn(agentActivities.list, 'invoke').mockImplementation(async () => ({ items: [row], next_cursor: null }));
  const stop = spyOn(agentActivities.stop, 'invoke').mockResolvedValue({ accepted: true, execution_stopped: false, resources: [] });
  const page = render(view());
  await waitFor(() => expect(page.container.textContent).toContain('Stop accepted'));
  expect(page.container.textContent).toContain('Execution stop is not yet confirmed');
  expect(page.container.textContent).toContain('Browser · Unconfirmed');
  expect(stop).not.toHaveBeenCalled();
  expect(read.mock.calls[0][0]).toEqual({ execution_id: executionId, step_id: stepId, attempt_id: attemptId });
  row = activity({ execution_stopped: true });await event.changed();
  await waitFor(() => expect(page.container.textContent).toContain('Execution stopped'));
  expect(page.container.textContent).toContain('Resource release is not fully confirmed');
  expect(page.container.textContent).toContain('Releasing');
  row = activity({ execution_stopped: true, can_retry_cleanup: false, resources: [{ kind: 'runtime', state: 'released', detail: null }, { kind: 'browser', state: 'released', detail: null }] });
  await event.reconnect();
  await waitFor(() => expect(page.container.textContent).not.toContain('Releasing'));
  expect(page.getAllByText(/Release confirmed/)).toHaveLength(2);
  expect(page.queryByRole('button', { name: 'Retry original cleanup' })).toBeNull();
  page.unmount();const count = read.mock.calls.length;await event.changed();expect(read.mock.calls.length).toBe(count);
});

test('lost cleanup acknowledgement retries the original Attempt target and request key', async () => {
  const event = events();let row = activity();
  spyOn(agentActivities.list, 'invoke').mockImplementation(async () => ({ items: [row], next_cursor: null }));
  const stop = spyOn(agentActivities.stop, 'invoke').mockRejectedValueOnce(new Error('response lost')).mockResolvedValue({ accepted: true, execution_stopped: false, resources: [] });
  const page = render(view());
  fireEvent.click(await page.findByRole('button', { name: 'Retry original cleanup' }));
  await waitFor(() => expect(page.container.textContent).toContain('Cleanup was not confirmed'));
  row = activity({ target: { ...target, expected_execution_version: 10, expected_step_version: 11 } });
  await event.changed();
  fireEvent.click(page.getByRole('button', { name: 'Retry original cleanup' }));
  await waitFor(() => expect(stop.mock.calls).toHaveLength(2));
  expect(stop.mock.calls[0][0]).toEqual(stop.mock.calls[1][0]);
  expect(stop.mock.calls[1][0].target).toEqual(target);
  expect(stop.mock.calls[1][0].retry_cleanup).toBe(true);
  expect(stop.mock.calls[1][0].scope).toBeUndefined();
});

test('an exact inspector refuses a successor or a globally paged result without issuing cleanup', async () => {
  events();
  const read = spyOn(agentActivities.list, 'invoke').mockResolvedValue({ items: [activity({ target: { ...target, attempt_id: successor } })], next_cursor: null });
  const stop = spyOn(agentActivities.stop, 'invoke').mockResolvedValue({ accepted: true, execution_stopped: false, resources: [] });
  const page = render(view());
  await waitFor(() => expect(page.container.textContent).toContain("This task's activity could not be refreshed"));
  expect(page.queryByRole('button', { name: 'Retry original cleanup' })).toBeNull();
  read.mockResolvedValue({ items: [activity()], next_cursor: 'global-page' });
  fireEvent.click(page.getByRole('button', { name: 'Refresh' }));
  await waitFor(() => expect(read.mock.calls).toHaveLength(2));
  expect(stop).not.toHaveBeenCalled();
  expect(read.mock.calls.every(([query]) => query.attempt_id === attemptId && !query.after_cursor)).toBe(true);
});

test('a late historical observation cannot overwrite a newly selected Attempt', async () => {
  events();
  let finishOriginal!: (value: { items: AgentActivity[]; next_cursor: null }) => void;
  const original = new Promise<{ items: AgentActivity[]; next_cursor: null }>(resolve => { finishOriginal = resolve; });
  spyOn(agentActivities.list, 'invoke').mockImplementation(request => request.attempt_id === attemptId ? original : Promise.resolve({
    items: [activity({ target: { ...target, attempt_id: successor }, resources: [{ kind: 'runtime', state: 'active', display_name: 'Successor resource', detail: null }] })], next_cursor: null,
  }));
  const page = render(view());
  page.rerender(view(successor));
  await waitFor(() => expect(page.container.textContent).toContain('Successor resource'));
  await act(async () => finishOriginal({ items: [activity({ resources: [{ kind: 'runtime', state: 'released', display_name: 'Original resource', detail: null }] })], next_cursor: null }));
  expect(page.container.textContent).toContain('Successor resource');expect(page.container.textContent).not.toContain('Original resource');
});
