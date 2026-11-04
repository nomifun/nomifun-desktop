import '../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import CollapsibleContent from './CollapsibleContent';

const originalScrollHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollHeight');
const originalResizeObserver = globalThis.ResizeObserver;
const originalRequestAnimationFrame = window.requestAnimationFrame;

afterEach(() => {
  cleanup();
  if (originalScrollHeight) Object.defineProperty(HTMLElement.prototype, 'scrollHeight', originalScrollHeight);
  else Reflect.deleteProperty(HTMLElement.prototype, 'scrollHeight');
  globalThis.ResizeObserver = originalResizeObserver;
  window.requestAnimationFrame = originalRequestAnimationFrame;
  document.documentElement.removeAttribute('data-theme');
});

test('collapsed content fades into its actual parent surface through a mask in both themes', () => {
  Object.defineProperty(HTMLElement.prototype, 'scrollHeight', { configurable: true, get: () => 360 });
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as typeof ResizeObserver;
  window.requestAnimationFrame = callback => { callback(0); return 1; };

  const page = render(<div style={{ backgroundColor: 'rebeccapurple' }}><CollapsibleContent maxHeight={120}><p>Long content</p></CollapsibleContent></div>);
  const content = page.getByText('Long content').parentElement as HTMLDivElement;
  expect(content.style.maskImage).toContain('linear-gradient');
  expect(content.style.background).toBe('');

  act(() => { document.documentElement.setAttribute('data-theme', 'dark'); });
  expect(content.style.maskImage).toContain('linear-gradient');
  fireEvent.click(page.getByRole('button'));
  expect(content.style.maskImage).toBe('');
});
