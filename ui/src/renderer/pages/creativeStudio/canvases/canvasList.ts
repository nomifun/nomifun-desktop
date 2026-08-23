/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeCanvasSummary } from '../domain';

export const mergeCanvases = (
  current: readonly CreativeCanvasSummary[],
  incoming: readonly CreativeCanvasSummary[]
): CreativeCanvasSummary[] => {
  const merged = new Map(current.map((canvas) => [canvas.canvasId, canvas]));
  for (const canvas of incoming) merged.set(canvas.canvasId, canvas);
  return [...merged.values()];
};

export const pruneCanvasSelection = (
  selectedIds: ReadonlySet<string>,
  canvases: readonly CreativeCanvasSummary[]
): Set<string> => {
  const present = new Set(canvases.map((canvas) => canvas.canvasId));
  return new Set([...selectedIds].filter((canvasId) => present.has(canvasId)));
};

export const formatCanvasTimestamp = (
  value: number,
  language: string | undefined
): string => {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return '—';
  return new Intl.DateTimeFormat(
    language?.toLowerCase().startsWith('zh') ? 'zh-CN' : 'en-US',
    {
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
    }
  ).format(date);
};

export const canvasErrorMessage = (error: unknown): string =>
  error instanceof Error && error.message.trim()
    ? error.message
    : String(error || 'Unknown error');
