import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { ipcBridge } from '@/common';
import type { ICsAgent, IKnowledgeBase } from '@/common/adapter/ipcBridge';
import { parseCsAgentId, parseKnowledgeBaseId, type CsAgentId } from '@/common/types/ids';
import { useCsAgent, useCsAgents } from './useCsAgents';
import { useKnowledgeBaseOptions } from './useKnowledgeBaseOptions';

const agent = (n: number): ICsAgent => ({
  cs_agent_id: parseCsAgentId('019b0000-0000-7000-8000-' + String(n).padStart(12, '0')),
  name: 'Support ' + n, greeting: '', persona: '', service_policy: '', provider_id: null, model: null,
  knowledge_base_ids: [], enabled: true, max_concurrent: 8, audit_retention_days: 30,
  created_at: 1, updated_at: 1,
});
const base = (n: number): IKnowledgeBase => ({
  knowledge_base_id: parseKnowledgeBaseId('019b0000-0000-7000-8000-' + String(n).padStart(12, '0')),
  name: 'Knowledge ' + n, description: '', root_path: '/fixture', managed: true,
  tree_access: 'read_only', created_at: 1, updated_at: 1, file_count: 0, total_size: 0,
  root_exists: true, tags: [], kind: 'blank',
});
function queue<T>() {
  const pending: Array<{ resolve: (value: T) => void; reject: (error: unknown) => void }> = [];
  return { pending, invoke: () => new Promise<T>((resolve, reject) => { pending.push({ resolve, reject }); }) };
}
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(dispose => dispose()); });
function fixture() {
  const lists = queue<ICsAgent[]>();
  const bases = queue<IKnowledgeBase[]>();
  const gets = queue<ICsAgent>();
  const patches = queue<ICsAgent>();
  const creates = queue<ICsAgent>();
  const spies = [
    spyOn(ipcBridge.customerService.listAgents, 'invoke').mockImplementation(lists.invoke),
    spyOn(ipcBridge.knowledge.listBases, 'invoke').mockImplementation(bases.invoke),
    spyOn(ipcBridge.customerService.getAgent, 'invoke').mockImplementation(gets.invoke),
    spyOn(ipcBridge.customerService.patchAgent, 'invoke').mockImplementation(patches.invoke),
    spyOn(ipcBridge.customerService.createAgent, 'invoke').mockImplementation(creates.invoke),
  ];
  restore.push(...spies.map(spy => () => spy.mockRestore()));
  return { lists: lists.pending, bases: bases.pending, gets: gets.pending,
    patches: patches.pending, creates: creates.pending };
}

// Exercise both real hooks with the same small deferred-request fixture.
describe.each(['roster', 'knowledge'] as const)('%s loading ownership', kind => {
  test.each([false, true])('latest GET owns data/loading; current failure still clears data (stale failure=%s)', async failure => {
    const f = fixture();
    const pending = kind === 'roster' ? f.lists : f.bases;
    const resolve = (index: number, n: number) => {
      if (kind === 'roster') f.lists[index]!.resolve([agent(n)]);
      else f.bases[index]!.resolve([base(n)]);
    };
    const stale = (index: number) => failure ? pending[index]!.reject(new Error('old failure')) : resolve(index, 1);
    const v = renderHook(() => {
      if (kind === 'roster') {
        const result = useCsAgents();
        return { ...result, values: result.agents.map(row => row.cs_agent_id) };
      }
      const result = useKnowledgeBaseOptions();
      return { ...result, values: result.options.map(row => row.value) };
    });
    act(() => { void v.result.current.refresh(); });
    await act(async () => { stale(0); });
    const loadingAfterStale = v.result.current.loading;
    await act(async () => { resolve(1, 2); });
    act(() => { void v.result.current.refresh(); void v.result.current.refresh(); });
    await act(async () => { resolve(3, 3); stale(2); });
    const valuesAfterStale = v.result.current.values;
    act(() => { void v.result.current.refresh(); });
    await act(async () => { pending[4]!.reject(new Error('current failure')); });
    expect(loadingAfterStale).toBe(true);
    expect(valuesAfterStale).toEqual([kind === 'roster' ? agent(3).cs_agent_id : base(3).knowledge_base_id]);
    expect(v.result.current.values).toEqual([]);
    expect(v.result.current.loading).toBe(false);
    const refresh = v.result.current.refresh;
    v.unmount();
    await act(async () => { void refresh(); });
    expect(pending.length).toBe(5);
  });
});

test('create keeps its result and refresh ordering, but does not refresh after unmount', async () => {
  const f = fixture();
  const v = renderHook(() => useCsAgents());
  await act(async () => { f.lists[0]!.resolve([]); });
  let created: Promise<ICsAgent>;
  let finished = false;
  act(() => { created = v.result.current.create({ name: 'first' }).then(value => { finished = true; return value; }); });
  await act(async () => { f.creates[0]!.resolve(agent(1)); });
  expect(finished).toBe(false);
  await act(async () => { f.lists[1]!.resolve([agent(1)]); });
  expect(await created!).toEqual(agent(1));
  let late: Promise<ICsAgent>;
  act(() => { late = v.result.current.create({ name: 'second' }); });
  v.unmount();
  await act(async () => { f.creates[1]!.resolve(agent(2)); });
  const readsAfterUnmount = f.lists.length;
  // Settle the old implementation's unwanted GET so the red run cannot hang.
  if (f.lists[2]) await act(async () => { f.lists[2]!.resolve([]); });
  expect(await late!).toEqual(agent(2));
  expect(readsAfterUnmount).toBe(2);
});

test.each([false, true])('agent identity changes clear old data and reject former GET ownership (failure=%s)', async failure => {
  const f = fixture();
  const v = renderHook(({ id }: { id: CsAgentId | null }) => useCsAgent(id), { initialProps: { id: agent(1).cs_agent_id as CsAgentId | null } });
  await act(async () => { f.gets[0]!.resolve(agent(1)); });
  const oldReload = v.result.current.reload;
  act(() => { void oldReload(); });
  v.rerender({ id: agent(2).cs_agent_id });
  const dataOnSwitch = v.result.current.agent;
  await act(async () => { f.gets[2]!.resolve(agent(2)); });
  await act(async () => { if (failure) f.gets[1]!.reject(new Error('old')); else f.gets[1]!.resolve(agent(1)); });
  const dataAfterOld = v.result.current.agent;
  v.rerender({ id: agent(1).cs_agent_id });
  act(() => { void oldReload(); });
  const readsAfterOldCallback = f.gets.length;
  v.rerender({ id: null });
  await act(async () => { f.gets[3]!.resolve(agent(1)); });
  expect(dataOnSwitch).toBeNull();
  expect(dataAfterOld).toEqual(agent(2));
  expect(readsAfterOldCallback).toBe(4);
  expect(v.result.current.agent).toBeNull();
  expect(v.result.current.loading).toBe(false);
});

test('agent reload is latest-only and current GET failure still yields null', async () => {
  const f = fixture();
  const v = renderHook(() => useCsAgent(agent(1).cs_agent_id));
  act(() => { void v.result.current.reload(); });
  await act(async () => { f.gets[0]!.reject(new Error('old')); });
  const loadingAfterStale = v.result.current.loading;
  await act(async () => { f.gets[1]!.resolve(agent(1)); });
  act(() => { void v.result.current.reload(); });
  await act(async () => { f.gets[2]!.reject(new Error('current')); });
  expect(loadingAfterStale).toBe(true);
  expect(v.result.current.agent).toBeNull();
  expect(v.result.current.loading).toBe(false);
});

test.each([false, true])('former PATCH settles for its caller without replacing or reloading the new agent (failure=%s)', async failure => {
  const f = fixture();
  const v = renderHook(({ id }) => useCsAgent(id), { initialProps: { id: agent(1).cs_agent_id } });
  await act(async () => { f.gets[0]!.resolve(agent(1)); });
  let outcome: Promise<unknown>;
  const error = new Error('old patch');
  act(() => { outcome = v.result.current.patch({ name: 'edited' }).catch(value => value); });
  expect(v.result.current.agent?.name).toBe('edited');
  v.rerender({ id: agent(2).cs_agent_id });
  await act(async () => { f.gets[1]!.resolve(agent(2)); });
  await act(async () => { if (failure) f.patches[0]!.reject(error); else f.patches[0]!.resolve(agent(1)); });
  const readsAfterPatch = f.gets.length;
  if (f.gets[2]) await act(async () => { f.gets[2]!.resolve(agent(1)); });
  expect(await outcome!).toEqual(failure ? error : agent(1));
  expect(readsAfterPatch).toBe(2);
  expect(v.result.current.agent).toEqual(agent(2));
});

test('current PATCH failure rolls back through GET and rethrows the original rejection', async () => {
  const f = fixture();
  const v = renderHook(() => useCsAgent(agent(1).cs_agent_id));
  await act(async () => { f.gets[0]!.resolve(agent(1)); });
  let outcome: Promise<unknown>;
  act(() => { outcome = v.result.current.patch({ name: 'edited' }).catch(error => error); });
  await act(async () => { f.patches[0]!.reject(null); });
  expect(f.gets.length).toBe(2);
  await act(async () => { f.gets[1]!.resolve(agent(1)); });
  expect(await outcome!).toBeNull();
  expect(v.result.current.agent).toEqual(agent(1));
  expect(v.result.current.loading).toBe(false);
});

test('PATCH result invalidates an overlapping older GET; unmounted failure cannot reload', async () => {
  const f = fixture();
  const v = renderHook(() => useCsAgent(agent(1).cs_agent_id));
  await act(async () => { f.gets[0]!.resolve(agent(1)); });
  act(() => { void v.result.current.reload(); });
  let saved: Promise<ICsAgent | undefined>;
  act(() => { saved = v.result.current.patch({ name: 'saved' }); });
  const updated = { ...agent(1), name: 'saved' };
  await act(async () => { f.patches[0]!.resolve(updated); });
  await act(async () => { f.gets[1]!.resolve(agent(1)); });
  const dataAfterGet = v.result.current.agent;
  let failed: Promise<unknown>;
  act(() => { failed = v.result.current.patch({ name: 'late' }).catch(error => error); });
  v.unmount();
  await act(async () => { f.patches[1]!.reject('offline'); });
  const readsAfterUnmount = f.gets.length;
  if (f.gets[2]) await act(async () => { f.gets[2]!.resolve(agent(1)); });
  expect(await saved!).toEqual(updated);
  expect(dataAfterGet).toEqual(updated);
  expect(await failed!).toBe('offline');
  expect(readsAfterUnmount).toBe(2);
});
