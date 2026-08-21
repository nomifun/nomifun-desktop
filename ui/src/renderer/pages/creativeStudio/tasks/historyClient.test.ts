/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  CreativeTaskHistoryClient,
  type CreationTaskHistoryWireApi,
} from './historyClient';
import { HttpCreationTaskApi } from './client';
import { CreativeTaskContractError } from './types';

const PROJECT_ID = '0190f5fe-7c00-7a00-8000-000000000031';
const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000032';
const TASK_A = '0190f5fe-7c00-7a00-8000-000000000033';
const TASK_B = '0190f5fe-7c00-7a00-8000-000000000034';

const task = (taskId: string, submittedAt: number) => ({
  creation_task_id: taskId,
  owner: {
    kind: 'standalone_workbench',
    project_id: PROJECT_ID,
    workbench_kind: 'image',
  },
  provider_id: PROVIDER_ID,
  model: 'image-v1',
  capability: 't2i',
  params: { prompt: 'Aurora' },
  inputs: [],
  status: 'failed',
  error: { kind: 'provider_error', message: 'failed' },
  result_asset_ids: [],
  attempt: 1,
  submitted_at: submittedAt,
  started_at: submittedAt + 1,
  finished_at: submittedAt + 2,
});

const activeTask = (taskId: string, submittedAt: number) => ({
  ...task(taskId, submittedAt),
  status: 'queued',
  error: null,
  started_at: null,
  finished_at: null,
});

describe('CreativeTaskHistoryClient', () => {
  test('maps an exact owner-scoped page and preserves the opaque cursor', async () => {
    let requested = '';
    const api: CreationTaskHistoryWireApi = {
      listStandalone: async (query) => {
        requested = query;
        return {
          items: [task(TASK_B, 20), task(TASK_A, 10)],
          next_cursor: `10:${TASK_A}`,
        };
      },
    };
    const page = await new CreativeTaskHistoryClient(api).listStandalone({
      projectId: PROJECT_ID,
      workbenchKind: 'image',
      limit: 2,
    });
    expect(requested).toBe(
      `project_id=${PROJECT_ID}&workbench_kind=image&limit=2`
    );
    expect(page.items.map((item) => item.taskId)).toEqual([TASK_B, TASK_A]);
    expect(page.nextCursor).toBe(`10:${TASK_A}`);
  });

  test('fails closed on cross-owner, duplicate, unstable, and malformed pages', async () => {
    const cases: unknown[] = [
      {
        items: [
          {
            ...task(TASK_A, 10),
            owner: {
              kind: 'standalone_workbench',
              project_id: PROJECT_ID,
              workbench_kind: 'video',
            },
          },
        ],
        next_cursor: null,
      },
      { items: [task(TASK_A, 10), task(TASK_A, 10)], next_cursor: null },
      { items: [task(TASK_A, 10), task(TASK_B, 20)], next_cursor: null },
      { items: [], next_cursor: `10:${TASK_A}` },
    ];
    for (const value of cases) {
      let error: unknown = null;
      try {
        await new CreativeTaskHistoryClient({
          listStandalone: async () => value,
        }).listStandalone({ projectId: PROJECT_ID, workbenchKind: 'image' });
      } catch (reason) {
        error = reason;
      }
      expect(error instanceof CreativeTaskContractError).toBe(true);
    }
  });

  test('binds every page and continuation cursor to the requested keyset window', async () => {
    const cases: Array<{
      value: unknown;
      input: Parameters<CreativeTaskHistoryClient['listStandalone']>[0];
    }> = [
      {
        value: {
          items: [task(TASK_B, 20), task(TASK_A, 10)],
          next_cursor: null,
        },
        input: { projectId: PROJECT_ID, workbenchKind: 'image', limit: 1 },
      },
      {
        value: { items: [task(TASK_A, 10)], next_cursor: `9:${TASK_A}` },
        input: { projectId: PROJECT_ID, workbenchKind: 'image', limit: 1 },
      },
      {
        value: { items: [task(TASK_B, 10)], next_cursor: null },
        input: {
          projectId: PROJECT_ID,
          workbenchKind: 'image',
          cursor: `10:${TASK_B}`,
        },
      },
    ];
    for (const { value, input } of cases) {
      let error: unknown = null;
      try {
        await new CreativeTaskHistoryClient({
          listStandalone: async () => value,
        }).listStandalone(input);
      } catch (reason) {
        error = reason;
      }
      expect(error instanceof CreativeTaskContractError).toBe(true);
    }

    const page = await new CreativeTaskHistoryClient({
      listStandalone: async () => ({
        items: [task(TASK_A, 10)],
        next_cursor: null,
      }),
    }).listStandalone({
      projectId: PROJECT_ID,
      workbenchKind: 'image',
      cursor: `20:${TASK_B}`,
    });
    expect(page.items.map((item) => item.taskId)).toEqual([TASK_A]);
  });

  test('uses the canonical GET route and forwards AbortSignal', async () => {
    const calls: Array<{ url: string; signal?: AbortSignal }> = [];
    const signal = new AbortController().signal;
    const http = new HttpCreationTaskApi({
      baseUrl: () => 'http://127.0.0.1:8788',
      authHeaders: () => ({}),
      fetch: (async (url: string | URL | Request, init?: RequestInit) => {
        calls.push({ url: String(url), signal: init?.signal ?? undefined });
        return new Response(
          JSON.stringify({ data: { items: [], next_cursor: null } }),
          { status: 200, headers: { 'Content-Type': 'application/json' } }
        );
      }) as typeof fetch,
    });
    const page = await new CreativeTaskHistoryClient(http).listStandalone(
      { projectId: PROJECT_ID, workbenchKind: 'image' },
      signal
    );
    expect(page.items).toEqual([]);
    expect(calls).toEqual([
      {
        url: `http://127.0.0.1:8788/api/creative-studio/tasks?project_id=${PROJECT_ID}&workbench_kind=image&limit=30`,
        signal,
      },
    ]);
  });

  test('requests and enforces an active-only recovery inventory', async () => {
    let requested = '';
    const client = new CreativeTaskHistoryClient({
      listStandalone: async (query) => {
        requested = query;
        return { items: [activeTask(TASK_A, 10)], next_cursor: null };
      },
    });
    const page = await client.listStandalone({
      projectId: PROJECT_ID,
      workbenchKind: 'image',
      limit: 100,
      activeOnly: true,
    });
    expect(requested).toBe(
      `project_id=${PROJECT_ID}&workbench_kind=image&limit=100&active_only=true`
    );
    expect(page.items[0]?.status).toBe('queued');

    let error: unknown = null;
    try {
      await new CreativeTaskHistoryClient({
        listStandalone: async () => ({ items: [task(TASK_A, 10)], next_cursor: null }),
      }).listStandalone({
        projectId: PROJECT_ID,
        workbenchKind: 'image',
        activeOnly: true,
      });
    } catch (reason) {
      error = reason;
    }
    expect(error instanceof CreativeTaskContractError).toBe(true);
  });
});
