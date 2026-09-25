import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { useAutoScroll } from './useAutoScroll';

afterEach(cleanup);

test('layout-driven scrolls keep following, while an intentional user scroll pauses it', () => {
  const originalObserver = globalThis.ResizeObserver;
  const originalRaf = globalThis.requestAnimationFrame;
  const originalCancel = globalThis.cancelAnimationFrame;
  const frames = new Map<number, FrameRequestCallback>();
  let frameId = 0;
  let resize: (() => void) | undefined;
  globalThis.ResizeObserver = class {
    constructor(callback: ResizeObserverCallback) { resize = () => callback([], this as unknown as ResizeObserver); }
    observe() {} unobserve() {} disconnect() {}
  } as unknown as typeof ResizeObserver;
  globalThis.requestAnimationFrame = (callback) => { frames.set(++frameId, callback); return frameId; };
  globalThis.cancelAnimationFrame = (id) => { frames.delete(id); };
  const flush = () => act(() => {
    for (let round = 0; frames.size && round < 5; round++) {
      const pending = [...frames.values()]; frames.clear(); pending.forEach((callback) => callback(0));
    }
  });
  function Harness() {
    const scroll = useAutoScroll({ messages: [], itemCount: 1 });
    return <div data-testid='scroller' ref={scroll.handleScrollerRef} onScroll={scroll.handleScroll}
      onWheel={scroll.handleWheel} onPointerDown={scroll.handlePointerDown} onKeyDown={scroll.handleKeyDown}>
      <div ref={scroll.handleContentRef}>Content</div>
    </div>;
  }
  try {
    const { getByTestId } = render(<Harness />);
    const scroller = getByTestId('scroller');
    let height = 1000;
    Object.defineProperties(scroller, {
      scrollHeight: { configurable: true, get: () => height },
      clientHeight: { configurable: true, get: () => 500 },
    });
    scroller.scrollTo = ((options: ScrollToOptions) => { scroller.scrollTop = options.top ?? 0; }) as typeof scroller.scrollTo;
    flush();
    scroller.scrollTop = 450;
    fireEvent.scroll(scroller);
    height = 1200;
    act(() => resize?.()); flush();
    expect(scroller.scrollTop).toBe(700);

    fireEvent.wheel(scroller, { deltaY: -100 });
    scroller.scrollTop = 300;
    fireEvent.scroll(scroller);
    height = 1400;
    act(() => resize?.()); flush();
    expect(scroller.scrollTop).toBe(300);
  } finally {
    cleanup();
    globalThis.ResizeObserver = originalObserver;
    globalThis.requestAnimationFrame = originalRaf;
    globalThis.cancelAnimationFrame = originalCancel;
  }
});
