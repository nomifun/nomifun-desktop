/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAssetUploadItem } from '../components';

export type CreativeAssetUploadQueueAction =
  | { type: 'enqueue'; item: CreativeAssetUploadItem }
  | { type: 'restart'; id: string }
  | { type: 'progress'; id: string; percent: number }
  | { type: 'complete'; id: string }
  | { type: 'fail'; id: string; error: string }
  | { type: 'dismiss'; id: string };

const clampPercent = (percent: number): number =>
  Math.max(0, Math.min(100, Number.isFinite(percent) ? percent : 0));

export function creativeAssetUploadQueueReducer(
  items: readonly CreativeAssetUploadItem[],
  action: CreativeAssetUploadQueueAction
): CreativeAssetUploadItem[] {
  if (action.type === 'enqueue') return [action.item, ...items];
  if (action.type === 'dismiss') return items.filter((item) => item.id !== action.id);

  return items.map((item) => {
    if (item.id !== action.id) return item;
    switch (action.type) {
      case 'restart':
        return { ...item, percent: 0, status: 'uploading', error: undefined };
      case 'progress':
        return item.status === 'uploading'
          ? { ...item, percent: clampPercent(action.percent) }
          : item;
      case 'complete':
        return { ...item, percent: 100, status: 'completed', error: undefined };
      case 'fail':
        return { ...item, status: 'error', error: action.error };
    }
  });
}
