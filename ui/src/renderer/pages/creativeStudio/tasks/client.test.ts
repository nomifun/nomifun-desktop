/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreationTaskWireApi } from './client';
import {
  CreativeTaskClient,
  HttpCreationTaskApi,
  mapCreationTaskWire,
} from './client';
import { CreativeTaskContractError } from './types';
import type {
  CreateCreativeTaskInput,
  CreativeTaskIdentity,
  CreativeTaskStatus,
} from './types';

const PROJECT_ID = '0190f5fe-7c00-7a00-8000-000000000001';
const NODE_ID = '0190f5fe-7c00-7a00-8000-000000000002';
const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000003';
const TASK_ID = '0190f5fe-7c00-7a00-8000-000000000004';
const ASSET_ID = '0190f5fe-7c00-7a00-8000-000000000005';

const identity: CreativeTaskIdentity = {
  projectId: PROJECT_ID,
  nodeId: NODE_ID,
  providerId: PROVIDER_ID,
  model: 'image-model-v1',
  task: 'image_generation',
  capability: 't2i',
};

function wireTask(
  status: CreativeTaskStatus = 'queued',
  overrides: Record<string, unknown> = {}
): Record<string, unknown> {
  const terminal = status === 'succeeded' || status === 'failed' || status === 'canceled';
  return {
    creation_task_id: TASK_ID,
    canvas_id: PROJECT_ID,
    node_id: NODE_ID,
    provider_id: PROVIDER_ID,
    model: 'image-model-v1',
    capability: 't2i',
    params: { prompt: 'Aurora', count: 1 },
    status,
    error: status === 'failed' ? { kind: 'provider_error', message: 'upstream failed' } : null,
    result_asset_ids: status === 'succeeded' ? [ASSET_ID] : [],
    attempt: status === 'queued' ? 0 : 1,
    submitted_at: 100,
    started_at: status === 'queued' ? null : 110,
    finished_at: terminal ? 150 : null,
    ...overrides,
  };
}

function createInput(overrides: Partial<CreateCreativeTaskInput> = {}): CreateCreativeTaskInput {
  return {
    ...identity,
    parameters: { prompt: 'Aurora', count: 1 },
    inputs: [{ assetId: ASSET_ID, role: 'reference' }],
    ...overrides,
  };
}

async function caught(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
    return null;
  } catch (error) {
    return error;
  }
}

describe('CreativeTaskClient', () => {
  test('sends the exact creation wire body and maps the response to camelCase', async () => {
    const calls: Array<{ body: unknown; signal?: AbortSignal }> = [];
    const api: CreationTaskWireApi = {
      create: async (body, signal) => {
        calls.push({ body, signal });
        return wireTask();
      },
      get: async () => wireTask(),
      cancel: async () => wireTask('canceled'),
    };
    const signal = new AbortController().signal;
    const task = await new CreativeTaskClient(api).create(createInput(), signal);

    expect(calls).toEqual([
      {
        signal,
        body: {
          canvas_id: PROJECT_ID,
          node_id: NODE_ID,
          provider_id: PROVIDER_ID,
          model: 'image-model-v1',
          capability: 't2i',
          params: { prompt: 'Aurora', count: 1 },
          inputs: [{ asset_id: ASSET_ID, role: 'reference' }],
        },
      },
    ]);
    expect(task).toEqual({
      taskId: TASK_ID,
      ...identity,
      parameters: { prompt: 'Aurora', count: 1 },
      status: 'queued',
      error: null,
      resultAssetIds: [],
      attempt: 0,
      submittedAt: 100,
      startedAt: null,
      finishedAt: null,
    });
  });

  test('rejects a mismatched ModelTask/capability pair before calling the backend', async () => {
    let calls = 0;
    const api: CreationTaskWireApi = {
      create: async () => {
        calls += 1;
        return wireTask();
      },
      get: async () => wireTask(),
      cancel: async () => wireTask(),
    };
    const error = await caught(
      new CreativeTaskClient(api).create(
        createInput({ task: 'video_generation', capability: 't2i' })
      )
    );

    expect(calls).toBe(0);
    expect(error instanceof CreativeTaskContractError).toBe(true);
    expect((error as CreativeTaskContractError).code).toBe('task_capability_mismatch');
  });

  test('rejects project/node ownership and provider/model identity mismatches', async () => {
    const client = new CreativeTaskClient({
      create: async () => wireTask(),
      get: async () => wireTask('running', { canvas_id: '0190f5fe-7c00-7a00-8000-000000000099' }),
      cancel: async () => wireTask('canceled', { model: 'other-model' }),
    });
    const reference = { taskId: TASK_ID, ...identity };

    const ownershipError = await caught(client.get(reference));
    const identityError = await caught(client.cancel(reference));
    expect((ownershipError as CreativeTaskContractError).code).toBe('ownership_mismatch');
    expect((identityError as CreativeTaskContractError).code).toBe('identity_mismatch');
  });

  test('fails closed on unknown status and impossible result/error states', () => {
    const cases = [
      wireTask('queued', { status: 'done' }),
      wireTask('succeeded', { result_asset_ids: [] }),
      wireTask('running', { result_asset_ids: [ASSET_ID] }),
      wireTask('failed', { error: null }),
      wireTask('queued', { finished_at: 200 }),
    ];
    const codes = cases.map((value) => {
      try {
        mapCreationTaskWire(value, identity);
        return 'accepted';
      } catch (error) {
        return error instanceof CreativeTaskContractError ? error.code : 'unexpected';
      }
    });
    expect(codes).toEqual([
      'invalid_response',
      'invalid_response',
      'invalid_response',
      'invalid_response',
      'invalid_response',
    ]);
  });
});
describe('HttpCreationTaskApi', () => {
  test('uses the audited create/get/cancel routes and forwards AbortSignal', async () => {
    const calls: Array<{ url: string; init?: RequestInit }> = [];
    const fetchStub = (async (url: string | URL | Request, init?: RequestInit) => {
      calls.push({ url: String(url), init });
      return new Response(JSON.stringify({ success: true, data: wireTask() }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;
    const api = new HttpCreationTaskApi({
      fetch: fetchStub,
      baseUrl: () => 'http://backend.test',
      authHeaders: () => ({ 'x-test-auth': 'present' }),
    });
    const signal = new AbortController().signal;

    await api.create({ provider_id: PROVIDER_ID }, signal);
    await api.get(TASK_ID, signal);
    await api.cancel(TASK_ID, signal);

    expect(calls.map((call) => call.url)).toEqual([
      'http://backend.test/api/creation/tasks',
      `http://backend.test/api/creation/tasks/${TASK_ID}`,
      `http://backend.test/api/creation/tasks/${TASK_ID}/cancel`,
    ]);
    expect(calls.map((call) => call.init?.method)).toEqual(['POST', 'GET', 'POST']);
    expect(calls.every((call) => call.init?.signal === signal)).toBe(true);
    expect((calls[0]?.init?.headers as Record<string, string>)['x-test-auth']).toBe('present');
  });

  test('aborts before transport and never returns a fabricated task', async () => {
    let calls = 0;
    const api = new HttpCreationTaskApi({
      fetch: (async () => {
        calls += 1;
        return new Response();
      }) as typeof fetch,
    });
    const controller = new AbortController();
    controller.abort();
    const error = await caught(api.get(TASK_ID, controller.signal));

    expect(calls).toBe(0);
    expect(error instanceof Error ? error.name : '').toBe('AbortError');
  });
});
