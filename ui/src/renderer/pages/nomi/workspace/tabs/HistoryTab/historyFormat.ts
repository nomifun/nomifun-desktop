/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import dayjs from 'dayjs';
import type { TMessage } from '@/common/chat/chatLib';

/**
 * Local calendar day key, `YYYYMMDD`. Always the BACKEND's day: it is what the
 * day index and the archive digests are partitioned by, and the browser's own
 * timezone may differ, so nothing here derives a day from a timestamp.
 */
export type DayKey = string;

/** `YYYYMMDD` → `YYYY-MM-DD` (defensive on unexpected shapes). */
export const formatDayKey = (day: DayKey): string =>
  day.length === 8 ? `${day.slice(0, 4)}-${day.slice(4, 6)}-${day.slice(6, 8)}` : day;

/** `YYYYMMDD` → `MM-DD`, the compact form used in the day rail. */
export const formatDayKeyShort = (day: DayKey): string =>
  day.length === 8 ? `${day.slice(4, 6)}-${day.slice(6, 8)}` : day;

/** `MMDD` → `MM-DD`. */
export const formatMmdd = (mmdd: string): string =>
  mmdd.length === 4 ? `${mmdd.slice(0, 2)}-${mmdd.slice(2, 4)}` : mmdd;

/** Today's `MMDD` (local) — the key for the 「去年今日」 query. */
export const todayMmdd = (): string => dayjs().format('MMDD');

export const isToday = (day: DayKey): boolean => day === dayjs().format('YYYYMMDD');
export const isYesterday = (day: DayKey): boolean => day === dayjs().subtract(1, 'day').format('YYYYMMDD');

/** `HH:mm` for a message timestamp. */
export const formatClock = (createdAtMs: number): string => dayjs(createdAtMs).format('HH:mm');

export type HistoryRole = 'user' | 'companion';

/** One renderable line of history. Deliberately lossy — this is a reader, not the chat. */
export interface HistoryEntry {
  key: string;
  role: HistoryRole;
  createdAt: number;
  kind: 'text' | 'thinking' | 'tool' | 'note';
  text: string;
}

const firstString = (...values: unknown[]): string => {
  for (const value of values) {
    if (typeof value === 'string' && value.trim()) return value;
  }
  return '';
};

/**
 * Project a persisted message onto a plain-text reader line, or `null` when it
 * carries nothing a human wants to re-read (permissions, status pings, command
 * manifests). `position === 'right'` is the user side, as everywhere else in the app.
 */
export const toHistoryEntry = (message: TMessage): HistoryEntry | null => {
  if (message.hidden) return null;
  const createdAt = message.created_at ?? 0;
  if (!createdAt) return null;

  const base = {
    key: String(message.message_id ?? message.msg_id ?? message.id),
    role: (message.position === 'right' ? 'user' : 'companion') as HistoryRole,
    createdAt,
  };

  switch (message.type) {
    case 'text': {
      const text = typeof message.content?.content === 'string' ? message.content.content : '';
      if (!text.trim()) return null;
      return { ...base, kind: 'text', text };
    }
    case 'thinking': {
      const text = typeof message.content?.content === 'string' ? message.content.content : '';
      if (!text.trim()) return null;
      return { ...base, kind: 'thinking', text };
    }
    case 'tips': {
      const text = typeof message.content?.content === 'string' ? message.content.content : '';
      if (!text.trim()) return null;
      return { ...base, kind: 'note', text };
    }
    case 'tool_call':
      return { ...base, kind: 'tool', text: firstString(message.content?.name, message.content?.description) };
    case 'tool_group': {
      const calls = Array.isArray(message.content) ? message.content : [];
      const names = calls
        .map((call) => firstString(call?.name, call?.description))
        .filter((name) => name.length > 0);
      return { ...base, kind: 'tool', text: names.join('、') };
    }
    default:
      return null;
  }
};

/** Parsed `highlights` JSON of a day digest. */
export const parseDigestHighlights = (raw: string | null): { topics: string[]; mood?: string } => {
  if (!raw) return { topics: [] };
  try {
    const parsed = JSON.parse(raw) as { topics?: unknown; mood?: unknown };
    return {
      topics: Array.isArray(parsed.topics) ? parsed.topics.filter((x): x is string => typeof x === 'string') : [],
      mood: typeof parsed.mood === 'string' ? parsed.mood : undefined,
    };
  } catch {
    return { topics: [] };
  }
};
