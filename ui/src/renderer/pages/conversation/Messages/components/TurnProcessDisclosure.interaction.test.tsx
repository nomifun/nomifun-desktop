import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import TurnProcessDisclosure from './TurnProcessDisclosure';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: {} } } });

type Step = { id: string; kind: 'thinking' | 'tool'; state: 'running' | 'completed' };
const steps: Step[] = [
  { id: 'thinking', kind: 'thinking', state: 'running' },
  { id: 'tool-result', kind: 'tool', state: 'running' },
];
const disclosure = (running: boolean) => ({
  id: 'turn-1', processItems: steps, startAt: 1, endAt: 1001,
  state: running ? 'running' as const : 'completed' as const,
  running, defaultCollapsed: true,
});
const view = (running: boolean) => (
  <I18nextProvider i18n={i18n}>
    <TurnProcessDisclosure
      item={disclosure(running)}
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
  expect(container.querySelector('.turn-process-disclosure__body')).toBeNull();
  fireEvent.click(container.querySelector('.turn-process-disclosure__toggle')!);
  const current = container.querySelector('.turn-process-disclosure__item--current');
  expect(current?.textContent).toBe('tool-result');
  expect(container.querySelector('.turn-process-disclosure__item--thinking.turn-process-disclosure__item--current')).toBeNull();

  rerender(view(false));
  expect(container.querySelector('.turn-process-disclosure__body')).toBeNull();
  fireEvent.click(container.querySelector('.turn-process-disclosure__toggle')!);
  expect(container.querySelector('.turn-process-disclosure--live')).toBeNull();
  expect(container.querySelector('.turn-process-disclosure__item--current')).toBeNull();
});
