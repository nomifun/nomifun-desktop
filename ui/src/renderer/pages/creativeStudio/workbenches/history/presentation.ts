/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAssetPort, CreativeAssetAvailability } from '../../assets';
import {
  creativeTaskReference,
  type CreativeTask,
} from '../../tasks';
import { committedWorkbenchOutputs } from '../runtime/assets';
import type {
  CreativeWorkbenchResumeRequest,
  CreativeWorkbenchRuntimeEntry,
  CreativeWorkbenchRuntimeSnapshot,
} from '../runtime';

import {
  isExactStandaloneWorkbenchHistoryTask,
  mergeStandaloneWorkbenchHistory,
  type StandaloneWorkbenchHistoryScope,
} from './model';

export function standaloneHistoryResumeRequests(
  scope: StandaloneWorkbenchHistoryScope,
  tasks: readonly CreativeTask[]
): CreativeWorkbenchResumeRequest[] {
  return tasks.map((task) => {
    if (
      !isExactStandaloneWorkbenchHistoryTask(task, scope) ||
      (task.status !== 'queued' && task.status !== 'running')
    ) {
      throw new Error(`Task ${task.taskId} is not an active ${scope.workbenchKind} recovery task`);
    }
    return {
      reference: creativeTaskReference(task),
      outputKind: scope.workbenchKind,
      retryInput: null,
    };
  });
}

export function standaloneHistoryRuntimeSnapshot(
  scope: StandaloneWorkbenchHistoryScope,
  durableTasks: readonly CreativeTask[],
  runtime: CreativeWorkbenchRuntimeSnapshot,
  assets: CreativeAssetPort,
  availability?: ReadonlyMap<string, CreativeAssetAvailability>
): CreativeWorkbenchRuntimeSnapshot {
  const history = mergeStandaloneWorkbenchHistory({
    scope,
    durableTasks,
    runtime,
  });
  const entries = history.map((item, order): CreativeWorkbenchRuntimeEntry => {
    const entry = item.runtimeEntry ?? {
      order,
      task: item.task,
      outputs: committedWorkbenchOutputs(item.task, scope.workbenchKind, assets),
      requestError: null,
      retryInput: null,
      outputKind: scope.workbenchKind,
    };
    return { ...entry, historyTaskIds: [...item.attemptTaskIds] };
  });
  return {
    ...runtime,
    entries: availability ? entries.map((entry) => ({
      ...entry,
      hasDeletedInputs: entry.task.inputs?.some((input) => availability.get(input.assetId) === 'deleted') ?? false,
      outputs: entry.outputs.map((output) => ({
        ...output,
        availability: availability.get(output.assetId) ?? 'loading',
      })),
    })) : entries,
  };
}

/** Expand visible logical cards to every durable retry attempt they represent. */
export function standaloneHistoryRetirementTaskIds(
  runtime: Pick<CreativeWorkbenchRuntimeSnapshot, 'entries'>,
  visibleTaskIds: readonly string[]
): string[] {
  const entries = new Map(runtime.entries.map((entry) => [entry.task.taskId, entry]));
  const expanded = new Set<string>();
  for (const taskId of visibleTaskIds) {
    const entry = entries.get(taskId);
    for (const historyTaskId of entry?.historyTaskIds ?? [taskId]) {
      expanded.add(historyTaskId);
    }
  }
  return [...expanded];
}
