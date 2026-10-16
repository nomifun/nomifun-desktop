import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import type { RefObject } from 'react';
import * as webviewApi from '@tauri-apps/api/webview';
import type { EventCallback } from '@tauri-apps/api/event';
import { PhysicalPosition } from '@tauri-apps/api/dpi';
import * as runtime from '@/common/adapter/tauriRuntime';
import { useNativeFileDrop } from './useNativeFileDrop';

type Payload = webviewApi.DragDropEvent;
let runtimeSpy: ReturnType<typeof spyOn<typeof runtime, 'isTauriRuntime'>>;
let webviewSpy: ReturnType<typeof spyOn<typeof webviewApi, 'getCurrentWebview'>>;
let hitSpy: ReturnType<typeof spyOn<typeof document, 'elementFromPoint'>>;
let handlers: EventCallback<Payload>[];
let stops: ReturnType<typeof mock>[];
let resolveSubscription: (() => void) | undefined;
let deferSubscription: boolean;
let originalDpr: PropertyDescriptor | undefined;
let target: HTMLDivElement;
let child: HTMLTextAreaElement;
let targetRef: RefObject<HTMLElement | null>;

beforeEach(() => {
  handlers = [];
  stops = [];
  deferSubscription = false;
  resolveSubscription = undefined;
  originalDpr = Object.getOwnPropertyDescriptor(window, 'devicePixelRatio');
  Object.defineProperty(window, 'devicePixelRatio', { value: 2, configurable: true });
  target = document.createElement('div');
  child = document.createElement('textarea');
  target.append(child);
  document.body.append(target);
  targetRef = { current: target };
  target.getBoundingClientRect = () => new DOMRect(10, 20, 100, 60);
  hitSpy = spyOn(document, 'elementFromPoint').mockReturnValue(child);
  runtimeSpy = spyOn(runtime, 'isTauriRuntime').mockReturnValue(true);
  webviewSpy = spyOn(webviewApi, 'getCurrentWebview').mockReturnValue({
    onDragDropEvent: (handler: EventCallback<Payload>) => {
      handlers.push(handler);
      const stop = mock(() => {});
      stops.push(stop);
      if (deferSubscription) return new Promise<() => void>(resolve => { resolveSubscription = () => resolve(stop); });
      return Promise.resolve(stop);
    },
  } as unknown as webviewApi.Webview);
});

afterEach(() => {
  cleanup();
  target.remove();
  runtimeSpy.mockRestore();
  webviewSpy.mockRestore();
  hitSpy.mockRestore();
  if (originalDpr) Object.defineProperty(window, 'devicePixelRatio', originalDpr);
  else Reflect.deleteProperty(window, 'devicePixelRatio');
});

async function flushSubscription() {
  await act(async () => { await Promise.resolve(); });
}

function emit(payload: Payload, index = 0) {
  act(() => { handlers[index]!({ event: 'tauri://drag-drop', id: index, payload }); });
}

const position = (x = 60, y = 80) => new PhysicalPosition(x, y);

test('WebUI and disabled surfaces do not subscribe to native drops', async () => {
  runtimeSpy.mockReturnValue(false);
  const onDrop = mock(() => {});
  const options = { targetRef, onHoverChange: mock(() => {}), onDrop };
  const hook = renderHook(({ enabled }) => useNativeFileDrop({ ...options, enabled }), { initialProps: { enabled: true } });
  await flushSubscription();
  expect(webviewSpy).not.toHaveBeenCalled();
  runtimeSpy.mockReturnValue(true);
  hook.rerender({ enabled: false });
  await flushSubscription();
  expect(webviewSpy).not.toHaveBeenCalled();
  expect(onDrop).not.toHaveBeenCalled();
});

test('native hover and drop use CSS coordinates and only notify on hover changes', async () => {
  const onHoverChange = mock(() => {});
  const onDrop = mock(() => {});
  renderHook(() => useNativeFileDrop({ targetRef, onHoverChange, onDrop }));
  await flushSubscription();
  emit({ type: 'enter', paths: ['C:/work/a.txt'], position: position() });
  emit({ type: 'over', position: position(100, 100) });
  expect(hitSpy).toHaveBeenLastCalledWith(50, 50);
  expect(onHoverChange.mock.calls).toEqual([[true]]);
  emit({ type: 'over', position: position(240, 100) });
  expect(onHoverChange.mock.calls).toEqual([[true], [false]]);
  emit({ type: 'over', position: position() });
  emit({ type: 'drop', paths: ['C:/work/a.txt', 'C:/work/b.png'], position: position() });
  expect(onHoverChange.mock.calls).toEqual([[true], [false], [true], [false]]);
  expect(onDrop.mock.calls).toEqual([[['C:/work/a.txt', 'C:/work/b.png']]]);
});

test('leave clears hover and empty or outside drops add no files', async () => {
  const onHoverChange = mock(() => {});
  const onDrop = mock(() => {});
  renderHook(() => useNativeFileDrop({ targetRef, onHoverChange, onDrop }));
  await flushSubscription();
  emit({ type: 'enter', paths: ['a'], position: position() });
  emit({ type: 'leave' });
  emit({ type: 'drop', paths: [], position: position() });
  emit({ type: 'drop', paths: ['a'], position: position(0, 0) });
  expect(onHoverChange.mock.calls).toEqual([[true], [false]]);
  expect(onDrop).not.toHaveBeenCalled();
});

test('native drags without file paths never show attachment feedback', async () => {
  const onHoverChange = mock(() => {});
  const onDrop = mock(() => {});
  renderHook(() => useNativeFileDrop({ targetRef, onHoverChange, onDrop }));
  await flushSubscription();
  emit({ type: 'enter', paths: [], position: position() });
  emit({ type: 'over', position: position() });
  emit({ type: 'drop', paths: [], position: position() });
  emit({ type: 'over', position: position() });
  expect(onHoverChange).not.toHaveBeenCalled();
  expect(onDrop).not.toHaveBeenCalled();
});

test('covered, hidden and disconnected targets reject native drops', async () => {
  const onDrop = mock(() => {});
  renderHook(() => useNativeFileDrop({ targetRef, onHoverChange: mock(() => {}), onDrop }));
  await flushSubscription();
  const drop = () => emit({ type: 'drop', paths: ['a'], position: position() });
  hitSpy.mockReturnValue(document.body);
  drop();
  hitSpy.mockReturnValue(null);
  drop();
  hitSpy.mockReturnValue(child);
  target.style.visibility = 'hidden';
  drop();
  target.style.visibility = '';
  target.getBoundingClientRect = () => new DOMRect(10, 20, 0, 0);
  drop();
  target.getBoundingClientRect = () => new DOMRect(10, 20, 100, 60);
  target.remove();
  drop();
  targetRef.current = null;
  drop();
  expect(onDrop).not.toHaveBeenCalled();
});

test('multiple mounted drop surfaces only receive their own hit target', async () => {
  const other = document.createElement('div');
  document.body.append(other);
  other.getBoundingClientRect = () => new DOMRect(120, 20, 100, 60);
  const onDrop = mock(() => {});
  const otherDrop = mock(() => {});
  renderHook(() => useNativeFileDrop({ targetRef, onHoverChange: mock(() => {}), onDrop }));
  renderHook(() => useNativeFileDrop({ targetRef: { current: other }, onHoverChange: mock(() => {}), onDrop: otherDrop }));
  await flushSubscription();
  const payload: Payload = { type: 'drop', paths: ['a'], position: position() };
  emit(payload, 0);
  emit(payload, 1);
  expect(onDrop).toHaveBeenCalledTimes(1);
  expect(otherDrop).not.toHaveBeenCalled();
  other.remove();
});

test('updated callbacks and display scaling apply without resubscribing', async () => {
  const firstDrop = mock(() => {});
  const nextDrop = mock(() => {});
  const firstHover = mock(() => {});
  const nextHover = mock(() => {});
  const hook = renderHook(({ onDrop, onHoverChange }) => useNativeFileDrop({ targetRef, onDrop, onHoverChange }), {
    initialProps: { onDrop: firstDrop, onHoverChange: firstHover },
  });
  await flushSubscription();
  hook.rerender({ onDrop: nextDrop, onHoverChange: nextHover });
  Object.defineProperty(window, 'devicePixelRatio', { value: 1.25, configurable: true });
  emit({ type: 'enter', paths: ['a'], position: position(50, 50) });
  emit({ type: 'drop', paths: ['a'], position: position(50, 50) });
  expect(webviewSpy).toHaveBeenCalledTimes(1);
  expect(hitSpy).toHaveBeenLastCalledWith(40, 40);
  expect(firstDrop).not.toHaveBeenCalled();
  expect(firstHover).not.toHaveBeenCalled();
  expect(nextDrop.mock.calls).toEqual([[['a']]]);
  expect(nextHover.mock.calls).toEqual([[true], [false]]);
});

test('disabling clears hover, unsubscribes and ignores callbacks already queued', async () => {
  const onHoverChange = mock(() => {});
  const onDrop = mock(() => {});
  const hook = renderHook(({ enabled }) => useNativeFileDrop({ targetRef, enabled, onHoverChange, onDrop }), {
    initialProps: { enabled: true },
  });
  await flushSubscription();
  emit({ type: 'enter', paths: ['a'], position: position() });
  hook.rerender({ enabled: false });
  emit({ type: 'drop', paths: ['a'], position: position() });
  expect(stops[0]).toHaveBeenCalledTimes(1);
  expect(onHoverChange.mock.calls).toEqual([[true], [false]]);
  expect(onDrop).not.toHaveBeenCalled();
});

test('unmount cleans up a delayed subscription and ignores its late events', async () => {
  deferSubscription = true;
  const onDrop = mock(() => {});
  const hook = renderHook(() => useNativeFileDrop({ targetRef, onHoverChange: mock(() => {}), onDrop }));
  await flushSubscription();
  expect(resolveSubscription).toBeDefined();
  hook.unmount();
  await act(async () => { resolveSubscription!(); await Promise.resolve(); });
  emit({ type: 'drop', paths: ['a'], position: position() });
  expect(stops[0]).toHaveBeenCalledTimes(1);
  expect(onDrop).not.toHaveBeenCalled();
});

test('unmount before the dynamic import resolves never starts a subscription', async () => {
  const hook = renderHook(() => useNativeFileDrop({ targetRef, onHoverChange: mock(() => {}), onDrop: mock(() => {}) }));
  hook.unmount();
  await flushSubscription();
  expect(webviewSpy).not.toHaveBeenCalled();
});
