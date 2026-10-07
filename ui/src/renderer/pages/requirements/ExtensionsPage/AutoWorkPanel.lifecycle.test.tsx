import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { IAutoWorkState, ITagBinding, ITagSummary } from '@/common/adapter/ipcBridge';
import { parseConversationId } from '@/common/types/ids';
import AutoWorkPanel from './AutoWorkPanel';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(fn => fn()); });
function deferred<T>() {
  let resolve!: (value: T) => void; let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const summary = (tag = 'fixture'): ITagSummary => ({ tag, pending: 1, in_progress: 0, done: 0, failed: 0, cancelled: 0, needs_review: 0, total: 1, paused: true });
const binding: ITagBinding = { kind: 'conversation', target_id: parseConversationId('019b0000-0000-7000-8000-000000000001'), name: 'Bound session', run_state: 'idle' };
function fixture() {
  const success = mock(() => () => {}); const error = mock(() => () => {});
  const messages = spyOn(Message, 'useMessage').mockReturnValue([{ success, error }, <></>]);
  const tags = spyOn(ipcBridge.requirements.tags, 'invoke').mockResolvedValue([summary()]);
  const bindings = spyOn(ipcBridge.requirements.tagBindings, 'invoke').mockResolvedValue([{ tag: 'fixture', bindings: [binding] }]);
  const listeners = new Map<string, () => void>();
  restore.push(() => messages.mockRestore(), () => tags.mockRestore(), () => bindings.mockRestore());
  for (const [name, source] of Object.entries({ created: ipcBridge.requirements.onCreated, updated: ipcBridge.requirements.onUpdated,
    status: ipcBridge.requirements.onStatusChanged, deleted: ipcBridge.requirements.onDeleted,
    paused: ipcBridge.requirements.onTagPaused, autowork: ipcBridge.requirements.onAutoWork, reconnect: ipcBridge.conversation.reconnected })) {
    const spy = spyOn(source, 'on').mockImplementation((listener: unknown) => {
      listeners.set(name, listener as () => void); return () => { listeners.delete(name); };
    });
    restore.push(() => spy.mockRestore());
  }
  return { tags, bindings, listeners, success, error };
}
const mount = () => render(<I18nextProvider i18n={i18n}><AutoWorkPanel /></I18nextProvider>);

test('newest load owns rows and stale errors cannot replace its state', async () => {
  const f = fixture(); const pending = [deferred<ITagSummary[]>(), deferred<ITagSummary[]>(), deferred<ITagSummary[]>()];
  let request = 0; f.tags.mockImplementation(() => pending[request++]!.promise);
  const v = mount();
  await act(async () => { f.listeners.get('autowork')!(); f.listeners.get('paused')!(); });
  await act(async () => { pending[1]!.reject(new Error('stale offline')); });
  await act(async () => { pending[2]!.resolve([summary('latest')]); });
  await act(async () => { pending[0]!.resolve([summary('old')]); });
  expect(v.queryByText('latest')).not.toBeNull(); expect(v.queryByText('old')).toBeNull();
  expect(f.error).not.toHaveBeenCalled();
});

test('count changes and reconnect refresh the panel and subscriptions are removed', async () => {
  const f = fixture(); const v = mount(); await act(async () => {});
  for (const name of ['created', 'updated', 'status', 'deleted', 'reconnect']) {
    expect(f.listeners.has(name)).toBe(true);
    const before = f.tags.mock.calls.length;
    await act(async () => { f.listeners.get(name)!(); });
    expect(f.tags).toHaveBeenCalledTimes(before + 1);
  }
  v.unmount(); expect(f.listeners.size).toBe(0);
});

test('bindings without requirements remain visible and retain their target identity', async () => {
  const f = fixture(); f.tags.mockResolvedValue([]);
  const put = spyOn(ipcBridge.requirements.setAutoWork, 'invoke').mockResolvedValue({ kind: binding.kind, target_id: binding.target_id, enabled: false, running: false, run_state: 'off', completed_count: 0 });
  restore.push(() => put.mockRestore());
  const v = mount(); await act(async () => {});
  expect(v.queryByText('fixture')).not.toBeNull();
  const expand = v.container.querySelector('.arco-table-expand-icon-cell button')!;
  fireEvent.click(expand); await act(async () => {});
  expect(v.queryByText('Bound session')).not.toBeNull();
  fireEvent.click(v.getByText('autowork.tagSessions.unbind')); await act(async () => {});
  expect(put).toHaveBeenCalledWith({ kind: binding.kind, target_id: binding.target_id, enabled: false, from_admin: true });
});

test('failed initial loading is retryable and an unmounted load cannot report errors', async () => {
  const f = fixture(); f.tags.mockRejectedValueOnce(new Error('offline'));
  const v = mount(); await act(async () => {});
  expect(f.error).toHaveBeenCalledTimes(1);
  fireEvent.click(v.getByText('requirements.retry')); await act(async () => {});
  expect(v.queryByText('fixture')).not.toBeNull();
  const pending = deferred<ITagSummary[]>(); f.tags.mockImplementation(() => pending.promise);
  await act(async () => { f.listeners.get('autowork')!(); });
  v.unmount(); await act(async () => { pending.reject(new Error('late offline')); });
  expect(f.error).toHaveBeenCalledTimes(1);
});

test.each(['resume', 'unbind'] as const)('%s is single-flight and cannot publish or refresh after unmount', async action => {
  const f = fixture(); const pending = deferred<void>();
  const resumed = spyOn(ipcBridge.requirements.resumeTag, 'invoke').mockImplementation(async () => { await pending.promise; return summary(); });
  const unbound = spyOn(ipcBridge.requirements.setAutoWork, 'invoke').mockImplementation(async (): Promise<IAutoWorkState> => { await pending.promise; return { kind: binding.kind, target_id: binding.target_id, enabled: false, running: false, run_state: 'off', completed_count: 0 }; });
  restore.push(() => resumed.mockRestore(), () => unbound.mockRestore());
  const v = mount(); await act(async () => {});
  if (action === 'unbind') { fireEvent.click(v.container.querySelector('.arco-table-expand-icon-cell button')!); await act(async () => {}); }
  const button = v.getByText('autowork.tagSessions.' + action);
  fireEvent.click(button); fireEvent.click(button);
  const calls = action === 'resume' ? resumed.mock.calls.length : unbound.mock.calls.length;
  v.unmount(); await act(async () => { pending.resolve(); });
  expect(calls).toBe(1); expect(f.success).not.toHaveBeenCalled(); expect(f.tags).toHaveBeenCalledTimes(1);
});
