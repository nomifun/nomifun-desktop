/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  canvasErrorMessage,
  formatCanvasTimestamp,
} from '../canvases/canvasList';
import type { CreativeStudioProjectSummary } from './types';

/** @deprecated Use mergeCanvases. */
export const mergeProjects = (
  current: readonly CreativeStudioProjectSummary[],
  incoming: readonly CreativeStudioProjectSummary[]
): CreativeStudioProjectSummary[] => {
  const merged = new Map(current.map((item) => [item.id, item]));
  for (const item of incoming) merged.set(item.id, item);
  return [...merged.values()];
};

/** @deprecated Use pruneCanvasSelection. */
export const pruneProjectSelection = (
  selectedIds: ReadonlySet<string>,
  items: readonly CreativeStudioProjectSummary[]
): Set<string> => {
  const present = new Set(items.map((item) => item.id));
  return new Set([...selectedIds].filter((id) => present.has(id)));
};

/** @deprecated Use formatCanvasTimestamp. */
export const formatProjectTimestamp = formatCanvasTimestamp;
/** @deprecated Use canvasErrorMessage. */
export const projectErrorMessage = canvasErrorMessage;
