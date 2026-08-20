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
  cloneWorkflowDefinition,
  cloneWorkflowOutput,
  cloneWorkflowRunAggregate,
  validateWorkflowRunAggregate,
  validateWorkflowRunTransition,
  type WorkflowRunAggregateV1,
} from '../domain';
import {
  IDS,
  createExecutableWorkflowFixture,
  createWorkflowRunFixture,
} from '../domain/testFixtures';
import type {
  CreateCreativeWorkflowRunRequest,
  CreativeWorkflowRunApi,
  SaveCreativeWorkflowRunRequest,
} from '../services';
import { createCreativeWorkflowRunController } from './controller';
import type { WorkflowTextAssetReader } from './types';

function requestedFrom(input: CreateCreativeWorkflowRunRequest): WorkflowRunAggregateV1 {
  const series = input.workflowId === IDS.workflow && input.workflowRevision === 2;
  const workflow = createExecutableWorkflowFixture(series);
  workflow.revision = input.workflowRevision;
  const run = createWorkflowRunFixture(series);
  run.workflowSnapshot = cloneWorkflowDefinition(workflow);
  run.request = {
    id: input.runId,
    idempotencyKey: input.runId,
    workflowId: input.workflowId,
    workflowRevision: input.workflowRevision,
    requestedAt: 2_000,
    output: cloneWorkflowOutput(workflow.output),
    inputs: input.inputs.map((value) =>
      value.type === 'image-series' ? { ...value, assetIds: [...value.assetIds] } : { ...value }
    ),
    referenceAssetIds: [...input.referenceAssetIds],
  };
  run.record.requestId = input.runId;
  run.record.workflowId = input.workflowId;
  expect(validateWorkflowRunAggregate(run).ok).toBe(true);
  return run;
}

class MemoryRunApi implements CreativeWorkflowRunApi {
  readonly store = new Map<string, WorkflowRunAggregateV1>();
  readonly created: CreateCreativeWorkflowRunRequest[] = [];
  readonly savedStatuses: string[] = [];
  listCalls = 0;

  async listRuns(): Promise<WorkflowRunAggregateV1[]> {
    this.listCalls += 1;
    return [...this.store.values()].map(cloneWorkflowRunAggregate);
  }

  async createRun(input: CreateCreativeWorkflowRunRequest): Promise<WorkflowRunAggregateV1> {
    this.created.push(input);
    const existing = this.store.get(input.runId);
    if (existing) return cloneWorkflowRunAggregate(existing);
    const run = requestedFrom(input);
    this.store.set(input.runId, run);
    return cloneWorkflowRunAggregate(run);
  }

  async getRun(runId: string): Promise<WorkflowRunAggregateV1> {
    const run = this.store.get(runId);
    if (!run) throw new Error(`missing run ${runId}`);
    return cloneWorkflowRunAggregate(run);
  }

  async saveRun(
    runId: string,
    input: SaveCreativeWorkflowRunRequest
  ): Promise<WorkflowRunAggregateV1> {
    const current = this.store.get(runId);
    if (!current) throw new Error(`missing run ${runId}`);
    expect(input.expectedRevision).toBe(String(current.revision));
    expect(validateWorkflowRunTransition(current, input.run).ok).toBe(true);
    this.savedStatuses.push(input.run.record.status);
    this.store.set(runId, cloneWorkflowRunAggregate(input.run));
    return cloneWorkflowRunAggregate(input.run);
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
    status,
    error: status === 'failed'
      ? { kind: 'provider_failure', message: 'provider rejected task', httpStatus: 400 }
      : null,
    resultAssetIds,
    attempt: 1,
    submittedAt: 2_100,
    startedAt: status === 'queued' ? null : 2_101,
    finishedAt: status === 'queued' || status === 'running' ? null : 2_102,
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

const textAssets = (body = '{"prompts":[]}'): WorkflowTextAssetReader => ({
  read: async () => body,
});

function clock(): () => number {
  let now = 2_100;
  return () => {
    now += 10;
    return now;
  };
}

describe('Creative Workflow run controller', () => {
  test('persists every task id before submitting and completes one image run', async () => {
    const runs = new MemoryRunApi();
    const tasks = new ImmediateTaskPort();
    const controller = createCreativeWorkflowRunController({
      runs,
      tasks,
      textAssets: textAssets(),
      createId: idFactory([IDS.request, IDS.task]),
      now: clock(),
      pollIntervalMs: 0,
    });
    const workflow = createExecutableWorkflowFixture();
    const result = await controller.start({
      workflow,
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
        kind: 'workflow_step',
        workflowRunId: IDS.request,
        workflowStepId: IDS.generateStep,
      },
      capability: 'i2i',
      inputs: [{ assetId: IDS.asset, role: 'reference' }],
    });
    expect(controller.getSnapshot().activities[IDS.request]).toBeUndefined();
  });

  test('stops for durable prompt review, then executes approved image tasks', async () => {
    const runs = new MemoryRunApi();
    const tasks = new ImmediateTaskPort();
    const controller = createCreativeWorkflowRunController({
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
    const workflow = createExecutableWorkflowFixture(true);
    workflow.revision = 2;
    const review = await controller.start({
      workflow,
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
    const controller = createCreativeWorkflowRunController({
      runs,
      tasks,
      textAssets: textAssets(),
      createId: idFactory([IDS.request, IDS.task]),
      now: clock(),
    });
    const run = await controller.start({
      workflow: createExecutableWorkflowFixture(),
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

  test('persists authoritative task failures as terminal workflow failures', async () => {
    const runs = new MemoryRunApi();
    const tasks = new ImmediateTaskPort();
    tasks.failImage = true;
    const controller = createCreativeWorkflowRunController({
      runs,
      tasks,
      textAssets: textAssets(),
      createId: idFactory([IDS.request, IDS.task]),
      now: clock(),
    });
    const run = await controller.start({
      workflow: createExecutableWorkflowFixture(),
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
    const controller = createCreativeWorkflowRunController({
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
    const workflow = createExecutableWorkflowFixture(true);
    workflow.revision = 2;
    const review = await controller.start({
      workflow,
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
    const queued = createWorkflowRunFixture();
    queued.revision = 2;
    queued.request.referenceAssetIds = [IDS.asset];
    queued.record.status = 'queued';
    queued.record.taskIds = [IDS.task];
    queued.record.queuedAt = 2_100;
    runs.store.set(IDS.request, queued);
    const tasks = new ImmediateTaskPort();
    const controller = createCreativeWorkflowRunController({
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
