/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';

import type { CreativeTaskPort } from './port';
import {
  CreativeTaskProgressGuard,
  CreativeTaskRequestFence,
  pollCreativeTask,
  projectCreativeTaskOutput,
} from './runtime';
import type { CreativeTaskPollOptions } from './runtime';
import {
  CreativeTaskContractError,
  creativeTaskReference,
  isTerminalCreativeTaskStatus,
} from './types';
import type {
  CreateCreativeTaskInput,
  CreativeTask,
  CreativeTaskIdentity,
  CreativeTaskInput,
  CreativeTaskOutput,
  CreativeTaskReference,
  CreativeTaskStatus,
} from './types';

export type CreativeTaskRuntimeStatus =
  | 'idle'
  | 'submitting'
  | 'recovering'
  | 'request_error'
  | CreativeTaskStatus;

export interface CreativeTaskRuntimeSnapshot {
  status: CreativeTaskRuntimeStatus;
  task: CreativeTask | null;
  output: CreativeTaskOutput | null;
  /** Transport/contract failure only. A provider failure remains `task.error`. */
  requestError: Error | null;
}
export interface UseCreativeTaskRuntimeOptions {
  port: CreativeTaskPort;
  identity: CreativeTaskIdentity;
  pendingTaskId?: string | null;
  autoResume?: boolean;
  poll?: Omit<CreativeTaskPollOptions, 'signal' | 'onTask'>;
  /** Persist this pending reference before the POST can create backend work. */
  onSubmission?: (reference: CreativeTaskReference) => void | Promise<void>;
  onTask?: (task: CreativeTask) => void;
  onOutput?: (output: CreativeTaskOutput) => void;
}

export interface CreativeTaskRunPayload {
  /** Stable for every retry of this one logical submission. */
  idempotencyKey: string;
  parameters: CreateCreativeTaskInput['parameters'];
  inputs?: readonly CreativeTaskInput[];
}

export interface UseCreativeTaskRuntimeResult extends CreativeTaskRuntimeSnapshot {
  run(payload: CreativeTaskRunPayload): Promise<CreativeTask | null>;
  resume(taskId?: string): Promise<CreativeTask | null>;
  cancel(): Promise<CreativeTask | null>;
  reset(): void;
  canCancel: boolean;
}

const INITIAL_SNAPSHOT: CreativeTaskRuntimeSnapshot = {
  status: 'idle',
  task: null,
  output: null,
  requestError: null,
};

function asError(value: unknown): Error {
  return value instanceof Error ? value : new Error(String(value));
}

function isAbortError(value: unknown): boolean {
  return value instanceof Error && value.name === 'AbortError';
}

function isBusy(status: CreativeTaskRuntimeStatus): boolean {
  return (
    status === 'submitting' ||
    status === 'recovering' ||
    status === 'queued' ||
    status === 'running'
  );
}

/**
 * Controlled single-node runtime. All durable writes stay with the caller;
 * callbacks expose authoritative task/output transitions for document updates.
 */
export function useCreativeTaskRuntime(
  options: UseCreativeTaskRuntimeOptions
): UseCreativeTaskRuntimeResult {
  const [snapshot, setSnapshotState] = useState<CreativeTaskRuntimeSnapshot>(INITIAL_SNAPSHOT);
  const snapshotRef = useRef(snapshot);
  const optionsRef = useRef(options);
  optionsRef.current = options;
  const fenceRef = useRef(new CreativeTaskRequestFence());
  const progressRef = useRef(new CreativeTaskProgressGuard());
  const abortRef = useRef<AbortController | null>(null);
  const activeReferenceRef = useRef<CreativeTaskReference | null>(null);
  const activePromiseRef = useRef<Promise<CreativeTask | null> | null>(null);
  const cancelAfterSubmitRef = useRef(false);

  const setSnapshot = useCallback((next: CreativeTaskRuntimeSnapshot): void => {
    snapshotRef.current = next;
    setSnapshotState(next);
  }, []);

  const begin = useCallback((): { revision: number; controller: AbortController } => {
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    return { revision: fenceRef.current.begin(), controller };
  }, []);

  const publishTask = useCallback((revision: number, task: CreativeTask): boolean => {
    return fenceRef.current.commit(revision, () => {
      progressRef.current.observe(task);
      const output = projectCreativeTaskOutput(task);
      const next: CreativeTaskRuntimeSnapshot = {
        status: task.status,
        task,
        output,
        requestError: null,
      };
      setSnapshot(next);
      activeReferenceRef.current = creativeTaskReference(task);
      optionsRef.current.onTask?.(task);
      if (output) optionsRef.current.onOutput?.(output);
    });
  }, [setSnapshot]);

  const publishRequestError = useCallback((revision: number, reason: unknown): void => {
    if (isAbortError(reason)) return;
    fenceRef.current.commit(revision, () => {
      setSnapshot({
        status: 'request_error',
        task: snapshotRef.current.task,
        output: null,
        requestError: asError(reason),
      });
    });
  }, [setSnapshot]);

  const pollReference = useCallback(
    async (
      revision: number,
      controller: AbortController,
      reference: CreativeTaskReference
    ): Promise<CreativeTask | null> => {
      const task = await pollCreativeTask(optionsRef.current.port, reference, {
        ...optionsRef.current.poll,
        signal: controller.signal,
        onTask: (update) => publishTask(revision, update),
      });
      return fenceRef.current.isCurrent(revision) && !controller.signal.aborted ? task : null;
    },
    [publishTask]
  );

  const run = useCallback(
    (payload: CreativeTaskRunPayload): Promise<CreativeTask | null> => {
      if (isBusy(snapshotRef.current.status)) {
        return activePromiseRef.current ?? Promise.resolve(null);
      }
      const unresolvedReference = snapshotRef.current.status === 'request_error'
        ? activeReferenceRef.current
        : null;
      if (
        unresolvedReference &&
        payload.idempotencyKey !== unresolvedReference.taskId
      ) {
        return Promise.reject(
          new CreativeTaskContractError(
            'invalid_request',
            `Submission outcome for ${unresolvedReference.taskId} is unresolved; retry with its exact Idempotency-Key or reset explicitly`,
            'idempotencyKey'
          )
        );
      }
      const { revision, controller } = begin();
      progressRef.current.reset();
      activeReferenceRef.current = {
        taskId: payload.idempotencyKey,
        ...optionsRef.current.identity,
      };
      cancelAfterSubmitRef.current = false;
      setSnapshot({ status: 'submitting', task: null, output: null, requestError: null });

      const operation = (async (): Promise<CreativeTask | null> => {
        try {
          await optionsRef.current.onSubmission?.(activeReferenceRef.current!);
          if (!fenceRef.current.isCurrent(revision) || controller.signal.aborted) return null;
          const created = await optionsRef.current.port.create(
            {
              ...optionsRef.current.identity,
              idempotencyKey: payload.idempotencyKey,
              parameters: payload.parameters,
              inputs: payload.inputs ?? [],
            },
            controller.signal
          );
          if (!fenceRef.current.isCurrent(revision) || controller.signal.aborted) return null;
          activeReferenceRef.current = creativeTaskReference(created);
          if (cancelAfterSubmitRef.current) {
            cancelAfterSubmitRef.current = false;
            const canceled = await optionsRef.current.port.cancel(
              activeReferenceRef.current,
              controller.signal
            );
            publishTask(revision, canceled);
            return canceled;
          }
          publishTask(revision, created);
          if (isTerminalCreativeTaskStatus(created.status)) return created;
          return await pollReference(revision, controller, activeReferenceRef.current);
        } catch (reason) {
          if (
            cancelAfterSubmitRef.current &&
            activeReferenceRef.current &&
            fenceRef.current.isCurrent(revision) &&
            !controller.signal.aborted
          ) {
            cancelAfterSubmitRef.current = false;
            try {
              const canceled = await optionsRef.current.port.cancel(
                activeReferenceRef.current,
                controller.signal
              );
              publishTask(revision, canceled);
              return canceled;
            } catch (cancelReason) {
              publishRequestError(revision, cancelReason);
              return null;
            }
          }
          publishRequestError(revision, reason);
          return null;
        }
      })();
      activePromiseRef.current = operation;
      void operation.finally(() => {
        if (activePromiseRef.current === operation) activePromiseRef.current = null;
      });
      return operation;
    },
    [begin, pollReference, publishRequestError, publishTask, setSnapshot]
  );

  const resume = useCallback(
    (taskId = optionsRef.current.pendingTaskId ?? ''): Promise<CreativeTask | null> => {
      if (!taskId) return Promise.resolve(null);
      if (isBusy(snapshotRef.current.status)) {
        return activePromiseRef.current ?? Promise.resolve(null);
      }
      const { revision, controller } = begin();
      progressRef.current.reset();
      const reference: CreativeTaskReference = {
        taskId,
        ...optionsRef.current.identity,
      };
      activeReferenceRef.current = reference;
      setSnapshot({ status: 'recovering', task: null, output: null, requestError: null });

      const operation = (async (): Promise<CreativeTask | null> => {
        try {
          return await pollReference(revision, controller, reference);
        } catch (reason) {
          publishRequestError(revision, reason);
          return null;
        }
      })();
      activePromiseRef.current = operation;
      void operation.finally(() => {
        if (activePromiseRef.current === operation) activePromiseRef.current = null;
      });
      return operation;
    },
    [begin, pollReference, publishRequestError, setSnapshot]
  );

  const cancel = useCallback(async (): Promise<CreativeTask | null> => {
    if (snapshotRef.current.status === 'submitting') {
      cancelAfterSubmitRef.current = true;
      return activePromiseRef.current ?? null;
    }
    const reference = activeReferenceRef.current;
    if (!reference) return null;
    const { revision, controller } = begin();
    try {
      const task = await optionsRef.current.port.cancel(reference, controller.signal);
      publishTask(revision, task);
      return task;
    } catch (reason) {
      publishRequestError(revision, reason);
      return null;
    }
  }, [begin, publishRequestError, publishTask]);

  const reset = useCallback((): void => {
    abortRef.current?.abort();
    abortRef.current = null;
    fenceRef.current.invalidate();
    progressRef.current.reset();
    activeReferenceRef.current = null;
    activePromiseRef.current = null;
    cancelAfterSubmitRef.current = false;
    setSnapshot(INITIAL_SNAPSHOT);
  }, [setSnapshot]);

  const identityKey = [
    JSON.stringify(options.identity.owner),
    options.identity.providerId,
    options.identity.model,
    options.identity.task,
    options.identity.capability,
  ].join('\u0000');

  useEffect(() => {
    progressRef.current.reset();
    if (options.autoResume !== false && options.pendingTaskId) {
      void resume(options.pendingTaskId);
    }
    return () => {
      abortRef.current?.abort();
      abortRef.current = null;
      fenceRef.current.invalidate();
      activeReferenceRef.current = null;
    };
  }, [identityKey, options.autoResume, options.pendingTaskId, resume]);

  return {
    ...snapshot,
    run,
    resume,
    cancel,
    reset,
    canCancel:
      snapshot.status === 'submitting' ||
      snapshot.status === 'recovering' ||
      snapshot.status === 'queued' ||
      snapshot.status === 'running' ||
      (snapshot.status === 'request_error' && activeReferenceRef.current !== null),
  };
}
