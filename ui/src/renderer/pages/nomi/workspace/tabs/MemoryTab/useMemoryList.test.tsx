/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { ICompanionMemory } from '@/common/adapter/ipcBridge';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { parseCompanionId, parseCompanionMemoryId } from '@/common/types/ids';
import { useMemoryList } from './useMemoryList';

const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000061');
const original: ICompanionMemory = { memory_id: parseCompanionMemoryId('019b0000-0000-7000-8000-000000000062'),
  companion_id: companionId, revision: 3, kind: 'knowledge', content: 'Observed content', tags: [], importance: 0.5,
  strength: 0.8, pinned: false, source: 'manual', status: 'active', created_at: 1, updated_at: 1, last_reinforced_at: 1 };
const conflict = () => new BackendHttpError({ method: 'PUT', path: '/api/companion/memories/test', status: 409,
  body: { code: 'COMPANION_MEMORY_REVISION_CONFLICT', error: 'changed elsewhere' } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });
const install = () => {
  let stored = { ...original };
  const list = spyOn(ipcBridge.companion.listMemories, 'invoke').mockImplementation(async () => ({ items: [{ ...stored }], total: 1 }));
  const update = spyOn(ipcBridge.companion.updateMemory, 'invoke').mockImplementation(async request => {
    if (request.expected_revision !== stored.revision) throw conflict();
    stored = { ...stored, ...request, revision: stored.revision + 1, content: request.content ?? stored.content } as ICompanionMemory;
    return { ...stored };
  });
  const remove = spyOn(ipcBridge.companion.deleteMemory, 'invoke').mockImplementation(async request => {
    if (request.expected_revision !== stored.revision) throw conflict();
  });
  const batch = spyOn(ipcBridge.companion.batchMemories, 'invoke').mockImplementation(async request => {
    if (request.expected_revisions[stored.memory_id] !== stored.revision) throw conflict();
  });
  const created = spyOn(ipcBridge.companion.onMemoryCreated, 'on').mockImplementation(() => () => {});
  const changed = spyOn(ipcBridge.companion.onMemoryUpdated, 'on').mockImplementation(() => () => {});
  const deleted = spyOn(ipcBridge.companion.onMemoryDeleted, 'on').mockImplementation(() => () => {});
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  for (const spy of [list, update, remove, batch, created, changed, deleted, error]) restores.push(() => spy.mockRestore());
  return { list, update, remove, batch, revise: () => { stored = { ...stored, revision: 4, content: 'Concurrent new fact' }; }, stored: () => stored };
};

test('batch selection retains the observed revision across a live list refresh and never retries a conflict automatically', async () => {
  const api = install();
  const view = renderHook(() => useMemoryList(companionId));
  await waitFor(() => expect(view.result.current.items).toHaveLength(1));
  act(() => view.result.current.toggleSelected(original.memory_id));
  api.revise();
  await act(async () => view.result.current.refresh());
  expect(view.result.current.items[0].revision).toBe(4);
  expect(view.result.current.selectedMemories[0].revision).toBe(3);
  await act(async () => { await expect(view.result.current.runBatch('archive')).rejects.toBeInstanceOf(BackendHttpError); });
  expect(api.batch.mock.calls[0][0].expected_revisions).toEqual({ [original.memory_id]: 3 });
  expect(api.batch).toHaveBeenCalledTimes(1);
  expect(view.result.current.conflict).toBe(true);
  expect(api.stored().content).toBe('Concurrent new fact');
  act(() => view.result.current.clearSelection());
  act(() => view.result.current.selectAllOnPage());
  await act(async () => { await view.result.current.runBatch('archive'); });
  expect(api.batch.mock.calls[1][0].expected_revisions).toEqual({ [original.memory_id]: 4 });
});

test('content, pin, archive and confirmed delete submit their exact observed version', async () => {
  const api = install();
  const view = renderHook(() => useMemoryList(companionId));
  await waitFor(() => expect(view.result.current.items).toHaveLength(1));
  const observed = view.result.current.items[0];
  api.revise();
  for (const command of [() => view.result.current.saveContent(observed, 'Draft'),
    () => view.result.current.setPinned(observed, true), () => view.result.current.setArchived(observed, true),
    () => view.result.current.removeMemory(observed)]) {
    await act(async () => { await expect(command()).rejects.toBeInstanceOf(BackendHttpError); });
  }
  expect(api.update.mock.calls.map(call => call[0].expected_revision)).toEqual([3, 3, 3]);
  expect(api.remove.mock.calls[0][0].expected_revision).toBe(3);
  expect(api.stored().content).toBe('Concurrent new fact');
  expect(api.stored().pinned).toBe(false);
});

test('a response from the previous companion cannot populate the new companion during debounce', async () => {
  const api = install();
  let resolve!: (value: { items: ICompanionMemory[]; total: number }) => void;
  api.list.mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  const other = parseCompanionId('019b0000-0000-7000-8000-000000000063');
  api.list.mockImplementation(async () => ({ items: [{ ...original, companion_id: other }], total: 1 }));
  const view = renderHook(({ id }) => useMemoryList(id), { initialProps: { id: companionId } });
  await waitFor(() => expect(resolve).toBeDefined());
  view.rerender({ id: other });
  await act(async () => resolve({ items: [original], total: 1 }));
  expect(view.result.current.items).toEqual([]);
  await waitFor(() => expect(view.result.current.items[0]?.companion_id).toBe(other));
});
