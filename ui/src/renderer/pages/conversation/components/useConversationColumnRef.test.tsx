import '../../../../../test/setup-dom.ts';
import { cleanup, render } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { useConversationColumnRef } from './useConversationColumnRef';

afterEach(cleanup);

test('tracks the measured transcript width across pane resize and releases its observer and scroll ref', () => {
  const originalObserver = globalThis.ResizeObserver;
  let notify = () => {};
  let observed: Element | undefined;
  let disconnected = false;
  const refs: Array<HTMLDivElement | null> = [];
  globalThis.ResizeObserver = class {
    constructor(callback: () => void) { notify = callback; }
    observe(node: Element) { observed = node; }
    unobserve() {}
    disconnect() { disconnected = true; }
  } as unknown as typeof ResizeObserver;
  const onNode = (node: HTMLDivElement | null) => refs.push(node);
  function Column() {
    const ref = useConversationColumnRef(onNode);
    return <div data-conversation-layout><div ref={ref} data-testid="column" /></div>;
  }
  try {
    const view = render(<Column />);
    const column = view.getByTestId('column');
    const host = column.parentElement!;
    let width = 800;
    column.getBoundingClientRect = () => ({ width } as DOMRect);
    expect(observed).toBe(column);
    notify();
    expect(host.style.getPropertyValue('--conversation-content-width')).toBe('800px');
    width = 488;
    notify();
    expect(host.style.getPropertyValue('--conversation-content-width')).toBe('488px');
    width = 0;
    notify();
    expect(host.style.getPropertyValue('--conversation-content-width')).toBe('488px');
    view.unmount();
    expect(disconnected).toBe(true);
    expect(host.style.getPropertyValue('--conversation-content-width')).toBe('');
    expect(refs).toEqual([column, null]);
  } finally {
    globalThis.ResizeObserver = originalObserver;
  }
});
