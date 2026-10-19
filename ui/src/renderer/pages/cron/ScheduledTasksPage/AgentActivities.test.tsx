import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { MemoryRouter } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { agentActivities } from '@/common/adapter/ipcBridge';
import type { AgentActivity, AgentActivityCommandReceipt } from '@/common/types/agentExecution/agentActivity';
import cron from '@/renderer/services/i18n/locales/en-US/cron.json';
import AgentActivities from './AgentActivities';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { cron } } } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });
const target = { kind: 'session_turn' as const, agent_session_id: 'original-session', operation_id: 'original-operation' };
const item: AgentActivity = { key: 'original', title: 'Original task', kind: 'conversation', state: 'paused', agent_session_id: null,
  target, continuation: 'resume_queued', waiting_reason: null, stop_accepted: false, execution_stopped: false,
  resources: [{ kind: 'browser', state: 'unconfirmed', detail: null }], can_stop: true, can_retry_cleanup: false, updated_at_ms: null };
const receipt: AgentActivityCommandReceipt = { accepted: true, execution_stopped: false, resources: [] };
function view() { return render(<MemoryRouter><I18nextProvider i18n={i18n}><AgentActivities /></I18nextProvider></MemoryRouter>); }
function discover(rows = [item]) {
  const spy = spyOn(agentActivities.list, 'invoke').mockResolvedValue({ items: rows, next_cursor: null });
  restores.push(() => spy.mockRestore()); return spy;
}

test('saved continuation stays visible and an accepted Stop does not claim native release', async () => {
  discover();
  const stop = spyOn(agentActivities.stop, 'invoke').mockResolvedValue(receipt); restores.push(() => stop.mockRestore());
  const screen = view();
  await screen.findByText('Resume request saved, waiting for execution capacity');
  fireEvent.click(screen.getByRole('button', { name: 'Stop this task' }));
  await screen.findByText('Stop accepted');
  expect(screen.queryByText('Execution stopped')).toBeNull();
  expect(screen.queryByText('Release confirmed')).toBeNull();
  expect(screen.getByText('Resource release is not fully confirmed')).toBeTruthy();
  expect(stop.mock.calls[0][0]?.target).toEqual(target);
});

test('all-stop signals a hidden domain target while another exact owner remains blocked', async () => {
  discover();
  const hidden = { kind: 'cron_run' as const, cron_job_id: 'original-job', cron_job_run_id: 'original-run' };
  const snapshot = spyOn(agentActivities.stopTargets, 'invoke').mockResolvedValue({ captured_at_ms: 123, targets: [target, hidden] });
  restores.push(() => snapshot.mockRestore());
  let release!: (value: AgentActivityCommandReceipt) => void;
  const stop = spyOn(agentActivities.stop, 'invoke').mockImplementation((request) => request.target.kind === 'session_turn'
    ? new Promise((resolve) => { release = resolve; }) : Promise.resolve(receipt));
  restores.push(() => stop.mockRestore());
  const screen = view(); await screen.findByText('Original task');
  fireEvent.click(screen.getByRole('button', { name: 'Stop all' }));
  await waitFor(() => expect(stop).toHaveBeenCalledTimes(2));
  expect(stop.mock.calls.map(([request]) => request.target)).toEqual([target, hidden]);
  expect(screen.getByText(/Accepted 1 · Pending 1 · Unconfirmed 0/)).toBeTruthy();
  release(receipt);
  await waitFor(() => expect(screen.getByText(/Accepted 2 · Pending 0 · Unconfirmed 0/)).toBeTruthy());
});

test('an unconfirmed exact Stop retries the original key after discovery changes', async () => {
  const listing = discover();
  const notice = spyOn(Message, 'error').mockImplementation(() => () => {}); restores.push(() => notice.mockRestore());
  const snapshot = spyOn(agentActivities.stopTargets, 'invoke').mockResolvedValue({ captured_at_ms: 123, targets: [target] });
  restores.push(() => snapshot.mockRestore());
  const stop = spyOn(agentActivities.stop, 'invoke').mockRejectedValueOnce(new Error('original owner unconfirmed')).mockResolvedValue(receipt);
  restores.push(() => stop.mockRestore());
  const screen = view(); await screen.findByText('Original task');
  fireEvent.click(screen.getByRole('button', { name: 'Stop all' }));
  await screen.findByRole('button', { name: 'Retry original Stop' });
  listing.mockResolvedValue({ items: [{ ...item, target: { ...target, operation_id: 'successor' } }], next_cursor: null });
  fireEvent.click(screen.getByRole('button', { name: 'Retry original Stop' }));
  await waitFor(() => expect(stop).toHaveBeenCalledTimes(2));
  expect(stop.mock.calls[1][0]).toEqual(stop.mock.calls[0][0]);
  expect(snapshot).toHaveBeenCalledTimes(1);
});

test('delegated-only control sends the explicit scope without marking the running parent stopped', async () => {
  const parent = { ...item, kind: 'delegated_task' as const, state: 'running' as const, continuation: 'none' as const,
    target: { kind: 'execution_task' as const, execution_id: 'original-execution', step_id: 'original-step', attempt_id: 'original-attempt', expected_execution_version: 9, expected_step_version: 4 } };
  discover([parent]);
  const notice = spyOn(Message, 'success').mockImplementation(() => () => {}); restores.push(() => notice.mockRestore());
  const stop = spyOn(agentActivities.stop, 'invoke').mockResolvedValue(receipt); restores.push(() => stop.mockRestore());
  const screen = view(); await screen.findByText('Original task');
  fireEvent.click(screen.getByRole('button', { name: 'Other stop scopes' }));
  fireEvent.click(await screen.findByText('Stop delegated work only'));
  await waitFor(() => expect(stop).toHaveBeenCalledTimes(1));
  expect(stop.mock.calls[0][0]?.scope).toBe('delegated_work');
  expect(stop.mock.calls[0][0]?.target).toEqual(parent.target);
  await waitFor(() => expect(notice).toHaveBeenCalledTimes(1));
  expect(screen.queryByText('Stop accepted')).toBeNull();
  expect(screen.queryByText('Execution stopped')).toBeNull();
  expect(screen.getByText('Running')).toBeTruthy();
});

test('each retained process, SSH link and browser workspace sends its original resource owner identity', async () => {
  const process = { kind: 'process_instance' as const, agent_session_id: 'session', owner_generation: 'original-generation', process_id: 'original-process', creator_turn_id: 'creator-turn', creator_call_id: 'creator-call' };
  const ssh = { kind: 'ssh_link' as const, agent_session_id: 'session', ssh_host_id: 'host', pool_instance_id: 'original-pool', instance_id: 17 };
  const browser = { kind: 'browser_workspace' as const, agent_session_id: 'session', service_instance_id: 'original-browser-service', runtime_generation: 1, expected_head_seq: 12 };
  discover([{ ...item, resources: [{ kind: 'processes', state: 'unconfirmed', detail: null, display_name: 'Shell process', target: process },
    { kind: 'ssh', state: 'unconfirmed', detail: null, display_name: 'Build host', target: ssh },
    { kind: 'browser', state: 'held_for_user', detail: null, target: browser }] }]);
  const stop = spyOn(agentActivities.stop, 'invoke').mockResolvedValue(receipt); restores.push(() => stop.mockRestore());
  const screen = view(); await screen.findByText('Original task');
  const resources = screen.getAllByRole('button', { name: 'End this resource' });
  fireEvent.click(resources[0]); fireEvent.click(resources[1]); fireEvent.click(resources[2]);
  await waitFor(() => expect(stop).toHaveBeenCalledTimes(3));
  expect(stop.mock.calls.map(([request]) => request.target)).toEqual([process, ssh, browser]);
  expect(screen.queryByText('Release confirmed')).toBeNull();
});

test('a plan without an Attempt exposes an explicit whole-collaboration control', async () => {
  const group = { kind: 'execution_group' as const, execution_id: 'planning-execution', expected_version: 12 };
  discover([{ ...item, kind: 'collaboration', state: 'planning', continuation: 'none', target: group }]);
  const stop = spyOn(agentActivities.stop, 'invoke').mockResolvedValue(receipt); restores.push(() => stop.mockRestore());
  const screen = view(); await screen.findByText('Planning');
  expect(screen.queryByRole('button', { name: 'Stop this task' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Stop whole collaboration' }));
  await waitFor(() => expect(stop).toHaveBeenCalledTimes(1));
  expect(stop.mock.calls[0][0]?.target).toEqual(group);
  expect(stop.mock.calls[0][0]?.scope).toBeUndefined();
});

test('an AutoWork root keeps its run-owner Stop separate from delegated work', async () => {
  const rootTarget = { kind: 'auto_work_run' as const, target_kind: 'conversation' as const,
    target_id: 'original-lead', requirement_id: 'original-requirement', claim_generation: 7 };
  const delegatedTarget = { kind: 'execution_task' as const, execution_id: 'original-execution', step_id: 'root-step',
    attempt_id: 'root-attempt', expected_execution_version: 9, expected_step_version: 4 };
  discover([{ ...item, kind: 'auto_work_run', state: 'running', continuation: 'none', target: rootTarget,
    delegation_target: delegatedTarget }]);
  const notice = spyOn(Message, 'success').mockImplementation(() => () => {}); restores.push(() => notice.mockRestore());
  const stop = spyOn(agentActivities.stop, 'invoke').mockResolvedValue(receipt); restores.push(() => stop.mockRestore());
  const screen = view(); await screen.findByText('Original task');
  expect(screen.queryByRole('button', { name: 'Stop this task' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Other stop scopes' })).toBeNull();
  expect(screen.queryByText('Stop task and its delegated work')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Stop delegated work only' }));
  await waitFor(() => expect(notice).toHaveBeenCalledTimes(1));
  expect(stop.mock.calls[0][0]?.target).toEqual(delegatedTarget);
  expect(stop.mock.calls[0][0]?.scope).toBe('delegated_work');
  expect(screen.queryByText('Stop accepted')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Stop current automatic run' }));
  await waitFor(() => expect(screen.getByText('Stop accepted')).toBeTruthy());
  expect(stop.mock.calls[1][0]?.target).toEqual(rootTarget);
  expect(stop.mock.calls[1][0]?.scope).toBeUndefined();
});

test('an AutoWork child retains task and combined scopes on its exact Execution target', async () => {
  const childTarget = { kind: 'execution_task' as const, execution_id: 'original-execution', step_id: 'child-step',
    attempt_id: 'child-attempt', expected_execution_version: 9, expected_step_version: 4 };
  discover([{ ...item, kind: 'auto_work_run', state: 'running', continuation: 'none', target: childTarget }]);
  const stop = spyOn(agentActivities.stop, 'invoke').mockResolvedValue(receipt); restores.push(() => stop.mockRestore());
  const screen = view(); await screen.findByText('Original task');
  expect(screen.queryByRole('button', { name: 'Stop current automatic run' })).toBeNull();
  expect(screen.getByRole('button', { name: 'Stop this task' })).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Other stop scopes' }));
  fireEvent.click(await screen.findByText('Stop task and its delegated work'));
  await waitFor(() => expect(stop).toHaveBeenCalledTimes(1));
  expect(stop.mock.calls[0][0]?.target).toEqual(childTarget);
  expect(stop.mock.calls[0][0]?.scope).toBe('task_and_delegated');
});
