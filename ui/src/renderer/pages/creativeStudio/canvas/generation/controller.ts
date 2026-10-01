/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CreativeAssetDeletedError, isCreativeAssetDeleted, type CreativeAsset, type CreativeAssetPort } from "../../assets";
import {
  assertCreativeTaskReference,
  assertTaskCapabilityPair,
  createCreativeTaskIdempotencyKey,
  creativeTaskReference,
  isTerminalCreativeTaskStatus,
  pollCreativeTask,
  sameCreativeTaskOwner,
} from "../../tasks";
import type {
  CreateCreativeTaskInput,
  CreativeTask,
  CreativeTaskPollOptions,
  CreativeTaskPort,
  CreativeTaskReference,
} from "../../tasks";
import { committedCanvasOutputs } from "./assets";
import { isPreparedCanvasGenerationRun } from "./plans";
import type {
  CanvasGenerationResumeRequest,
  CanvasGenerationRuntimeEntry,
  CanvasGenerationRuntimeSnapshot,
  PreparedCanvasGenerationRun,
} from "./types";
import { GenerationError } from "./types";

export interface CanvasGenerationRuntimeControllerOptions {
  poll?: Omit<CreativeTaskPollOptions, "signal" | "onTask">;
  /** Resolve the authored node when the backend owner is a generated config node. */
  nodeIdForTask?: (reference: CreativeTaskReference) => string;
  /** Must durably record the known idempotency task reference before POST. */
  onPendingTask?: (
    reference: CreativeTaskReference,
    signal: AbortSignal,
  ) => void | Promise<void>;
  /** Removes a terminal task from durable pending state; mount recovery may repeat it. */
  onSettledTask?: (
    task: CreativeTask,
    signal: AbortSignal,
  ) => void | Promise<void>;
  /** Return true only after an orphaned pending reference was durably removed. */
  onRecoveryFailure?: (
    reference: CreativeTaskReference,
    error: unknown,
    signal: AbortSignal,
  ) => boolean | Promise<boolean>;
}

export type CanvasGenerationRuntimeListener = (
  snapshot: CanvasGenerationRuntimeSnapshot,
) => void;

const INITIAL_SNAPSHOT: CanvasGenerationRuntimeSnapshot = {
  state: "idle",
  entries: [],
  submissionFailures: [],
  submittingCount: 0,
  recoveringCount: 0,
  requestError: null,
};

function asError(value: unknown): Error {
  return value instanceof Error ? value : new Error(String(value));
}

class TaskSettledElsewhereError extends Error {
  constructor() {
    super(
      "Task reached a terminal state through another authoritative request",
    );
    this.name = "TaskSettledElsewhereError";
  }
}

function runtimeState(
  entries: readonly CanvasGenerationRuntimeEntry[],
  submittingCount: number,
  recoveringCount: number,
  requestError: Error | null,
): CanvasGenerationRuntimeSnapshot["state"] {
  if (submittingCount > 0) return "submitting";
  if (recoveringCount > 0) return "recovering";
  if (requestError || entries.some((entry) => entry.requestError))
    return "request_error";
  if (entries.some((entry) => entry.task.status === "running"))
    return "running";
  if (entries.some((entry) => entry.task.status === "queued")) return "queued";
  if (entries.length === 0) return "idle";
  const statuses = new Set(entries.map((entry) => entry.task.status));
  if (statuses.size !== 1) return "mixed";
  const [status] = statuses;
  return status === "succeeded" || status === "failed" || status === "canceled"
    ? status
    : "mixed";
}

function cloneInput(input: CreateCreativeTaskInput): CreateCreativeTaskInput {
  return structuredClone(input);
}

function cloneTask(task: CreativeTask): CreativeTask {
  return {
    ...task,
    owner: { ...task.owner },
    parameters: structuredClone(task.parameters),
    error: task.error ? { ...task.error } : null,
    resultAssetIds: [...task.resultAssetIds],
  };
}

function pendingReference(
  input: CreateCreativeTaskInput,
): CreativeTaskReference {
  return {
    taskId: input.idempotencyKey,
    owner: { ...input.owner },
    providerId: input.providerId,
    model: input.model,
    task: input.task,
    capability: input.capability,
  };
}

function expectedOutputKind(
  input: Pick<CreateCreativeTaskInput, "task">,
): "image" | "video" | "audio" {
  if (input.task === "image_generation" || input.task === "image_edit")
    return "image";
  if (input.task === "video_generation") return "video";
  if (input.task === "speech_synthesis") return "audio";
  throw new GenerationError(
    "task_capability_mismatch",
    `Canvas generation runtime cannot project outputs for ${input.task}`,
    "task",
  );
}

function assertPreparedRun(plan: PreparedCanvasGenerationRun): void {
  if (!isPreparedCanvasGenerationRun(plan)) {
    throw new GenerationError(
      "invalid_parameters",
      "Canvas generation runs must be created by a validated runtime plan builder",
      "plan",
    );
  }
  assertTaskCapabilityPair(plan.input.task, plan.input.capability);
  const expectedKind = expectedOutputKind(plan.input);
  if (plan.outputKind !== expectedKind || plan.kind !== expectedKind) {
    throw new GenerationError(
      "task_capability_mismatch",
      `${plan.input.task}/${plan.input.capability} must project ${expectedKind} output`,
      "outputKind",
    );
  }
  if (
    !Number.isSafeInteger(plan.repeat) ||
    plan.repeat < 1 ||
    plan.repeat > 6
  ) {
    throw new GenerationError(
      "invalid_parameters",
      "Canvas generation repeat must be an integer between 1 and 6",
      "repeat",
    );
  }
  if (
    plan.model.providerId !== plan.input.providerId ||
    plan.model.model !== plan.input.model ||
    plan.model.task !== plan.input.task
  ) {
    throw new GenerationError(
      "model_not_compatible",
      "Prepared run model identity does not match its task request",
      "model",
    );
  }
  assertInputAssets(plan.input, plan.references, "plan.references");
}

function assertInputAssets(
  input: CreateCreativeTaskInput,
  references: readonly CreativeAsset[],
  field: string,
): void {
  const assetIds = references.map((asset) => asset.id);
  const deleted = references.find(isCreativeAssetDeleted);
  if (deleted) throw new CreativeAssetDeletedError(deleted.id);
  if (new Set(assetIds).size !== assetIds.length) {
    throw new GenerationError(
      "reference_contract_mismatch",
      `${field} contains duplicate asset ids`,
      field,
    );
  }
  const inputIds = input.inputs.map((entry) => entry.assetId);
  if (
    inputIds.length !== assetIds.length ||
    new Set(inputIds).size !== inputIds.length ||
    inputIds.some((assetId) => !assetIds.includes(assetId))
  ) {
    throw new GenerationError(
      "reference_contract_mismatch",
      `${field} does not exactly match task inputs`,
      field,
    );
  }
}

function assertRetryIdentity(
  request: CanvasGenerationResumeRequest,
  requireAssetProof = true,
): void {
  const input = request.retryInput;
  if (!input) return;
  if (!sameCreativeTaskOwner(input.owner, request.reference.owner)) {
    throw new GenerationError(
      "model_not_compatible",
      "Resume retryInput owner does not match its task reference",
      "retryInput.owner",
    );
  }
  for (const field of ["providerId", "model", "task", "capability"] as const) {
    if (input[field] !== request.reference[field]) {
      throw new GenerationError(
        "model_not_compatible",
        `Resume retryInput ${field} does not match its task reference`,
        `retryInput.${field}`,
      );
    }
  }
  if (requireAssetProof) {
    assertInputAssets(input, request.retryReferences ?? [], "retryReferences");
  }
}

/**
 * Imperative, subscribable controller shared by image/video/audio hooks. It
 * keeps real task ids only; no placeholder task, URL, progress, or terminal
 * state is synthesized while a create request is pending.
 */
export class CanvasGenerationRuntimeController {
  private readonly listeners = new Set<CanvasGenerationRuntimeListener>();
  private readonly pollOptions: Omit<
    CreativeTaskPollOptions,
    "signal" | "onTask"
  >;
  private readonly onPendingTask: CanvasGenerationRuntimeControllerOptions["onPendingTask"];
  private readonly onSettledTask: CanvasGenerationRuntimeControllerOptions["onSettledTask"];
  private readonly onRecoveryFailure: CanvasGenerationRuntimeControllerOptions["onRecoveryFailure"];
  private readonly nodeIdForTask: CanvasGenerationRuntimeControllerOptions["nodeIdForTask"];
  private readonly taskNodeKeys = new Map<string, string>();
  private readonly recoveryFailureTaskIds = new Set<string>();
  private current: CanvasGenerationRuntimeSnapshot = INITIAL_SNAPSHOT;
  private generation = 0;
  private nextOrder = 0;
  private operationController: AbortController | null = null;
  private readonly submittingTaskIds = new Set<string>();
  private readonly cancelRequestedIds = new Set<string>();
  private readonly cancelDispatchedIds = new Set<string>();
  private readonly terminalTaskIds = new Set<string>();
  private readonly activeWorkerTaskIds = new Set<string>();
  private readonly recoveringRequests = new Map<
    string,
    { request: CanvasGenerationResumeRequest; order: number }
  >();
  private disposed = false;

  constructor(
    private readonly tasks: CreativeTaskPort,
    private readonly assets: CreativeAssetPort,
    options: CanvasGenerationRuntimeControllerOptions = {},
  ) {
    this.pollOptions = options.poll ?? {};
    this.onPendingTask = options.onPendingTask;
    this.onSettledTask = options.onSettledTask;
    this.onRecoveryFailure = options.onRecoveryFailure;
    this.nodeIdForTask = options.nodeIdForTask;
  }

  snapshot(): CanvasGenerationRuntimeSnapshot {
    return {
      ...this.current,
      entries: this.current.entries.map((entry) => ({
        ...entry,
        task: cloneTask(entry.task),
        outputs: entry.outputs.map((output) => ({ ...output })),
        retryInput: entry.retryInput ? cloneInput(entry.retryInput) : null,
      })),
      submissionFailures: this.current.submissionFailures.map((failure) => ({
        ...failure,
        input: cloneInput(failure.input),
      })),
    };
  }

  subscribe(listener: CanvasGenerationRuntimeListener): () => void {
    // React StrictMode intentionally performs an effect cleanup/setup cycle.
    this.disposed = false;
    this.listeners.add(listener);
    listener(this.snapshot());
    return () => this.listeners.delete(listener);
  }

  private emit(next: Omit<CanvasGenerationRuntimeSnapshot, "state">): void {
    this.current = {
      ...next,
      state: runtimeState(
        next.entries,
        next.submittingCount,
        next.recoveringCount,
        next.requestError,
      ),
    };
    const snapshot = this.snapshot();
    for (const listener of this.listeners) listener(snapshot);
  }

  private update(
    patch: Partial<Omit<CanvasGenerationRuntimeSnapshot, "state">>,
  ): void {
    this.emit({
      entries: patch.entries ?? this.current.entries,
      submissionFailures:
        patch.submissionFailures ?? this.current.submissionFailures,
      submittingCount: patch.submittingCount ?? this.current.submittingCount,
      recoveringCount: patch.recoveringCount ?? this.current.recoveringCount,
      requestError:
        patch.requestError === undefined
          ? this.current.requestError
          : patch.requestError,
    });
  }

  private isCurrent(generation: number, controller: AbortController): boolean {
    return generation === this.generation && !controller.signal.aborted;
  }

  private upsert(entry: CanvasGenerationRuntimeEntry): void {
    const existing = this.current.entries.find(
      (candidate) => candidate.task.taskId === entry.task.taskId,
    );
    if (existing && existing.order !== entry.order) {
      throw new GenerationError(
        "invalid_parameters",
        `Backend reused task id ${entry.task.taskId} for two canvas generation slots`,
        "taskId",
      );
    }
    if (existing && isTerminalCreativeTaskStatus(existing.task.status)) {
      if (
        !isTerminalCreativeTaskStatus(entry.task.status) ||
        entry.requestError === null
      ) {
        return;
      }
    }
    const entries = this.current.entries.filter(
      (candidate) => candidate.task.taskId !== entry.task.taskId,
    );
    entries.push(entry);
    entries.sort((left, right) => left.order - right.order);
    this.update({ entries });
  }

  private publishTask(
    generation: number,
    controller: AbortController,
    order: number,
    task: CreativeTask,
    retryInput: CreateCreativeTaskInput | null,
    outputKind: "image" | "video" | "audio",
    requestError: Error | null = null,
  ): void {
    if (!this.isCurrent(generation, controller)) return;
    const existing = this.current.entries.find(
      (entry) => entry.task.taskId === task.taskId,
    );
    if (
      (this.terminalTaskIds.has(task.taskId) ||
        (existing && isTerminalCreativeTaskStatus(existing.task.status))) &&
      !isTerminalCreativeTaskStatus(task.status)
    ) {
      return;
    }
    if (isTerminalCreativeTaskStatus(task.status))
      this.terminalTaskIds.add(task.taskId);
    this.upsert({
      order,
      task,
      outputs: committedCanvasOutputs(task, outputKind, this.assets),
      requestError,
      retryInput: retryInput ? cloneInput(retryInput) : null,
      outputKind,
    });
  }

  private publishWorkerError(
    generation: number,
    controller: AbortController,
    order: number,
    task: CreativeTask | null,
    retryInput: CreateCreativeTaskInput | null,
    outputKind: "image" | "video" | "audio",
    reason: unknown,
    submissionFailure = false,
  ): void {
    if (!this.isCurrent(generation, controller)) return;
    const error = asError(reason);
    if (task) {
      try {
        this.publishTask(
          generation,
          controller,
          order,
          task,
          retryInput,
          outputKind,
          error,
        );
      } catch {
        try {
          this.upsert({
            order,
            task,
            outputs: [],
            requestError: error,
            retryInput: retryInput ? cloneInput(retryInput) : null,
            outputKind,
          });
        } catch (mappingError) {
          this.update({ requestError: asError(mappingError) });
        }
      }
    } else {
      const failure =
        submissionFailure && retryInput
          ? {
              order,
              input: cloneInput(retryInput),
              outputKind,
              error,
            }
          : null;
      this.update({
        requestError: error,
        submissionFailures: failure
          ? [
              ...this.current.submissionFailures.filter(
                (candidate) => candidate.order !== order,
              ),
              failure,
            ].sort((left, right) => left.order - right.order)
          : this.current.submissionFailures,
      });
    }
  }

  private decrement(field: "submittingCount" | "recoveringCount"): void {
    this.update({ [field]: Math.max(0, this.current[field] - 1) });
  }

  private clearSubmissionFailure(order: number): void {
    const previous = this.current.submissionFailures.find((failure) => failure.order === order);
    if (!previous) return;
    const submissionFailures = this.current.submissionFailures.filter((failure) => failure.order !== order);
    this.update({
      submissionFailures,
      requestError: this.current.requestError === previous.error
        ? (submissionFailures.at(-1)?.error ?? null)
        : this.current.requestError,
    });
  }

  private async notifySettled(
    task: CreativeTask,
    controller: AbortController,
  ): Promise<void> {
    if (!isTerminalCreativeTaskStatus(task.status) || !this.onSettledTask)
      return;
    controller.signal.throwIfAborted();
    await this.onSettledTask(cloneTask(task), controller.signal);
  }

  private async createWorker(
    generation: number,
    controller: AbortController,
    order: number,
    input: CreateCreativeTaskInput,
    outputKind: "image" | "video" | "audio",
  ): Promise<void> {
    if (!this.isCurrent(generation, controller)) return;
    let task: CreativeTask | null = null;
    let submissionReleased = false;
    this.submittingTaskIds.add(input.idempotencyKey);
    this.activeWorkerTaskIds.add(input.idempotencyKey);
    try {
      await this.onPendingTask?.(pendingReference(input), controller.signal);
      if (!this.isCurrent(generation, controller)) return;
      const created = await this.tasks.create(cloneInput(input), controller.signal);
      if (!this.isCurrent(generation, controller)) return;
      assertCreativeTaskReference(created, pendingReference(input));
      task = created;
      this.clearSubmissionFailure(order);
      this.publishTask(generation, controller, order, task, input, outputKind);
      this.submittingTaskIds.delete(input.idempotencyKey);
      this.decrement("submittingCount");
      submissionReleased = true;
      if (isTerminalCreativeTaskStatus(task.status)) {
        await this.notifySettled(task, controller);
        return;
      }
      if (
        this.cancelRequestedIds.has(task.taskId) &&
        !this.cancelDispatchedIds.has(task.taskId)
      ) {
        this.cancelDispatchedIds.add(task.taskId);
        const canceled = await this.tasks.cancel(
          creativeTaskReference(task),
          controller.signal,
        );
        if (!this.isCurrent(generation, controller)) return;
        assertCreativeTaskReference(canceled, pendingReference(input));
        task = canceled;
        this.publishTask(
          generation,
          controller,
          order,
          task,
          input,
          outputKind,
        );
        if (isTerminalCreativeTaskStatus(task.status)) {
          await this.notifySettled(task, controller);
          return;
        }
      }
      const reference = creativeTaskReference(task);
      task = await pollCreativeTask(this.tasks, reference, {
        ...this.pollOptions,
        signal: controller.signal,
        onTask: (update) => {
          task = update;
          if (this.terminalTaskIds.has(update.taskId)) {
            throw new TaskSettledElsewhereError();
          }
          this.publishTask(
            generation,
            controller,
            order,
            update,
            input,
            outputKind,
          );
        },
      });
      await this.notifySettled(task, controller);
    } catch (reason) {
      if (reason instanceof TaskSettledElsewhereError) return;
      this.publishWorkerError(
        generation,
        controller,
        order,
        task,
        input,
        outputKind,
        reason,
        true,
      );
      if (this.isCurrent(generation, controller) && !submissionReleased) {
        this.decrement("submittingCount");
        submissionReleased = true;
      }
    } finally {
      if (this.isCurrent(generation, controller)) {
        this.submittingTaskIds.delete(input.idempotencyKey);
        this.activeWorkerTaskIds.delete(input.idempotencyKey);
        this.update({});
      }
    }
  }

  private assertWorkersIdle(): void {
    if (
      this.current.submittingCount > 0 ||
      this.current.recoveringCount > 0 ||
      this.activeWorkerTaskIds.size > 0
    ) {
      throw new GenerationError(
        "busy",
        "A canvas generation task is already active",
      );
    }
  }

  private assertNotBusy(): void {
    this.assertWorkersIdle();
    if (
      this.current.submissionFailures.length > 0 ||
      this.current.requestError !== null ||
      this.current.entries.some((entry) => entry.requestError !== null)
    ) {
      throw new GenerationError(
        "busy",
        "A canvas generation request has an unresolved outcome",
      );
    }
  }

  private assertUsable(): void {
    if (this.disposed) {
      throw new GenerationError(
        "disposed",
        "This canvas generation runtime controller has been disposed",
      );
    }
  }

  private ensureOperation(): {
    generation: number;
    controller: AbortController;
  } {
    this.assertUsable();
    if (!this.operationController || this.operationController.signal.aborted) {
      this.operationController = new AbortController();
      this.generation += 1;
    }
    return {
      generation: this.generation,
      controller: this.operationController,
    };
  }

  private async runInput(
    input: CreateCreativeTaskInput,
    repeat: number,
    outputKind: "image" | "video" | "audio",
  ): Promise<CanvasGenerationRuntimeSnapshot> {
    this.assertUsable();
    if (
      this.activeWorkerTaskIds.has(input.idempotencyKey) ||
      this.current.entries.some((entry) => entry.task.taskId === input.idempotencyKey) ||
      this.current.submissionFailures.some((failure) => failure.input.idempotencyKey === input.idempotencyKey)
    ) {
      throw new GenerationError("busy", "This canvas generation request already exists; retry its existing task", "idempotencyKey");
    }
    const nodeKey = this.assertNodeIdle(pendingReference(input));
    const { generation, controller } = this.ensureOperation();
    const firstOrder = this.nextOrder;
    this.nextOrder += repeat;
    const cloned = cloneInput(input);
    const inputs = Array.from({ length: repeat }, (_, index) => index === 0
      ? cloned
      : { ...cloneInput(cloned), idempotencyKey: createCreativeTaskIdempotencyKey() });
    for (const request of inputs) {
      this.taskNodeKeys.set(request.idempotencyKey, nodeKey);
      this.submittingTaskIds.add(request.idempotencyKey);
      this.activeWorkerTaskIds.add(request.idempotencyKey);
    }
    this.update({ submittingCount: this.current.submittingCount + repeat });
    await Promise.all(
      inputs.map((request, index) =>
        this.createWorker(
          generation,
          controller,
          firstOrder + index,
          request,
          outputKind,
        ),
      ),
    );
    return this.snapshot();
  }

  private nodeKey(reference: CreativeTaskReference): string {
    if (reference.owner.kind !== "canvas_node") {
      throw new GenerationError("invalid_parameters", "Canvas generation requires a canvas node owner", "owner");
    }
    return JSON.stringify([
      reference.owner.canvasId,
      this.nodeIdForTask?.(reference) ?? reference.owner.nodeId,
    ]);
  }

  private assertNodeIdle(reference: CreativeTaskReference, excludeTaskId?: string): string {
    const nodeKey = this.nodeKey(reference);
    if (this.hasUnfinishedNodeTask(nodeKey, excludeTaskId)) {
      throw new GenerationError("busy", "This canvas node already has an unfinished generation task", "nodeId");
    }
    return nodeKey;
  }

  /** Includes submission, recovery, and the final canvas persistence step. */
  isNodeBusy(canvasId: string, nodeId: string): boolean {
    return this.hasUnfinishedNodeTask(JSON.stringify([canvasId, nodeId]));
  }

  private hasUnfinishedNodeTask(nodeKey: string, excludeTaskId?: string): boolean {
    for (const [taskId, key] of this.taskNodeKeys) {
      if (key !== nodeKey || taskId === excludeTaskId) continue;
      const entry = this.current.entries.find((candidate) => candidate.task.taskId === taskId);
      if (
        this.activeWorkerTaskIds.has(taskId) ||
        this.recoveryFailureTaskIds.has(taskId) ||
        this.current.submissionFailures.some((failure) => failure.input.idempotencyKey === taskId) ||
        (entry && (!isTerminalCreativeTaskStatus(entry.task.status) || entry.requestError !== null))
      ) {
        return true;
      }
    }
    return false;
  }

  async run(
    plan: PreparedCanvasGenerationRun,
  ): Promise<CanvasGenerationRuntimeSnapshot> {
    this.assertUsable();
    assertPreparedRun(plan);
    return this.runInput(plan.input, plan.repeat, plan.outputKind);
  }

  private async resumeWorker(
    generation: number,
    controller: AbortController,
    order: number,
    request: CanvasGenerationResumeRequest,
  ): Promise<void> {
    if (!this.isCurrent(generation, controller)) return;
    let task: CreativeTask | null = null;
    let firstResponse = true;
    const releaseRecovery = (): void => {
      if (this.isCurrent(generation, controller) && this.recoveringRequests.delete(request.reference.taskId)) {
        this.decrement("recoveringCount");
      }
      firstResponse = false;
    };
    try {
      this.activeWorkerTaskIds.add(request.reference.taskId);
      const recovered = await this.tasks.get(request.reference, controller.signal);
      if (!this.isCurrent(generation, controller)) return;
      assertCreativeTaskReference(recovered, request.reference);
      task = recovered;
      this.recoveryFailureTaskIds.delete(request.reference.taskId);
      this.clearSubmissionFailure(order);
      this.publishTask(
        generation,
        controller,
        order,
        task,
        request.retryInput ?? null,
        request.outputKind,
      );
      releaseRecovery();
      if (isTerminalCreativeTaskStatus(task.status)) {
        await this.notifySettled(task, controller);
        return;
      }
      if (
        this.cancelRequestedIds.has(task.taskId) &&
        !this.cancelDispatchedIds.has(task.taskId)
      ) {
        this.cancelDispatchedIds.add(task.taskId);
        const canceled = await this.tasks.cancel(request.reference, controller.signal);
        if (!this.isCurrent(generation, controller)) return;
        assertCreativeTaskReference(canceled, request.reference);
        task = canceled;
        this.publishTask(
          generation,
          controller,
          order,
          task,
          request.retryInput ?? null,
          request.outputKind,
        );
        if (isTerminalCreativeTaskStatus(task.status)) {
          await this.notifySettled(task, controller);
          return;
        }
      }
      task = await pollCreativeTask(this.tasks, request.reference, {
        ...this.pollOptions,
        signal: controller.signal,
        onTask: (update) => {
          task = update;
          if (this.terminalTaskIds.has(update.taskId)) {
            throw new TaskSettledElsewhereError();
          }
          this.publishTask(
            generation,
            controller,
            order,
            update,
            request.retryInput ?? null,
            request.outputKind,
          );
        },
      });
      await this.notifySettled(task, controller);
    } catch (reason) {
      if (reason instanceof TaskSettledElsewhereError) return;
      if (task === null && this.isCurrent(generation, controller)) {
        this.recoveryFailureTaskIds.add(request.reference.taskId);
      }
      if (
        task === null &&
        this.onRecoveryFailure &&
        this.isCurrent(generation, controller) &&
        (await this.onRecoveryFailure(
          request.reference,
          reason,
          controller.signal,
        ))
      ) {
        this.recoveryFailureTaskIds.delete(request.reference.taskId);
        this.clearSubmissionFailure(order);
        return;
      }
      this.publishWorkerError(
        generation,
        controller,
        order,
        task,
        request.retryInput ?? null,
        request.outputKind,
        reason,
      );
      if (this.isCurrent(generation, controller) && firstResponse)
        releaseRecovery();
    } finally {
      if (this.isCurrent(generation, controller)) {
        this.activeWorkerTaskIds.delete(request.reference.taskId);
        if (firstResponse) releaseRecovery();
        this.update({});
      }
    }
  }

  async resume(
    requests: readonly CanvasGenerationResumeRequest[],
  ): Promise<CanvasGenerationRuntimeSnapshot> {
    this.assertUsable();
    if (requests.length === 0) return this.snapshot();
    const ids = requests.map((request) => request.reference.taskId);
    if (new Set(ids).size !== ids.length) {
      throw new GenerationError(
        "invalid_parameters",
        "Resume requests contain duplicate task ids",
        "requests",
      );
    }
    for (const request of requests) {
      if (request.reference.owner.kind !== "canvas_node") {
        throw new GenerationError("invalid_parameters", "Only canvas node tasks can resume in a canvas", "owner");
      }
      assertTaskCapabilityPair(
        request.reference.task,
        request.reference.capability,
      );
      if (expectedOutputKind(request.reference) !== request.outputKind) {
        throw new GenerationError(
          "task_capability_mismatch",
          `Resume task ${request.reference.taskId} cannot project ${request.outputKind} output`,
          "outputKind",
        );
      }
      assertRetryIdentity(request);
    }
    const { generation, controller } = this.ensureOperation();
    const workers = requests.filter((request) => !this.activeWorkerTaskIds.has(request.reference.taskId)).map((request) => {
      const existing = this.current.entries.find((entry) => entry.task.taskId === request.reference.taskId);
      if (existing) assertCreativeTaskReference(existing.task, request.reference);
      const failure = this.current.submissionFailures.find((candidate) => candidate.input.idempotencyKey === request.reference.taskId);
      const order = existing?.order ?? failure?.order ?? this.nextOrder++;
      return { request, order, nodeKey: this.nodeKey(request.reference) };
    });
    for (const { request, order, nodeKey } of workers) {
      this.taskNodeKeys.set(request.reference.taskId, nodeKey);
      this.recoveringRequests.set(request.reference.taskId, { request, order });
      this.activeWorkerTaskIds.add(request.reference.taskId);
    }
    this.update({ recoveringCount: this.current.recoveringCount + workers.length });
    await Promise.all(
      workers.map(({ request, order }) =>
        this.resumeWorker(generation, controller, order, request),
      ),
    );
    return this.snapshot();
  }

  async cancel(taskId?: string): Promise<CanvasGenerationRuntimeSnapshot> {
    this.assertUsable();
    if (taskId) {
      if (!this.activeWorkerTaskIds.has(taskId) && !this.current.entries.some(
        (entry) => entry.task.taskId === taskId && !isTerminalCreativeTaskStatus(entry.task.status)
      )) {
        throw new GenerationError("task_not_found", `No active canvas generation task ${taskId}`, "taskId");
      }
      this.cancelRequestedIds.add(taskId);
    }
    else {
      for (const id of this.submittingTaskIds) this.cancelRequestedIds.add(id);
      for (const id of this.recoveringRequests.keys()) this.cancelRequestedIds.add(id);
      for (const entry of this.current.entries) {
        if (!isTerminalCreativeTaskStatus(entry.task.status)) this.cancelRequestedIds.add(entry.task.taskId);
      }
    }
    const targets = new Map<
      string,
      {
        reference: ReturnType<typeof creativeTaskReference>;
        order: number;
        task: CreativeTask | null;
        retryInput: CreateCreativeTaskInput | null;
        outputKind: "image" | "video" | "audio";
      }
    >();
    for (const entry of this.current.entries) {
      if (
        !isTerminalCreativeTaskStatus(entry.task.status) &&
        (taskId === undefined || entry.task.taskId === taskId)
      ) {
        targets.set(entry.task.taskId, {
          reference: creativeTaskReference(entry.task),
          order: entry.order,
          task: entry.task,
          retryInput: entry.retryInput,
          outputKind: entry.outputKind,
        });
      }
    }
    for (const recovering of this.recoveringRequests.values()) {
      const { request, order } = recovering;
      if (taskId === undefined || request.reference.taskId === taskId) {
        targets.set(request.reference.taskId, {
          reference: request.reference,
          order,
          task: null,
          retryInput: request.retryInput ?? null,
          outputKind: request.outputKind,
        });
      }
    }
    if (
      taskId &&
      targets.size === 0 &&
      this.current.submittingCount === 0 &&
      this.current.recoveringCount === 0
    ) {
      throw new GenerationError(
        "task_not_found",
        `No active canvas generation task ${taskId}`,
        "taskId",
      );
    }
    const generation = this.generation;
    const controller = this.operationController;
    if (!controller) return this.snapshot();
    await Promise.all(
      [...targets.values()].map(async (target) => {
        const taskIdToCancel = target.reference.taskId;
        if (this.cancelDispatchedIds.has(taskIdToCancel)) return;
        this.cancelDispatchedIds.add(taskIdToCancel);
        try {
          const task = await this.tasks.cancel(
            target.reference,
            controller.signal,
          );
          if (!this.isCurrent(generation, controller)) return;
          assertCreativeTaskReference(task, target.reference);
          this.publishTask(
            generation,
            controller,
            target.order,
            task,
            target.retryInput,
            target.outputKind,
          );
          if (isTerminalCreativeTaskStatus(task.status)) {
            await this.notifySettled(task, controller);
          }
          if (this.recoveringRequests.delete(taskIdToCancel)) {
            this.decrement("recoveringCount");
          }
        } catch (reason) {
          this.cancelDispatchedIds.delete(taskIdToCancel);
          this.publishWorkerError(
            generation,
            controller,
            target.order,
            target.task,
            target.retryInput,
            target.outputKind,
            reason,
          );
        }
      }),
    );
    return this.snapshot();
  }

  async retry(taskId: string): Promise<CanvasGenerationRuntimeSnapshot> {
    this.assertUsable();
    const entry = this.current.entries.find(
      (candidate) => candidate.task.taskId === taskId,
    );
    if (!entry) {
      throw new GenerationError(
        "task_not_found",
        `Unknown canvas generation task ${taskId}`,
        "taskId",
      );
    }
    if (
      entry.requestError &&
      !isTerminalCreativeTaskStatus(entry.task.status)
    ) {
      if (this.activeWorkerTaskIds.has(taskId)) {
        throw new GenerationError(
          "busy",
          `Canvas generation task ${taskId} still has an active worker`,
          "taskId",
        );
      }
      const request: CanvasGenerationResumeRequest = {
        reference: creativeTaskReference(entry.task),
        outputKind: entry.outputKind,
        retryInput: entry.retryInput,
      };
      assertRetryIdentity(request, false);
      const { generation, controller } = this.ensureOperation();
      this.cancelRequestedIds.delete(taskId);
      this.cancelDispatchedIds.delete(taskId);
      this.recoveringRequests.set(taskId, { request, order: entry.order });
      this.update({ recoveringCount: this.current.recoveringCount + 1 });
      await this.resumeWorker(generation, controller, entry.order, request);
      return this.snapshot();
    }
    if (entry.task.status === "succeeded" && entry.requestError) {
      const { controller } = this.ensureOperation();
      let replacement: CanvasGenerationRuntimeEntry;
      try {
        const outputs = committedCanvasOutputs(
          entry.task,
          entry.outputKind,
          this.assets,
        );
        await this.notifySettled(entry.task, controller);
        replacement = { ...entry, outputs, requestError: null };
      } catch (reason) {
        replacement = { ...entry, outputs: [], requestError: asError(reason) };
      }
      this.update({
        entries: this.current.entries.map((candidate) =>
          candidate.task.taskId === taskId ? replacement : candidate,
        ),
      });
      return this.snapshot();
    }
    if (
      entry.requestError &&
      (entry.task.status === "failed" || entry.task.status === "canceled")
    ) {
      const { controller } = this.ensureOperation();
      try {
        await this.notifySettled(entry.task, controller);
        this.update({
          entries: this.current.entries.map((candidate) =>
            candidate.task.taskId === taskId
              ? { ...candidate, requestError: null }
              : candidate,
          ),
        });
      } catch (reason) {
        this.update({
          entries: this.current.entries.map((candidate) =>
            candidate.task.taskId === taskId
              ? { ...candidate, requestError: asError(reason) }
              : candidate,
          ),
        });
      }
      return this.snapshot();
    }
    if (
      (entry.task.status !== "failed" && entry.task.status !== "canceled") ||
      !entry.retryInput
    ) {
      throw new GenerationError(
        "task_not_retryable",
        `Canvas generation task ${taskId} cannot be retried`,
        "taskId",
      );
    }
    const { generation, controller } = this.ensureOperation();
    const nodeKey = this.assertNodeIdle(creativeTaskReference(entry.task), taskId);
    const order = this.nextOrder++;
    const retryInput = {
      ...cloneInput(entry.retryInput),
      idempotencyKey: createCreativeTaskIdempotencyKey(),
    };
    this.taskNodeKeys.set(retryInput.idempotencyKey, nodeKey);
    this.submittingTaskIds.add(retryInput.idempotencyKey);
    this.activeWorkerTaskIds.add(retryInput.idempotencyKey);
    this.cancelRequestedIds.delete(taskId);
    this.cancelDispatchedIds.delete(taskId);
    this.update({ submittingCount: this.current.submittingCount + 1 });
    await this.createWorker(
      generation,
      controller,
      order,
      retryInput,
      entry.outputKind,
    );
    return this.snapshot();
  }

  /** Retry a create request that failed before the backend allocated a task id. */
  async retrySubmission(
    order: number,
  ): Promise<CanvasGenerationRuntimeSnapshot> {
    this.assertUsable();
    const failure = this.current.submissionFailures.find(
      (candidate) => candidate.order === order,
    );
    if (!failure) {
      throw new GenerationError(
        "task_not_found",
        `Unknown canvas generation submission slot ${order}`,
        "order",
      );
    }
    this.assertNodeIdle(pendingReference(failure.input), failure.input.idempotencyKey);
    const { generation, controller } = this.ensureOperation();
    this.submittingTaskIds.add(failure.input.idempotencyKey);
    this.activeWorkerTaskIds.add(failure.input.idempotencyKey);
    const submissionFailures = this.current.submissionFailures.filter(
      (candidate) => candidate.order !== order,
    );
    this.update({
      submissionFailures,
      submittingCount: this.current.submittingCount + 1,
      requestError: submissionFailures.at(-1)?.error ?? null,
    });
    await this.createWorker(
      generation,
      controller,
      order,
      cloneInput(failure.input),
      failure.outputKind,
    );
    return this.snapshot();
  }

  /** Caller must first confirm that this exact submission does not exist on the backend. */
  dismissSubmission(order: number): CanvasGenerationRuntimeSnapshot {
    this.assertUsable();
    const failure = this.current.submissionFailures.find((candidate) => candidate.order === order);
    if (!failure) throw new GenerationError("task_not_found", `Unknown canvas generation submission slot ${order}`, "order");
    if (this.activeWorkerTaskIds.has(failure.input.idempotencyKey)) {
      throw new GenerationError("busy", "Cannot dismiss an active canvas generation submission", "order");
    }
    this.clearSubmissionFailure(order);
    this.recoveryFailureTaskIds.delete(failure.input.idempotencyKey);
    this.cancelRequestedIds.delete(failure.input.idempotencyKey);
    return this.snapshot();
  }

  /** Remove terminal presentation entries after the backend owner index retires them. */
  dismiss(taskIds: readonly string[]): CanvasGenerationRuntimeSnapshot {
    this.assertUsable();
    if (new Set(taskIds).size !== taskIds.length) {
      throw new GenerationError(
        "invalid_parameters",
        "Dismiss task ids must be unique",
        "taskIds",
      );
    }
    const selected = new Set(taskIds);
    const live = this.current.entries.find(
      (entry) =>
        selected.has(entry.task.taskId) &&
        !isTerminalCreativeTaskStatus(entry.task.status),
    );
    if (live) {
      throw new GenerationError(
        "busy",
        `Cannot dismiss live task ${live.task.taskId}`,
        "taskIds",
      );
    }
    if (taskIds.length === 0) return this.snapshot();
    this.update({
      entries: this.current.entries.filter(
        (entry) => !selected.has(entry.task.taskId),
      ),
    });
    return this.snapshot();
  }

  reset(): void {
    this.assertUsable();
    this.assertNotBusy();
    this.operationController?.abort();
    this.operationController = null;
    this.generation += 1;
    this.nextOrder = 0;
    this.submittingTaskIds.clear();
    this.taskNodeKeys.clear();
    this.recoveryFailureTaskIds.clear();
    this.cancelRequestedIds.clear();
    this.cancelDispatchedIds.clear();
    this.terminalTaskIds.clear();
    this.activeWorkerTaskIds.clear();
    this.recoveringRequests.clear();
    this.current = INITIAL_SNAPSHOT;
    const snapshot = this.snapshot();
    for (const listener of this.listeners) listener(snapshot);
  }

  dispose(): void {
    this.operationController?.abort();
    this.operationController = null;
    this.generation += 1;
    this.disposed = true;
    this.nextOrder = 0;
    this.submittingTaskIds.clear();
    this.taskNodeKeys.clear();
    this.recoveryFailureTaskIds.clear();
    this.cancelRequestedIds.clear();
    this.cancelDispatchedIds.clear();
    this.terminalTaskIds.clear();
    this.activeWorkerTaskIds.clear();
    this.recoveringRequests.clear();
    this.current = INITIAL_SNAPSHOT;
    this.listeners.clear();
  }
}
