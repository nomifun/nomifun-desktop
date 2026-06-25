/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { SessionEntry, SessionKind, WorkpathNode } from './workpathTree';

export const WORKPATH_COLLAPSED_SESSION_LIMIT = 5;

export type VisibleWorkpathKindMeta = {
  hasOverflow: boolean;
  hiddenCount: number;
};

export type VisibleWorkpathEntries = {
  interactive: SessionEntry[];
  terminal: SessionEntry[];
  kindMeta: Record<SessionKind, VisibleWorkpathKindMeta>;
  hasOverflow: boolean;
  hiddenCount: number;
};

type ExpandedKinds = boolean | Partial<Record<SessionKind, boolean>>;

function isKindExpanded(expanded: ExpandedKinds, kind: SessionKind): boolean {
  return typeof expanded === 'boolean' ? expanded : !!expanded[kind];
}

function getVisibleKindEntries(
  entries: SessionEntry[],
  expanded: boolean,
  limit: number
): { entries: SessionEntry[]; meta: VisibleWorkpathKindMeta } {
  const hasOverflow = entries.length > limit;

  if (expanded || !hasOverflow) {
    return {
      entries,
      meta: {
        hasOverflow,
        hiddenCount: 0,
      },
    };
  }

  const visibleEntries = entries.slice(0, limit);

  return {
    entries: visibleEntries,
    meta: {
      hasOverflow,
      hiddenCount: entries.length - visibleEntries.length,
    },
  };
}

export function getVisibleWorkpathEntries(
  node: WorkpathNode,
  expanded: ExpandedKinds,
  limit = WORKPATH_COLLAPSED_SESSION_LIMIT
): VisibleWorkpathEntries {
  const interactive = getVisibleKindEntries(node.interactive, isKindExpanded(expanded, 'interactive'), limit);
  const terminal = getVisibleKindEntries(node.terminal, isKindExpanded(expanded, 'terminal'), limit);
  const hasOverflow = interactive.meta.hasOverflow || terminal.meta.hasOverflow;
  const hiddenCount = interactive.meta.hiddenCount + terminal.meta.hiddenCount;

  return {
    interactive: interactive.entries,
    terminal: terminal.entries,
    kindMeta: {
      interactive: interactive.meta,
      terminal: terminal.meta,
    },
    hasOverflow,
    hiddenCount,
  };
}

export function getWorkpathEntryDisplayIndex(node: WorkpathNode, entry: Pick<SessionEntry, 'kind' | 'id'>): number | null {
  if (entry.kind === 'interactive') {
    const index = node.interactive.findIndex((item) => item.id === entry.id);
    return index === -1 ? null : index;
  }

  const index = node.terminal.findIndex((item) => item.id === entry.id);
  return index === -1 ? null : index;
}
