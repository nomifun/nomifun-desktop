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
    if (item.runtimeEntry) return item.runtimeEntry;
    return {
      order,
      task: item.task,
      outputs: committedWorkbenchOutputs(item.task, scope.workbenchKind, assets),
      requestError: null,
      retryInput: null,
      outputKind: scope.workbenchKind,
    };
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
