/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { BackendHttpError } from '@/common/adapter/httpBridge';

import type { CreativeProjectDocument } from '../domain/schema';
import type { CreativeTaskPort } from './port';
import {
  CreativeTaskContractError,
  assertTaskCapabilityPair,
  isCreativeTaskCapability,
  isTerminalCreativeTaskStatus,
  sameCreativeTaskOwner,
} from './types';
import type {
  CreativeTask,
  CreativeTaskOutput,
  CreativeTaskReference,
} from './types';

export type CreativeTaskPollWait = (delayMs: number, signal?: AbortSignal) => Promise<void>;

export interface CreativeTaskPollOptions {
  signal?: AbortSignal;
  intervalMs?: number;
  /** A local deadline rejects; it never fabricates a backend terminal state. */
  maxWaitMs?: number;
  onTask?: (task: CreativeTask) => void;
  wait?: CreativeTaskPollWait;
  now?: () => number;
}
export class CreativeTaskPollTimeoutError extends Error {
  readonly taskId: string;
  readonly maxWaitMs: number;

  constructor(taskId: string, maxWaitMs: number) {
    super(`Creative task ${taskId} did not reach a terminal state within ${maxWaitMs}ms`);
    this.name = 'CreativeTaskPollTimeoutError';
    this.taskId = taskId;
    this.maxWaitMs = maxWaitMs;
  }
}

/** Enforces the backend state machine at every observed response boundary. */
export class CreativeTaskProgressGuard {
  private previous: CreativeTask | null = null;

  reset(): void {
    this.previous = null;
  }

  observe(task: CreativeTask): void {
    const previous = this.previous;
    if (!previous) {
      this.previous = task;
      return;
    }
    assertCreativeTaskReference(task, previous);
    if (previous.submittedAt !== task.submittedAt) {
      throw new CreativeTaskContractError(
        'invalid_response',
        `Creative task ${task.taskId} changed submittedAt`,
        'submittedAt'
      );
    }
    if (JSON.stringify(previous.parameters) !== JSON.stringify(task.parameters)) {
      throw new CreativeTaskContractError(
        'invalid_response',
        `Creative task ${task.taskId} changed immutable parameters`,
        'parameters'
      );
    }
    if (previous.status === 'running' && task.status === 'queued') {
      throw new CreativeTaskContractError(
        'invalid_response',
        `Creative task ${task.taskId} moved from running back to queued`,
        'status'
      );
    }
    if (isTerminalCreativeTaskStatus(previous.status) && task.status !== previous.status) {
      throw new CreativeTaskContractError(
        'invalid_response',
        `Creative task ${task.taskId} changed terminal status from ${previous.status} to ${task.status}`,
        'status'
      );
    }
    if (task.attempt < previous.attempt) {
      throw new CreativeTaskContractError(
        'invalid_response',
        `Creative task ${task.taskId} attempt moved backwards`,
        'attempt'
      );
    }
    if (previous.startedAt !== null && task.startedAt !== previous.startedAt) {
      throw new CreativeTaskContractError(
        'invalid_response',
        `Creative task ${task.taskId} changed its start authority`,
        'startedAt'
      );
    }
    if (previous.finishedAt !== null && task.finishedAt !== previous.finishedAt) {
      throw new CreativeTaskContractError(
        'invalid_response',
        `Creative task ${task.taskId} changed its terminal timestamp`,
        'finishedAt'
      );
    }
    if (
      isTerminalCreativeTaskStatus(previous.status) &&
      (task.attempt !== previous.attempt ||
        JSON.stringify(task.error) !== JSON.stringify(previous.error) ||
        JSON.stringify(task.resultAssetIds) !== JSON.stringify(previous.resultAssetIds))
    ) {
      throw new CreativeTaskContractError(
        'invalid_response',
        `Creative task ${task.taskId} mutated after reaching ${previous.status}`,
        'status'
      );
    }
    this.previous = task;
  }
}

function abortError(message = 'Creative task operation was aborted'): Error {
  const error = new Error(message);
  error.name = 'AbortError';
  return error;
}

function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted) throw abortError();
}

const defaultWait: CreativeTaskPollWait = (delayMs, signal) =>
  new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(abortError());
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', handleAbort);
      resolve();
    }, delayMs);
    const handleAbort = (): void => {
      clearTimeout(timer);
      reject(abortError());
    };
    signal?.addEventListener('abort', handleAbort, { once: true });
  });

/** Defense in depth for alternate port implementations and tests. */
export function assertCreativeTaskReference(
  task: CreativeTask,
  reference: CreativeTaskReference
): void {
  if (task.taskId !== reference.taskId) {
    throw new CreativeTaskContractError(
      'identity_mismatch',
      `Creative task id mismatch: expected ${reference.taskId}, received ${task.taskId}`,
      'taskId'
    );
  }
  if (!sameCreativeTaskOwner(task.owner, reference.owner)) {
    throw new CreativeTaskContractError(
      'ownership_mismatch',
      `Creative task ${task.taskId} does not belong to the expected owner`,
      'owner'
    );
  }
  for (const field of ['providerId', 'model', 'task', 'capability'] as const) {
    if (task[field] !== reference[field]) {
      throw new CreativeTaskContractError(
        'identity_mismatch',
        `Creative task ${field} mismatch: expected ${reference[field]}, received ${task[field]}`,
        field
      );
    }
  }
}

/** Poll one authoritative task until the backend reports a terminal state. */
export async function pollCreativeTask(
  port: CreativeTaskPort,
  reference: CreativeTaskReference,
  options: CreativeTaskPollOptions = {}
): Promise<CreativeTask> {
  const intervalMs = options.intervalMs ?? 1_500;
  const maxWaitMs = options.maxWaitMs;
  if (!Number.isFinite(intervalMs) || intervalMs < 0) {
    throw new RangeError('Creative task polling intervalMs must be a finite non-negative number');
  }
  if (maxWaitMs !== undefined && (!Number.isFinite(maxWaitMs) || maxWaitMs < 0)) {
    throw new RangeError('Creative task polling maxWaitMs must be a finite non-negative number');
  }
  const wait = options.wait ?? defaultWait;
  const now = options.now ?? Date.now;
  const startedAt = now();
  const progress = new CreativeTaskProgressGuard();

  for (;;) {
    throwIfAborted(options.signal);
    const task = await port.get(reference, options.signal);
    throwIfAborted(options.signal);
    assertCreativeTaskReference(task, reference);
    progress.observe(task);
    options.onTask?.(task);
    if (isTerminalCreativeTaskStatus(task.status)) return task;
    if (maxWaitMs !== undefined && now() - startedAt >= maxWaitMs) {
      throw new CreativeTaskPollTimeoutError(reference.taskId, maxWaitMs);
    }
    await wait(intervalMs, options.signal);
  }
}

/** Project committed output ids only. Failed/canceled/running tasks never look successful. */
export function projectCreativeTaskOutput(task: CreativeTask): CreativeTaskOutput | null {
  if (task.status !== 'succeeded') return null;
  if (task.resultAssetIds.length === 0) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Succeeded creative task ${task.taskId} has no result assets`,
      'resultAssetIds'
    );
  }
  return {
    taskId: task.taskId,
    owner: { ...task.owner },
    assetIds: [...task.resultAssetIds],
  };
}

/**
 * Resolve durable `pendingTaskIds` through config nodes. A pending id without
 * one exact project/node/provider/model/task/capability owner fails closed.
 */
export function pendingCreativeTaskReferences(
  document: Pick<CreativeProjectDocument, 'projectId' | 'pendingTaskIds' | 'nodes'>
): CreativeTaskReference[] {
  const uniqueIds = new Set<string>();
  return document.pendingTaskIds.map((taskId) => {
    if (uniqueIds.has(taskId)) {
      throw new CreativeTaskContractError(
        'invalid_request',
        `Duplicate pending creative task id: ${taskId}`,
        'pendingTaskIds'
      );
    }
    uniqueIds.add(taskId);
    const owners = document.nodes.filter(
      (node) => node.type === 'config' && node.data.taskId === taskId
    );
    if (owners.length !== 1) {
      throw new CreativeTaskContractError(
        'ownership_mismatch',
        `Pending creative task ${taskId} must have exactly one config-node owner`,
        'pendingTaskIds'
      );
    }
    const owner = owners[0];
    if (!owner || owner.type !== 'config') {
      throw new CreativeTaskContractError(
        'ownership_mismatch',
        `Pending creative task ${taskId} has no config-node owner`,
        'pendingTaskIds'
      );
    }
    const { data } = owner;
    if (!data.providerId || !data.model || !isCreativeTaskCapability(data.capability)) {
      throw new CreativeTaskContractError(
        'invalid_request',
        `Pending creative task ${taskId} has incomplete or unsupported model identity`,
        'nodes[].data'
      );
    }
    assertTaskCapabilityPair(data.task, data.capability);
    return {
      taskId,
      owner: {
        kind: 'canvas_node',
        projectId: document.projectId,
        nodeId: owner.id,
      },
      providerId: data.providerId,
      model: data.model,
      task: data.task,
      capability: data.capability,
    };
  });
}

export interface CreativeTaskRecovery {
  tasks: CreativeTask[];
  outputs: CreativeTaskOutput[];
  issues: CreativeTaskRecoveryIssue[];
}

export interface CreativeTaskRecoveryIssue {
  reference: CreativeTaskReference;
  kind: 'orphaned' | 'contract' | 'request';
  error: Error;
}

/** Recover all persisted pending references; caller ownership is checked on every response. */
export async function recoverPendingCreativeTasks(
  port: CreativeTaskPort,
  references: readonly CreativeTaskReference[],
  options: CreativeTaskPollOptions = {}
): Promise<CreativeTaskRecovery> {
  const taskIds = references.map((reference) => reference.taskId);
  if (new Set(taskIds).size !== taskIds.length) {
    throw new CreativeTaskContractError(
      'invalid_request',
      'Pending creative task references contain duplicate task ids',
      'references'
    );
  }

  throwIfAborted(options.signal);
  const settled = await Promise.all(
    references.map(async (reference) => {
      try {
        const task = await pollCreativeTask(port, reference, options);
        return { reference, task, error: null };
      } catch (reason) {
        return {
          reference,
          task: null,
          error: reason instanceof Error ? reason : new Error(String(reason)),
        };
      }
    })
  );
  throwIfAborted(options.signal);
  const tasks = settled
    .map((result) => result.task)
    .filter((task): task is CreativeTask => task !== null);
  const issues = settled.flatMap((result): CreativeTaskRecoveryIssue[] => {
    if (!result.error) return [];
    const kind = result.error instanceof BackendHttpError && result.error.status === 404
      ? 'orphaned'
      : result.error instanceof CreativeTaskContractError
        ? 'contract'
        : 'request';
    return [{ reference: result.reference, kind, error: result.error }];
  });
  return {
    tasks,
    outputs: tasks
      .map(projectCreativeTaskOutput)
      .filter((output): output is CreativeTaskOutput => output !== null),
    issues,
  };
}

/** Monotonic response fence used by hooks to ignore late responses even when a port ignores abort. */
export class CreativeTaskRequestFence {
  private revision = 0;

  begin(): number {
    this.revision += 1;
    return this.revision;
  }

  invalidate(): void {
    this.revision += 1;
  }

  isCurrent(revision: number): boolean {
    return revision === this.revision;
  }

  commit(revision: number, effect: () => void): boolean {
    if (!this.isCurrent(revision)) return false;
    effect();
    return true;
  }
}
