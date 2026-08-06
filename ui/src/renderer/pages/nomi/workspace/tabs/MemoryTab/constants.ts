/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ICompanionMemoryKind } from '@/common/adapter/ipcBridge';

/** The six memory kinds, in the order the filter and pickers list them. */
export const MEMORY_KINDS: readonly ICompanionMemoryKind[] = [
  'profile',
  'preference',
  'knowledge',
  'episode',
  'task',
  'affective',
];

/**
 * One dot colour per kind. The row itself stays monochrome — a 6px dot carries
 * the type instead of a coloured tag, so a hundred rows never read as confetti.
 */
export const MEMORY_KIND_DOT: Record<ICompanionMemoryKind, string> = {
  profile: 'rgb(var(--arcoblue-6))',
  preference: 'rgb(var(--pinkpurple-6))',
  knowledge: 'rgb(var(--green-6))',
  episode: 'rgb(var(--orange-6))',
  task: 'rgb(var(--red-6))',
  affective: 'rgb(var(--purple-6))',
};

/** Status filter of the list; `all` maps to the backend's `status=all`. */
export type MemoryStatusFilter = 'active' | 'archived' | 'all';

export const MEMORY_STATUS_FILTERS: readonly MemoryStatusFilter[] = ['active', 'archived', 'all'];

export const MEMORY_PAGE_SIZE_OPTIONS = [10, 20, 50];

/** Compact, locale-aware, 24h — memory rows show a stamp, not a sentence. */
export const formatMemoryTime = (timestamp: number): string =>
  new Date(timestamp).toLocaleString(undefined, {
    year: '2-digit',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  });
