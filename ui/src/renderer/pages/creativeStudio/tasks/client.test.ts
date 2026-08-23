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
const IDEMPOTENCY_KEY = '0190f5fe-7c00-7a00-8000-000000000006';
const WORKFLOW_ID = '0190f5fe-7c00-7a00-8000-000000000007';
const WORKFLOW_RUN_ID = '0190f5fe-7c00-7a00-8000-000000000008';
const WORKFLOW_STEP_ID = '0190f5fe-7c00-7a00-8000-000000000009';

const identity: CreativeTaskIdentity = {
  owner: {
    kind: 'canvas_node',
    canvasId: PROJECT_ID,
    nodeId: NODE_ID,
  },
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
    owner: {
      kind: 'canvas_node',
      canvas_id: PROJECT_ID,
      node_id: NODE_ID,
    },
    provider_id: PROVIDER_ID,
    model: 'image-model-v1',
    capability: 't2i',
    params: { prompt: 'Aurora', count: 1 },
    inputs: [{ asset_id: ASSET_ID, kind: 'image', role: 'reference' }],
    status,
    error: status === 'failed' ? { kind: 'provider_error', message: 'upstream failed' } : null,
    result_asset_ids: status === 'succeeded' ? [ASSET_ID] : [],
    attempt: status === 'queued' ? 0 : 1,
    submitted_at: 100,
    started_at: status === 'queued' ? null : 110,
    finished_at: terminal ? 150 : null,
    deleted_at: null,
    ...overrides,
  };
}

function createInput(overrides: Partial<CreateCreativeTaskInput> = {}): CreateCreativeTaskInput {
  return {
    ...identity,
    idempotencyKey: IDEMPOTENCY_KEY,
    parameters: { prompt: 'Aurora', count: 1 },
    inputs: [{ assetId: ASSET_ID, kind: 'image', role: 'reference' }],
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
    const calls: Array<{ body: unknown; idempotencyKey: string; signal?: AbortSignal }> = [];
    const api: CreationTaskWireApi = {
      create: async (body, idempotencyKey, signal) => {
        calls.push({ body, idempotencyKey, signal });
        return wireTask('queued', { creation_task_id: idempotencyKey });
      },
      get: async () => wireTask(),
      cancel: async () => wireTask('canceled'),
    };
    const signal = new AbortController().signal;
    const task = await new CreativeTaskClient(api).create(createInput(), signal);

    expect(calls).toEqual([
      {
        signal,
        idempotencyKey: IDEMPOTENCY_KEY,
        body: {
          owner: {
            kind: 'canvas_node',
            canvas_id: PROJECT_ID,
            node_id: NODE_ID,
          },
          provider_id: PROVIDER_ID,
          model: 'image-model-v1',
          capability: 't2i',
          params: { prompt: 'Aurora', count: 1 },
          inputs: [{ asset_id: ASSET_ID, kind: 'image', role: 'reference' }],
        },
      },
    ]);
    expect(task).toEqual({
      taskId: IDEMPOTENCY_KEY,
      ...identity,
      parameters: { prompt: 'Aurora', count: 1 },
      inputs: [{ assetId: ASSET_ID, kind: 'image', role: 'reference' }],
      status: 'queued',
      error: null,
      resultAssetIds: [],
      attempt: 0,
      submittedAt: 100,
      startedAt: null,
      finishedAt: null,
      deletedAt: null,
    });
  });

  test('round-trips the exact workflow-step owner without canvas aliases', async () => {
    const owner = {
      kind: 'workflow_step' as const,
      workflowId: WORKFLOW_ID,
      workflowRunId: WORKFLOW_RUN_ID,
      workflowStepId: WORKFLOW_STEP_ID,
    };
    let body: unknown;
    const client = new CreativeTaskClient({
      create: async (value, key) => {
        body = value;
        return wireTask('queued', {
          creation_task_id: key,
          owner: {
            kind: 'workflow_step',
            workflow_id: WORKFLOW_ID,
            workflow_run_id: WORKFLOW_RUN_ID,
            workflow_step_id: WORKFLOW_STEP_ID,
          },
        });
      },
      get: async () => wireTask(),
      cancel: async () => wireTask('canceled'),
    });

    const task = await client.create(createInput({ owner }));

    expect(body).toEqual({
      owner: {
        kind: 'workflow_step',
        workflow_id: WORKFLOW_ID,
        workflow_run_id: WORKFLOW_RUN_ID,
        workflow_step_id: WORKFLOW_STEP_ID,
      },
      provider_id: PROVIDER_ID,
      model: 'image-model-v1',
      capability: 't2i',
      params: { prompt: 'Aurora', count: 1 },
      inputs: [{ asset_id: ASSET_ID, kind: 'image', role: 'reference' }],
    });
    expect(task.owner).toEqual(owner);
  });

  test('round-trips the exact standalone workbench owner without a config node', async () => {
    const owner = {
      kind: 'standalone_workbench' as const,
      workbenchKind: 'image' as const,
    };
    let body: unknown;
    const client = new CreativeTaskClient({
      create: async (value, key) => {
        body = value;
        return wireTask('queued', {
          creation_task_id: key,
          owner: {
            kind: 'standalone_workbench',
            workbench_kind: 'image',
          },
        });
      },
      get: async () => wireTask(),
      cancel: async () => wireTask('canceled'),
    });

    const task = await client.create(createInput({ owner }));

    expect(body).toEqual({
      owner: {
        kind: 'standalone_workbench',
        workbench_kind: 'image',
      },
      provider_id: PROVIDER_ID,
      model: 'image-model-v1',
      capability: 't2i',
      params: { prompt: 'Aurora', count: 1 },
      inputs: [{ asset_id: ASSET_ID, kind: 'image', role: 'reference' }],
    });
    expect(task.owner).toEqual(owner);
  });

  test('does not resurrect a retired exact replay through create', async () => {
    const owner = {
      kind: 'standalone_workbench' as const,
      workbenchKind: 'image' as const,
    };
    const client = new CreativeTaskClient({
      create: async (_value, key) =>
        wireTask('failed', {
          creation_task_id: key,
          owner: {
            kind: 'standalone_workbench',
            workbench_kind: 'image',
          },
          deleted_at: 200,
        }),
      get: async () => wireTask(),
      cancel: async () => wireTask('canceled'),
    });
    const error = await caught(client.create(createInput({ owner })));
    expect(error instanceof CreativeTaskContractError).toBe(true);
    expect((error as CreativeTaskContractError).field).toBe('deleted_at');
  });

  test('keeps unprovable legacy inputs nullable and rejects invented kinds', () => {
    expect(mapCreationTaskWire(wireTask('queued', { inputs: null })).inputs).toBeNull();
    let invalid: unknown = null;
    try {
      mapCreationTaskWire(
        wireTask('queued', {
          inputs: [
            { asset_id: ASSET_ID, kind: 'panorama', role: 'reference' },
          ],
        })
      );
    } catch (error) {
      invalid = error;
    }
    expect(invalid instanceof CreativeTaskContractError).toBe(true);
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
      get: async () =>
        wireTask('running', {
          owner: {
            kind: 'canvas_node',
            canvas_id: '0190f5fe-7c00-7a00-8000-000000000099',
            node_id: NODE_ID,
          },
        }),
      cancel: async () => wireTask('canceled', { model: 'other-model' }),
    });
    const reference = { taskId: TASK_ID, ...identity };

    const ownershipError = await caught(client.get(reference));
    const identityError = await caught(client.cancel(reference));
    expect((ownershipError as CreativeTaskContractError).code).toBe('ownership_mismatch');
    expect((identityError as CreativeTaskContractError).code).toBe('identity_mismatch');
  });

  test('rejects the legacy project_id canvas owner wire shape', () => {
    const error = caught(
      Promise.resolve().then(() =>
        mapCreationTaskWire(
          wireTask('running', {
            owner: {
              kind: 'canvas_node',
              project_id: PROJECT_ID,
              node_id: NODE_ID,
            },
          })
        )
      )
    );
    return error.then((value) => {
      expect(value instanceof CreativeTaskContractError).toBe(true);
      expect((value as CreativeTaskContractError).field).toBe('owner.project_id');
    });
  });

  test('rejects a create response that does not echo the submission idempotency key', async () => {
    const client = new CreativeTaskClient({
      create: async () => wireTask(),
      get: async () => wireTask(),
      cancel: async () => wireTask('canceled'),
    });
    const error = await caught(client.create(createInput()));

    expect(error instanceof CreativeTaskContractError).toBe(true);
    expect((error as CreativeTaskContractError).code).toBe('identity_mismatch');
    expect((error as CreativeTaskContractError).field).toBe('taskId');
  });

  test('rejects a create response without the exact ordered input snapshot', async () => {
    const client = new CreativeTaskClient({
      create: async (_body, key) =>
        wireTask('queued', { creation_task_id: key, inputs: null }),
      get: async () => wireTask(),
      cancel: async () => wireTask('canceled'),
    });
    const error = await caught(client.create(createInput()));
    expect(error instanceof CreativeTaskContractError).toBe(true);
    expect((error as CreativeTaskContractError).field).toBe('inputs');
  });

  test('reuses the exact header key for concurrent StrictMode-style duplicate calls', async () => {
    const keys: string[] = [];
    const client = new CreativeTaskClient({
      create: async (_body, key) => {
        keys.push(key);
        return wireTask('queued', { creation_task_id: key });
      },
      get: async () => wireTask(),
      cancel: async () => wireTask('canceled'),
    });
    const input = createInput();
    const [first, second] = await Promise.all([client.create(input), client.create(input)]);

    expect(keys).toEqual([IDEMPOTENCY_KEY, IDEMPOTENCY_KEY]);
    expect(first.taskId).toBe(IDEMPOTENCY_KEY);
    expect(second.taskId).toBe(IDEMPOTENCY_KEY);
  });

  test('requires cancel to return the authoritative terminal race winner', async () => {
    const client = new CreativeTaskClient({
      create: async (_body, key) => wireTask('queued', { creation_task_id: key }),
      get: async () => wireTask('running'),
      cancel: async () => wireTask('running'),
    });
    const error = await caught(client.cancel({ taskId: TASK_ID, ...identity }));

    expect(error instanceof CreativeTaskContractError).toBe(true);
    expect((error as CreativeTaskContractError).code).toBe('invalid_response');
    expect((error as CreativeTaskContractError).field).toBe('status');
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

  test('accepts tombstones only for terminal standalone history tasks', () => {
    const owner = {
      kind: 'standalone_workbench',
      workbench_kind: 'image',
    };
    expect(
      mapCreationTaskWire(
        wireTask('failed', { owner, deleted_at: 200 }),
      ).deletedAt
    ).toBe(200);
    for (const value of [
      wireTask('queued', { owner, deleted_at: 200 }),
      wireTask('succeeded', { deleted_at: 200 }),
      wireTask('failed', { owner, deleted_at: 50 }),
    ]) {
      let error: unknown = null;
      try {
        mapCreationTaskWire(value);
      } catch (reason) {
        error = reason;
      }
      expect(error instanceof CreativeTaskContractError).toBe(true);
    }
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

    await api.create({ provider_id: PROVIDER_ID }, IDEMPOTENCY_KEY, signal);
    await api.get(TASK_ID, signal);
    await api.cancel(TASK_ID, signal);

    expect(calls.map((call) => call.url)).toEqual([
      'http://backend.test/api/creative-studio/tasks',
      `http://backend.test/api/creative-studio/tasks/${TASK_ID}`,
      `http://backend.test/api/creative-studio/tasks/${TASK_ID}/cancel`,
    ]);
    expect(calls.map((call) => call.init?.method)).toEqual(['POST', 'GET', 'POST']);
    expect(calls.every((call) => call.init?.signal === signal)).toBe(true);
    expect((calls[0]?.init?.headers as Record<string, string>)['x-test-auth']).toBe('present');
    expect((calls[0]?.init?.headers as Record<string, string>)['Idempotency-Key']).toBe(
      IDEMPOTENCY_KEY
    );
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
