import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { executionId, makeDetail, makeStep } from '../../../../../test/fixtures/conversationDelegation';
import agentExecution from '@/renderer/services/i18n/locales/en-US/agentExecution.json';
import ExecutionStepStopControl from './ExecutionStepStopControl';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentExecution } } } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });
const step = makeStep(0, { version: 7 });
const accepted = () => makeDetail({ steps: [{ ...step, status: 'cancelled' }, makeStep(1)] });
function notices() {
  const success = mock(() => () => {}), error = mock(() => () => {}), warning = mock(() => () => {});
  const spy = spyOn(Message, 'useMessage').mockReturnValue([{ success, error, warning }, <></>]);
  restores.push(() => spy.mockRestore());
  return { success, error, warning };
}
function control(stepStatus = 'running', refetch = mock(async () => {}), executionStatus = 'running', delegatedOnly = false) {
  return <I18nextProvider i18n={i18n}><ExecutionStepStopControl executionId={executionId}
    executionVersion={41} executionStatus={executionStatus} stepId={step.step_id}
    stepVersion={7} stepStatus={stepStatus} delegatedOnly={delegatedOnly} refetch={refetch} /></I18nextProvider>;
}

test('task stop captures one task and does not send a collaboration cancel', async () => {
  const notice = notices();
  let finish!: (detail: ReturnType<typeof accepted>) => void;
  const cancelTask = spyOn(ipcBridge.agentExecution.cancelStep, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const cancelAll = spyOn(ipcBridge.agentExecution.cancel, 'invoke');
  restores.push(() => cancelTask.mockRestore(), () => cancelAll.mockRestore());
  const refetch = mock(async () => {});
  const view = render(control('running', refetch));
  const stop = view.getByRole('button', { name: 'Stop task' });
  fireEvent.click(stop);
  fireEvent.click(stop);
  expect(cancelTask).toHaveBeenCalledTimes(1);
  expect(cancelTask.mock.calls[0][0]).toEqual({ execution_id: executionId, step_id: step.step_id,
    updates: { expected_execution_version: 41, expected_step_version: 7, scope: 'task' } });
  expect(cancelAll).not.toHaveBeenCalled();
  expect(notice.success).not.toHaveBeenCalled();
  await act(async () => { finish(accepted()); });
  expect(refetch).toHaveBeenCalledTimes(1);
  expect(view.getByRole('status').textContent).toContain('resource release is not yet confirmed');
  expect(notice.success).toHaveBeenCalledTimes(1);
});

test('delegation stop is an explicit scope on the captured task', async () => {
  notices();
  const cancelTask = spyOn(ipcBridge.agentExecution.cancelStep, 'invoke').mockResolvedValue(accepted());
  restores.push(() => cancelTask.mockRestore());
  const view = render(control());
  fireEvent.click(view.getByRole('button', { name: 'Choose task stop scope' }));
  fireEvent.click(await view.findByText('Stop task and its delegated work'));
  await waitFor(() => expect(cancelTask).toHaveBeenCalledTimes(1));
  expect(cancelTask.mock.calls[0][0]?.updates).toEqual({
    expected_execution_version: 41, expected_step_version: 7, scope: 'task_and_delegated',
  });
});

test('waiting and paused recovery tasks remain stoppable without steering', () => {
  notices();
  const view = render(control('pending'));
  for (const [status, executionStatus] of [['pending', 'running'], ['running', 'paused'], ['waiting_input', 'waiting_input']]) {
    view.rerender(control(status, mock(async () => {}), executionStatus));
    expect((view.getByRole('button', { name: 'Stop task' }) as HTMLButtonElement).disabled).toBe(false);
  }
});

test('a completed parent retains a delegated-work stop without changing its result', async () => {
  notices();
  const result = makeDetail({ steps: [{ ...step, status: 'completed' }, { ...makeStep(1), status: 'cancelled' }] });
  const cancelTask = spyOn(ipcBridge.agentExecution.cancelStep, 'invoke').mockResolvedValue(result);
  restores.push(() => cancelTask.mockRestore());
  const view = render(control('completed'));
  expect(view.queryByRole('button', { name: 'Stop task' })).toBeNull();
  fireEvent.click(view.getByRole('button', { name: 'Choose task stop scope' }));
  fireEvent.click(await view.findByText('Stop task and its delegated work'));
  await waitFor(() => expect(view.getByRole('status')).toBeTruthy());
  expect(cancelTask.mock.calls[0][0]?.updates.scope).toBe('task_and_delegated');
});

test('delegated-only scope leaves the running parent directly stoppable', async () => {
  notices();
  const result=makeDetail({steps:[{...step,version:8,status:'running'},{...makeStep(1),status:'cancelled'}]});
  const cancelTask=spyOn(ipcBridge.agentExecution.cancelStep,'invoke').mockResolvedValue(result);
  restores.push(()=>cancelTask.mockRestore());
  const view=render(control());
  fireEvent.click(view.getByRole('button',{name:'Choose task stop scope'}));
  fireEvent.click(await view.findByText('Stop only its delegated work'));
  await waitFor(()=>expect(view.getByRole('status')).toBeTruthy());
  expect(cancelTask.mock.calls[0][0]?.updates).toEqual({expected_execution_version:41,expected_step_version:7,scope:'delegated_work'});
  expect(view.getByRole('button',{name:'Stop task'})).toBeTruthy();
  expect(view.queryByRole('button',{name:'Choose task stop scope'})).toBeNull();
});

test('stopping only a parent leaves its delegated-work stop available', () => {
  notices();
  const view = render(control('cancelled'));
  expect(view.queryByRole('button', { name: 'Stop task' })).toBeNull();
  expect(view.getByRole('button', { name: 'Choose task stop scope' })).toBeTruthy();
  expect(view.getByRole('status').textContent).toContain('resource release is not yet confirmed');
});

test('a stale task command refreshes without retargeting or claiming stop acceptance', async () => {
  const notice = notices();
  const cancelTask = spyOn(ipcBridge.agentExecution.cancelStep, 'invoke').mockRejectedValue(new BackendHttpError({
    method: 'POST', path: '/api/agent-executions/exact/steps/exact/cancel', status: 409, body: { error: 'version changed' },
  }));
  restores.push(() => cancelTask.mockRestore());
  const refetch = mock(async () => {});
  const view = render(control('running', refetch));
  fireEvent.click(view.getByRole('button', { name: 'Stop task' }));
  await waitFor(() => expect(notice.error).toHaveBeenCalledTimes(1));
  expect(refetch).toHaveBeenCalledTimes(1);
  expect(cancelTask).toHaveBeenCalledTimes(1);
  expect(notice.success).not.toHaveBeenCalled();
  expect(view.queryByRole('status')).toBeNull();
});

test('a display failure preserves durable acceptance and exposes unconfirmed cleanup', async () => {
  const notice = notices();
  const cancelTask = spyOn(ipcBridge.agentExecution.cancelStep, 'invoke').mockResolvedValue(accepted());
  restores.push(() => cancelTask.mockRestore());
  const view = render(control('running', mock(async () => { throw new Error('display unavailable'); })));
  fireEvent.click(view.getByRole('button', { name: 'Stop task' }));
  await waitFor(() => expect(notice.warning).toHaveBeenCalledTimes(1));
  expect(notice.success).toHaveBeenCalledTimes(1);
  expect(notice.error).not.toHaveBeenCalled();
  expect(view.queryByRole('button', { name: 'Stop task' })).toBeNull();
  expect(view.getByRole('status').textContent).toContain('resource release is not yet confirmed');
});

for (const status of ['running', 'completed']) test(`an AutoWork ${status} root exposes only its delegated-work scope`, async () => {
  notices();
  const result = makeDetail({ steps: [{ ...step, status: status as 'running' | 'completed' }, { ...makeStep(1), status: 'cancelled' }] });
  const cancelTask = spyOn(ipcBridge.agentExecution.cancelStep, 'invoke').mockResolvedValue(result);
  restores.push(() => cancelTask.mockRestore());
  const view = render(control(status, mock(async () => {}), 'running', true));
  expect(view.queryByRole('button', { name: 'Stop task' })).toBeNull();
  expect(view.queryByRole('button', { name: 'Choose task stop scope' })).toBeNull();
  expect(view.queryByText('Stop task and its delegated work')).toBeNull();
  fireEvent.click(view.getByRole('button', { name: 'Stop only its delegated work' }));
  await waitFor(() => expect(view.getByRole('status')).toBeTruthy());
  expect(cancelTask.mock.calls[0][0]).toEqual({ execution_id: executionId, step_id: step.step_id,
    updates: { expected_execution_version: 41, expected_step_version: 7, scope: 'delegated_work' } });
  expect(view.queryByRole('button')).toBeNull();
});
