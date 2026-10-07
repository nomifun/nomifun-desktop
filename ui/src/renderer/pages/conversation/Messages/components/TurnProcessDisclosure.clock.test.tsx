import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import TurnProcessDisclosure, { type TurnProcessDisclosureView } from './TurnProcessDisclosure';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: {} } } });
afterEach(cleanup);

test('the work clock advances without messages while collapsed and stops at the Turn receipt', () => {
  let now = 100_000;
  let tick: (() => void) | undefined;
  const clock = spyOn(Date, 'now').mockImplementation(() => now);
  const interval = spyOn(window, 'setInterval').mockImplementation(((handler: TimerHandler) => {
    tick = handler as () => void;
    return 42;
  }) as typeof window.setInterval);
  const clear = spyOn(window, 'clearInterval').mockImplementation(() => {});
  let journalRenders = 0;
  const item: TurnProcessDisclosureView<{ id: string }> = {
    id: 'active-turn', startAt: 90_000, endAt: 95_000,
    running: true, state: 'running', defaultCollapsed: false,
    processItems: [{ id: 'completed-tool' }],
  };
  const view = (snapshot = item) => <I18nextProvider i18n={i18n}>
    <TurnProcessDisclosure item={snapshot} renderProcessItem={step => {
      journalRenders++;
      return <span>{step.id}</span>;
    }}
      getProcessItemKey={step => step.id} getProcessItemState={() => 'completed'} />
  </I18nextProvider>;
  try {
    const page = render(view());
    const label = () => page.container.querySelector('.turn-process-disclosure__label')?.textContent;
    expect(label()).toBe('Took 10s');
    const rendersBeforeTick = journalRenders;
    now += 1000;
    act(() => tick!());
    expect(label()).toBe('Took 11s');
    expect(journalRenders).toBe(rendersBeforeTick);
    fireEvent.click(page.getByRole('button', { name: 'Collapse thinking process' }));
    expect(page.container.querySelector('.turn-process-disclosure__body')).toBeNull();
    now += 1000;
    act(() => tick!());
    expect(label()).toBe('Took 12s');
    now += 2000;
    act(() => tick!());
    expect(label()).toBe('Took 14s');
    // A stream render still samples wall time even if the interval is delayed.
    now += 1000;
    page.rerender(view({ ...item, processItems: [...item.processItems] }));
    expect(label()).toBe('Took 15s');
    now += 5000;
    act(() => window.dispatchEvent(new Event('focus')));
    expect(label()).toBe('Took 20s');
    expect(interval).toHaveBeenCalledTimes(1);
    const longStart = now - 3_661_000;
    page.rerender(view({ ...item, startAt: longStart }));
    expect(label()).toBe('Took 1h 1m 1s');
    now += 1000;
    act(() => tick!());
    expect(label()).toBe('Took 1h 1m 2s');
    expect(interval).toHaveBeenCalledTimes(2);
    page.rerender(view({ ...item, running: false, state: 'completed', endAt: 102_000 }));
    expect(label()).toBe('Took 12s');
    expect(clear).toHaveBeenCalledWith(42);
    now += 10_000;
    act(() => tick!());
    expect(label()).toBe('Took 12s');
    page.unmount();
  } finally {
    clock.mockRestore(); interval.mockRestore(); clear.mockRestore();
  }
});
