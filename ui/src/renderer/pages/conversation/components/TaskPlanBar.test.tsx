import { afterEach, expect, test } from 'bun:test';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { TaskPlanSnapshot } from '@/common/protocolBindings/TaskPlanSnapshot';
import TaskPlanBar from './TaskPlanBar';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: { messages } } }, interpolation: { escapeValue: false } });
afterEach(cleanup);

function snapshot(status = 'running'): TaskPlanSnapshot {
  return {
    conversation_id: 'session', sequence: 8, turn_id: 'turn', turn_status: status,
    plan: { revision: 1, explanation: 'Verify the requested change', needs_replan: false,
      steps: [{ step: 'Read', status: 'completed' }, { step: 'Implement', status: 'in_progress' },
        { step: 'Verify', status: 'blocked' }] },
  };
}
const view = (data: TaskPlanSnapshot | null) => <I18nextProvider i18n={i18n}><TaskPlanBar snapshot={data} /></I18nextProvider>;

test('the progress bar renders without transcript providers and exposes blocked steps on hover', () => {
  const result = render(view(snapshot()));
  expect(result.getByTestId('task-plan-summary').textContent).toContain('1/3');
  expect(result.queryByTestId('task-plan-list')).toBeNull();
  fireEvent.mouseEnter(result.getByTestId('task-plan-bar'));
  expect(result.getByTestId('task-plan-list').querySelectorAll('li')).toHaveLength(3);
  expect(result.getByTestId('task-plan-list').textContent).toContain('VerifyBlocked');
  expect(result.getByText('Verify the requested change')).toBeTruthy();
});

test('paused and terminal tasks preserve progress without an active spinner', () => {
  const result = render(view(snapshot('paused')));
  expect(result.getByTestId('task-plan-summary').textContent).toContain('Paused');
  expect(result.queryByTestId('task-plan-progress-indicator')).toBeNull();
  result.rerender(view(snapshot('cancelled')));
  expect(result.getByTestId('task-plan-summary').textContent).toContain('1/3');
  expect(result.getByTestId('task-plan-summary').textContent).toContain('Stopped');
  expect(result.queryByTestId('task-plan-progress-indicator')).toBeNull();
  result.rerender(view(snapshot('failed')));
  expect(result.getByTestId('task-plan-summary').textContent).toContain('Failed');
  expect(result.queryByTestId('task-plan-progress-indicator')).toBeNull();
});

test('replanning and keyboard focus expose current progress, then a reset hides it', () => {
  const data = snapshot();
  data.plan!.needs_replan = true;
  const result = render(view(data));
  expect(result.getByTestId('task-plan-summary').textContent).toContain('Plan needs updating');
  expect(result.queryByTestId('task-plan-progress-indicator')).toBeNull();
  fireEvent.focus(result.getByTestId('task-plan-summary'));
  expect(result.getByTestId('task-plan-list')).toBeTruthy();
  fireEvent.keyDown(result.getByTestId('task-plan-summary'), { key: 'Escape' });
  expect(result.queryByTestId('task-plan-list')).toBeNull();
  result.rerender(view({ ...data, plan: null }));
  expect(result.queryByTestId('task-plan-bar')).toBeNull();
});

test('completed tasks keep the full checklist', () => {
  const data = snapshot('completed');
  data.plan!.steps.forEach((step) => { step.status = 'completed'; });
  const result = render(view(data));
  expect(result.getByTestId('task-plan-summary').textContent).toContain('3/3');
  expect(result.queryByTestId('task-plan-progress-indicator')).toBeNull();
});

test('all four step states remain visible and only completed steps advance the counter', () => {
  const data = snapshot();
  data.plan!.steps.push({ step: 'Wait', status: 'pending' });
  const result = render(view(data));
  expect(result.getByTestId('task-plan-summary').textContent).toContain('1/4');
  fireEvent.click(result.getByTestId('task-plan-summary'));
  expect(result.getByRole('listitem', { name: 'Completed: Read' })).toBeTruthy();
  expect(result.getByRole('listitem', { name: 'In progress: Implement' })).toBeTruthy();
  expect(result.getByRole('listitem', { name: 'Blocked: Verify' })).toBeTruthy();
  expect(result.getByRole('listitem', { name: 'Pending: Wait' })).toBeTruthy();
});
