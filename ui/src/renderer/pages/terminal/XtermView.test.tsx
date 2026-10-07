import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { ipcBridge } from '@/common';
import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import { parseTerminalId } from '@/common/types/ids';
import XtermView, { type XtermViewHandle } from './XtermView';
import { decodeBase64ToString, encodeStringToBase64 } from './terminalEncoding';

const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(fn => fn()); });
function deferredCalls<T>() {
  const pending: Array<{ resolve: (value: T) => void; reject: (error: unknown) => void }> = [];
  return { pending, invoke: () => new Promise<T>((resolve, reject) => pending.push({ resolve, reject })) };
}
function events<T>(source: { on: (callback: (event: T) => void) => () => void }) {
  const listeners = new Set<(event: T) => void>();
  const spy = spyOn(source, 'on').mockImplementation(fn => { listeners.add(fn); return () => { listeners.delete(fn); }; });
  restore.push(() => spy.mockRestore());
  return { listeners, emit: (event: T) => { for (const fn of [...listeners]) fn(event); } };
}
const id = parseTerminalId('019b0000-0000-7000-8000-000000000001');
const snapshot = (text: string): ITerminalSession => ({
  terminal_id: id, name: 'fixture', cwd: '/fixture', command: 'shell', args: [],
  cols: 80, rows: 24, last_status: 'running', created_at: 1, updated_at: 1,
  scrollback_b64: encodeStringToBase64(text),
});
function fixture(isRunning = true) {
  const loads = deferredCalls<ITerminalSession>();
  const sizes = deferredCalls<void>();
  const inputs = deferredCalls<void>();
  const get = spyOn(ipcBridge.terminal.get, 'invoke').mockImplementation(loads.invoke);
  const resize = spyOn(ipcBridge.terminal.resize, 'invoke').mockImplementation(sizes.invoke);
  const input = spyOn(ipcBridge.terminal.input, 'invoke').mockImplementation(inputs.invoke);
  const log = spyOn(console, 'error').mockImplementation(() => {});
  const output = events(ipcBridge.terminal.onOutput);
  const exit = events(ipcBridge.terminal.onExit);
  const reconnect = events<void>(ipcBridge.terminal.onReconnected);
  const writes: string[] = [];
  let rawInput!: (data: string) => void;
  let measurement = { cols: 80, rows: 24 };
  // Keep the actual component, input pipeline and streaming decoder. Only the
  // canvas/layout boundary is inert; every backend operation is a deferred spy.
  const open = spyOn(Terminal.prototype, 'open').mockImplementation(function (this: Terminal, container) {
    Object.defineProperties(container, { clientWidth: { value: 800 }, clientHeight: { value: 480 } });
    Object.defineProperties(this, {
      cols: { get: () => measurement.cols }, rows: { get: () => measurement.rows },
      onData: { value: (fn: (data: string) => void) => { rawInput = fn; return { dispose: () => {} }; } },
    });
  });
  const loadAddon = spyOn(Terminal.prototype, 'loadAddon').mockImplementation(() => {});
  const write = spyOn(Terminal.prototype, 'write').mockImplementation(data => { writes.push(String(data)); });
  const reset = spyOn(Terminal.prototype, 'reset').mockImplementation(() => { writes.length = 0; });
  const focus = spyOn(Terminal.prototype, 'focus').mockImplementation(() => {});
  const fit = spyOn(FitAddon.prototype, 'fit').mockImplementation(() => {});
  const frames = new Map<number, FrameRequestCallback>();
  let frameId = 0;
  const raf = spyOn(globalThis, 'requestAnimationFrame').mockImplementation(fn => { frames.set(++frameId, fn); return frameId; });
  const cancelRaf = spyOn(globalThis, 'cancelAnimationFrame').mockImplementation(n => { frames.delete(n); });
  let measure!: () => void;
  const observer = Object.getOwnPropertyDescriptor(globalThis, 'ResizeObserver');
  Object.defineProperty(globalThis, 'ResizeObserver', { configurable: true, value: class {
    constructor(fn: () => void) { measure = fn; }
    observe() {} disconnect() {}
  } });
  restore.push(() => { if (observer) Object.defineProperty(globalThis, 'ResizeObserver', observer); else Reflect.deleteProperty(globalThis, 'ResizeObserver'); });
  for (const spy of [get, resize, input, log, open, loadAddon, write, reset, focus, fit, raf, cancelRaf]) restore.push(() => spy.mockRestore());
  const apiRef: { current: XtermViewHandle | null } = { current: null };
  const failure = mock();
  const escalate = mock();
  const view = render(<XtermView sessionId={id} isRunning={isRunning} apiRef={apiRef} onResizeFailure={failure} onEscalateShell={escalate} />);
  const frame = () => act(() => { const work = [...frames.values()]; frames.clear(); work.forEach(fn => fn(0)); });
  frame(); frame();
  return {
    view, api: apiRef.current!, apiRef, loads: loads.pending, sizes: sizes.pending, inputs: inputs.pending,
    get, resize, input, log, failure, escalate, writes, output, exit, reconnect,
    raw: (text: string) => act(() => rawInput(text)),
    fit: (cols: number) => { measurement = { cols, rows: 24 }; act(() => measure()); frame(); },
    live: (text: string) => act(() => output.emit({ terminal_id: id, data_b64: encodeStringToBase64(text) })),
  };
}

test('snapshot precedes buffered live bytes and exit; UTF-8 continues across chunks', async () => {
  const f = fixture();
  act(() => f.output.emit({ terminal_id: id, data_b64: btoa('\xe4\xb8') }));
  await act(async () => { f.loads[0]!.resolve(snapshot('history:')); });
  act(() => f.output.emit({ terminal_id: id, data_b64: btoa('\xad') }));
  expect(f.writes.join('')).toBe('history:中');
  act(() => f.exit.emit({ terminal_id: id, exit_code: 3 }));
  expect(f.writes.join('')).toContain('history:中\r\n\x1b[2m[process exited with code 3]');
});

test('only the newest reconnect replay can publish; a failed replay releases live output', async () => {
  const f = fixture();
  act(() => { f.reconnect.emit(); f.reconnect.emit(); });
  f.live('live');
  await act(async () => { f.loads[2]!.resolve(snapshot('new:')); });
  await act(async () => { f.loads[0]!.resolve(snapshot('initial')); f.loads[1]!.resolve(snapshot('old')); });
  expect(f.writes.join('')).toBe('new:live');
  act(() => f.reconnect.emit());
  f.live('kept');
  await act(async () => { f.loads[3]!.reject('offline'); });
  expect(f.writes.join('')).toBe('kept');
});

test('return to the acknowledged size supersedes an in-flight resize', async () => {
  const f = fixture();
  await act(async () => { f.sizes[0]!.resolve(); });
  f.fit(100); f.fit(80);
  await act(async () => { f.sizes[1]!.resolve(); });
  expect(f.resize.mock.calls.map(([p]) => p.cols)).toEqual([80, 100, 80]);
  await act(async () => { f.sizes[2]!.resolve(); });
  f.fit(80);
  expect(f.resize).toHaveBeenCalledTimes(3);
});

test('input waits for activation, coalesces raw bytes and stays serial after activation', async () => {
  const f = fixture();
  const first = f.api.writeToPty('你');
  const second = f.api.writeToPty('好\r');
  expect(f.input).not.toHaveBeenCalled();
  await act(async () => { f.sizes[0]!.resolve(); });
  expect(decodeBase64ToString(f.input.mock.calls[0]![0].data_b64)).toBe('你好\r');
  const third = f.api.writeToPty('next');
  expect(f.input).toHaveBeenCalledTimes(1);
  await act(async () => { f.inputs[0]!.resolve(); await Promise.all([first, second]); });
  expect(f.input).toHaveBeenCalledTimes(2);
  await act(async () => { f.inputs[1]!.resolve(); await third; });
});

test('exit rejects queued input before activation and ignores late resize failure', async () => {
  const f = fixture();
  let result = 'pending';
  void f.api.writeToPty('command\r').then(() => { result = 'sent'; }, () => { result = 'rejected'; });
  await act(async () => { f.exit.emit({ terminal_id: id, exit_code: 0 }); });
  expect(result).toBe('rejected');
  f.raw('\x03'); f.raw('\x03'); f.raw('\x03');
  await act(async () => { f.sizes[0]!.reject('PTY exited'); f.loads[0]!.resolve(snapshot('last output')); });
  await expect(f.api.writeToPty('late')).rejects.toThrow('not running');
  expect(f.input).not.toHaveBeenCalled();
  expect(f.failure).not.toHaveBeenCalled();
  expect(f.escalate).not.toHaveBeenCalled();
  expect(f.writes.join('')).toContain('last output\r\n\x1b[2m[process exited');
});

test('exhausted activation rejects queued input once instead of leaving the submit pending', async () => {
  const f = fixture();
  let result = 'pending';
  void f.api.writeToPty('command').catch(() => { result = 'rejected'; });
  for (let attempt = 0; attempt < 3; attempt++) {
    await waitFor(() => expect(f.sizes.length).toBe(attempt + 1));
    await act(async () => { f.sizes[attempt]!.reject(new Error('offline')); });
  }
  expect(result).toBe('rejected');
  expect(f.failure).toHaveBeenCalledTimes(1);
  expect(f.input).not.toHaveBeenCalled();
});

test('unmount rejects active input and silences its late failure and retained subscriptions', async () => {
  const f = fixture();
  await act(async () => { f.sizes[0]!.resolve(); });
  const pending = f.api.writeToPty('command').catch(() => 'rejected');
  const retained = [...f.output.listeners][0]!;
  f.view.unmount();
  expect(await pending).toBe('rejected');
  await act(async () => { f.inputs[0]!.reject('late input error'); f.loads[0]!.resolve(snapshot('late')); });
  retained({ terminal_id: id, data_b64: encodeStringToBase64('late') });
  expect(f.log).not.toHaveBeenCalled();
  expect(f.failure).not.toHaveBeenCalled();
  expect(f.writes).toEqual([]);
  expect(f.apiRef.current).toBeNull();
  expect([f.output, f.exit, f.reconnect].map(e => e.listeners.size)).toEqual([0, 0, 0]);
});

test('an exited view replays history without resize, input or escalation', async () => {
  const f = fixture(false);
  f.raw('\x03'); f.fit(100);
  await act(async () => { f.loads[0]!.resolve(snapshot('history')); });
  await expect(f.api.writeToPty('input')).rejects.toThrow('not running');
  expect(f.writes.join('')).toBe('history');
  expect(f.resize).not.toHaveBeenCalled();
  expect(f.input).not.toHaveBeenCalled();
  expect(f.escalate).not.toHaveBeenCalled();
});
