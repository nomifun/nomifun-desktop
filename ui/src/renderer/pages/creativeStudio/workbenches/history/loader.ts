/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeStandaloneTaskHistoryPage,
  CreativeStandaloneTaskHistoryQuery,
  CreativeTask,
} from '../../tasks';

import type { StandaloneWorkbenchHistoryScope } from './model';

export interface StandaloneTaskHistoryReader {
  listStandalone(
    input: CreativeStandaloneTaskHistoryQuery,
    signal?: AbortSignal
  ): Promise<CreativeStandaloneTaskHistoryPage>;
}

export interface StandaloneWorkbenchHistoryBootstrap {
  tasks: CreativeTask[];
  activeTasks: CreativeTask[];
  nextCursor: string | null;
}

export const STANDALONE_HISTORY_PAGE_LIMIT = 30;
export const STANDALONE_ACTIVE_PAGE_LIMIT = 100;
const MAX_ACTIVE_PAGES = 100;

const ensureNoRepeatedTaskIds = (
  current: readonly CreativeTask[],
  incoming: readonly CreativeTask[]
): void => {
  const ids = new Set(current.map((task) => task.taskId));
  const duplicate = incoming.find((task) => ids.has(task.taskId));
  if (duplicate) {
    throw new Error(`Standalone history repeated task ${duplicate.taskId} across pages`);
  }
};

export function appendStandaloneHistoryPage(
  current: readonly CreativeTask[],
  page: CreativeStandaloneTaskHistoryPage
): CreativeTask[] {
  ensureNoRepeatedTaskIds(current, page.items);
  return [...current, ...page.items];
}

/** Visible history wins a same-task race against the active-only inventory. */
export function reconcileStandaloneActiveTasks(
  visible: readonly CreativeTask[],
  active: readonly CreativeTask[]
): CreativeTask[] {
  const visibleById = new Map(visible.map((task) => [task.taskId, task]));
  const reconciled = active.flatMap((task) => {
    const current = visibleById.get(task.taskId);
    if (!current) return [task];
    return current.status === 'queued' || current.status === 'running' ? [current] : [];
  });
  const reconciledIds = new Set(reconciled.map((task) => task.taskId));
  for (const task of visible) {
    if (
      (task.status === 'queued' || task.status === 'running') &&
      !reconciledIds.has(task.taskId)
    ) {
      reconciled.push(task);
      reconciledIds.add(task.taskId);
    }
  }
  return reconciled.sort((left, right) => {
    if (left.submittedAt !== right.submittedAt) {
      return left.submittedAt > right.submittedAt ? -1 : 1;
    }
    return left.taskId === right.taskId ? 0 : left.taskId > right.taskId ? -1 : 1;
  });
}

export function combineStandaloneHistoryTasks(
  visible: readonly CreativeTask[],
  active: readonly CreativeTask[]
): CreativeTask[] {
  const combined = [...visible];
  const visibleIds = new Set(visible.map((task) => task.taskId));
  for (const task of active) {
    if (!visibleIds.has(task.taskId)) combined.push(task);
  }
  return combined.sort((left, right) => {
    if (left.submittedAt !== right.submittedAt) {
      return left.submittedAt > right.submittedAt ? -1 : 1;
    }
    return left.taskId === right.taskId ? 0 : left.taskId > right.taskId ? -1 : 1;
  });
}

async function loadAllActiveTasks(
  reader: StandaloneTaskHistoryReader,
  scope: StandaloneWorkbenchHistoryScope,
  signal?: AbortSignal
): Promise<CreativeTask[]> {
  let cursor: string | null = null;
  let tasks: CreativeTask[] = [];
  for (let pageIndex = 0; pageIndex < MAX_ACTIVE_PAGES; pageIndex += 1) {
    const page = await reader.listStandalone(
      {
        ...scope,
        limit: STANDALONE_ACTIVE_PAGE_LIMIT,
        cursor,
        activeOnly: true,
      },
      signal
    );
    tasks = appendStandaloneHistoryPage(tasks, page);
    if (!page.nextCursor) return tasks;
    cursor = page.nextCursor;
  }
  throw new Error(
    `Standalone recovery inventory exceeds ${MAX_ACTIVE_PAGES * STANDALONE_ACTIVE_PAGE_LIMIT} active tasks`
  );
}

export async function loadStandaloneWorkbenchHistoryBootstrap(
  reader: StandaloneTaskHistoryReader,
  scope: StandaloneWorkbenchHistoryScope,
  signal?: AbortSignal
): Promise<StandaloneWorkbenchHistoryBootstrap> {
  const [visible, active] = await Promise.all([
    reader.listStandalone(
      { ...scope, limit: STANDALONE_HISTORY_PAGE_LIMIT },
      signal
    ),
    loadAllActiveTasks(reader, scope, signal),
  ]);
  return {
    tasks: visible.items,
    activeTasks: reconcileStandaloneActiveTasks(visible.items, active),
    nextCursor: visible.nextCursor,
  };
}
