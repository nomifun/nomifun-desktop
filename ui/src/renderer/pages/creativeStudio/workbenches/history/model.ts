/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CreativeTaskContractError,
  isStandaloneWorkbenchTaskOwner,
  sameCreativeTaskOwner,
  type CreateCreativeTaskInput,
  type CreativeStandaloneWorkbenchKind,
  type CreativeTask,
} from '../../tasks';
import type {
  CreativeWorkbenchRuntimeEntry,
  CreativeWorkbenchRuntimeSnapshot,
} from '../runtime';

export interface StandaloneWorkbenchHistoryScope {
  workbenchKind: CreativeStandaloneWorkbenchKind;
}

export interface StandaloneWorkbenchHistoryItem {
  /** Live state is authoritative when the same durable task is still mounted. */
  task: CreativeTask;
  source: 'durable' | 'live';
  /** Preserves all committed outputs as one task instead of splitting by asset. */
  runtimeEntry: CreativeWorkbenchRuntimeEntry | null;
  canRetry: boolean;
}

export interface MergeStandaloneWorkbenchHistoryInput {
  scope: StandaloneWorkbenchHistoryScope;
  durableTasks: readonly CreativeTask[];
  runtime: Pick<CreativeWorkbenchRuntimeSnapshot, 'entries'>;
}

const WORKBENCH_KINDS = new Set<CreativeStandaloneWorkbenchKind>([
  'image',
  'video',
  'audio',
]);

const invalidResponse = (message: string, field: string): never => {
  throw new CreativeTaskContractError(
    'invalid_response',
    message,
    field
  );
};

const assertScope = (scope: StandaloneWorkbenchHistoryScope): void => {
  if (!WORKBENCH_KINDS.has(scope.workbenchKind)) {
    throw new CreativeTaskContractError(
      'invalid_request',
      'Invalid standalone history workbenchKind',
      'scope.workbenchKind'
    );
  }
};

const taskMatchesWorkbenchKind = (
  task: CreativeTask,
  workbenchKind: CreativeStandaloneWorkbenchKind
): boolean => {
  if (workbenchKind === 'image') {
    return task.task === 'image_generation' || task.task === 'image_edit';
  }
  if (workbenchKind === 'video') return task.task === 'video_generation';
  return task.task === 'speech_synthesis';
};

/** Runtime-safe owner check used by both durable pages and live snapshots. */
export function isExactStandaloneWorkbenchHistoryTask(
  task: CreativeTask,
  scope: StandaloneWorkbenchHistoryScope
): boolean {
  return (
    isStandaloneWorkbenchTaskOwner(task.owner) &&
    task.owner.workbenchKind === scope.workbenchKind &&
    task.deletedAt === null &&
    taskMatchesWorkbenchKind(task, scope.workbenchKind)
  );
}

const assertTaskScope = (
  task: CreativeTask,
  scope: StandaloneWorkbenchHistoryScope,
  field: string
): void => {
  if (!isExactStandaloneWorkbenchHistoryTask(task, scope)) {
    throw new CreativeTaskContractError(
      'ownership_mismatch',
      `Task ${task.taskId} escaped its standalone history owner`,
      field
    );
  }
};

const canonicalize = (value: unknown): unknown => {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value === null || typeof value !== 'object') return value;
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
      .map(([key, item]) => [key, canonicalize(item)])
  );
};

const structurallyEqual = (left: unknown, right: unknown): boolean =>
  JSON.stringify(canonicalize(left)) === JSON.stringify(canonicalize(right));

const immutableTaskIdentity = (task: CreativeTask): unknown => ({
  taskId: task.taskId,
  owner: task.owner,
  providerId: task.providerId,
  model: task.model,
  task: task.task,
  capability: task.capability,
  parameters: task.parameters,
  inputs: task.inputs,
  submittedAt: task.submittedAt,
});

const assertLiveIdentity = (
  durable: CreativeTask,
  live: CreativeTask
): void => {
  if (!structurallyEqual(immutableTaskIdentity(durable), immutableTaskIdentity(live))) {
    invalidResponse(
      `Live task ${live.taskId} drifted from its durable identity`,
      'runtime.entries.task'
    );
  }
};

const assertRetryInput = (
  input: CreateCreativeTaskInput | null,
  task: CreativeTask,
  scope: StandaloneWorkbenchHistoryScope,
  field: string
): void => {
  if (!input) return;
  if (
    task.inputs === null ||
    !isStandaloneWorkbenchTaskOwner(input.owner) ||
    input.owner.workbenchKind !== scope.workbenchKind ||
    !sameCreativeTaskOwner(input.owner, task.owner) ||
    input.providerId !== task.providerId ||
    input.model !== task.model ||
    input.task !== task.task ||
    input.capability !== task.capability ||
    !structurallyEqual(input.parameters, task.parameters) ||
    !structurallyEqual(input.inputs, task.inputs)
  ) {
    invalidResponse(
      `Live task ${task.taskId} carries a mismatched retry input`,
      field
    );
  }
};

const assertRuntimeEntry = (
  entry: CreativeWorkbenchRuntimeEntry,
  scope: StandaloneWorkbenchHistoryScope,
  index: number
): void => {
  const field = `runtime.entries[${index}]`;
  assertTaskScope(entry.task, scope, `${field}.task.owner`);
  if (entry.outputKind !== scope.workbenchKind) {
    invalidResponse(
      `Live task ${entry.task.taskId} has output kind ${entry.outputKind}, expected ${scope.workbenchKind}`,
      `${field}.outputKind`
    );
  }
  if (entry.outputs.some((output) => output.kind !== scope.workbenchKind)) {
    invalidResponse(
      `Live task ${entry.task.taskId} contains a cross-workbench output`,
      `${field}.outputs`
    );
  }
  if (
    entry.outputs.some(
      (output) =>
        (output.kind !== 'audio' &&
          (typeof output.url !== 'string' || output.url.trim().length === 0)) ||
        (output.kind === 'audio' &&
          output.url !== null &&
          (typeof output.url !== 'string' || output.url.trim().length === 0))
    )
  ) {
    invalidResponse(
      `Live task ${entry.task.taskId} contains an invalid committed output URL`,
      `${field}.outputs`
    );
  }
  const outputAssetIds = entry.outputs.map((output) => output.assetId);
  if (entry.task.status !== 'succeeded' && outputAssetIds.length > 0) {
    invalidResponse(
      `Non-succeeded live task ${entry.task.taskId} cannot expose committed outputs`,
      `${field}.outputs`
    );
  }
  if (
    entry.task.status === 'succeeded' &&
    !structurallyEqual(outputAssetIds, entry.task.resultAssetIds) &&
    !(entry.requestError && outputAssetIds.length === 0)
  ) {
    invalidResponse(
      `Succeeded live task ${entry.task.taskId} outputs do not match its result asset order`,
      `${field}.outputs`
    );
  }
  assertRetryInput(entry.retryInput, entry.task, scope, `${field}.retryInput`);
};

const terminalStatuses = new Set<CreativeTask['status']>([
  'succeeded',
  'failed',
  'canceled',
]);

const taskProgress = (task: CreativeTask): number => {
  if (terminalStatuses.has(task.status)) return 2;
  return task.status === 'running' ? 1 : 0;
};

const mutableTaskState = (task: CreativeTask): unknown => ({
  status: task.status,
  error: task.error,
  resultAssetIds: task.resultAssetIds,
  attempt: task.attempt,
  startedAt: task.startedAt,
  finishedAt: task.finishedAt,
});

const liveMayReplaceDurable = (
  durable: CreativeTask,
  live: CreativeTask
): boolean => {
  const durableTerminal = terminalStatuses.has(durable.status);
  const liveTerminal = terminalStatuses.has(live.status);
  if (durableTerminal && liveTerminal) {
    if (!structurallyEqual(mutableTaskState(durable), mutableTaskState(live))) {
      invalidResponse(
        `Task ${live.taskId} has conflicting durable and live terminal states`,
        'runtime.entries.task'
      );
    }
    return true;
  }
  return taskProgress(live) >= taskProgress(durable);
};

/**
 * Legacy rows with `inputs: null` have no exact binding proof. Only failed or
 * canceled tasks with a complete input snapshot may create a new retry.
 */
export function canRetryStandaloneWorkbenchHistoryTask(
  task: CreativeTask
): boolean {
  return (
    task.inputs !== null &&
    (task.status === 'failed' || task.status === 'canceled')
  );
}

const descendingTaskOrder = (
  left: StandaloneWorkbenchHistoryItem,
  right: StandaloneWorkbenchHistoryItem
): number => {
  if (left.task.submittedAt !== right.task.submittedAt) {
    return left.task.submittedAt > right.task.submittedAt ? -1 : 1;
  }
  if (left.task.taskId === right.task.taskId) return 0;
  return left.task.taskId > right.task.taskId ? -1 : 1;
};

/** Merge one exact owner scope without mutating either durable or live input. */
export function mergeStandaloneWorkbenchHistory(
  input: MergeStandaloneWorkbenchHistoryInput
): StandaloneWorkbenchHistoryItem[] {
  assertScope(input.scope);
  const merged = new Map<string, StandaloneWorkbenchHistoryItem>();

  for (const [index, task] of input.durableTasks.entries()) {
    assertTaskScope(task, input.scope, `durableTasks[${index}].owner`);
    const existing = merged.get(task.taskId);
    if (existing) {
      if (!structurallyEqual(existing.task, task)) {
        invalidResponse(
          `Durable history repeats ${task.taskId} with drifted data`,
          `durableTasks[${index}]`
        );
      }
      continue;
    }
    merged.set(task.taskId, {
      task,
      source: 'durable',
      runtimeEntry: null,
      canRetry: canRetryStandaloneWorkbenchHistoryTask(task),
    });
  }

  const liveIds = new Set<string>();
  for (const [index, entry] of input.runtime.entries.entries()) {
    assertRuntimeEntry(entry, input.scope, index);
    const taskId = entry.task.taskId;
    if (liveIds.has(taskId)) {
      invalidResponse(
        `Runtime history repeats live task ${taskId}`,
        `runtime.entries[${index}]`
      );
    }
    liveIds.add(taskId);
    const durable = merged.get(taskId)?.task;
    if (durable) {
      assertLiveIdentity(durable, entry.task);
      if (!liveMayReplaceDurable(durable, entry.task)) continue;
    }
    merged.set(taskId, {
      task: entry.task,
      source: 'live',
      runtimeEntry: entry,
      canRetry: canRetryStandaloneWorkbenchHistoryTask(entry.task),
    });
  }

  return [...merged.values()].sort(descendingTaskOrder);
}
