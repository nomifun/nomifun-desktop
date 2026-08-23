/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { BackendHttpError } from '@/common/adapter/httpBridge';
import { uuidv7 } from '@/common/utils/uuidv7';

import {
  CreativeTaskContractError,
  creativeTaskClient,
  isTerminalCreativeTaskStatus,
  pollCreativeTask,
  type CreateCreativeTaskInput,
  type CreativeTask,
  type CreativeTaskReference,
} from '../../tasks';
import {
  cloneTemplateRunAggregate,
  validateTemplateDefinition,
  validateTemplateInputsForDefinition,
  validateTemplateRunTransition,
  type CreativeTemplateRunAggregateV1,
} from '../domain';
import { creativeTemplateRunApi } from '../services';
import {
  buildTemplateTaskPlan,
  createImageTaskInput,
  createPlannerTaskInput,
  parsePlannerPromptDrafts,
  templateTaskReference,
  type CreativeTemplateTaskPlanEntry,
} from './plan';
import { templateTextAssetReader } from './textAssetReader';
import type {
  CreativeTemplateRunActivity,
  CreativeTemplateRunner,
  CreativeTemplateRuntimeDependencies,
  CreativeTemplateRuntimeSnapshot,
  ReviewCreativeTemplateDraft,
  StartCreativeTemplateRun,
} from './types';
import {
  CreativeTemplateRunRuntimeError,
  CreativeTemplateTextAssetHttpError,
} from './types';

const TERMINAL_RUN_STATUSES = new Set(['succeeded', 'failed', 'cancelled']);

function messageOf(error: unknown): string {
  return error instanceof Error && error.message.trim()
    ? error.message
    : String(error || 'template runtime failed');
}

function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === 'AbortError';
}

function isHttpStatus(error: unknown, status: number): boolean {
  return error instanceof BackendHttpError
    ? error.status === status
    : !!error
      && typeof error === 'object'
      && 'status' in error
      && (error as { status: unknown }).status === status;
}

function fatalExecutionError(error: unknown): boolean {
  if (error instanceof CreativeTemplateRunRuntimeError || error instanceof CreativeTaskContractError) {
    return true;
  }
  if (error instanceof CreativeTemplateTextAssetHttpError) {
    return error.status < 500 && ![408, 425, 429].includes(error.status);
  }
  if (error instanceof BackendHttpError) {
    return error.status >= 400
      && error.status < 500
      && ![408, 425, 429].includes(error.status);
  }
  return false;
}

function failureCode(error: unknown): string {
  const source = error instanceof CreativeTemplateRunRuntimeError
    ? error.code
    : error instanceof CreativeTaskContractError
      ? error.code
      : 'template_runtime_failure';
  const normalized = source
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/gu, '_')
    .replace(/^[^a-z]+/u, '');
  return normalized.slice(0, 80) || 'template_runtime_failure';
}

function terminalRun(run: CreativeTemplateRunAggregateV1): boolean {
  return TERMINAL_RUN_STATUSES.has(run.record.status);
}

function taskError(task: CreativeTask): CreativeTemplateRunRuntimeError | null {
  if (task.status === 'failed') {
    return new CreativeTemplateRunRuntimeError(
      'task-failed',
      task.error?.message || `template task ${task.taskId} failed`
    );
  }
  return task.status === 'canceled'
    ? new CreativeTemplateRunRuntimeError('task-cancelled', `template task ${task.taskId} was cancelled`)
    : null;
}

function copyActivity(activity: CreativeTemplateRunActivity): CreativeTemplateRunActivity {
  return {
    state: activity.state,
    taskStatuses: { ...activity.taskStatuses },
    error: activity.error,
  };
}

export class CreativeTemplateRunController implements CreativeTemplateRunner {
  private readonly listeners = new Set<() => void>();
  private readonly runById = new Map<string, CreativeTemplateRunAggregateV1>();
  private readonly inFlight = new Map<string, Promise<CreativeTemplateRunAggregateV1>>();
  private readonly abortByRun = new Map<string, AbortController>();
  private activities: Record<string, CreativeTemplateRunActivity> = {};
  private loading = false;
  private loadError: string | null = null;
  private loadPromise: Promise<void> | null = null;
  private snapshot: CreativeTemplateRuntimeSnapshot = {
    loading: false,
    loadError: null,
    runs: [],
    activities: {},
  };

  constructor(private readonly dependencies: CreativeTemplateRuntimeDependencies) {}

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): CreativeTemplateRuntimeSnapshot => this.snapshot;

  private publish(): void {
    this.snapshot = {
      loading: this.loading,
      loadError: this.loadError,
      runs: [...this.runById.values()]
        .sort(
          (left, right) =>
            right.request.requestedAt - left.request.requestedAt
            || right.request.id.localeCompare(left.request.id)
        )
        .map(cloneTemplateRunAggregate),
      activities: Object.fromEntries(
        Object.entries(this.activities).map(([templateRunId, activity]) => [templateRunId, copyActivity(activity)])
      ),
    };
    for (const listener of this.listeners) listener();
  }

  private upsert(run: CreativeTemplateRunAggregateV1): CreativeTemplateRunAggregateV1 {
    const copy = cloneTemplateRunAggregate(run);
    this.runById.set(copy.request.id, copy);
    if (copy.record.status === 'awaiting-review') {
      this.activities[copy.request.id] = {
        state: 'awaiting-review',
        taskStatuses: this.activities[copy.request.id]?.taskStatuses ?? {},
        error: null,
      };
    } else if (terminalRun(copy)) {
      delete this.activities[copy.request.id];
    }
    this.publish();
    return cloneTemplateRunAggregate(copy);
  }

  private activity(
    templateRunId: string,
    patch: Partial<CreativeTemplateRunActivity> & Pick<CreativeTemplateRunActivity, 'state'>
  ): void {
    const current = this.activities[templateRunId];
    this.activities = {
      ...this.activities,
      [templateRunId]: {
        state: patch.state,
        taskStatuses: patch.taskStatuses ?? current?.taskStatuses ?? {},
        error: patch.error === undefined ? current?.error ?? null : patch.error,
      },
    };
    this.publish();
  }

  private observeTask(templateRunId: string, task: CreativeTask): void {
    const current = this.activities[templateRunId];
    this.activity(templateRunId, {
      state: current?.state === 'cancelling' ? 'cancelling' : 'executing',
      error: null,
      taskStatuses: {
        ...(current?.taskStatuses ?? {}),
        [task.taskId]: task.status,
      },
    });
  }

  private timestamp(run: CreativeTemplateRunAggregateV1): number {
    return Math.max(
      this.dependencies.now(),
      run.request.requestedAt,
      run.record.queuedAt ?? 0,
      run.record.startedAt ?? 0,
      run.record.completedAt ?? 0
    );
  }

  async load(): Promise<void> {
    if (this.loadPromise) return this.loadPromise;
    this.loadPromise = (async () => {
      this.loading = true;
      this.loadError = null;
      this.publish();
      try {
        const runs = await this.dependencies.runs.listRuns();
        this.runById.clear();
        for (const run of runs) this.runById.set(run.request.id, cloneTemplateRunAggregate(run));
        this.loading = false;
        this.publish();
        for (const run of runs) {
          if (run.record.status === 'requested' || run.record.status === 'queued' || run.record.status === 'running') {
            void this.resume(run.request.id).catch(() => undefined);
          } else if (run.record.status === 'awaiting-review') {
            this.activity(run.request.id, { state: 'awaiting-review', error: null });
          }
        }
      } catch (error) {
        this.loading = false;
        this.loadError = messageOf(error);
        this.publish();
        throw error;
      }
    })().finally(() => {
      this.loadPromise = null;
    });
    return this.loadPromise;
  }

  async start(input: StartCreativeTemplateRun): Promise<CreativeTemplateRunAggregateV1> {
    const definition = validateTemplateDefinition(input.template, '$.template');
    if (!definition.ok) {
      throw new CreativeTemplateRunRuntimeError(
        'invalid-plan',
        `${definition.error.path}: ${definition.error.message}`
      );
    }
    const inputs = validateTemplateInputsForDefinition(input.template, input.inputs, '$.inputs');
    if (!inputs.ok) {
      throw new CreativeTemplateRunRuntimeError('invalid-plan', `${inputs.error.path}: ${inputs.error.message}`);
    }
    const templateRunId = this.dependencies.createId();
    this.activity(templateRunId, {
      state: 'submitting',
      taskStatuses: {},
      error: null,
    });
    const request = {
      templateRunId,
      templateId: input.template.id,
      templateRevision: input.template.revision,
      inputs: input.inputs,
      referenceAssetIds: input.referenceAssetIds,
    };
    let run: CreativeTemplateRunAggregateV1;
    try {
      try {
        run = await this.dependencies.runs.createRun(request);
      } catch (error) {
        if (fatalExecutionError(error) || isAbortError(error)) throw error;
        run = await this.dependencies.runs.createRun(request);
      }
    } catch (error) {
      this.activity(templateRunId, { state: 'paused', error: messageOf(error) });
      throw error;
    }
    this.upsert(run);
    return this.resume(run.request.id);
  }

  async resume(templateRunId: string): Promise<CreativeTemplateRunAggregateV1> {
    const current = this.inFlight.get(templateRunId);
    if (current) return current;
    const promise = this.advance(templateRunId).finally(() => {
      if (this.inFlight.get(templateRunId) === promise) this.inFlight.delete(templateRunId);
    });
    this.inFlight.set(templateRunId, promise);
    return promise;
  }

  private async currentRun(templateRunId: string): Promise<CreativeTemplateRunAggregateV1> {
    const local = this.runById.get(templateRunId);
    return local ? cloneTemplateRunAggregate(local) : this.upsert(await this.dependencies.runs.getRun(templateRunId));
  }

  private async persist(
    current: CreativeTemplateRunAggregateV1,
    next: CreativeTemplateRunAggregateV1
  ): Promise<CreativeTemplateRunAggregateV1> {
    const transition = validateTemplateRunTransition(current, next);
    if (!transition.ok) {
      throw new CreativeTemplateRunRuntimeError(
        'invalid-plan',
        `${transition.error.path}: ${transition.error.message}`
      );
    }
    try {
      return this.upsert(await this.dependencies.runs.saveRun(current.request.id, {
        expectedRevision: String(current.revision),
        run: next,
      }));
    } catch (error) {
      if (!isHttpStatus(error, 409)) throw error;
      return this.upsert(await this.dependencies.runs.getRun(current.request.id));
    }
  }

  private async runTask(
    templateRunId: string,
    input: CreateCreativeTaskInput,
    signal: AbortSignal
  ): Promise<CreativeTask> {
    let task = await this.dependencies.tasks.create(input, signal);
    this.observeTask(templateRunId, task);
    if (!isTerminalCreativeTaskStatus(task.status)) {
      const reference: CreativeTaskReference = {
        taskId: input.idempotencyKey,
        owner: input.owner,
        providerId: input.providerId,
        model: input.model,
        task: input.task,
        capability: input.capability,
      };
      task = await pollCreativeTask(this.dependencies.tasks, reference, {
        signal,
        intervalMs: this.dependencies.pollIntervalMs,
        maxWaitMs: this.dependencies.pollMaxWaitMs,
        onTask: (progress) => this.observeTask(templateRunId, progress),
      });
    }
    const terminalError = taskError(task);
    if (terminalError) throw terminalError;
    return task;
  }

  private async executePlanner(
    run: CreativeTemplateRunAggregateV1,
    plan: readonly CreativeTemplateTaskPlanEntry[],
    signal: AbortSignal
  ): Promise<CreativeTemplateRunAggregateV1> {
    const planner = plan.find(
      (entry): entry is Extract<CreativeTemplateTaskPlanEntry, { kind: 'planner' }> => entry.kind === 'planner'
    );
    if (!planner) throw new CreativeTemplateRunRuntimeError('invalid-plan', 'multi-image run has no planner task');
    const task = await this.runTask(run.request.id, createPlannerTaskInput(run, planner), signal);
    if (task.resultAssetIds.length !== 1) {
      throw new CreativeTemplateRunRuntimeError(
        'planner-output',
        'planner task must produce exactly one text asset'
      );
    }
    const text = await this.dependencies.textAssets.read(task.resultAssetIds[0], signal);
    const drafts = parsePlannerPromptDrafts(
      text,
      run,
      this.dependencies.createId,
      () => this.timestamp(run)
    );
    const next = cloneTemplateRunAggregate(run);
    next.revision += 1;
    next.promptDrafts = drafts;
    next.record.promptDraftIds = drafts.map((draft) => draft.id);
    next.record.status = run.templateSnapshot.output.kind === 'multi-image-series'
      && run.templateSnapshot.output.reviewRequired
      ? 'awaiting-review'
      : 'running';
    return this.persist(run, next);
  }

  private async executeImageTasks(
    run: CreativeTemplateRunAggregateV1,
    plan: readonly CreativeTemplateTaskPlanEntry[],
    signal: AbortSignal
  ): Promise<CreativeTemplateRunAggregateV1> {
    const images = plan.filter(
      (entry): entry is Extract<CreativeTemplateTaskPlanEntry, { kind: 'image' }> => entry.kind === 'image'
    );
    const concurrency = run.templateSnapshot.output.kind === 'multi-image-series'
      ? run.templateSnapshot.output.concurrency
      : 1;
    const results: string[][] = Array.from({ length: images.length }, () => []);
    let cursor = 0;
    let firstError: unknown = null;
    const worker = async (): Promise<void> => {
      while (firstError === null && cursor < images.length) {
        const index = cursor;
        cursor += 1;
        const entry = images[index];
        try {
          const task = await this.runTask(run.request.id, createImageTaskInput(run, entry), signal);
          if (task.resultAssetIds.length !== entry.step.generation.imagesPerPrompt) {
            throw new CreativeTemplateRunRuntimeError(
              'task-failed',
              `image task ${task.taskId} returned an unexpected result count`
            );
          }
          results[index] = [...task.resultAssetIds];
        } catch (error) {
          firstError ??= error;
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(concurrency, images.length) }, worker));
    if (firstError !== null) throw firstError;
    const resultAssetIds = results.flat();
    if (
      run.record.resultAssetIds.length > resultAssetIds.length
      || !run.record.resultAssetIds.every((assetId, index) => assetId === resultAssetIds[index])
    ) {
      throw new CreativeTemplateRunRuntimeError(
        'invalid-plan',
        'persisted template results do not match task output order'
      );
    }
    const next = cloneTemplateRunAggregate(run);
    next.revision += 1;
    next.record.status = 'succeeded';
    next.record.resultAssetIds = resultAssetIds;
    next.record.completedAt = this.timestamp(run);
    return this.persist(run, next);
  }

  private async persistFailure(
    run: CreativeTemplateRunAggregateV1,
    error: unknown
  ): Promise<CreativeTemplateRunAggregateV1> {
    if (terminalRun(run)) return run;
    const next = cloneTemplateRunAggregate(run);
    next.revision += 1;
    next.record.status = 'failed';
    next.record.completedAt = this.timestamp(run);
    next.record.failure = {
      code: failureCode(error),
      message: messageOf(error).slice(0, 2_000),
    };
    return this.persist(run, next);
  }

  private async advance(templateRunId: string): Promise<CreativeTemplateRunAggregateV1> {
    let run = await this.currentRun(templateRunId);
    if (terminalRun(run) || run.record.status === 'awaiting-review') return run;
    const controller = new AbortController();
    this.abortByRun.set(templateRunId, controller);
    this.activity(templateRunId, { state: 'executing', error: null });
    try {
      for (let transitions = 0; transitions < 16; transitions += 1) {
        if (controller.signal.aborted) throw new DOMException('Template run aborted', 'AbortError');
        if (terminalRun(run) || run.record.status === 'awaiting-review') return run;
        if (run.record.status === 'requested') {
          const plan = buildTemplateTaskPlan(
            run.templateSnapshot,
            undefined,
            this.dependencies.createId
          );
          const next = cloneTemplateRunAggregate(run);
          next.revision += 1;
          next.record.status = 'queued';
          next.record.taskIds = plan.map((entry) => entry.taskId);
          next.record.queuedAt = this.timestamp(run);
          run = await this.persist(run, next);
          continue;
        }
        const plan = buildTemplateTaskPlan(
          run.templateSnapshot,
          run.record.taskIds,
          this.dependencies.createId
        );
        if (run.record.status === 'queued') {
          const next = cloneTemplateRunAggregate(run);
          next.revision += 1;
          next.record.status = 'running';
          next.record.startedAt = this.timestamp(run);
          run = await this.persist(run, next);
          continue;
        }
        if (run.record.status !== 'running') {
          throw new CreativeTemplateRunRuntimeError('invalid-plan', `unsupported active status ${run.record.status}`);
        }
        if (run.templateSnapshot.output.kind === 'multi-image-series' && run.promptDrafts.length === 0) {
          run = await this.executePlanner(run, plan, controller.signal);
          continue;
        }
        run = await this.executeImageTasks(run, plan, controller.signal);
        return run;
      }
      throw new CreativeTemplateRunRuntimeError('invalid-plan', 'template run exceeded its transition budget');
    } catch (error) {
      if (isAbortError(error)) throw error;
      if (fatalExecutionError(error)) {
        run = await this.persistFailure(run, error);
        return run;
      }
      this.activity(templateRunId, { state: 'paused', error: messageOf(error) });
      return run;
    } finally {
      if (this.abortByRun.get(templateRunId) === controller) this.abortByRun.delete(templateRunId);
    }
  }

  async review(
    templateRunId: string,
    drafts: readonly ReviewCreativeTemplateDraft[]
  ): Promise<CreativeTemplateRunAggregateV1> {
    const currentFlight = this.inFlight.get(templateRunId);
    if (currentFlight) await currentFlight;
    const run = await this.currentRun(templateRunId);
    if (run.record.status !== 'awaiting-review') {
      throw new CreativeTemplateRunRuntimeError('invalid-plan', 'template run is not awaiting review');
    }
    if (
      drafts.length !== run.promptDrafts.length
      || new Set(drafts.map((draft) => draft.id)).size !== drafts.length
    ) {
      throw new CreativeTemplateRunRuntimeError('invalid-plan', 'review must include every prompt draft exactly once');
    }
    const reviewedAt = this.timestamp(run);
    const next = cloneTemplateRunAggregate(run);
    next.revision += 1;
    next.record.status = 'running';
    next.promptDrafts = run.promptDrafts.map((draft) => {
      const replacement = drafts.find((candidate) => candidate.id === draft.id);
      if (!replacement) throw new CreativeTemplateRunRuntimeError('invalid-plan', 'review draft identity mismatch');
      return {
        ...draft,
        title: replacement.title,
        prompt: replacement.prompt,
        status: 'approved',
        reviewedAt,
        reviewNote: replacement.reviewNote ?? null,
      };
    });
    const saved = await this.persist(run, next);
    if (saved.record.status !== 'running') {
      throw new CreativeTemplateRunRuntimeError('invalid-plan', 'template review lost a concurrent CAS race');
    }
    return this.resume(templateRunId);
  }

  async cancel(templateRunId: string): Promise<CreativeTemplateRunAggregateV1> {
    this.activity(templateRunId, { state: 'cancelling', error: null });
    this.abortByRun.get(templateRunId)?.abort();
    const currentFlight = this.inFlight.get(templateRunId);
    if (currentFlight) {
      try {
        await currentFlight;
      } catch (error) {
        if (!isAbortError(error)) throw error;
      }
    }
    let run = await this.currentRun(templateRunId);
    if (terminalRun(run)) return run;
    if (run.record.taskIds.length > 0) {
      const plan = buildTemplateTaskPlan(
        run.templateSnapshot,
        run.record.taskIds,
        this.dependencies.createId
      );
      await Promise.all(
        plan.map(async (entry) => {
          try {
            const reference = templateTaskReference(run, entry);
            const task = await this.dependencies.tasks.cancel(reference);
            this.observeTask(templateRunId, task);
          } catch (error) {
            if (!isHttpStatus(error, 404)) throw error;
          }
        })
      );
    }
    for (let attempt = 0; attempt < 3 && !terminalRun(run); attempt += 1) {
      const next = cloneTemplateRunAggregate(run);
      next.revision += 1;
      next.record.status = 'cancelled';
      next.record.completedAt = this.timestamp(run);
      next.record.failure = null;
      run = await this.persist(run, next);
    }
    if (!terminalRun(run)) {
      throw new CreativeTemplateRunRuntimeError('invalid-plan', 'template cancellation lost repeated CAS races');
    }
    return run;
  }
}

export function createCreativeTemplateRunController(
  dependencies: CreativeTemplateRuntimeDependencies
): CreativeTemplateRunController {
  return new CreativeTemplateRunController(dependencies);
}

export const creativeTemplateRunController = createCreativeTemplateRunController({
  runs: creativeTemplateRunApi,
  tasks: creativeTaskClient,
  textAssets: templateTextAssetReader,
  createId: uuidv7,
  now: Date.now,
});
