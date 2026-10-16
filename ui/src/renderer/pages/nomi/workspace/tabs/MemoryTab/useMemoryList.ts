/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type {
  ICompanionMemory,
  ICompanionMemoryBatchAction,
  ICompanionMemoryKind,
  ICompanionMemorySort,
} from '@/common/adapter/ipcBridge';
import type { CompanionId, CompanionMemoryId } from '@/common/types/ids';
import type { MemoryStatusFilter } from './constants';
import { isMemoryRevisionConflict } from './memoryMutationError';

/**
 * The memory list of ONE companion. Every request carries
 * `companion_id: companionId` — this surface has no cross-companion mode
 * and no scope picker, so the companion id is the single source of truth for
 * what the user is looking at and who a new memory belongs to. Memory is
 * strictly per-companion: there is no shared/install-wide scope to pick.
 */
export const useMemoryList = (companionId: CompanionId) => {
  const [q, setQ] = useState('');
  const [kind, setKind] = useState<'' | ICompanionMemoryKind>('');
  const [status, setStatus] = useState<MemoryStatusFilter>('active');
  const [sort, setSort] = useState<ICompanionMemorySort>('relevance');
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);

  const [items, setItems] = useState<ICompanionMemory[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);
  const [selectedMemories, setSelectedMemories] = useState<ICompanionMemory[]>([]);
  const selected = selectedMemories.map(memory => memory.memory_id);
  const [conflict, setConflict] = useState(false);

  // Out-of-order guard: rapid typing fires overlapping requests and a slow stale
  // response must never clobber a newer one.
  const seqRef = useRef(0);
  const scope = JSON.stringify([companionId, kind, q, status, sort, page, pageSize]);
  const currentScope = useRef(scope);
  currentScope.current = scope;

  const refresh = useCallback(async () => {
    if (currentScope.current !== scope) return;
    const seq = ++seqRef.current;
    setLoading(true);
    try {
      const result = await ipcBridge.companion.listMemories.invoke({
        kind: kind || undefined,
        q: q.trim() || undefined,
        status,
        companion_id: companionId,
        sort,
        limit: pageSize,
        offset: (page - 1) * pageSize,
      });
      if (seq !== seqRef.current || currentScope.current !== scope) return;
      setTotal(result.total);
      const maxPage = Math.max(1, Math.ceil(result.total / pageSize));
      // A deletion can leave the current page past the end. Keep the visible
      // rows while the next request loads the last valid page.
      if (page > maxPage) {
        setPage(maxPage);
        return;
      }
      setItems(result.items);
    } catch (e) {
      if (seq === seqRef.current && currentScope.current === scope) Message.error(String(e));
    } finally {
      if (seq === seqRef.current && currentScope.current === scope) setLoading(false);
    }
  }, [companionId, kind, q, status, sort, page, pageSize, scope]);

  // Debounced so typing in the search field does not queue a request per keystroke.
  useEffect(() => {
    const timer = setTimeout(() => void refresh(), 250);
    return () => clearTimeout(timer);
  }, [refresh]);

  // A new result set always starts at its first page; page navigation alone
  // keeps the current filters.
  useEffect(() => {
    setPage(1);
  }, [companionId, kind, q, status, sort, pageSize]);

  // Selection belongs to one result set.
  useEffect(() => {
    setSelectedMemories([]);
    setConflict(false);
  }, [companionId, kind, q, status, sort, page, pageSize]);

  // `refresh` changes identity on every keystroke; the live subscriptions must
  // not churn with it, so they read the latest fetcher through a ref and are
  // wired exactly once.
  const refreshRef = useRef(refresh);
  useEffect(() => {
    refreshRef.current = refresh;
  }, [refresh]);

  // nomi writes memories mid-chat and other surfaces edit them — reflect live.
  useEffect(() => {
    const run = () => void refreshRef.current();
    const unsubs = [
      ipcBridge.companion.onMemoryCreated.on(run),
      ipcBridge.companion.onMemoryUpdated.on(run),
      ipcBridge.companion.onMemoryDeleted.on(run),
    ];
    return () => unsubs.forEach((u) => u());
  }, []);

  const toggleSelected = useCallback((id: CompanionMemoryId) => {
    const observed = items.find(memory => memory.memory_id === id);
    if (!observed) return;
    setSelectedMemories(prev => prev.some(memory => memory.memory_id === id)
      ? prev.filter(memory => memory.memory_id !== id) : [...prev, observed]);
  }, [items]);

  const clearSelection = useCallback(() => { setSelectedMemories([]); setConflict(false); }, []);

  const selectAllOnPage = useCallback(() => {
    setSelectedMemories([...items]);
  }, [items]);

  const handlePageChange = useCallback(
    (nextPage: number, nextPageSize: number) => {
      const pageSizeChanged = nextPageSize !== pageSize;
      if (pageSizeChanged) setPageSize(nextPageSize);
      setPage(pageSizeChanged ? 1 : nextPage);
    },
    [pageSize]
  );

  // ── mutations ──
  // Every mutation carries `companion_id: companionId` — the companion DOING
  // it, never a new owner: ownership is fixed at write time and the store rejects
  // a row belonging to anybody else. This tab can therefore only ever change the
  // memories it is displaying.
  const mutate = useCallback(async <T,>(command: () => Promise<T>): Promise<T> => {
    try {
      const result = await command();
      if (currentScope.current === scope) { setConflict(false); void refresh(); }
      return result;
    } catch (error) {
      if (currentScope.current === scope && isMemoryRevisionConflict(error)) { setConflict(true); void refresh(); }
      throw error;
    }
  }, [refresh, scope]);

  const addMemory = useCallback(
    async (nextKind: ICompanionMemoryKind, content: string) => {
      await ipcBridge.companion.addMemory.invoke({
        kind: nextKind,
        content: content.trim(),
        companion_id: companionId,
      });
      void refresh();
    },
    [companionId, refresh]
  );

  const saveContent = useCallback(
    async (memory: ICompanionMemory, content: string) => mutate(() => ipcBridge.companion.updateMemory.invoke({
        memory_id: memory.memory_id,
        expected_revision: memory.revision,
        content: content.trim(),
        companion_id: companionId,
      })),
    [companionId, mutate]
  );

  const setPinned = useCallback(
    async (memory: ICompanionMemory, pinned: boolean) => mutate(() => ipcBridge.companion.updateMemory.invoke({
        memory_id: memory.memory_id,
        expected_revision: memory.revision,
        pinned,
        companion_id: companionId,
      })),
    [companionId, mutate]
  );

  const setArchived = useCallback(
    async (memory: ICompanionMemory, archived: boolean) => mutate(() => ipcBridge.companion.updateMemory.invoke({
        memory_id: memory.memory_id,
        expected_revision: memory.revision,
        status: archived ? 'archived' : 'active',
        companion_id: companionId,
      })),
    [companionId, mutate]
  );

  const removeMemory = useCallback(
    async (memory: ICompanionMemory) => mutate(() => ipcBridge.companion.deleteMemory.invoke({
      memory_id: memory.memory_id, expected_revision: memory.revision, companion_id: companionId })),
    [companionId, mutate]
  );

  /**
   * Runs an atomic batch over the current selection. Returns how many memories
   * the request covered — `0` means the selection was already empty and nothing
   * was sent, so the caller must not report success.
   */
  const runBatch = useCallback(
    async (action: ICompanionMemoryBatchAction, batchKind?: ICompanionMemoryKind, targets = selectedMemories): Promise<number> => {
      if (targets.length === 0) return 0;
      const count = targets.length;
      await mutate(() => ipcBridge.companion.batchMemories.invoke({
        ids: targets.map(memory => memory.memory_id),
        expected_revisions: Object.fromEntries(targets.map(memory => [memory.memory_id, memory.revision])),
        action,
        kind: batchKind,
        companion_id: companionId,
      }));
      setSelectedMemories(current => current.length === targets.length && current.every((memory, index) =>
        memory.memory_id === targets[index].memory_id && memory.revision === targets[index].revision) ? [] : current);
      return count;
    },
    [companionId, selectedMemories, mutate]
  );

  return {
    // query
    q,
    setQ,
    kind,
    setKind,
    status,
    setStatus,
    sort,
    setSort,
    // result set
    items,
    total,
    loading,
    page,
    pageSize,
    handlePageChange,
    refresh,
    // selection
    selected,
    selectedMemories,
    conflict,
    toggleSelected,
    clearSelection,
    selectAllOnPage,
    // mutations
    addMemory,
    saveContent,
    setPinned,
    setArchived,
    removeMemory,
    runBatch,
  };
};

export type MemoryListHandle = ReturnType<typeof useMemoryList>;
