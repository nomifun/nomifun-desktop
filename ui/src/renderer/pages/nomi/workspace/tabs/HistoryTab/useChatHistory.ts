/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import type { ICompanionDayDigest } from '@/common/adapter/ipcBridge';
import type { TMessage } from '@/common/chat/chatLib';
import type { CompanionId, ConversationId } from '@/common/types/ids';
import { messageCursorOf, toHistoryEntry, type DayKey, type HistoryEntry } from './historyFormat';

/** One window per 「加载更早」 press. Large enough that a normal day arrives whole. */
const WINDOW_SIZE = 80;
/** Digests are small summary rows; one fetch covers a long tail of days. */
const DIGEST_LIMIT = 200;

export interface HistoryDay {
  day: DayKey;
  entries: HistoryEntry[];
  digests: ICompanionDayDigest[];
}

export interface ChatHistory {
  /** `undefined` while resolving, `null` when this companion has never chatted. */
  conversationId: ConversationId | null | undefined;
  days: HistoryDay[];
  /** Total reader lines loaded so far (not raw rows). */
  entryCount: number;
  /** Oldest day loaded — the honest boundary of what the rail can show. */
  oldestDay: DayKey | null;
  hasMore: boolean;
  loading: boolean;
  loadingMore: boolean;
  failed: boolean;
  loadMore: () => void;
  retry: () => void;
}

/**
 * Client-side day index over a companion's single long-lived conversation.
 *
 * Read-only by construction: the session is resolved with `getCompanionSession`
 * (never `ensureCompanionSession`, which would MINT one and 400 for a companion
 * with no model configured). There is no day-index endpoint, so days are derived
 * by paging the keyset cursor newest→oldest and grouping on the LOCAL calendar
 * day; the UI says so rather than pretending the index is complete.
 */
export const useChatHistory = (companionId: CompanionId): ChatHistory => {
  const [conversationId, setConversationId] = useState<ConversationId | null | undefined>(undefined);
  const [messages, setMessages] = useState<TMessage[]>([]);
  const [digests, setDigests] = useState<ICompanionDayDigest[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);

  // Every async result is stamped with the run it belongs to; a companion switch
  // or a retry bumps the run so a late response can never repaint the new view.
  const runRef = useRef(0);
  const cursorRef = useRef<string | null>('');
  const busyRef = useRef(false);

  const fetchWindow = useCallback(
    async (run: number, id: ConversationId, cursor: string) => {
      const page = await ipcBridge.database.getConversationMessages.invoke({
        conversation_id: id,
        cursor,
        page_size: WINDOW_SIZE,
        content_mode: 'compact',
      });
      if (runRef.current !== run) return;
      // The keyset path returns each window oldest-first, so items[0] is the
      // oldest row and becomes the cursor for the next (older) window.
      const older = page.items ?? [];
      const nextCursor = older.length ? messageCursorOf(older[0]) : null;
      setMessages((prev) => {
        const seen = new Set(prev.map((m) => String(m.message_id ?? m.msg_id ?? m.id)));
        const fresh = older.filter((m) => !seen.has(String(m.message_id ?? m.msg_id ?? m.id)));
        return fresh.length ? [...fresh, ...prev] : prev;
      });
      // An empty window — or one whose oldest row carries no keyset identity —
      // leaves nothing we could actually ask for next, so 「加载更早」 must not
      // appear: a control that cannot move is worse than an honest boundary.
      setHasMore(Boolean(page.has_more) && nextCursor !== null);
      cursorRef.current = nextCursor;
    },
    []
  );

  useEffect(() => {
    const run = runRef.current + 1;
    runRef.current = run;
    cursorRef.current = '';
    busyRef.current = false;
    setConversationId(undefined);
    setMessages([]);
    setDigests([]);
    setHasMore(false);
    setFailed(false);
    setLoading(true);
    // A 加载更早 still in flight belongs to the previous run: its `finally` will
    // no longer clear this flag, so clear it here or the new companion inherits
    // a permanently spinning (and unclickable) 「加载更早」.
    setLoadingMore(false);

    void (async () => {
      try {
        const active = await ipcBridge.companion.getCompanionSession.invoke({ companion_id: companionId });
        if (runRef.current !== run) return;
        setConversationId(active.conversation_id);
        if (!active.conversation_id) {
          setLoading(false);
          return;
        }
        await fetchWindow(run, active.conversation_id, '');
      } catch {
        if (runRef.current !== run) return;
        setFailed(true);
      } finally {
        if (runRef.current === run) setLoading(false);
      }
      // Digests are a bonus layer (archiving defaults OFF): their absence or
      // failure must never block the message reader.
      try {
        const rows = await ipcBridge.companion.listDayDigests.invoke({
          companion_id: companionId,
          limit: DIGEST_LIMIT,
        });
        if (runRef.current === run) setDigests(rows);
      } catch {
        /* no digests — the reader stands on messages alone */
      }
    })();

    // Unmounting is just another run change: bump the stamp so nothing in flight
    // can land on a dead component.
    return () => {
      runRef.current += 1;
    };
  }, [companionId, attempt, fetchWindow]);

  const loadMore = useCallback(() => {
    const run = runRef.current;
    const id = conversationId;
    const cursor = cursorRef.current;
    if (!id || !cursor || busyRef.current || !hasMore) return;
    busyRef.current = true;
    setLoadingMore(true);
    setFailed(false);
    void (async () => {
      try {
        await fetchWindow(run, id, cursor);
      } catch {
        if (runRef.current === run) setFailed(true);
      } finally {
        // Only the run that owns the flags may clear them; a newer run has
        // already reset them and must not have its own in-flight load unlocked.
        if (runRef.current === run) {
          busyRef.current = false;
          setLoadingMore(false);
        }
      }
    })();
  }, [conversationId, fetchWindow, hasMore]);

  const retry = useCallback(() => setAttempt((n) => n + 1), []);

  const { days, entryCount } = useMemo(() => {
    const digestsByDay = new Map<DayKey, ICompanionDayDigest[]>();
    for (const digest of digests) {
      const list = digestsByDay.get(digest.session_day) ?? [];
      list.push(digest);
      digestsByDay.set(digest.session_day, list);
    }

    const byDay = new Map<DayKey, HistoryEntry[]>();
    let total = 0;
    for (const message of messages) {
      const entry = toHistoryEntry(message);
      if (!entry) continue;
      total += 1;
      const list = byDay.get(entry.day) ?? [];
      list.push(entry);
      byDay.set(entry.day, list);
    }

    const ordered = Array.from(byDay.entries())
      .sort((a, b) => b[0].localeCompare(a[0]))
      .map(([day, entries]) => ({
        day,
        entries: entries.slice().sort((a, b) => a.createdAt - b.createdAt),
        digests: digestsByDay.get(day) ?? [],
      }));
    return { days: ordered, entryCount: total };
  }, [messages, digests]);

  return {
    conversationId,
    days,
    entryCount,
    oldestDay: days.length ? days[days.length - 1].day : null,
    hasMore,
    loading,
    loadingMore,
    failed,
    loadMore,
    retry,
  };
};
