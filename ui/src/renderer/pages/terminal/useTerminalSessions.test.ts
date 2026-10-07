import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { ipcBridge } from '@/common';
import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import { parseConversationId, parseTerminalId } from '@/common/types/ids';
import { emitter } from '@/renderer/utils/emitter';
import { useTerminalSessions } from './useTerminalSessions';

const session = (n: number): ITerminalSession => ({
  terminal_id: parseTerminalId('019b0000-0000-7000-8000-' + String(n).padStart(12, '0')),
  name: 'terminal ' + n, cwd: '/fixture', command: 'shell', args: [], cols: 80, rows: 24,
  created_at: 1, updated_at: 1, last_status: 'running',
});
function queue<T>() {
  const pending: Array<{ resolve: (value: T) => void; reject: (error: unknown) => void }> = [];
  return { pending, invoke: () => new Promise<T>((resolve, reject) => { pending.push({ resolve, reject }); }) };
}
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(dispose => dispose()); });
function event<T>(source: { on: (handler: (data: T) => void) => () => void }) {
  const handlers = new Set<(data: T) => void>();
  const spy = spyOn(source, 'on').mockImplementation(handler => {
    handlers.add(handler);
    return () => { handlers.delete(handler); };
  });
  restore.push(() => spy.mockRestore());
  return { handlers, emit: (data: T) => { for (const handler of handlers) handler(data); } };
}
function fixture() {
  const lists = queue<ITerminalSession[]>();
  const removes = queue<void>();
  const list = spyOn(ipcBridge.terminal.list, 'invoke').mockImplementation(lists.invoke);
  const remove = spyOn(ipcBridge.terminal.remove, 'invoke').mockImplementation(removes.invoke);
  restore.push(() => list.mockRestore(), () => remove.mockRestore());
  const created = event(ipcBridge.terminal.onCreated);
  const updated = event(ipcBridge.terminal.onUpdated);
  const removed = event(ipcBridge.terminal.onRemoved);
  const exited = event(ipcBridge.terminal.onExit);
  const reconnected = event<void>(ipcBridge.terminal.onReconnected);
  return { lists: lists.pending, removes: removes.pending, created, updated, removed, exited, reconnected };
}
const sorted = (rows: ITerminalSession[]) => [...rows].sort((a, b) => a.name.localeCompare(b.name));

test.each([false, true])('manual/reconnect GETs keep latest data and loading (old failure=%s)', async failure => {
  const f = fixture();
  const v = renderHook(useTerminalSessions);
  act(() => emitter.emit('terminal.list.refresh'));
  await act(async () => { if (failure) f.lists[0]!.reject('old'); else f.lists[0]!.resolve([session(1)]); });
  const loadingAfterOld = v.result.current.loading;
  act(() => f.reconnected.emit(undefined));
  await act(async () => { f.lists[2]!.resolve([session(3)]); f.lists[1]!.resolve([session(2)]); });
  expect(loadingAfterOld).toBe(true);
  expect(v.result.current.sessions).toEqual([session(3)]);
  expect(v.result.current.loading).toBe(false);
  act(() => { void v.result.current.refresh(); });
  await act(async () => { f.lists[3]!.reject('current'); });
  expect(v.result.current.sessions).toEqual([]);
});

test.each([false, true])('live changes survive an in-flight snapshot without losing other sessions (GET failure=%s)', async failure => {
  const f = fixture();
  const v = renderHook(useTerminalSessions);
  const initial = [session(1), session(2), session(3), session(4), session(6)];
  await act(async () => { f.lists[0]!.resolve(initial); });
  act(() => { void v.result.current.refresh(); });
  const renamed = { ...session(2), name: 'renamed' };
  const owned = { ...session(6), owner_conversation_id: parseConversationId('019b0000-0000-7000-8000-000000000001') };
  act(() => {
    f.created.emit(session(5));
    f.created.emit(session(5));
    f.updated.emit(renamed);
    f.updated.emit(session(7)); // Full update for a row missed before subscription/reconnect.
    f.removed.emit({ terminal_id: session(3).terminal_id });
    f.exited.emit({ terminal_id: session(4).terminal_id, exit_code: 7 });
    f.updated.emit(owned);
    f.created.emit({ ...owned, terminal_id: session(8).terminal_id });
  });
  await act(async () => { if (failure) f.lists[1]!.reject('offline'); else f.lists[1]!.resolve(initial); });
  expect(sorted(v.result.current.sessions)).toEqual(sorted([
    session(1), renamed, { ...session(4), last_status: 'exited', exit_code: 7 }, session(5), session(7),
  ]));
  expect(v.result.current.loading).toBe(false);
  expect(f.lists.length).toBe(2); // Lifecycle events retain their immediate, local path.
  // The event overlay is per request, not permanent tombstones/history.
  act(() => { void v.result.current.refresh(); });
  await act(async () => { f.lists[2]!.resolve([session(3)]); });
  expect(v.result.current.sessions).toEqual([session(3)]);
});

test('exit followed by relaunch during initial GET keeps the latest status and unrelated rows', async () => {
  const f = fixture();
  const v = renderHook(useTerminalSessions);
  act(() => {
    f.exited.emit({ terminal_id: session(1).terminal_id, exit_code: 1 });
    f.updated.emit({ ...session(1), name: 'relaunched' });
    f.exited.emit({ terminal_id: session(1).terminal_id, exit_code: 2 });
  });
  await act(async () => { f.lists[0]!.resolve([session(1), session(2)]); });
  expect(v.result.current.sessions).toEqual([
    { ...session(1), name: 'relaunched', last_status: 'exited', exit_code: 2 }, session(2),
  ]);
});

test('successful local removal cannot be undone by an old GET, and failure still rejects', async () => {
  const f = fixture();
  const v = renderHook(useTerminalSessions);
  await act(async () => { f.lists[0]!.resolve([session(1), session(2)]); });
  act(() => { void v.result.current.refresh(); });
  let removal: Promise<void>;
  act(() => { removal = v.result.current.removeSession(session(1).terminal_id); });
  await act(async () => { f.removes[0]!.resolve(); await removal!; });
  await act(async () => { f.lists[1]!.resolve([session(1), session(2)]); });
  expect(v.result.current.sessions).toEqual([session(2)]);
  let failure: Promise<unknown>;
  act(() => { failure = v.result.current.removeSession(session(2).terminal_id).catch(error => error); });
  await act(async () => { f.removes[1]!.reject('denied'); });
  expect(await failure!).toBe('denied');
  expect(v.result.current.sessions).toEqual([session(2)]);
});

test('unmount removes subscriptions and makes retained refresh/late completions inert', async () => {
  const f = fixture();
  const before = emitter.listenerCount('terminal.list.refresh');
  const v = renderHook(useTerminalSessions);
  const refresh = v.result.current.refresh;
  const lateUpdate = [...f.updated.handlers][0]!;
  let removal: Promise<void>;
  act(() => { removal = v.result.current.removeSession(session(1).terminal_id); });
  v.unmount();
  act(() => { emitter.emit('terminal.list.refresh'); void refresh(); lateUpdate(session(2)); });
  await act(async () => { f.lists[0]!.resolve([session(1)]); f.removes[0]!.resolve(); await removal!; });
  expect(f.lists.length).toBe(1);
  expect(emitter.listenerCount('terminal.list.refresh')).toBe(before);
  expect([f.created, f.updated, f.removed, f.exited, f.reconnected].map(source => source.handlers.size)).toEqual([0, 0, 0, 0, 0]);
});
