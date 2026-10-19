import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { agentActivities } from '@/common/adapter/ipcBridge';
import type { AgentActivityCommandReceipt } from '@/common/types/agentExecution/agentActivity';
import { leadConversationId, makeStep } from '../../../../../test/fixtures/conversationDelegation';
import agentExecution from '@/renderer/services/i18n/locales/en-US/agentExecution.json';
import AutomationRunStopControl from './AutomationRunStopControl';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentExecution } } } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });
const control = { root_step_id: makeStep(0).step_id, lead_conversation_id: leadConversationId,
  requirement_id: 'original-requirement', claim_generation: 7 };
const receipt: AgentActivityCommandReceipt = { accepted: true, execution_stopped: false, resources: [] };
function notices() {
  const success = mock(() => () => {}), error = mock(() => () => {}), warning = mock(() => () => {});
  const spy = spyOn(Message, 'useMessage').mockReturnValue([{ success, error, warning }, <></>]);
  restores.push(() => spy.mockRestore()); return { success, error, warning };
}
function element(source = control, refetch = mock(async () => {})) {
  return <I18nextProvider i18n={i18n}><AutomationRunStopControl control={source} status='running' refetch={refetch} /></I18nextProvider>;
}

test('an unanswered automatic run Stop is sent once and acceptance preserves cleanup uncertainty', async () => {
  const notice = notices();
  let finish!: (value: AgentActivityCommandReceipt) => void;
  const stop = spyOn(agentActivities.stop, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  restores.push(() => stop.mockRestore());
  const refetch = mock(async () => { throw new Error('display unavailable'); });
  const view = render(element(control, refetch));
  const button = view.getByRole('button', { name: 'Stop current automatic run' });
  fireEvent.click(button); fireEvent.click(button);
  expect(stop).toHaveBeenCalledTimes(1);
  expect(notice.success).not.toHaveBeenCalled();
  expect(stop.mock.calls[0][0]?.target).toEqual({ kind: 'auto_work_run', target_kind: 'conversation',
    target_id: leadConversationId, requirement_id: control.requirement_id, claim_generation: 7 });
  await act(async () => { finish(receipt); });
  expect(view.getByRole('status').textContent).toContain('resource release is not yet confirmed');
  expect(view.queryByRole('button')).toBeNull();
  expect(notice.warning).toHaveBeenCalledTimes(1);
  expect(notice.error).not.toHaveBeenCalled();
});

test('retry after a lost response retains the exact source and original idempotency key', async () => {
  const notice = notices();
  const stop = spyOn(agentActivities.stop, 'invoke').mockRejectedValueOnce(new Error('response lost')).mockResolvedValue(receipt);
  restores.push(() => stop.mockRestore());
  const view = render(element());
  fireEvent.click(view.getByRole('button', { name: 'Stop current automatic run' }));
  await waitFor(() => expect(notice.error).toHaveBeenCalledTimes(1));
  fireEvent.click(view.getByRole('button', { name: 'Stop current automatic run' }));
  await waitFor(() => expect(view.getByRole('status')).toBeTruthy());
  expect(stop.mock.calls[1][0]).toEqual(stop.mock.calls[0][0]);
});

test('a late old-run receipt cannot hide or stop a successor run', async () => {
  notices();
  let finish!: (value: AgentActivityCommandReceipt) => void;
  const stop = spyOn(agentActivities.stop, 'invoke').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  restores.push(() => stop.mockRestore());
  const view = render(element());
  fireEvent.click(view.getByRole('button', { name: 'Stop current automatic run' }));
  view.rerender(element({ ...control, requirement_id: 'successor-requirement', claim_generation: 8 }));
  await act(async () => { finish(receipt); });
  expect(view.queryByRole('status')).toBeNull();
  expect(view.getByRole('button', { name: 'Stop current automatic run' })).toBeTruthy();
  expect(stop).toHaveBeenCalledTimes(1);
  expect(stop.mock.calls[0][0]?.target).toMatchObject({ requirement_id: 'original-requirement', claim_generation: 7 });
});

test('a refused owner receipt never claims durable acceptance', async () => {
  const notice = notices();
  const stop = spyOn(agentActivities.stop, 'invoke').mockResolvedValue({ ...receipt, accepted: false });
  restores.push(() => stop.mockRestore());
  const view = render(element());
  fireEvent.click(view.getByRole('button', { name: 'Stop current automatic run' }));
  await waitFor(() => expect(notice.error).toHaveBeenCalledTimes(1));
  expect(view.queryByRole('status')).toBeNull();
  expect(notice.success).not.toHaveBeenCalled();
});
