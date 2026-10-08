/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'bun:test';
import { useAutoScroll } from './useAutoScroll';

const originalObserver = globalThis.ResizeObserver;
const originalRaf = globalThis.requestAnimationFrame;
const originalCancel = globalThis.cancelAnimationFrame;
let frames: Map<number, FrameRequestCallback>;
let observers: Set<() => void>;
let frameId: number;

beforeEach(() => {
  frames = new Map();
  observers = new Set();
  frameId = 0;
  globalThis.ResizeObserver = class {
    private notify: () => void;
    constructor(callback: ResizeObserverCallback) {
      this.notify = () => callback([], this as unknown as ResizeObserver);
      observers.add(this.notify);
    }
    observe() {} unobserve() {}
    disconnect() { observers.delete(this.notify); }
  } as unknown as typeof ResizeObserver;
  globalThis.requestAnimationFrame = callback => { frames.set(++frameId, callback); return frameId; };
  globalThis.cancelAnimationFrame = id => { frames.delete(id); };
});

afterEach(() => {
  cleanup();
  globalThis.ResizeObserver = originalObserver;
  globalThis.requestAnimationFrame = originalRaf;
  globalThis.cancelAnimationFrame = originalCancel;
});

const flushFrame = () => act(() => {
  const pending = [...frames.values()];
  frames.clear();
  pending.forEach(callback => callback(0));
});
const flush = () => { for (let round = 0; frames.size && round < 10; round++) flushFrame(); };
const resize = () => act(() => { for (const notify of [...observers]) notify(); });

function Harness({ running = true, follow = true, turnId = 'current-turn' }) {
  const scroll = useAutoScroll({ messages: [], itemCount: 1,
    followTurn: follow ? { id: turnId, running } : undefined });
  return <>
    <div data-testid='scroller' ref={scroll.handleScrollerRef} onScroll={scroll.handleScroll}
      onWheel={scroll.handleWheel} onPointerDown={scroll.handlePointerDown} onKeyDown={scroll.handleKeyDown}>
      <div ref={scroll.handleContentRef}>
        <div data-testid='thinking-body' style={{ overflowY: 'auto' }}>Reasoning</div>
        <div data-testid='older-message'>Older reply</div>
        <button onClick={event => scroll.scrollElementIntoView(event.currentTarget.previousElementSibling as HTMLElement,
          { behavior: 'auto' })}>Read older reply</button>
      </div>
    </div>
    {scroll.showScrollButton && <button onClick={() => scroll.scrollToBottom('auto')}>Latest reply</button>}
  </>;
}

function setup(follow = true) {
  const page = render(<Harness follow={follow} />);
  const scroller = page.getByTestId('scroller');
  let height = 1500;
  Object.defineProperties(scroller, {
    scrollHeight: { configurable: true, get: () => height },
    clientHeight: { configurable: true, value: 500 },
  });
  scroller.scrollTo = ((options: ScrollToOptions) => {
    scroller.scrollTop = Math.max(0, Math.min(options.top ?? 0, height - 500));
  }) as typeof scroller.scrollTo;
  flush();
  fireEvent.scroll(scroller);
  return { page, scroller, setHeight: (value: number) => { height = value; } };
}

test('a followed turn finishes at the reply bottom after process collapse and delayed Markdown layout', () => {
  const { page, scroller, setHeight } = setup();
  fireEvent.pointerDown(scroller);
  setHeight(900);
  page.rerender(<Harness running={false} />);
  // The browser clamps scrollTop as the process body disappears. This is a
  // layout scroll, even if the user clicked inside the list moments earlier.
  scroller.scrollTop = 400;
  fireEvent.scroll(scroller);
  flushFrame();
  setHeight(1800);
  resize();
  flush();
  expect(scroller.scrollTop).toBe(1300);
  expect(page.queryByRole('button', { name: 'Latest reply' })).toBeNull();
});

test('the default main-chat policy keeps its existing pointer resize guard', () => {
  const { page, scroller, setHeight } = setup(false);
  fireEvent.pointerDown(scroller);
  setHeight(1800);
  page.rerender(<Harness running={false} follow={false} />);
  resize();
  flush();
  expect(scroller.scrollTop).toBe(1000);
});

test('late final-reply layout keeps following even when the first completion frame was already at bottom', () => {
  const { page, scroller, setHeight } = setup();
  fireEvent.pointerDown(scroller);
  page.rerender(<Harness running={false} />);
  flush();
  setHeight(1900);
  resize();
  flush();
  expect(scroller.scrollTop).toBe(1400);
});

test('reading earlier messages pauses streaming and completion until the user returns to latest', () => {
  const { page, scroller, setHeight } = setup();
  fireEvent.wheel(scroller, { deltaY: -100 });
  scroller.scrollTop = 350;
  fireEvent.scroll(scroller);
  setHeight(1800);
  page.rerender(<Harness running={false} />);
  resize();
  flush();
  expect(scroller.scrollTop).toBe(350);
  fireEvent.click(page.getByRole('button', { name: 'Latest reply' }));
  expect(scroller.scrollTop).toBe(1300);
  setHeight(1900);
  resize();
  flush();
  expect(scroller.scrollTop).toBe(1400);
});

test('scrolling a nested thinking body does not pause the transcript when that body collapses', () => {
  const { page, scroller, setHeight } = setup();
  const thought = page.getByTestId('thinking-body');
  Object.defineProperties(thought, {
    scrollHeight: { configurable: true, value: 600 },
    clientHeight: { configurable: true, value: 150 },
  });
  thought.scrollTop = 200;
  fireEvent.wheel(thought, { deltaY: -80 });
  setHeight(900);
  scroller.scrollTop = 400;
  setHeight(1800);
  // The outer scroll event can arrive after the final reply has grown.
  fireEvent.scroll(scroller);
  page.rerender(<Harness running={false} />);
  resize();
  flush();
  expect(scroller.scrollTop).toBe(1300);
});

test('explicit message navigation keeps its reading position across turn completion', () => {
  const { page, scroller, setHeight } = setup();
  page.getByTestId('older-message').scrollIntoView = () => { scroller.scrollTop = 200; };
  fireEvent.click(page.getByRole('button', { name: 'Read older reply' }));
  fireEvent.scroll(scroller);
  setHeight(1800);
  page.rerender(<Harness running={false} />);
  resize();
  flush();
  expect(scroller.scrollTop).toBe(200);
});

test('a completion belonging to a different turn cannot override a pending user scroll', () => {
  const { page, scroller, setHeight } = setup();
  fireEvent.wheel(scroller, { deltaY: -100 });
  setHeight(1800);
  page.rerender(<Harness running={false} turnId='older-turn' />);
  resize();
  flush();
  expect(scroller.scrollTop).toBe(1000);
});

test('an upward gesture cancels completion follow between its two layout frames', () => {
  const { page, scroller, setHeight } = setup();
  page.rerender(<Harness running={false} />);
  flushFrame();
  fireEvent.wheel(scroller, { deltaY: -100 });
  setHeight(1800);
  resize();
  flush();
  expect(scroller.scrollTop).toBe(1000);
});
