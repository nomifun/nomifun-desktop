/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';

export type KnowledgeSort = 'updated' | 'created' | 'name' | 'size';

export type KnowledgeSortDirection = 'asc' | 'desc';

/** Sort a copy of the knowledge-base list without mutating the source array. */
export function sortKnowledgeBases(
  bases: IKnowledgeBase[],
  sort: KnowledgeSort,
  direction: KnowledgeSortDirection
): IKnowledgeBase[] {
  const directionFactor = direction === 'asc' ? 1 : -1;

  return [...bases].sort((a, b) => {
    let comparison = 0;

    switch (sort) {
      case 'updated':
        comparison = a.updated_at - b.updated_at;
        break;
      case 'created':
        comparison = a.created_at - b.created_at;
        break;
      case 'name':
        comparison = a.name.localeCompare(b.name);
        break;
      case 'size':
        comparison = a.total_size - b.total_size;
        break;
    }

    return comparison * directionFactor;
  });
}
