import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { parseMessageId } from '@/common/types/ids';
import { buildTurnDisclosureItems } from '../turnDisclosureModel';
import TurnProcessDisclosure from './TurnProcessDisclosure';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: {} } } });

type Step = { id: string; kind: 'thinking' | 'tool'; state: 'running' | 'completed' };
const steps: Step[] = [
  { id: 'thinking', kind: 'thinking', state: 'running' },
  { id: 'tool-result', kind: 'tool', state: 'running' },
];
const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000001');
const disclosure = (running: boolean, processItems: Step[]) => {
  const items = buildTurnDisclosureItems([
    { id: 'user', turnId, role: 'user', createdAt: 1 },
    ...processItems.map((step, index) => ({
      id: step.id,
      turnId,
      role: step.kind === 'thinking' ? 'process_content' as const : 'process' as const,
      processState: step.state,
      createdAt: index + 2,
    })),
  ], { tailClosed: !running });
  const item = items.find((entry) => entry.type === 'turn_disclosure');
  if (!item) throw new Error('Expected a turn disclosure');
  return { ...item, processItems };
};
const view = (running: boolean, processItems: Step[] = steps) => (
  <I18nextProvider i18n={i18n}>
    <TurnProcessDisclosure
      item={disclosure(running, processItems)}
      renderProcessItem={(step) => <span>{step.id}</span>}
      getProcessItemKey={(step) => step.id}
      getProcessItemState={(step) => step.state}
      getProcessItemLayoutKind={(step) => step.kind}
    />
  </I18nextProvider>
);

afterEach(cleanup);

test('only the latest running process row is current; no row animates after the turn finishes', () => {
  const { container, rerender } = render(view(true));
  expect(container.querySelector('.turn-process-disclosure__body')).not.toBeNull();
  const current = container.querySelector('.turn-process-disclosure__item--current');
  expect(current?.textContent).toBe('tool-result');
  expect(container.querySelector('.turn-process-disclosure__item--thinking.turn-process-disclosure__item--current')).toBeNull();

  rerender(view(false));
  expect(container.querySelector('.turn-process-disclosure__body')).toBeNull();
  fireEvent.click(container.querySelector('.turn-process-disclosure__toggle')!);
  expect(container.querySelector('.turn-process-disclosure--live')).toBeNull();
  expect(container.querySelector('.turn-process-disclosure__item--current')).toBeNull();
});

test('completed history starts collapsed and preserves manual expansion on refresh', () => {
  const page = render(view(false));
  const toggle = page.getByRole('button', { name: 'Expand thinking process' });
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(page.queryByText('thinking')).toBeNull();

  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(page.getByText('thinking')).toBeDefined();

  page.rerender(view(false, [...steps]));
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(page.queryByText('thinking')).toBeNull();
});

test('stream updates preserve a manual collapse until the turn resumes', () => {
  const page = render(view(true));
  const toggle = page.getByRole('button', { name: 'Collapse thinking process' });
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  fireEvent.click(toggle);

  const updatedSteps: Step[] = [...steps, { id: 'next-tool', kind: 'tool', state: 'running' }];
  page.rerender(view(true, updatedSteps));
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(page.queryByText('next-tool')).toBeNull();

  page.rerender(view(false, updatedSteps));
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  page.rerender(view(true, updatedSteps));
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(page.getByText('next-tool')).toBeDefined();
});

test('opens when the first live process items arrive', () => {
  const page = render(view(true, []));
  expect(page.queryByRole('button')).toBeNull();

  page.rerender(view(true));
  expect(page.getByRole('button').getAttribute('aria-expanded')).toBe('true');
  expect(page.getByText('thinking')).toBeDefined();
});
