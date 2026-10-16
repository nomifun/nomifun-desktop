/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { act, cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { MemoryRouter } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import MarkdownView from '@/renderer/components/Markdown';
import * as streamingMarkdown from '@/renderer/components/Markdown/streamingMarkdown';
import { StreamingProse, StreamingProseProvider, useStreamingProse, useStreamingProseIngress } from '.';

const originalRaf = globalThis.requestAnimationFrame;
const originalCancel = globalThis.cancelAnimationFrame;
const originalMatchMedia = window.matchMedia;
const originalVisibility = Object.getOwnPropertyDescriptor(document, 'visibilityState');
let frames: Map<number, FrameRequestCallback>;
let now = 0;
let nextId = 0;
let reduced = false;
let mediaListeners: Set<() => void>;
let nowSpy: ReturnType<typeof spyOn<typeof performance, 'now'>>;

beforeEach(() => {
  now = 0;
  nextId = 0;
  reduced = false;
  frames = new Map();
  mediaListeners = new Set();
  nowSpy = spyOn(performance, 'now').mockImplementation(() => now);
  globalThis.requestAnimationFrame = callback => { frames.set(++nextId, callback); return nextId; };
  globalThis.cancelAnimationFrame = id => { frames.delete(id); };
  window.matchMedia = query => ({
    media: query, get matches() { return reduced; },
    addEventListener: (_event: string, callback: () => void) => { mediaListeners.add(callback); },
    removeEventListener: (_event: string, callback: () => void) => { mediaListeners.delete(callback); },
  }) as unknown as MediaQueryList;
  Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' });
});

afterEach(() => {
  cleanup();
  nowSpy.mockRestore();
  globalThis.requestAnimationFrame = originalRaf;
  globalThis.cancelAnimationFrame = originalCancel;
  window.matchMedia = originalMatchMedia;
  if (originalVisibility) Object.defineProperty(document, 'visibilityState', originalVisibility);
  else Reflect.deleteProperty(document, 'visibilityState');
});

function frame(elapsed: number) {
  act(() => {
    now += elapsed;
    const pending = [...frames.values()];
    frames.clear();
    pending.forEach(callback => callback(now));
  });
}

type Input = Parameters<typeof useStreamingProse>[0];
function Readout(props: Input) {
  const cursor = useStreamingProse(props);
  return <span data-testid='prose'>{props.text.slice(0, cursor)}</span>;
}
function Harness({ scope = 'session-a', ...props }: Input & { scope?: string }) {
  return <StreamingProseProvider scope={scope}><Readout {...props} /></StreamingProseProvider>;
}

test('new live prose is paced, but mounted history remains immediately readable', () => {
  const text = '平滑推出的正文。'.repeat(20);
  const page = render(<Harness identity='source' text={text} animate />);
  expect(page.getByTestId('prose').textContent).toBe('');
  frame(32);
  expect(page.getByTestId('prose').textContent?.length).toBeGreaterThan(0);
  expect(page.getByTestId('prose').textContent?.length).toBeLessThan(text.length);
  frame(568);
  expect(page.getByTestId('prose').textContent).toBe(text);
  page.rerender(<Harness identity='history' text={text} animate={false} />);
  expect(page.getByTestId('prose').textContent).toBe(text);
});

test('a process/public DOM remount preserves progress for exact canonical source ranges', () => {
  const introduction = '这是已经显示的过程说明。';
  const conclusion = '这是最后的公开回答。'.repeat(20);
  const source = introduction + conclusion;
  function Scene({ final = false }) {
    return <StreamingProseProvider scope='session'>
      {final ? <Readout key='public' identity='source' text={conclusion} sourceText={source}
        sourceOffset={introduction.length} animate />
        : <Readout key='process' identity='source' text={introduction} sourceText={source} animate />}
    </StreamingProseProvider>;
  }
  const page = render(<Scene />);
  frame(200);
  const sourceVisible = page.getByTestId('prose').textContent ?? '';
  expect(sourceVisible).toBe(introduction);
  page.rerender(<Scene final />);
  const finalVisible = page.getByTestId('prose').textContent ?? '';
  expect(finalVisible.length).toBeGreaterThan(0);
  expect(finalVisible.length).toBeLessThan(conclusion.length);
  frame(400);
  expect(page.getByTestId('prose').textContent).toBe(conclusion);
});

test('noncontiguous filtered projections render immediately without advancing another view cursor', () => {
  const text = '这是完整的正文。'.repeat(20);
  const page = render(<Harness identity='source' text={text} animate />);
  frame(32);
  const before = page.getByTestId('prose').textContent;
  page.rerender(<Harness identity='source' text='过滤后的正文' sourceText={text} animate />);
  expect(page.getByTestId('prose').textContent).toBe('过滤后的正文');
  page.rerender(<Harness identity='source' text={text} animate />);
  expect(page.getByTestId('prose').textContent).toBe(before);
});

test('authoritative cancellation immediately shows received text and cancels the animation', () => {
  const text = '已收到的正文。'.repeat(20);
  const page = render(<Harness identity='source' text={text} animate />);
  frame(32);
  page.rerender(<Harness identity='source' text={text} animate immediate />);
  expect(page.getByTestId('prose').textContent).toBe(text);
  expect(frames.size).toBe(0);
  frame(1000);
  expect(page.getByTestId('prose').textContent).toBe(text);
});

test('switching Session scope disposes old animation and never imports display progress into new history', () => {
  const text = '新的 Session 历史。'.repeat(20);
  const page = render(<Harness identity='source' text={text} animate />);
  frame(32);
  page.rerender(<Harness scope='session-b' identity='source' text={text} animate={false} />);
  expect(page.getByTestId('prose').textContent).toBe(text);
  expect(frames.size).toBe(0);
});

test('visibility and reduced-motion preference changes flush once without replay on return', () => {
  const text = '尊重环境的正文。'.repeat(20);
  const page = render(<Harness identity='source' text={text} animate />);
  frame(32);
  act(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'hidden' });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  expect(page.getByTestId('prose').textContent).toBe(text);
  expect(frames.size).toBe(0);
  act(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  page.rerender(<Harness identity='source' text={text + '新实时增量'} animate />);
  act(() => { reduced = true; mediaListeners.forEach(listener => listener()); });
  expect(page.getByTestId('prose').textContent).toBe(text + '新实时增量');
  expect(frames.size).toBe(0);
});

test('an initial reduced-motion preference renders the complete source before the first animation frame', () => {
  reduced = true;
  const text = '立即可读的正文。';
  const page = render(<Harness identity='source' text={text} animate />);
  expect(page.getByTestId('prose').textContent).toBe(text);
  expect(frames.size).toBe(0);
});

test('live ingress is explicit and isolated by Session while other component consumers need no provider', () => {
  let ingress: ReturnType<typeof useStreamingProseIngress> | undefined;
  function Capture() { ingress = useStreamingProseIngress(); return null; }
  const page = render(<StreamingProseProvider scope='session-a'><Capture /></StreamingProseProvider>);
  expect(ingress?.isLive('source')).toBe(false);
  ingress?.markLive('source');
  expect(ingress?.isLive('source')).toBe(true);
  ingress?.revoke('source');
  expect(ingress?.isLive('source')).toBe(false);
  ingress?.markLive('source');
  page.rerender(<StreamingProseProvider scope='session-b'><Capture /></StreamingProseProvider>);
  expect(ingress?.isLive('source')).toBe(false);
  page.rerender(<Readout identity='source' text='完整正文' animate />);
  expect(page.getByTestId('prose').textContent).toBe('完整正文');
});

test('standalone prose outside a provider preserves the default Markdown renderer output', () => {
  const source = 'Standalone **Markdown**.\n\n- A list item\n- Another item';
  const page = render(<MemoryRouter>
    <MarkdownView>{source}</MarkdownView>
    <StreamingProse identity='standalone' text={source} animate />
  </MemoryRouter>);
  const hosts = page.container.querySelectorAll('.markdown-shadow');
  expect(hosts).toHaveLength(2);
  const body = (host: Element) => host.shadowRoot!.querySelector('.markdown-article')!;
  expect(body(hosts[1]).innerHTML).toBe(body(hosts[0]).innerHTML);
  expect(frames.size).toBe(0);
});

test('multiple canonical append updates in one paint trigger only one new Markdown parse on the shared frame', async () => {
  const i18n = createInstance();
  await i18n.init({ lng: 'en-US', resources: {} });
  let parses = 0;
  const originalParse = streamingMarkdown.createStreamingMarkdown;
  const parseSpy = spyOn(streamingMarkdown, 'createStreamingMarkdown').mockImplementation((...args) => {
    parses++;
    return originalParse(...args);
  });
  try {
    const history = 'Previously visible.';
    const view = (text: string, animate = true) => <I18nextProvider i18n={i18n}><StreamingProseProvider scope='session'>
      <MemoryRouter><StreamingProse identity='source' text={text} animate={animate} /></MemoryRouter>
    </StreamingProseProvider></I18nextProvider>;
    const page = render(view(history, false));
    expect(parses).toBe(1);
    page.rerender(view(history + '甲'));
    page.rerender(view(history + '甲乙'));
    page.rerender(view(history + '甲乙丙'));
    expect(parses).toBe(1);
    frame(16);
    expect(parses).toBe(2);
    frame(16);
    expect(parses).toBe(2);
  } finally {
    parseSpy.mockRestore();
  }
});
