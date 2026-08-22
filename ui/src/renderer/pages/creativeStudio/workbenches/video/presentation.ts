/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { VideoResultsState, VideoWorkbenchTask } from './types';

export const videoResultsState = (tasks: readonly VideoWorkbenchTask[]): VideoResultsState => {
  if (tasks.length === 0) return 'empty';
  const statuses = new Set(tasks.map((task) => task.status));
  if (statuses.size > 1) return 'mixed';
  return tasks[0].status;
};

export const clampVideoProgress = (progress: number | undefined): number | null => {
  if (progress === undefined || !Number.isFinite(progress)) return null;
  return Math.max(0, Math.min(100, Math.floor(progress)));
};

export const normalizeVideoTaskCount = (value: number): number => {
  if (!Number.isFinite(value)) return 1;
  return Math.max(1, Math.min(6, Math.floor(value)));
};

export const toggleVideoTaskSelection = (
  selectedIds: readonly string[],
  taskId: string,
  selected: boolean
): string[] => {
  const current = new Set(selectedIds);
  if (selected) current.add(taskId);
  else current.delete(taskId);
  return [...current];
};

export const toggleAllVideoTasks = (
  taskIds: readonly string[],
  selectedIds: readonly string[]
): string[] => {
  const visible = new Set(taskIds);
  const allSelected = taskIds.length > 0 && taskIds.every((id) => selectedIds.includes(id));
  if (allSelected) return selectedIds.filter((id) => !visible.has(id));
  return [...new Set([...selectedIds, ...taskIds])];
};
