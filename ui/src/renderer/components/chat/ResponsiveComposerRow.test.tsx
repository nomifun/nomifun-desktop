import '../../../../test/setup-dom.ts';
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import ResponsiveComposerRow from './ResponsiveComposerRow';

afterEach(cleanup);

test('uses expanded content width for resizing and changing labels, without compact oscillation', async () => {
  const original = globalThis.ResizeObserver;
  let resize!: () => void;
  globalThis.ResizeObserver = class {
    constructor(callback: () => void) { resize = callback; }
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  try {
    const view = render(<ResponsiveComposerRow data-testid='row'><button><span className='sendbox-responsive-label'>模型</span></button></ResponsiveComposerRow>);
    const row = view.getByTestId('row');
    let available = 800, needed = 520;
    Object.defineProperty(row, 'clientWidth', { get: () => available });
    Object.defineProperty(row, 'scrollWidth', { get: () => row.dataset.measuring ? Math.max(available, needed) : available });
    act(resize);
    await waitFor(() => expect(row.dataset.compact).toBe('false'));
    available = 400;
    act(resize);
    await waitFor(() => expect(row.dataset.compact).toBe('true'));
    act(resize);
    await waitFor(() => expect(row.dataset.measuring).toBeUndefined());
    expect(row.dataset.compact).toBe('true');
    available = 800;
    act(resize);
    await waitFor(() => expect(row.dataset.compact).toBe('false'));
    needed = 950;
    view.rerender(<ResponsiveComposerRow data-testid='row'><button><span className='sendbox-responsive-label'>更多选项和更长的模型名称</span></button></ResponsiveComposerRow>);
    await waitFor(() => expect(row.dataset.compact).toBe('true'));
  } finally { globalThis.ResizeObserver = original; }
});
