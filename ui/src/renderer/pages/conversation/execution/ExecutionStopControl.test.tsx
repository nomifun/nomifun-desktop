import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { executionId, makeDetail } from '../../../../../test/fixtures/conversationDelegation';
import agentExecution from '@/renderer/services/i18n/locales/en-US/agentExecution.json';
import ExecutionStopControl from './ExecutionStopControl';
import { ExecutionControls } from './ExecutionControls';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentExecution } } } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });
const stopped = () => {
  const detail = makeDetail();
  detail.execution = { ...detail.execution, status: 'cancelled', version: 42 };
  return detail;
};
function notices() {
  const success = mock(() => () => {}), error = mock(() => () => {}), warning = mock(() => () => {});
  const spy = spyOn(Message, 'useMessage').mockReturnValue([{ success, error, warning }, <></>]);
  restores.push(() => spy.mockRestore());
  return { success, error, warning };
}
function control(status = 'waiting_input', refetch = mock(async () => {})) {
  return <I18nextProvider i18n={i18n}><ExecutionStopControl executionId={executionId} executionVersion={41}
    status={status} refetch={refetch} /></I18nextProvider>;
}

test('all known nonterminal states retain stop and terminal states hide it', () => {
  notices();
  const view = render(control('planning'));
  for (const status of ['planning', 'running', 'waiting_input', 'paused']) {
    view.rerender(control(status));
    expect(view.getByRole('button', { name: 'Stop collaboration' }).getAttribute('aria-disabled')).toBe('false');
  }
  for (const status of ['completed', 'completed_with_failures', 'failed', 'cancelled', '']) {
    view.rerender(control(status));
    expect(view.queryByRole('button')).toBeNull();
  }
});

test('stop targets its exact version and waits for the real response without duplicate clicks', async () => {
  const notice = notices();
  let finish!: (detail: ReturnType<typeof stopped>) => void;
  const cancel = spyOn(ipcBridge.agentExecution.cancel, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  restores.push(() => cancel.mockRestore());
  const refetch = mock(async () => {});
  const view = render(control('waiting_input', refetch));
  const stop = view.getByRole('button', { name: 'Stop collaboration' });
  fireEvent.click(stop);
  fireEvent.click(stop);
  expect(cancel).toHaveBeenCalledTimes(1);
  expect(cancel.mock.calls[0][0]).toEqual({ execution_id: executionId, updates: { expected_version: 41 } });
  expect(view.getByRole('button', { name: 'Stopping…' }).getAttribute('aria-disabled')).toBe('true');
  expect(notice.success).not.toHaveBeenCalled();
  expect(refetch).not.toHaveBeenCalled();
  await act(async () => { finish(stopped()); });
  expect(refetch).toHaveBeenCalledTimes(1);
  expect(notice.success).toHaveBeenCalledTimes(1);
  expect(notice.error).not.toHaveBeenCalled();
  expect(view.queryByRole('button')).toBeNull();
});

test('a conflict refreshes canonical state and reports failure without claiming cancellation', async () => {
  const notice = notices();
  const cancel = spyOn(ipcBridge.agentExecution.cancel, 'invoke').mockRejectedValue(new BackendHttpError({
    method: 'POST', path: '/api/agent-executions/exact/cancel', status: 409, body: { error: 'version changed' },
  }));
  restores.push(() => cancel.mockRestore());
  const refetch = mock(async () => {});
  const view = render(control('running', refetch));
  fireEvent.keyDown(view.getByRole('button', { name: 'Stop collaboration' }), { key: 'Enter' });
  await waitFor(() => expect(notice.error).toHaveBeenCalledTimes(1));
  expect(refetch).toHaveBeenCalledTimes(1);
  expect(notice.success).not.toHaveBeenCalled();
  expect(view.getByRole('button', { name: 'Stop collaboration' }).getAttribute('aria-disabled')).toBe('false');
});

test('a committed cancellation remains stopped when the subsequent display refresh fails', async () => {
  const notice = notices();
  const cancel = spyOn(ipcBridge.agentExecution.cancel, 'invoke').mockResolvedValue(stopped());
  restores.push(() => cancel.mockRestore());
  const refetch = mock(async () => { throw new Error('display refresh failed'); });
  const view = render(control('paused', refetch));
  fireEvent.click(view.getByRole('button', { name: 'Stop collaboration' }));
  await waitFor(() => expect(notice.warning).toHaveBeenCalledTimes(1));
  expect(notice.success).toHaveBeenCalledTimes(1);
  expect(notice.error).not.toHaveBeenCalled();
  expect(view.queryByRole('button')).toBeNull();
});

test('a paused review barrier cannot resume but remains cancellable', () => {
  notices();
  const resume = spyOn(ipcBridge.agentExecution.resume, 'invoke');
  restores.push(() => resume.mockRestore());
  const view = render(<I18nextProvider i18n={i18n}><ExecutionControls executionId={executionId} executionVersion={41}
    status='paused' resumeBlocked refetch={async () => {}} onReplan={() => {}} /></I18nextProvider>);
  const resumeButton = view.getByRole('button', { name: 'Resume' });
  expect(resumeButton.getAttribute('aria-disabled')).toBe('true');
  fireEvent.click(resumeButton);
  fireEvent.keyDown(resumeButton, { key: 'Enter' });
  expect(resume).not.toHaveBeenCalled();
  expect(view.getByRole('button', { name: 'Stop collaboration' }).getAttribute('aria-disabled')).toBe('false');
});
