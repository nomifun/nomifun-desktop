/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type {
  CreateCreativeTaskInput,
  CreativeTask,
  CreativeTaskPort,
  CreativeTaskReference,
} from '../../tasks';
import {
  cloneTemplateDefinition,
  cloneTemplateOutput,
  cloneTemplateRunAggregate,
  validateTemplateRunAggregate,
  validateTemplateRunTransition,
  type CreativeTemplateRunAggregateV1,
} from '../domain';
import {
  IDS,
  createExecutableTemplateFixture,
  createTemplateRunFixture,
} from '../domain/testFixtures';
import type {
  CreateCreativeTemplateRunRequest,
  CreativeTemplateRunApi,
  SaveCreativeTemplateRunRequest,
} from '../services';
import { createCreativeTemplateRunController } from './controller';
import type { CreativeTemplateTextAssetReader } from './types';

function requestedFrom(input: CreateCreativeTemplateRunRequest): CreativeTemplateRunAggregateV1 {
  const series = input.templateId === IDS.template && input.templateRevision === 2;
  const template = createExecutableTemplateFixture(series);
  template.revision = input.templateRevision;
  const run = createTemplateRunFixture(series);
  run.templateSnapshot = cloneTemplateDefinition(template);
  run.request = {
    id: input.templateRunId,
    idempotencyKey: input.templateRunId,
    templateId: input.templateId,
    templateRevision: input.templateRevision,
    requestedAt: 2_000,
    output: cloneTemplateOutput(template.output),
    inputs: input.inputs.map((value) =>
      value.type === 'image-series' ? { ...value, assetIds: [...value.assetIds] } : { ...value }
    ),
    referenceAssetIds: [...input.referenceAssetIds],
  };
  run.record.requestId = input.templateRunId;
  run.record.templateId = input.templateId;
  expect(validateTemplateRunAggregate(run).ok).toBe(true);
  return run;
}

class MemoryRunApi implements CreativeTemplateRunApi {
  readonly store = new Map<string, CreativeTemplateRunAggregateV1>();
  readonly created: CreateCreativeTemplateRunRequest[] = [];
  readonly savedStatuses: string[] = [];
  listCalls = 0;

  async listRuns(): Promise<CreativeTemplateRunAggregateV1[]> {
    this.listCalls += 1;
    return [...this.store.values()].map(cloneTemplateRunAggregate);
  }

  async createRun(input: CreateCreativeTemplateRunRequest): Promise<CreativeTemplateRunAggregateV1> {
    this.created.push(input);
    const existing = this.store.get(input.templateRunId);
    if (existing) return cloneTemplateRunAggregate(existing);
    const run = requestedFrom(input);
    this.store.set(input.templateRunId, run);
    return cloneTemplateRunAggregate(run);
  }

  async getRun(templateRunId: string): Promise<CreativeTemplateRunAggregateV1> {
    const run = this.store.get(templateRunId);
    if (!run) throw new Error(`missing run ${templateRunId}`);
    return cloneTemplateRunAggregate(run);
  }

  async saveRun(
    templateRunId: string,
    input: SaveCreativeTemplateRunRequest
  ): Promise<CreativeTemplateRunAggregateV1> {
    const current = this.store.get(templateRunId);
    if (!current) throw new Error(`missing run ${templateRunId}`);
    expect(input.expectedRevision).toBe(String(current.revision));
    expect(validateTemplateRunTransition(current, input.run).ok).toBe(true);
    this.savedStatuses.push(input.run.record.status);
    this.store.set(templateRunId, cloneTemplateRunAggregate(input.run));
    return cloneTemplateRunAggregate(input.run);
  }
}

function taskFrom(
  input: CreateCreativeTaskInput,
  status: CreativeTask['status'],
  resultAssetIds: string[] = []
): CreativeTask {
  return {
    taskId: input.idempotencyKey,
    owner: { ...input.owner },
    providerId: input.providerId,
    model: input.model,
    task: input.task,
    capability: input.capability,
    parameters: { ...input.parameters },
    inputs: input.inputs,
    status,
    error: status === 'failed'
      ? { kind: 'provider_failure', message: 'provider rejected task', httpStatus: 400 }
      : null,
    resultAssetIds,
    attempt: 1,
    submittedAt: 2_100,
    startedAt: status === 'queued' ? null : 2_101,
    finishedAt: status === 'queued' || status === 'running' ? null : 2_102,
    deletedAt: null,
  };
}

class ImmediateTaskPort implements CreativeTaskPort {
  readonly created: CreateCreativeTaskInput[] = [];
  readonly tasks = new Map<string, CreativeTask>();
  transientError: Error | null = null;
  failImage = false;

  async create(input: CreateCreativeTaskInput): Promise<CreativeTask> {
    this.created.push(input);
    if (this.transientError) throw this.transientError;
    const existing = this.tasks.get(input.idempotencyKey);
    if (existing) return existing;
    const resultAssetIds = input.capability === 'text'
      ? [IDS.asset]
      : input.idempotencyKey === IDS.task2
        ? [IDS.history]
        : [IDS.result2];
    const task = taskFrom(
      input,
      this.failImage && input.capability !== 'text' ? 'failed' : 'succeeded',
      this.failImage && input.capability !== 'text' ? [] : resultAssetIds
    );
    this.tasks.set(input.idempotencyKey, task);
    return task;
  }

  async get(reference: CreativeTaskReference): Promise<CreativeTask> {
    const task = this.tasks.get(reference.taskId);
    if (!task) throw new Error(`missing task ${reference.taskId}`);
    return task;
  }

  async cancel(reference: CreativeTaskReference): Promise<CreativeTask> {
    const existing = this.tasks.get(reference.taskId);
    if (existing?.status === 'succeeded' || existing?.status === 'failed') return existing;
    const input: CreateCreativeTaskInput = {
      ...reference,
      idempotencyKey: reference.taskId,
      parameters: {},
      inputs: [],
    };
    const canceled = taskFrom(input, 'canceled');
    this.tasks.set(reference.taskId, canceled);
    return canceled;
  }
}

function idFactory(ids: string[]): () => string {
  return () => {
    const id = ids.shift();
    if (!id) throw new Error('test exhausted UUIDv7 ids');
    return id;
  };
}

const textAssets = (body = '{"prompts":[]}'): CreativeTemplateTextAssetReader => ({
  read: async () => body,
});

function clock(): () => number {
  let now = 2_100;
  return () => {
    now += 10;
    return now;
  };
}

describe('Creative Template run controller', () => {
  test('persists every task id before submitting and completes one image run', async () => {
    const runs = new MemoryRunApi();
    const tasks = new ImmediateTaskPort();
    const controller = createCreativeTemplateRunController({
      runs,
      tasks,
      textAssets: textAssets(),
      createId: idFactory([IDS.request, IDS.task]),
      now: clock(),
      pollIntervalMs: 0,
    });
    const template = createExecutableTemplateFixture();
    const result = await controller.start({
      template,
      inputs: [{ variableId: IDS.variable, type: 'text', value: 'NomiFun' }],
      referenceAssetIds: [IDS.asset],
    });

    expect(result.record.status).toBe('succeeded');
    expect(result.record.taskIds).toEqual([IDS.task]);
    expect(result.record.resultAssetIds).toEqual([IDS.result2]);
    expect(runs.savedStatuses).toEqual(['queued', 'running', 'succeeded']);
    expect(tasks.created).toHaveLength(1);
    expect(tasks.created[0]).toMatchObject({
      idempotencyKey: IDS.task,
      owner: {
        kind: 'template_step',
        templateRunId: IDS.request,
        templateStepId: IDS.generateStep,
      },
      capability: 'i2i',
      inputs: [{ assetId: IDS.asset, kind: 'image', role: 'reference' }],
    });
    expect(controller.getSnapshot().activities[IDS.request]).toBeUndefined();
  });

  test('stops for durable prompt review, then executes approved image tasks', async () => {
    const runs = new MemoryRunApi();
    const tasks = new ImmediateTaskPort();
    const controller = createCreativeTemplateRunController({
      runs,
      tasks,
      textAssets: textAssets(JSON.stringify({
        prompts: [
          { title: 'Front', prompt: 'Front view' },
          { title: 'Detail', prompt: 'Detail view' },
        ],
      })),
      createId: idFactory([
        IDS.request,
        IDS.task,
        IDS.task2,
        IDS.task3,
        IDS.draft1,
        IDS.draft2,
      ]),
      now: clock(),
      pollIntervalMs: 0,
    });
    const template = createExecutableTemplateFixture(true);
    template.revision = 2;
    const review = await controller.start({
      template,
      inputs: [{ variableId: IDS.variable, type: 'text', value: 'NomiFun' }],
      referenceAssetIds: [IDS.idempotency],
    });
    expect(review.record.status).toBe('awaiting-review');
    expect(review.record.taskIds).toEqual([IDS.task, IDS.task2, IDS.task3]);
    expect(tasks.created.map((task) => task.idempotencyKey)).toEqual([IDS.task]);
    expect(controller.getSnapshot().activities[IDS.request]?.state).toBe('awaiting-review');

    const completed = await controller.review(
      IDS.request,
      review.promptDrafts.map((draft) => ({
        id: draft.id,
        title: draft.title,
        prompt: `${draft.prompt} approved`,
        reviewNote: 'Approved by user',
      }))
    );
    expect(completed.record.status).toBe('succeeded');
    expect(completed.record.resultAssetIds).toEqual([IDS.history, IDS.result2]);
    expect(completed.promptDrafts.every((draft) => draft.status === 'approved')).toBe(true);
    expect(completed.promptDrafts[0].prompt).toBe('Front view approved');
    expect(tasks.created.map((task) => task.idempotencyKey)).toEqual([
      IDS.task,
      IDS.task2,
      IDS.task3,
    ]);
    expect(runs.savedStatuses).toEqual([
      'queued',
      'running',
      'awaiting-review',
      'running',
      'succeeded',
    ]);
  });

  test('leaves transient transport failures durable and resumable instead of fabricating failure', async () => {
    const runs = new MemoryRunApi();
    const tasks = new ImmediateTaskPort();
    tasks.transientError = new TypeError('network offline');
    const controller = createCreativeTemplateRunController({
      runs,
      tasks,
      textAssets: textAssets(),
      createId: idFactory([IDS.request, IDS.task]),
      now: clock(),
    });
    const run = await controller.start({
      template: createExecutableTemplateFixture(),
      inputs: [{ variableId: IDS.variable, type: 'text', value: 'NomiFun' }],
      referenceAssetIds: [IDS.asset],
    });
    expect(run.record.status).toBe('running');
    expect(run.record.failure).toBeNull();
    expect(runs.savedStatuses).toEqual(['queued', 'running']);
    expect(controller.getSnapshot().activities[IDS.request]).toMatchObject({
      state: 'paused',
      error: 'network offline',
    });
  });

  test('persists authoritative task failures as terminal template failures', async () => {
    const runs = new MemoryRunApi();
    const tasks = new ImmediateTaskPort();
    tasks.failImage = true;
    const controller = createCreativeTemplateRunController({
      runs,
      tasks,
      textAssets: textAssets(),
      createId: idFactory([IDS.request, IDS.task]),
      now: clock(),
    });
    const run = await controller.start({
      template: createExecutableTemplateFixture(),
      inputs: [{ variableId: IDS.variable, type: 'text', value: 'NomiFun' }],
      referenceAssetIds: [IDS.asset],
    });
    expect(run.record.status).toBe('failed');
    expect(run.record.failure).toMatchObject({
      code: 'task-failed',
      message: 'provider rejected task',
    });
    expect(runs.savedStatuses).toEqual(['queued', 'running', 'failed']);
  });

  test('cancels every persisted task reference before committing the run terminal state', async () => {
    const runs = new MemoryRunApi();
    const tasks = new ImmediateTaskPort();
    const controller = createCreativeTemplateRunController({
      runs,
      tasks,
      textAssets: textAssets(JSON.stringify({
        prompts: [
          { title: 'Front', prompt: 'Front view' },
          { title: 'Detail', prompt: 'Detail view' },
        ],
      })),
      createId: idFactory([
        IDS.request,
        IDS.task,
        IDS.task2,
        IDS.task3,
        IDS.draft1,
        IDS.draft2,
      ]),
      now: clock(),
    });
    const template = createExecutableTemplateFixture(true);
    template.revision = 2;
    const review = await controller.start({
      template,
      inputs: [{ variableId: IDS.variable, type: 'text', value: 'NomiFun' }],
      referenceAssetIds: [IDS.idempotency],
    });
    expect(review.record.status).toBe('awaiting-review');

    const cancelled = await controller.cancel(IDS.request);
    expect(cancelled.record.status).toBe('cancelled');
    expect(cancelled.record.completedAt).not.toBeNull();
    expect(tasks.tasks.get(IDS.task)?.status).toBe('succeeded');
    expect(tasks.tasks.get(IDS.task2)?.status).toBe('canceled');
    expect(tasks.tasks.get(IDS.task3)?.status).toBe('canceled');
    expect(runs.savedStatuses.at(-1)).toBe('cancelled');
    expect(controller.getSnapshot().activities[IDS.request]).toBeUndefined();
  });

  test('singleflights route load and resumes a preallocated queued task id', async () => {
    const runs = new MemoryRunApi();
    const queued = createTemplateRunFixture();
    queued.revision = 2;
    queued.request.referenceAssetIds = [IDS.asset];
    queued.record.status = 'queued';
    queued.record.taskIds = [IDS.task];
    queued.record.queuedAt = 2_100;
    runs.store.set(IDS.request, queued);
    const tasks = new ImmediateTaskPort();
    const controller = createCreativeTemplateRunController({
      runs,
      tasks,
      textAssets: textAssets(),
      createId: () => {
        throw new Error('resume must not allocate replacement ids');
      },
      now: clock(),
      pollIntervalMs: 0,
    });
    await Promise.all([controller.load(), controller.load()]);
    const completed = await controller.resume(IDS.request);
    expect(runs.listCalls).toBe(1);
    expect(completed.record.status).toBe('succeeded');
    expect(tasks.created[0].idempotencyKey).toBe(IDS.task);
  });
});
