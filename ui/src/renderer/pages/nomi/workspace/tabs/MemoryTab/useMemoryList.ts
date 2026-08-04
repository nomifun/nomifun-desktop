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

/**
 * The memory list of ONE companion. Every request carries
 * `scope_companion_id: companionId` — this surface has no cross-companion mode
 * and no scope picker, so the companion id is the single source of truth for
 * what the user is looking at and what a new memory belongs to.
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
  const [selected, setSelected] = useState<CompanionMemoryId[]>([]);

  // Out-of-order guard: rapid typing fires overlapping requests and a slow stale
  // response must never clobber a newer one.
  const seqRef = useRef(0);

  const refresh = useCallback(async () => {
    const seq = ++seqRef.current;
    setLoading(true);
    try {
      const result = await ipcBridge.companion.listMemories.invoke({
        kind: kind || undefined,
        q: q.trim() || undefined,
        status,
        scope_companion_id: companionId,
        sort,
        limit: pageSize,
        offset: (page - 1) * pageSize,
      });
      if (seq !== seqRef.current) return;
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
      if (seq === seqRef.current) Message.error(String(e));
    } finally {
      if (seq === seqRef.current) setLoading(false);
    }
  }, [companionId, kind, q, status, sort, page, pageSize]);

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
    setSelected([]);
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
    setSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));
  }, []);

  const clearSelection = useCallback(() => setSelected([]), []);

  const selectAllOnPage = useCallback(() => {
    setSelected(items.map((m) => m.memory_id));
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
  // `updateMemory` never sends a scope: the backend rejects `scope_companion_id`
  // without an explicit `scope_kind`, and omitting both means "scope unchanged",
  // which is exactly right — editing text must not re-home a memory.

  const addMemory = useCallback(
    async (nextKind: ICompanionMemoryKind, content: string) => {
      await ipcBridge.companion.addMemory.invoke({
        kind: nextKind,
        content: content.trim(),
        scope_companion_id: companionId,
      });
      void refresh();
    },
    [companionId, refresh]
  );

  const saveContent = useCallback(
    async (memoryId: CompanionMemoryId, content: string) => {
      await ipcBridge.companion.updateMemory.invoke({ memory_id: memoryId, content: content.trim() });
      void refresh();
    },
    [refresh]
  );

  const setPinned = useCallback(
    async (memory: ICompanionMemory, pinned: boolean) => {
      await ipcBridge.companion.updateMemory.invoke({ memory_id: memory.memory_id, pinned });
      void refresh();
    },
    [refresh]
  );

  const setArchived = useCallback(
    async (memory: ICompanionMemory, archived: boolean) => {
      await ipcBridge.companion.updateMemory.invoke({
        memory_id: memory.memory_id,
        status: archived ? 'archived' : 'active',
      });
      void refresh();
    },
    [refresh]
  );

  const removeMemory = useCallback(
    async (memoryId: CompanionMemoryId) => {
      await ipcBridge.companion.deleteMemory.invoke({ memory_id: memoryId });
      void refresh();
    },
    [refresh]
  );

  /**
   * Runs an atomic batch over the current selection. Returns how many memories
   * the request covered — `0` means the selection was already empty and nothing
   * was sent, so the caller must not report success.
   */
  const runBatch = useCallback(
    async (action: ICompanionMemoryBatchAction, batchKind?: ICompanionMemoryKind): Promise<number> => {
      if (selected.length === 0) return 0;
      const count = selected.length;
      await ipcBridge.companion.batchMemories.invoke({ ids: selected, action, kind: batchKind });
      setSelected([]);
      void refresh();
      return count;
    },
    [selected, refresh]
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
