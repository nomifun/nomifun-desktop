/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CANONICAL_UUID_V7 } from '@/common/types/ids';

import type { CreativeJsonObject } from '../domain/schema';
import type { CreativeTask } from '../tasks';

export const STANDALONE_RETRY_SLOT_PARAMETER = 'nomifunRetrySlot';
export const STANDALONE_RETRY_SLOT_MAX_ATTEMPTS = 100;

export interface StandaloneRetrySlot {
  taskId: string;
  submittedAt: number;
  predecessorTaskIds: readonly string[];
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);

/** Read only metadata minted by this workbench; malformed values never collapse history. */
export function standaloneRetrySlot(
  task: Pick<CreativeTask, 'taskId' | 'submittedAt' | 'parameters'>
): StandaloneRetrySlot | null {
  const value = task.parameters[STANDALONE_RETRY_SLOT_PARAMETER];
  if (
    !isRecord(value) ||
    Object.keys(value).length !== 3 ||
    !CANONICAL_UUID_V7.test(String(value.taskId)) ||
    !Number.isSafeInteger(value.submittedAt) ||
    (value.submittedAt as number) < 0 ||
    (value.submittedAt as number) > task.submittedAt ||
    !Array.isArray(value.predecessorTaskIds) ||
    value.predecessorTaskIds.length < 1 ||
    value.predecessorTaskIds.length >= STANDALONE_RETRY_SLOT_MAX_ATTEMPTS
  ) {
    return null;
  }
  const predecessorTaskIds = value.predecessorTaskIds.map(String);
  if (
    predecessorTaskIds.some((taskId) => !CANONICAL_UUID_V7.test(taskId)) ||
    predecessorTaskIds[0] !== value.taskId ||
    predecessorTaskIds.includes(task.taskId) ||
    new Set(predecessorTaskIds).size !== predecessorTaskIds.length
  ) {
    return null;
  }
  return {
    taskId: String(value.taskId),
    submittedAt: value.submittedAt as number,
    predecessorTaskIds,
  };
}

/** Preserve the first card's slot while appending the task being replaced. */
export function parametersForStandaloneRetry(
  task: Pick<CreativeTask, 'taskId' | 'submittedAt' | 'parameters'>
): CreativeJsonObject {
  const existing = standaloneRetrySlot(task);
  const taskId = existing?.taskId ?? task.taskId;
  const submittedAt = existing?.submittedAt ?? task.submittedAt;
  const predecessorTaskIds = [
    ...(existing?.predecessorTaskIds ?? []),
    task.taskId,
  ];
  if (predecessorTaskIds.length >= STANDALONE_RETRY_SLOT_MAX_ATTEMPTS) {
    throw new Error('Standalone workbench retry history reached its safe attempt limit');
  }
  return {
    ...structuredClone(task.parameters),
    [STANDALONE_RETRY_SLOT_PARAMETER]: {
      taskId,
      submittedAt,
      predecessorTaskIds,
    },
  };
}
