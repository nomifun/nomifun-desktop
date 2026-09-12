import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { ipcBridge } from '@/common';
import type { IRequirement, ITagSummary } from '@/common/adapter/ipcBridge';
import { parseRequirementId } from '@/common/types/ids';
import { useRequirements, useRequirementTags } from './useRequirements';
import { useWorkspaceTags } from './WorkspacePage/useWorkspaceTags';

const item = (n: number): IRequirement => ({
  requirement_id: parseRequirementId('019b0000-0000-7000-8000-' + String(n).padStart(12, '0')),
  display_no: n, title: 'task-' + n, content: '', tag: 'fixture', order_key: '', status: 'pending',
  attempt_count: 0, created_by: 'user', created_at: 1, updated_at: 1,
});
const tag = (name: string): ITagSummary => ({ tag: name, pending: 1, in_progress: 0, done: 0,
  failed: 0, cancelled: 0, needs_review: 0, total: 1, paused: false });
function queue<T>() {
  const pending: Array<{ resolve: (value: T) => void; reject: (error: unknown) => void }> = [];
  return { pending, invoke: () => new Promise<T>((resolve, reject) => { pending.push({ resolve, reject }); }) };
}
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach((dispose) => dispose()); });
function event<T>(source: { on: (handler: (data: T) => void) => () => void }) {
  const handlers = new Set<(data: T) => void>();
  const spy = spyOn(source, 'on').mockImplementation((handler) => {
    handlers.add(handler); return () => { handlers.delete(handler); };
  });
  restore.push(() => spy.mockRestore());
  return { emit: (data: T) => { handlers.forEach((handler) => handler(data)); }, handlers };
}
function fixture() {
  const lists = queue<{ items: IRequirement[]; total: number; has_more: boolean }>();
  const tags = queue<ITagSummary[]>();
  const listing = spyOn(ipcBridge.requirements.list, 'invoke').mockImplementation(lists.invoke);
  const tagging = spyOn(ipcBridge.requirements.tags, 'invoke').mockImplementation(tags.invoke);
  const logging = spyOn(console, 'error').mockImplementation(() => {});
  restore.push(...[listing, tagging, logging].map((spy) => () => spy.mockRestore()));
  const updated = event(ipcBridge.requirements.onUpdated);
  const subscriptions = [event(ipcBridge.requirements.onCreated), event(ipcBridge.requirements.onDeleted),
    event(ipcBridge.requirements.onStatusChanged), event(ipcBridge.requirements.onTagPaused)];
  const reconnected = event<void>(ipcBridge.conversation.reconnected);
  return { lists: lists.pending, tags: tags.pending, listing, logging, updated, reconnected, subscriptions };
}
const page = (...items: IRequirement[]) => ({ items, total: items.length, has_more: false });

describe('requirements query lifecycle', () => {
  test.each([false, true])('old filter completion cannot replace the new result (failure=%s)', async (failure) => {
    const f = fixture(); const v = renderHook(({ q }) => useRequirements({ q }), { initialProps: { q: 'old' } });
    v.rerender({ q: 'new' });
    await act(async () => { f.lists[1]!.resolve(page(item(2))); });
    await act(async () => { if (failure) f.lists[0]!.reject(new Error('old')); else f.lists[0]!.resolve(page(item(1))); });
    expect(v.result.current.items).toEqual([item(2)]);
    expect(v.result.current.total).toBe(1); expect(v.result.current.error).toBeNull();
  });

  test.each([false, true])('events/reconnect own loading until the latest GET settles (failure=%s)', async (failure) => {
    const f = fixture(); const v = renderHook(() => useRequirements({}));
    act(() => { f.updated.emit(item(1)); f.reconnected.emit(); });
    await act(async () => { f.lists[1]!.resolve(page(item(1))); if (failure) f.lists[0]!.reject(new Error('old')); else f.lists[0]!.resolve(page()); });
    expect(v.result.current.loading).toBe(true);
    await act(async () => { f.lists[2]!.resolve(page(item(2))); });
    expect(v.result.current.items).toEqual([item(2)]);
    expect(v.result.current.loading).toBe(false);
  });

  test('switching filters clears old rows and late mutation callbacks cannot reload the old query', async () => {
    const f = fixture(); const v = renderHook(({ q }) => useRequirements({ q }), { initialProps: { q: 'old' } });
    await act(async () => { f.lists[0]!.resolve(page(item(1))); });
    const oldRefresh = v.result.current.refresh;
    v.rerender({ q: 'new' });
    expect(v.result.current.items).toEqual([]); expect(v.result.current.total).toBe(0);
    await act(async () => { void oldRefresh(); });
    expect(f.listing).toHaveBeenCalledTimes(2);
    v.unmount();
    await act(async () => { f.lists[1]!.reject(new Error('unmounted')); });
    expect(f.logging).not.toHaveBeenCalled();
    expect(f.reconnected.handlers.size).toBe(0);
    expect(f.updated.handlers.size).toBe(0);
    expect(f.subscriptions.every((source) => source.handlers.size === 0)).toBe(true);
  });
});

describe('board pagination through the real query hook', () => {
  test('follows has_more and publishes only the complete board', async () => {
    const f = fixture(); const v = renderHook(() => useRequirements({ page: 1, page_size: 200 }, true));
    await act(async () => { f.lists[0]!.resolve({ items: [item(1)], total: 2, has_more: true }); });
    expect(f.listing).toHaveBeenCalledTimes(2);
    expect(f.listing.mock.calls[1]![0]).toMatchObject({ page: 2, page_size: 200 });
    expect(v.result.current.items).toEqual([]); expect(v.result.current.loading).toBe(true);
    await act(async () => { f.lists[1]!.resolve({ items: [item(2)], total: 2, has_more: false }); });
    expect(v.result.current.items).toEqual([item(1), item(2)]); expect(v.result.current.total).toBe(2);
  });

  test('a superseded board page cannot overwrite a newer list query', async () => {
    const f = fixture(); const v = renderHook(({ board }) => useRequirements({ page: 1, page_size: 200 }, board), { initialProps: { board: true } });
    await act(async () => { f.lists[0]!.resolve({ items: [item(1)], total: 2, has_more: true }); });
    expect(f.listing).toHaveBeenCalledTimes(2);
    v.rerender({ board: false });
    await act(async () => { f.lists[2]!.resolve(page(item(3))); f.lists[1]!.resolve(page(item(2))); });
    expect(v.result.current.items).toEqual([item(3)]);
  });

  test('a later-page failure is not reported as a successful partial board', async () => {
    const f = fixture(); const v = renderHook(() => useRequirements({ page_size: 200 }, true));
    await act(async () => { f.lists[0]!.resolve({ items: [item(1)], total: 2, has_more: true }); });
    expect(f.listing).toHaveBeenCalledTimes(2);
    await act(async () => { f.lists[1]!.reject(new Error('page offline')); });
    expect(v.result.current.items).toEqual([]); expect(v.result.current.loading).toBe(false);
    expect(v.result.current.error).toContain('page offline');
  });
});

describe.each(['workspace', 'picker'])('%s tags', (kind) => {
  test('latest refresh wins and reconnect resyncs with cleanup', async () => {
    const f = fixture(); const v = renderHook(() => kind === 'workspace' ? useWorkspaceTags() : useRequirementTags());
    act(() => f.updated.emit(item(1)));
    await act(async () => { f.tags[1]!.resolve([tag('new')]); f.tags[0]!.resolve([tag('old')]); });
    expect(v.result.current.tags[0]?.tag).toBe('new');
    act(() => f.reconnected.emit());
    expect(f.tags).toHaveLength(3);
    await act(async () => { f.tags[2]!.resolve([tag('reconnected')]); });
    expect(v.result.current.tags[0]?.tag).toBe('reconnected');
    act(() => f.updated.emit(item(1))); v.unmount();
    await act(async () => { f.tags[3]!.reject(new Error('unmounted')); });
    expect(f.logging).not.toHaveBeenCalled(); expect(f.reconnected.handlers.size).toBe(0);
  });
});
