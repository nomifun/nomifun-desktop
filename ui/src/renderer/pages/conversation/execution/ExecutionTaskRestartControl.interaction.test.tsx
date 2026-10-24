import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { TAgentExecutionEvent, TAgentExecutionMessageReceipt } from '@/common/types/agentExecution/agentExecutionTypes';
import { parseExecutionAttemptId } from '@/common/types/ids';
import agentExecution from '@/renderer/services/i18n/locales/en-US/agentExecution.json';
import { executionId, makeAttempt, makeDetail, makeStep } from '../../../../../test/fixtures/conversationDelegation';
import ExecutionTaskRestartControl from './ExecutionTaskRestartControl';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentExecution } } } });
const restores: Array<() => void> = [];
let notify: (event: { execution_id: typeof executionId; sequence: number; change_kind: 'step_changed' }) => void;
beforeEach(() => {
  const changed = spyOn(ipcBridge.agentExecution.events.changed, 'on').mockImplementation(handler => {
    notify = handler; return () => {};
  });
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  const events = spyOn(ipcBridge.agentExecution.listEvents, 'invoke').mockResolvedValue([]);
  for (const spy of [changed, reconnect, events]) restores.push(() => spy.mockRestore());
});
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });

function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((accept, decline) => { resolve = accept; reject = decline; });
  return { promise, resolve, reject };
}
const step = makeStep(0), attempt = makeAttempt(step);
const receipt: TAgentExecutionMessageReceipt = { execution_id: executionId, step_ids: [step.step_id], sequence: 42,
  operation_id: 'restart-step:original-command', replayed: false };
const resolvedEvent = (disposition = 'continued_original', operation = receipt.operation_id): TAgentExecutionEvent => ({
  execution_id: executionId, step_id: step.step_id, attempt_id: attempt.attempt_id, sequence: 43,
  event_type: 'step_changed', actor_type: 'system', actor_id: null, actor_conversation_id: null,
  actor_attempt_id: null, on_behalf_of_user_id: 'test-user' as TAgentExecutionEvent['on_behalf_of_user_id'],
  created_at: 3000, payload: { change: 'restart_resolved', operation_id: operation, disposition, reason: 'The original pause needs user action.' },
});
function scene(props: Partial<Parameters<typeof ExecutionTaskRestartControl>[0]> = {}) {
  return <I18nextProvider i18n={i18n}><ExecutionTaskRestartControl executionId={executionId} stepId={step.step_id}
    attemptId={attempt.attempt_id} executionVersion={31} stepVersion={7} refetch={async () => {}} {...props} /></I18nextProvider>;
}

test('rapid restart clicks send one exact command and acceptance does not claim task completion', async () => {
  const pending = deferred<TAgentExecutionMessageReceipt>();
  const restart = spyOn(ipcBridge.agentExecution.restart, 'invoke').mockReturnValue(pending.promise);
  restores.push(() => restart.mockRestore());
  const refresh = mock(async () => {}), view = render(scene({ refetch: refresh }));
  const button = await view.findByRole('button', { name: 'Restart task' });
  fireEvent.click(button); fireEvent.click(button);
  expect(restart).toHaveBeenCalledTimes(1);
  expect(restart.mock.calls[0][0]).toMatchObject({ execution_id: executionId, step_id: step.step_id,
    updates: { expected_execution_version: 31, expected_step_version: 7, expected_attempt_id: attempt.attempt_id } });
  expect(restart.mock.calls[0][0].updates.idempotency_key.length).toBeGreaterThan(0);
  expect(view.queryByRole('status')).toBeNull();
  await act(async () => pending.resolve(receipt));
  expect(view.getByRole('status').textContent).toBe(agentExecution.restart.accepted);
  expect((view.getByRole('button', { name: 'Restarting…' }) as HTMLButtonElement).disabled).toBe(true);
  expect(refresh).toHaveBeenCalledTimes(1);
  expect(view.container.textContent).not.toContain('Task retried');
});

test('a lost ACK retries the same frozen request after versions refresh, then follows the exact same-Attempt resolution', async () => {
  const restart = spyOn(ipcBridge.agentExecution.restart, 'invoke')
    .mockRejectedValueOnce(new TypeError('Network response lost')).mockResolvedValue({ ...receipt, replayed: true });
  const events = spyOn(ipcBridge.agentExecution.listEvents, 'invoke').mockResolvedValue([]);
  restores.push(() => restart.mockRestore(), () => events.mockRestore());
  const refresh = mock(async () => {}), view = render(scene({ refetch: refresh }));
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  await view.findByRole('alert');
  const original = structuredClone(restart.mock.calls[0][0]);
  view.rerender(scene({ executionVersion: 38, stepVersion: 9, refetch: refresh }));
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  await view.findByRole('status');
  expect(restart.mock.calls[1][0]).toEqual(original);
  events.mockResolvedValueOnce([resolvedEvent('continued_original', 'unrelated-command')]);
  await act(async () => notify({ execution_id: executionId, sequence: 43, change_kind: 'step_changed' }));
  expect((view.getByRole('button', { name: 'Restarting…' }) as HTMLButtonElement).disabled).toBe(true);
  events.mockResolvedValueOnce([{ ...resolvedEvent(), sequence: 44 }]);
  await act(async () => notify({ execution_id: executionId, sequence: 44, change_kind: 'step_changed' }));
  await view.findByRole('button', { name: 'Restart task' });
  expect(view.getByRole('status').textContent).toBe(agentExecution.restart.resolved);
  expect(restart).toHaveBeenCalledTimes(2);
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  await waitFor(() => expect(restart).toHaveBeenCalledTimes(3));
  expect(restart.mock.calls[2][0].updates).toMatchObject({ expected_execution_version: 38, expected_step_version: 9, expected_attempt_id: attempt.attempt_id });
  expect(restart.mock.calls[2][0].updates.idempotency_key).not.toBe(original.updates.idempotency_key);
});

test('an old request reply cannot control or label a newly selected exact Attempt', async () => {
  const old = deferred<TAgentExecutionMessageReceipt>(), next = deferred<TAgentExecutionMessageReceipt>();
  const restart = spyOn(ipcBridge.agentExecution.restart, 'invoke').mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
  restores.push(() => restart.mockRestore());
  const view = render(scene());
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  const nextAttempt = parseExecutionAttemptId('00000000-09c4-7000-8000-000000000032');
  view.rerender(scene({ attemptId: nextAttempt, executionVersion: 39, stepVersion: 10 }));
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  expect(restart).toHaveBeenCalledTimes(2);
  expect(restart.mock.calls[1][0].updates.expected_attempt_id).toBe(nextAttempt);
  expect(restart.mock.calls[1][0].updates.idempotency_key).not.toBe(restart.mock.calls[0][0].updates.idempotency_key);
  await act(async () => old.resolve(receipt));
  expect(view.queryByRole('status')).toBeNull();
  await act(async () => next.resolve({ ...receipt, sequence: 45, operation_id: 'restart-next' }));
  expect(view.getByRole('status').textContent).toBe(agentExecution.restart.accepted);
});

test('blocked recovery is an observed command outcome and never triggers another mutation', async () => {
  const restart = spyOn(ipcBridge.agentExecution.restart, 'invoke').mockResolvedValue(receipt);
  const events = spyOn(ipcBridge.agentExecution.listEvents, 'invoke').mockResolvedValue([resolvedEvent('blocked')]);
  restores.push(() => restart.mockRestore(), () => events.mockRestore());
  const view = render(scene());
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  const blocked = await view.findByRole('alert');
  expect(blocked.textContent).toBe(agentExecution.restart.blocked);
  expect(blocked.getAttribute('title')).toBe('The original pause needs user action.');
  expect(view.getByRole('button', { name: 'Restart task' })).toBeTruthy();
  expect(restart).toHaveBeenCalledTimes(1);
});

test('an unavailable observation preserves the accepted request without announcing success or replaying it', async () => {
  const restart = spyOn(ipcBridge.agentExecution.restart, 'invoke').mockResolvedValue(receipt);
  const events = spyOn(ipcBridge.agentExecution.listEvents, 'invoke').mockRejectedValue(new TypeError('Read unavailable'));
  restores.push(() => restart.mockRestore(), () => events.mockRestore());
  const view = render(scene());
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  await view.findByRole('status');
  await waitFor(() => expect(events).toHaveBeenCalledTimes(2));
  fireEvent.click(view.getByRole('button', { name: 'Restarting…' }));
  expect(restart).toHaveBeenCalledTimes(1);
  expect(view.getByRole('status').textContent).toBe(agentExecution.restart.accepted);
});

test('an explicit version rejection releases the draft only after complete facts prove no acceptance for the same Attempt', async () => {
  const conflict = new BackendHttpError({ method: 'POST', path: '/restart', status: 409, body: { error: 'Version changed' } });
  const restart = spyOn(ipcBridge.agentExecution.restart, 'invoke').mockRejectedValueOnce(conflict).mockResolvedValue(receipt);
  const fresh = makeDetail({ steps: [step], attempts: [attempt] });
  const get = spyOn(ipcBridge.agentExecution.get, 'invoke').mockResolvedValue(fresh);
  const events = spyOn(ipcBridge.agentExecution.listEvents, 'invoke').mockResolvedValue([
    { ...resolvedEvent(), event_type: 'created', step_id: null, attempt_id: null, sequence: 1, payload: {} },
  ]);
  restores.push(() => restart.mockRestore(), () => get.mockRestore(), () => events.mockRestore());
  const view = render(scene());
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  await view.findByRole('alert');
  expect(view.getByRole('alert').textContent).toBe(agentExecution.restart.blocked);
  const original = structuredClone(restart.mock.calls[0][0]);
  view.rerender(scene({ executionVersion: 39, stepVersion: 9 }));
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  await waitFor(() => expect(restart).toHaveBeenCalledTimes(2));
  expect(restart.mock.calls[1][0].updates).toMatchObject({ expected_execution_version: 39, expected_step_version: 9 });
  expect(restart.mock.calls[1][0].updates.idempotency_key).not.toBe(original.updates.idempotency_key);
});

test('a failed conflict observation keeps the original key instead of guessing that no command committed', async () => {
  const conflict = new BackendHttpError({ method: 'POST', path: '/restart', status: 409, body: { error: 'Version changed' } });
  const restart = spyOn(ipcBridge.agentExecution.restart, 'invoke').mockRejectedValue(conflict);
  const get = spyOn(ipcBridge.agentExecution.get, 'invoke').mockRejectedValue(new Error('Read unavailable'));
  restores.push(() => restart.mockRestore(), () => get.mockRestore());
  const view = render(scene());
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  await view.findByRole('alert');
  expect(view.getByRole('alert').textContent).toBe(agentExecution.restart.unconfirmed);
  const original = structuredClone(restart.mock.calls[0][0]);
  view.rerender(scene({ executionVersion: 40, stepVersion: 10 }));
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  await waitFor(() => expect(restart).toHaveBeenCalledTimes(2));
  expect(restart.mock.calls[1][0]).toEqual(original);
});

test('the original accepted command wins over a later rejection response and is observed without replay', async () => {
  const conflict = new BackendHttpError({ method: 'POST', path: '/restart', status: 409, body: { error: 'Version changed' } });
  const restart = spyOn(ipcBridge.agentExecution.restart, 'invoke').mockRejectedValue(conflict);
  const get = spyOn(ipcBridge.agentExecution.get, 'invoke').mockResolvedValue(makeDetail({ steps: [step], attempts: [attempt] }));
  const events = spyOn(ipcBridge.agentExecution.listEvents, 'invoke').mockImplementation(async () => restart.mock.calls.length ? [{
    ...resolvedEvent(), sequence: 42, payload: { change: 'restart_requested', operation_id: receipt.operation_id,
      command_request: { request: restart.mock.calls[0][0].updates } },
  }] : []);
  restores.push(() => restart.mockRestore(), () => get.mockRestore(), () => events.mockRestore());
  const view = render(scene());
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  await view.findByRole('status');
  expect(view.getByRole('status').textContent).toBe(agentExecution.restart.accepted);
  expect(view.queryByRole('alert')).toBeNull();
  expect(restart).toHaveBeenCalledTimes(1);
});

test('a typed user-pause rejection points to Continue and grants no automatic resume', async () => {
  const restart = spyOn(ipcBridge.agentExecution.restart, 'invoke').mockResolvedValue(receipt);
  const event = resolvedEvent('blocked');
  const events = spyOn(ipcBridge.agentExecution.listEvents, 'invoke').mockResolvedValue([{ ...event,
    payload: { ...(event.payload as object), code: 'USER_PAUSE_REQUIRES_CONTINUE' } }]);
  restores.push(() => restart.mockRestore(), () => events.mockRestore());
  const view = render(scene());
  fireEvent.click(await view.findByRole('button', { name: 'Restart task' }));
  expect((await view.findByRole('alert')).textContent).toContain(agentExecution.restart.continueRequired);
  expect(restart).toHaveBeenCalledTimes(1);
});
