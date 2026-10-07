import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, mock, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { executionId, makeAttempt, makeDetail, makeStep, requestId } from '../../../../../../test/fixtures/conversationDelegation';
import type { ConversationDelegation } from '../conversationDelegationModel';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import agentExecution from '@/renderer/services/i18n/locales/en-US/agentExecution.json';
import DelegationProgress from './DelegationProgress';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { messages, agentExecution } } } });
afterEach(cleanup);
const refetch = async () => {};
const delegation = (detail: ConversationDelegation['detail'] = makeDetail()): ConversationDelegation => ({
  executionId, turnId: requestId, detail, unfinished: true,
});

test('planning and synchronizing show actual aggregate status before tasks exist', () => {
  const detail = makeDetail(); detail.execution.status = 'planning';
  const page = render(<I18nextProvider i18n={i18n}><DelegationProgress delegation={delegation(detail)} projectStep={() => {}} refetch={refetch} /></I18nextProvider>);
  expect(page.getByRole('status').textContent).toContain(agentExecution.status.execution.planning);
  expect(page.getByRole('status').textContent).toContain(detail.execution.goal);
  page.rerender(<I18nextProvider i18n={i18n}><DelegationProgress delegation={delegation(null)} projectStep={() => {}} refetch={refetch} /></I18nextProvider>);
  expect(page.getByRole('status').textContent).toBe(messages.delegation.syncing);
});

test('each current task has its own state, queued work stays pending, and superseded tasks disappear', () => {
  const steps = [makeStep(1), makeStep(2, { status: 'completed' }), makeStep(3, { status: 'waiting_input' }),
    makeStep(4, { status: 'failed' }), makeStep(5, { superseded_in_revision: 2 })];
  const attempts = [makeAttempt(steps[0], { status: 'queued', conversation_id: null }),
    makeAttempt(steps[2], { status: 'waiting_input', question: 'Choose a game style' }),
    makeAttempt(steps[3], { status: 'failed', error: 'Build check failed' })];
  const page = render(<I18nextProvider i18n={i18n}><DelegationProgress delegation={delegation(makeDetail({ steps, attempts }))} projectStep={() => {}} refetch={refetch} /></I18nextProvider>);
  expect(page.container.querySelectorAll('.conversation-delegation__task')).toHaveLength(4);
  expect(page.container.querySelectorAll('.conversation-delegation__task--pending')).toHaveLength(1);
  expect(page.container.querySelector('.conversation-delegation__count')?.textContent).toBe(i18n.t('agentExecution.progress.summary', { done: 1, total: 4 }));
  expect(page.container.textContent).toContain('Choose a game style');
  expect(page.container.textContent).toContain('Build check failed');
  expect(page.queryByText('Task 5')).toBeNull();
  expect(page.container.textContent).not.toMatch(/\d+(s|m|h)\b/);
  expect(page.queryByRole('button', { name: 'View subtask: Task 1' })).toBeNull();
});

test('opening a task projects the latest retry transcript through the existing execution UI', () => {
  const step = makeStep(1);
  const oldAttempt = makeAttempt(step, { status: 'failed', error: 'Old failure' });
  const retry = makeAttempt(step, { attempt_no: 2, status: 'running' });
  const projectStep = mock(() => {});
  const page = render(<I18nextProvider i18n={i18n}><DelegationProgress delegation={delegation(makeDetail({ steps: [step], attempts: [oldAttempt, retry] }))} projectStep={projectStep} refetch={refetch} /></I18nextProvider>);
  fireEvent.click(page.getByRole('button', { name: 'View subtask: Task 1' }));
  expect(projectStep).toHaveBeenCalledWith({ step, attempt: retry, participant: undefined, participants: [], executionId, refetch });
  expect(page.container.textContent).not.toContain('Old failure');
});
