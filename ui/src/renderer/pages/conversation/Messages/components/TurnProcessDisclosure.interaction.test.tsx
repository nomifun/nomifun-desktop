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

test('a cancelled disclosure names its outcome even while collapsed', () => {
  const page=render(<I18nextProvider i18n={i18n}><TurnProcessDisclosure
    item={{...disclosure(false,steps),state:'canceled'}}
    renderProcessItem={step=><span>{step.id}</span>}
    getProcessItemKey={step=>step.id} getProcessItemState={step=>step.state}
  /></I18nextProvider>);
  expect(page.container.querySelector('.turn-process-disclosure__label')?.textContent).toContain('Execution canceled');
});

test('a stopped partial reply remains readable with its historical status', () => {
  const reply = 'The process is still running; waiting for you to stop it.';
  const snapshot = { ...disclosure(false, steps), state: 'canceled' as const, hasInterruptedReply: true };
  const page = render(<I18nextProvider i18n={i18n}><>
    <TurnProcessDisclosure item={snapshot}
      renderProcessItem={step => <span>{step.id}</span>}
      getProcessItemKey={step => step.id} getProcessItemState={step => step.state} />
    <p>{reply}</p>
  </></I18nextProvider>);
  expect(page.container.querySelector('[data-testid="interrupted-reply-notice"]')?.textContent)
    .toBe('The reply below was written before stopping and is incomplete.');
  expect(page.container.textContent).toContain(reply);
  expect(snapshot.state).toBe('canceled');
  expect(snapshot.processItems).toEqual(steps);
});

test('the interrupted reply notice is limited to a closed cancellation with partial text', () => {
  for (const variant of [
    { state: 'completed' as const, running: false, hasInterruptedReply: true },
    { state: 'failed' as const, running: false, hasInterruptedReply: true },
    { state: 'canceled' as const, running: true, hasInterruptedReply: true },
    { state: 'canceled' as const, running: false, hasInterruptedReply: false },
  ]) {
    const page = render(<I18nextProvider i18n={i18n}><TurnProcessDisclosure
      item={{ ...disclosure(false, steps), ...variant }}
      renderProcessItem={step => <span>{step.id}</span>}
      getProcessItemKey={step => step.id} getProcessItemState={step => step.state}
    /></I18nextProvider>);
    expect(page.container.querySelector('[data-testid="interrupted-reply-notice"]')).toBeNull();
    page.unmount();
  }
});

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

test('stream updates preserve manual collapse and a resumed task opens by default', () => {
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

test('canonical groups keep narration outside, current work open, and manual choices across increments', () => {
  type Entry = { id: string; group?: string; kind: 'thinking' | 'tool' | 'text'; state: 'running' | 'completed' };
  const items: Entry[] = [
    { id: 'past-thinking', group: 'canonical-first', kind: 'thinking', state: 'completed' },
    { id: 'past-tool', group: 'canonical-first', kind: 'tool', state: 'completed' },
    { id: 'public-narration', kind: 'text', state: 'completed' },
    { id: 'current-tool', group: 'canonical-second', kind: 'tool', state: 'running' },
  ];
  const stageView = (running: boolean, entries = items) => <I18nextProvider i18n={i18n}>
    <TurnProcessDisclosure item={{ id: 'turn', processItems: entries, startAt: 1, endAt: 10,
      state: running ? 'running' : 'completed', running, defaultCollapsed: !running }}
      renderProcessItem={entry => <span>{entry.id}</span>} getProcessItemKey={entry => entry.id}
      getProcessItemState={entry => entry.state} getProcessItemLayoutKind={entry => entry.kind}
      getProcessGroupId={entry => entry.group} getProcessGroupLabel={entries => entries[0].group!} />
  </I18nextProvider>;
  const page = render(stageView(true));
  const first = () => page.container.querySelector('[data-process-group-id="canonical-first"] > button')!;
  const second = () => page.container.querySelector('[data-process-group-id="canonical-second"] > button')!;
  expect(first().getAttribute('aria-expanded')).toBe('false');
  expect(second().getAttribute('aria-expanded')).toBe('true');
  expect(first().parentElement?.classList.contains('turn-process-group--running')).toBe(false);
  expect(second().parentElement?.classList.contains('turn-process-group--running')).toBe(true);
  expect(page.getByText('public-narration').closest('.turn-process-group')).toBeNull();
  fireEvent.click(first());
  fireEvent.click(second());
  page.rerender(stageView(true, [...items, { id: 'later-output', group: 'canonical-second', kind: 'tool', state: 'running' }]));
  expect(first().getAttribute('aria-expanded')).toBe('true');
  expect(second().getAttribute('aria-expanded')).toBe('false');
  expect(second().parentElement?.classList.contains('turn-process-group--running')).toBe(true);
  page.rerender(stageView(false));
  expect(page.container.querySelector('.turn-process-disclosure__body')).toBeNull();
  fireEvent.click(page.container.querySelector('.turn-process-disclosure__toggle')!);
  expect(page.container.querySelector('.turn-process-group--running')).toBeNull();
});
