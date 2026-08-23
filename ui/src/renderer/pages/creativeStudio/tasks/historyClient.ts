/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CANONICAL_UUID_V7 } from '@/common/types/ids';

import { HttpCreationTaskApi, mapCreationTaskWire } from './client';
import {
  CreativeTaskContractError,
  isStandaloneWorkbenchTaskOwner,
  type CreativeStandaloneTaskHistoryPage,
  type CreativeStandaloneTaskHistoryQuery,
  type CreativeStandaloneTaskRetireInput,
  type CreativeStandaloneTaskRetireResult,
  type CreativeStandaloneWorkbenchKind,
  type CreativeTask,
} from './types';

export interface CreationTaskHistoryWireApi {
  listStandalone(query: string, signal?: AbortSignal): Promise<unknown>;
  retireStandalone?(body: unknown, signal?: AbortSignal): Promise<unknown>;
}

const WORKBENCH_KINDS = new Set<CreativeStandaloneWorkbenchKind>([
  'image',
  'video',
  'audio',
]);
const UUID_V7_SOURCE = CANONICAL_UUID_V7.source.replace(/^\^/, '').replace(/\$$/, '');
const CURSOR = new RegExp(`^(0|[1-9][0-9]*):(${UUID_V7_SOURCE})$`);

const record = (value: unknown, field: string): Record<string, unknown> => {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Invalid standalone task history ${field}`,
      field
    );
  }
  return value as Record<string, unknown>;
};

const exactKeys = (
  value: Record<string, unknown>,
  keys: readonly string[],
  field: string
): void => {
  const expected = new Set(keys);
  const unknown = Object.keys(value).find((key) => !expected.has(key));
  const missing = keys.find((key) => !(key in value));
  if (unknown || missing) {
    throw new CreativeTaskContractError(
      'invalid_response',
      `Invalid standalone task history ${field} fields`,
      unknown ? `${field}.${unknown}` : `${field}.${missing}`
    );
  }
};

interface StandaloneTaskHistoryCursor {
  raw: string;
  submittedAt: number;
  taskId: string;
}

const normalizedCursor = (
  value: unknown,
  field: string,
  code: 'invalid_request' | 'invalid_response'
): StandaloneTaskHistoryCursor => {
  if (typeof value !== 'string') {
    throw new CreativeTaskContractError(code, `Invalid task history ${field}`, field);
  }
  const match = CURSOR.exec(value);
  const submittedAt = match ? Number(match[1]) : Number.NaN;
  if (!match || !Number.isSafeInteger(submittedAt) || submittedAt < 0) {
    throw new CreativeTaskContractError(code, `Invalid task history ${field}`, field);
  }
  return {
    raw: value,
    submittedAt,
    taskId: match[2],
  };
};

const normalizeQuery = (
  input: CreativeStandaloneTaskHistoryQuery
): Required<Omit<CreativeStandaloneTaskHistoryQuery, 'cursor'>> & {
  cursor: StandaloneTaskHistoryCursor | null;
} => {
  if (!WORKBENCH_KINDS.has(input.workbenchKind)) {
    throw new CreativeTaskContractError(
      'invalid_request',
      'Invalid standalone task history workbenchKind',
      'workbenchKind'
    );
  }
  const limit = input.limit ?? 30;
  if (!Number.isInteger(limit) || limit < 1 || limit > 100) {
    throw new CreativeTaskContractError(
      'invalid_request',
      'Standalone task history limit must be an integer from 1 to 100',
      'limit'
    );
  }
  const activeOnly = input.activeOnly ?? false;
  if (typeof activeOnly !== 'boolean') {
    throw new CreativeTaskContractError(
      'invalid_request',
      'Standalone task history activeOnly must be boolean',
      'activeOnly'
    );
  }
  return {
    workbenchKind: input.workbenchKind,
    limit,
    activeOnly,
    cursor:
      input.cursor == null
        ? null
        : normalizedCursor(input.cursor, 'cursor', 'invalid_request'),
  };
};

const assertOwned = (
  task: CreativeTask,
  query: ReturnType<typeof normalizeQuery>
): void => {
  if (
    !isStandaloneWorkbenchTaskOwner(task.owner) ||
    task.owner.workbenchKind !== query.workbenchKind ||
    task.deletedAt !== null
  ) {
    throw new CreativeTaskContractError(
      'ownership_mismatch',
      `Task ${task.taskId} escaped its standalone history owner`,
      'items.owner'
    );
  }
};

const assertStableOrder = (items: readonly CreativeTask[]): void => {
  for (let index = 1; index < items.length; index += 1) {
    const previous = items[index - 1];
    const current = items[index];
    if (
      !previous ||
      !current ||
      previous.submittedAt < current.submittedAt ||
      (previous.submittedAt === current.submittedAt &&
        previous.taskId < current.taskId)
    ) {
      throw new CreativeTaskContractError(
        'invalid_response',
        'Standalone task history is not in stable descending order',
        `items[${index}]`
      );
    }
  }
};

export class CreativeTaskHistoryClient {
  constructor(
    private readonly api: CreationTaskHistoryWireApi = new HttpCreationTaskApi()
  ) {}

  async listStandalone(
    input: CreativeStandaloneTaskHistoryQuery,
    signal?: AbortSignal
  ): Promise<CreativeStandaloneTaskHistoryPage> {
    const query = normalizeQuery(input);
    const params = new URLSearchParams({
      workbench_kind: query.workbenchKind,
      limit: String(query.limit),
    });
    if (query.activeOnly) params.set('active_only', 'true');
    if (query.cursor) params.set('cursor', query.cursor.raw);
    const wire = record(
      await this.api.listStandalone(params.toString(), signal),
      'response'
    );
    exactKeys(wire, ['items', 'next_cursor'], 'response');
    if (!Array.isArray(wire.items)) {
      throw new CreativeTaskContractError(
        'invalid_response',
        'Standalone task history items must be an array',
        'response.items'
      );
    }
    const items = wire.items.map((item) => mapCreationTaskWire(item));
    if (items.length > query.limit) {
      throw new CreativeTaskContractError(
        'invalid_response',
        'Standalone task history exceeds the requested page limit',
        'response.items'
      );
    }
    const seen = new Set<string>();
    for (const task of items) {
      assertOwned(task, query);
      if (
        query.activeOnly &&
        task.status !== 'queued' &&
        task.status !== 'running'
      ) {
        throw new CreativeTaskContractError(
          'invalid_response',
          `Active standalone history returned terminal task ${task.taskId}`,
          'response.items'
        );
      }
      if (seen.has(task.taskId)) {
        throw new CreativeTaskContractError(
          'invalid_response',
          `Standalone task history repeats ${task.taskId}`,
          'response.items'
        );
      }
      seen.add(task.taskId);
    }
    assertStableOrder(items);
    if (
      query.cursor &&
      items.some(
        (task) =>
          task.submittedAt > query.cursor!.submittedAt ||
          (task.submittedAt === query.cursor!.submittedAt &&
            task.taskId >= query.cursor!.taskId)
      )
    ) {
      throw new CreativeTaskContractError(
        'invalid_response',
        'Standalone task history page crosses its requested cursor boundary',
        'response.items'
      );
    }
    const nextCursor =
      wire.next_cursor === null
        ? null
        : normalizedCursor(
            wire.next_cursor,
            'response.next_cursor',
            'invalid_response'
          );
    if (items.length === 0 && nextCursor !== null) {
      throw new CreativeTaskContractError(
        'invalid_response',
        'Empty standalone history cannot advertise another page',
        'response.next_cursor'
      );
    }
    if (nextCursor) {
      const last = items.at(-1)!;
      if (
        items.length !== query.limit ||
        nextCursor.submittedAt !== last.submittedAt ||
        nextCursor.taskId !== last.taskId
      ) {
        throw new CreativeTaskContractError(
          'invalid_response',
          'Standalone task history next cursor must match the last visible task',
          'response.next_cursor'
        );
      }
    }
    return { items, nextCursor: nextCursor?.raw ?? null };
  }

  async retireStandalone(
    input: CreativeStandaloneTaskRetireInput,
    signal?: AbortSignal
  ): Promise<CreativeStandaloneTaskRetireResult> {
    const query = normalizeQuery({
      workbenchKind: input.workbenchKind,
      limit: 1,
    });
    if (
      !Array.isArray(input.taskIds) ||
      input.taskIds.length < 1 ||
      input.taskIds.length > 100
    ) {
      throw new CreativeTaskContractError(
        'invalid_request',
        'Standalone history retirement requires 1 to 100 task ids',
        'taskIds'
      );
    }
    const taskIds = input.taskIds.map((taskId, index) => {
      if (typeof taskId !== 'string' || !CANONICAL_UUID_V7.test(taskId)) {
        throw new CreativeTaskContractError(
          'invalid_request',
          `Invalid standalone history task id at index ${index}`,
          `taskIds[${index}]`
        );
      }
      return taskId;
    });
    if (new Set(taskIds).size !== taskIds.length) {
      throw new CreativeTaskContractError(
        'invalid_request',
        'Standalone history retirement task ids must be unique',
        'taskIds'
      );
    }
    if (!this.api.retireStandalone) {
      throw new CreativeTaskContractError(
        'invalid_request',
        'Standalone history retirement is unavailable',
        'retireStandalone'
      );
    }
    const wire = record(
      await this.api.retireStandalone(
        {
          workbench_kind: query.workbenchKind,
          task_ids: taskIds,
        },
        signal
      ),
      'retire_response'
    );
    exactKeys(wire, ['retired_task_ids'], 'retire_response');
    if (
      !Array.isArray(wire.retired_task_ids) ||
      wire.retired_task_ids.length !== taskIds.length ||
      wire.retired_task_ids.some((taskId, index) => taskId !== taskIds[index])
    ) {
      throw new CreativeTaskContractError(
        'invalid_response',
        'Standalone history retirement must echo every task id in request order',
        'retire_response.retired_task_ids'
      );
    }
    return { retiredTaskIds: [...taskIds] };
  }
}

export const creativeTaskHistoryClient = new CreativeTaskHistoryClient();
