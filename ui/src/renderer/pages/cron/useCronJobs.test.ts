/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { ipcBridge } from '@/common';
import type { ICronJob, ICronJobRun } from '@/common/adapter/ipcBridge';
import { parseConversationId, parseCronJobId, parseCronJobRunId, type ConversationId, type CronJobId } from '@/common/types/ids';
import * as storageKeys from '@/common/utils/browserStorageKey';
import { emitter } from '@/renderer/utils/emitter';
import { useAllCronJobs, useCronJobRuns, useCronJobs, useCronJobsMap } from './useCronJobs';

const first = parseConversationId('019b0000-0000-7000-8000-000000000078');
const second = parseConversationId('019b0000-0000-7000-8000-000000000079');
const jobId = (n: number) => parseCronJobId('019b0000-0000-7000-8000-' + String(n).padStart(12, '0'));
const job = (n: number, conversation_id: ConversationId | undefined = first): ICronJob => ({
  cron_job_id: jobId(n), name: 'job-' + n, enabled: true,
  schedule: { kind: 'cron', expr: '0 0 9 * * ?', tz: 'UTC', description: 'Daily' },
  message: '', execution_mode: 'existing',
  metadata: { conversation_id, agent_type: 'claude', created_by: 'user', created_at: 1, updated_at: 1 },
  state: { run_count: 0, retry_count: 0, max_retries: 3 },
});
const run = (n: number): ICronJobRun => ({
  cron_job_run_id: parseCronJobRunId('019b0000-0000-7000-8000-' + String(n).padStart(12, '0')),
  cron_job_id: jobId(n), executed_at_ms: n, status: 'ok',
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function queue<T>() {
  const pending: ReturnType<typeof deferred<T>>[] = [];
  return { pending, invoke: () => { const next = deferred<T>(); pending.push(next); return next.promise; } };
}
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach((dispose) => dispose()); });

function event<T>(source: { on: (handler: (data: T) => void) => () => void }) {
  const listeners = new Set<(data: T) => void>();
  const spy = spyOn(source, 'on').mockImplementation((handler) => {
    listeners.add(handler);
    return () => { listeners.delete(handler); };
  });
  restore.push(() => spy.mockRestore());
  return { listeners, emit: (data: T) => { for (const listener of listeners) listener(data); } };
}

function fixture() {
  const lists = queue<ICronJob[]>(); const runs = queue<ICronJobRun[]>(); const repairs = queue<ICronJob>();
  const created = event(ipcBridge.cron.onJobCreated);
  const updated = event(ipcBridge.cron.onJobUpdated);
  const removed = event(ipcBridge.cron.onJobRemoved);
  const executed = event(ipcBridge.cron.onJobExecuted);
  const reconnected = event<void>(ipcBridge.conversation.reconnected);
  const refresh = spyOn(emitter, 'emit').mockReturnValue(false);
  const key = 'r78-cron-unread-fixture';
  const spies = [
    spyOn(storageKeys, 'browserStorageGenerationKey').mockReturnValue(key),
    spyOn(ipcBridge.cron.listJobs, 'invoke').mockImplementation(lists.invoke),
    spyOn(ipcBridge.cron.listJobsByConversation, 'invoke').mockImplementation(lists.invoke),
    spyOn(ipcBridge.cron.listRuns, 'invoke').mockImplementation(runs.invoke),
    spyOn(ipcBridge.cron.updateJob, 'invoke').mockImplementation(repairs.invoke),
    spyOn(ipcBridge.cron.removeJob, 'invoke').mockResolvedValue(undefined),
    spyOn(console, 'error').mockImplementation(() => {}),
    spyOn(console, 'log').mockImplementation(() => {}), refresh,
  ];
  restore.push(...spies.map((spy) => () => spy.mockRestore()));
  const saved = localStorage.getItem(key);
  localStorage.removeItem(key);
  restore.push(() => { if (saved === null) localStorage.removeItem(key); else localStorage.setItem(key, saved); });
  return { lists: lists.pending, runs: runs.pending, repairs: repairs.pending, created, updated, removed, executed, reconnected, refresh };
}

const modes = ['conversation', 'all', 'map'] as const;
function mountList(mode: typeof modes[number]) {
  return renderHook(() => {
    if (mode === 'conversation') return useCronJobs(first);
    if (mode === 'all') return useAllCronJobs();
    const result = useCronJobsMap();
    return { ...result, jobs: [...result.jobsMap.values()].flat() };
  });
}
const names = (jobs: ICronJob[]) => jobs.map((item) => item.name).sort();
const noZone = (item: ICronJob): ICronJob => ({ ...item, schedule: { kind: 'cron', expr: '0 0 9 * * ?', description: '' } });

describe.each([...modes])('%s snapshot lifecycle', (mode) => {
  test.each([false, true])('old completion cannot end the latest loading (failure=%s)', async (failure) => {
    const f = fixture(); const v = mountList(mode);
    act(() => f.reconnected.emit(undefined));
    await act(async () => { if (failure) f.lists[0]!.reject(new Error('old')); else f.lists[0]!.resolve([job(1)]); });
    expect(v.result.current.loading).toBe(true);
    expect(v.result.current.jobs).toEqual([]);
    await act(async () => { f.lists[1]!.resolve([job(2)]); });
    expect(names(v.result.current.jobs)).toEqual(['job-2']);
    expect(v.result.current.loading).toBe(false);
  });

  test('late older snapshot is ignored before timezone repair', async () => {
    const f = fixture(); const v = mountList(mode);
    act(() => f.reconnected.emit(undefined));
    await act(async () => { f.lists[1]!.resolve([job(2)]); });
    await act(async () => { f.lists[0]!.resolve([noZone(job(1))]); });
    const repairs = f.repairs.length;
    await act(async () => { for (const request of f.repairs) request.resolve(job(1)); });
    expect(repairs).toBe(0);
    expect(names(v.result.current.jobs)).toEqual(['job-2']);
  });

  test('merges created/updated/removed events without losing unrelated snapshot jobs', async () => {
    const f = fixture(); const v = mountList(mode);
    const changed = { ...job(1), name: 'updated' };
    act(() => {
      f.updated.emit(changed); f.created.emit(job(3)); f.removed.emit({ cron_job_id: jobId(2) });
    });
    await act(async () => { f.lists[0]!.resolve([job(1), job(2), job(4)]); });
    expect(names(v.result.current.jobs)).toEqual(['job-3', 'job-4', 'updated']);
    act(() => f.reconnected.emit(undefined));
    await act(async () => { f.lists[1]!.resolve([]); });
    expect(v.result.current.jobs).toEqual([]); // A later authoritative GET must not replay the prior journal.
  });

  test('applies the latest event per ID while an actual timezone repair is pending', async () => {
    const f = fixture(); const v = mountList(mode);
    await act(async () => { f.lists[0]!.resolve([noZone(job(1)), job(4)]); });
    expect(f.repairs).toHaveLength(1);
    act(() => {
      f.updated.emit({ ...job(1), name: 'updated' });
      f.removed.emit({ cron_job_id: jobId(1) }); f.created.emit({ ...job(1), name: 'recreated' });
    });
    await act(async () => { f.repairs[0]!.resolve(job(1)); });
    expect(names(v.result.current.jobs)).toEqual(['job-4', 'recreated']);
  });

  test('an older pending repair cannot replace a newer snapshot', async () => {
    const f = fixture(); const v = mountList(mode);
    await act(async () => { f.lists[0]!.resolve([noZone(job(1))]); });
    act(() => f.reconnected.emit(undefined));
    await act(async () => { f.lists[1]!.resolve([job(2)]); f.repairs[0]!.resolve(job(1)); });
    expect(names(v.result.current.jobs)).toEqual(['job-2']);
  });

  test('failed snapshot preserves events already received', async () => {
    const f = fixture(); const v = mountList(mode);
    act(() => f.created.emit(job(3)));
    await act(async () => { f.lists[0]!.reject(new Error('offline')); });
    expect(names(v.result.current.jobs)).toEqual(['job-3']);
    expect(v.result.current.loading).toBe(false);
  });

  test('unmount removes subscriptions and prevents late GET from starting repair', async () => {
    const f = fixture(); const v = mountList(mode); v.unmount();
    for (const source of [f.created, f.updated, f.removed, f.reconnected]) expect(source.listeners.size).toBe(0);
    await act(async () => { f.lists[0]!.resolve([noZone(job(1))]); });
    const repairs = f.repairs.length;
    await act(async () => { for (const request of f.repairs) request.resolve(job(1)); });
    expect(repairs).toBe(0);
  });
});

describe('conversation identity', () => {
  test.each([false, true])('empty identity stays idle after a late result (failure=%s)', async (failure) => {
    const f = fixture(); const v = renderHook(({ id }) => useCronJobs(id), { initialProps: { id: first as ConversationId | undefined } });
    v.rerender({ id: undefined });
    expect(v.result.current.loading).toBe(false);
    await act(async () => { if (failure) f.lists[0]!.reject(new Error('old')); else f.lists[0]!.resolve([job(1)]); });
    expect(v.result.current.jobs).toEqual([]); expect(v.result.current.error).toBeNull();
  });

  test.each([false, true])('ignores prior identity completion (failure=%s)', async (failure) => {
    const f = fixture(); const v = renderHook(({ id }) => useCronJobs(id), { initialProps: { id: first } });
    v.rerender({ id: second });
    await act(async () => { f.lists[1]!.resolve([job(2, second)]); });
    await act(async () => { if (failure) f.lists[0]!.reject(new Error('old')); else f.lists[0]!.resolve([job(1)]); });
    expect(names(v.result.current.jobs)).toEqual(['job-2']);
    expect(v.result.current.error).toBeNull();
  });

  test('clears old rows on switching and resets loading/error when identity is removed', async () => {
    const f = fixture(); const v = renderHook(({ id }) => useCronJobs(id), { initialProps: { id: first as ConversationId | undefined } });
    await act(async () => { f.lists[0]!.resolve([job(1)]); });
    v.rerender({ id: second });
    expect(v.result.current.jobs).toEqual([]);
    v.rerender({ id: undefined });
    expect(v.result.current.loading).toBe(false);
    await act(async () => { f.lists[1]!.reject(new Error('old')); });
    expect(v.result.current.error).toBeNull();
  });

  test('binding moves override snapshot membership in both directions', async () => {
    const f = fixture(); const v = renderHook(() => useCronJobs(first));
    act(() => { f.updated.emit(job(1, second)); f.updated.emit(job(2)); });
    await act(async () => { f.lists[0]!.resolve([job(1), job(3)]); });
    expect(names(v.result.current.jobs)).toEqual(['job-2', 'job-3']);
  });

  test('events for other conversations do not initiate timezone repairs', async () => {
    const f = fixture(); const v = renderHook(() => useCronJobs(first));
    act(() => f.updated.emit(noZone(job(2, second))));
    await act(async () => { f.lists[0]!.resolve([job(1)]); });
    const repairs = f.repairs.length;
    await act(async () => { for (const request of f.repairs) request.resolve(job(2, second)); });
    expect(repairs).toBe(0);
    expect(names(v.result.current.jobs)).toEqual(['job-1']);
  });
});

describe('all jobs local actions and conversation map', () => {
  test('successful local update/delete survive an in-flight GET without websocket events', async () => {
    const f = fixture(); const v = renderHook(() => useAllCronJobs());
    await act(async () => { f.lists[0]!.resolve([job(1), job(2)]); });
    act(() => { void v.result.current.refetch(); void v.result.current.pauseJob(jobId(1)); });
    await act(async () => { f.repairs[0]!.resolve({ ...job(1), enabled: false }); await v.result.current.deleteJob(jobId(2)); });
    await act(async () => { f.lists[1]!.resolve([job(1), job(2)]); });
    expect(v.result.current.jobs.map((item) => [item.cron_job_id, item.enabled])).toEqual([[jobId(1), false]]);
  });

  test('map binding moves and run timestamp survive an older snapshot', async () => {
    const f = fixture(); const v = renderHook(() => useCronJobsMap());
    const moved = { ...job(1, second), state: { ...job(1).state, last_run_at_ms: 20 } };
    act(() => f.updated.emit(moved));
    await act(async () => { f.lists[0]!.resolve([{ ...job(1), state: { ...job(1).state, last_run_at_ms: 10 } }, job(2)]); });
    expect(v.result.current.getJobsForConversation(second)).toEqual([moved]);
    expect(names(v.result.current.getJobsForConversation(first))).toEqual(['job-2']);
    act(() => { v.result.current.markAsRead(second); v.result.current.setActiveConversation(first); });
    f.refresh.mockClear();
    act(() => f.updated.emit(moved));
    expect(v.result.current.hasUnread(second)).toBe(false);
    expect(f.refresh).not.toHaveBeenCalled();
  });
});

describe('run history request lifecycle', () => {
  test.each([false, true])('empty identity ignores late completion (failure=%s)', async (failure) => {
    const f = fixture(); const v = renderHook(({ id }) => useCronJobRuns(id), { initialProps: { id: jobId(1) as CronJobId | undefined } });
    v.rerender({ id: undefined });
    expect(v.result.current.loading).toBe(false);
    await act(async () => { if (failure) f.runs[0]!.reject(new Error('old')); else f.runs[0]!.resolve([run(1)]); });
    expect(v.result.current.runs).toEqual([]);
  });

  test.each([false, true])('ignores old identity results (failure=%s)', async (failure) => {
    const f = fixture(); const v = renderHook(({ id }) => useCronJobRuns(id), { initialProps: { id: jobId(1) } });
    v.rerender({ id: jobId(2) });
    await act(async () => { f.runs[1]!.resolve([run(2)]); });
    await act(async () => { if (failure) f.runs[0]!.reject(new Error('old')); else f.runs[0]!.resolve([run(1)]); });
    expect(v.result.current.runs).toEqual([run(2)]);
  });

  test('clears rows on identity switch, and empty identity ends loading', async () => {
    const f = fixture(); const v = renderHook(({ id }) => useCronJobRuns(id), { initialProps: { id: jobId(1) as CronJobId | undefined } });
    await act(async () => { f.runs[0]!.resolve([run(1)]); });
    v.rerender({ id: jobId(2) });
    expect(v.result.current.runs).toEqual([]);
    v.rerender({ id: undefined });
    expect(v.result.current.loading).toBe(false);
    await act(async () => { f.runs[1]!.resolve([run(2)]); });
    expect(v.result.current.runs).toEqual([]);
  });

  test('execution and reconnect reloads have latest-request ownership and clean subscriptions', async () => {
    const f = fixture(); const v = renderHook(() => useCronJobRuns(jobId(1)));
    act(() => { f.executed.emit({ cron_job_id: jobId(1), status: 'ok' }); f.reconnected.emit(undefined); });
    await act(async () => { f.runs[1]!.resolve([run(1)]); f.runs[0]!.reject(new Error('old')); });
    expect(v.result.current.loading).toBe(true);
    await act(async () => { f.runs[2]!.resolve([run(2)]); });
    expect(v.result.current.runs).toEqual([run(2)]);
    v.unmount();
    expect(f.executed.listeners.size).toBe(0); expect(f.reconnected.listeners.size).toBe(0);
  });
});
