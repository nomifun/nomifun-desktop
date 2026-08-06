/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import type { ICompanionDayDigest } from '@/common/adapter/ipcBridge';
import type { CompanionId, ConversationId } from '@/common/types/ids';
import { toHistoryEntry, type DayKey, type HistoryEntry } from './historyFormat';

/**
 * Cap on the messages one day's read may return. The backend caps too and says
 * so via `has_more`; a day past this is surfaced as truncated rather than
 * silently cut.
 */
const DAY_PAGE_SIZE = 500;
/** One day can hold several archived windows; this is far above any real count. */
const DAY_DIGEST_LIMIT = 20;

/** One day in the rail — server-side, complete, and never derived here. */
export interface HistoryDay {
  day: DayKey;
  /** Visible messages the backend counted for that day. */
  messageCount: number;
  /** 会话归档 left a diary on that day. */
  hasDigest: boolean;
}

export interface HistoryIndex {
  /** `undefined` while resolving, `null` when this companion has never chatted. */
  conversationId: ConversationId | null | undefined;
  /** Every day this companion has history on, newest first. Complete. */
  days: HistoryDay[];
  /** Visible messages across the whole history. */
  messageCount: number;
  loading: boolean;
  failed: boolean;
  retry: () => void;
}

/** The selected day's content: its messages, plus that day's diaries. */
export interface DayContent {
  entries: HistoryEntry[];
  digests: ICompanionDayDigest[];
  /** The day is longer than one read; only its earliest messages are shown. */
  truncated: boolean;
  loading: boolean;
  failed: boolean;
  retry: () => void;
}

/**
 * The companion's history day index, read from the server.
 *
 * Read-only by construction: the conversation is RESOLVED with
 * `getCompanionSession` (never `ensureCompanionSession`, which would MINT one and
 * 400 for a companion with no model configured), and the index endpoint resolves
 * it the same way. The days are the backend's LOCAL calendar days — the same key
 * that partitions archive digests — so nothing here re-buckets timestamps in a
 * browser whose timezone need not match the backend's.
 */
export const useHistoryDays = (companionId: CompanionId): HistoryIndex => {
  const [conversationId, setConversationId] = useState<ConversationId | null | undefined>(undefined);
  const [days, setDays] = useState<HistoryDay[]>([]);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);

  // Every async result is stamped with the run it belongs to; a companion switch
  // or a retry bumps the run so a late response can never repaint the new view.
  const runRef = useRef(0);

  useEffect(() => {
    const run = runRef.current + 1;
    runRef.current = run;
    setConversationId(undefined);
    setDays([]);
    setFailed(false);
    setLoading(true);

    void (async () => {
      try {
        const [active, index] = await Promise.all([
          ipcBridge.companion.getCompanionSession.invoke({ companion_id: companionId }),
          ipcBridge.companion.listHistoryDays.invoke({ companion_id: companionId }),
        ]);
        if (runRef.current !== run) return;
        setConversationId(active.conversation_id);
        setDays(
          index.map((entry) => ({
            day: entry.day,
            messageCount: entry.message_count,
            hasDigest: entry.has_digest,
          }))
        );
      } catch {
        if (runRef.current !== run) return;
        setFailed(true);
      } finally {
        if (runRef.current === run) setLoading(false);
      }
    })();

    // Unmounting is just another run change: bump the stamp so nothing in flight
    // can land on a dead component.
    return () => {
      runRef.current += 1;
    };
  }, [companionId, attempt]);

  const retry = useCallback(() => setAttempt((n) => n + 1), []);
  const messageCount = useMemo(() => days.reduce((total, entry) => total + entry.messageCount, 0), [days]);

  return { conversationId, days, messageCount, loading, failed, retry };
};

/**
 * One day of the conversation, fetched by day rather than by cursor: the backend
 * owns the day boundary, so the slice it returns IS that day and the reader never
 * has to trim or re-group it.
 *
 * Digests ride along as a bonus layer (归档 defaults off): their absence — or a
 * failed read — must never block the messages.
 */
export const useDayMessages = (
  companionId: CompanionId,
  conversationId: ConversationId | null | undefined,
  day: DayKey | null
): DayContent => {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [digests, setDigests] = useState<ICompanionDayDigest[]>([]);
  const [truncated, setTruncated] = useState(false);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);

  const runRef = useRef(0);

  useEffect(() => {
    const run = runRef.current + 1;
    runRef.current = run;
    setEntries([]);
    setDigests([]);
    setTruncated(false);
    setFailed(false);
    if (!conversationId || !day) {
      setLoading(false);
      return;
    }
    setLoading(true);

    void (async () => {
      try {
        const page = await ipcBridge.database.getConversationMessages.invoke({
          conversation_id: conversationId,
          day,
          page_size: DAY_PAGE_SIZE,
          content_mode: 'compact',
        });
        if (runRef.current !== run) return;
        setEntries((page.items ?? []).map(toHistoryEntry).filter((entry): entry is HistoryEntry => entry !== null));
        setTruncated(Boolean(page.has_more));
      } catch {
        if (runRef.current !== run) return;
        setFailed(true);
      } finally {
        if (runRef.current === run) setLoading(false);
      }
      try {
        const rows = await ipcBridge.companion.listDayDigests.invoke({
          companion_id: companionId,
          since: day,
          until: day,
          limit: DAY_DIGEST_LIMIT,
        });
        if (runRef.current === run) setDigests(rows);
      } catch {
        /* no digest — the reader stands on messages alone */
      }
    })();

    return () => {
      runRef.current += 1;
    };
  }, [companionId, conversationId, day, attempt]);

  const retry = useCallback(() => setAttempt((n) => n + 1), []);

  return { entries, digests, truncated, loading, failed, retry };
};
