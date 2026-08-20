/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset, CreativeAssetPort } from "../../assets";
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
import { committedWorkbenchOutputs } from "./assets";
import { isPreparedCreativeWorkbenchRun } from "./plans";
import type {
  CreativeWorkbenchResumeRequest,
  CreativeWorkbenchRuntimeEntry,
  CreativeWorkbenchRuntimeSnapshot,
  PreparedCreativeWorkbenchRun,
} from "./types";
import { CreativeWorkbenchRuntimeError } from "./types";

export interface CreativeWorkbenchRuntimeControllerOptions {
  poll?: Omit<CreativeTaskPollOptions, "signal" | "onTask">;
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

export type CreativeWorkbenchRuntimeListener = (
  snapshot: CreativeWorkbenchRuntimeSnapshot,
) => void;

const INITIAL_SNAPSHOT: CreativeWorkbenchRuntimeSnapshot = {
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
  entries: readonly CreativeWorkbenchRuntimeEntry[],
  submittingCount: number,
  recoveringCount: number,
  requestError: Error | null,
): CreativeWorkbenchRuntimeSnapshot["state"] {
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
  throw new CreativeWorkbenchRuntimeError(
    "task_capability_mismatch",
    `Workbench runtime cannot project outputs for ${input.task}`,
    "task",
  );
}

function assertPreparedRun(plan: PreparedCreativeWorkbenchRun): void {
  if (!isPreparedCreativeWorkbenchRun(plan)) {
    throw new CreativeWorkbenchRuntimeError(
      "invalid_parameters",
      "Workbench runs must be created by a validated runtime plan builder",
      "plan",
    );
  }
  assertTaskCapabilityPair(plan.input.task, plan.input.capability);
  const expectedKind = expectedOutputKind(plan.input);
  if (plan.outputKind !== expectedKind || plan.kind !== expectedKind) {
    throw new CreativeWorkbenchRuntimeError(
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
    throw new CreativeWorkbenchRuntimeError(
      "invalid_parameters",
      "Workbench repeat must be an integer between 1 and 6",
      "repeat",
    );
  }
  if (
    plan.model.providerId !== plan.input.providerId ||
    plan.model.model !== plan.input.model ||
    plan.model.task !== plan.input.task
  ) {
    throw new CreativeWorkbenchRuntimeError(
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
  if (new Set(assetIds).size !== assetIds.length) {
    throw new CreativeWorkbenchRuntimeError(
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
    throw new CreativeWorkbenchRuntimeError(
      "reference_contract_mismatch",
      `${field} does not exactly match task inputs`,
      field,
    );
  }
}

function assertRetryIdentity(
  request: CreativeWorkbenchResumeRequest,
  requireAssetProof = true,
): void {
  const input = request.retryInput;
  if (!input) return;
  if (!sameCreativeTaskOwner(input.owner, request.reference.owner)) {
    throw new CreativeWorkbenchRuntimeError(
      "model_not_compatible",
      "Resume retryInput owner does not match its task reference",
      "retryInput.owner",
    );
  }
  for (const field of ["providerId", "model", "task", "capability"] as const) {
    if (input[field] !== request.reference[field]) {
      throw new CreativeWorkbenchRuntimeError(
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
export class CreativeWorkbenchRuntimeController {
  private readonly listeners = new Set<CreativeWorkbenchRuntimeListener>();
  private readonly pollOptions: Omit<
    CreativeTaskPollOptions,
    "signal" | "onTask"
  >;
  private readonly onPendingTask: CreativeWorkbenchRuntimeControllerOptions["onPendingTask"];
  private readonly onSettledTask: CreativeWorkbenchRuntimeControllerOptions["onSettledTask"];
  private readonly onRecoveryFailure: CreativeWorkbenchRuntimeControllerOptions["onRecoveryFailure"];
  private current: CreativeWorkbenchRuntimeSnapshot = INITIAL_SNAPSHOT;
  private generation = 0;
  private operationController: AbortController | null = null;
  private cancelAllRequested = false;
  private readonly cancelRequestedIds = new Set<string>();
  private readonly cancelDispatchedIds = new Set<string>();
  private readonly terminalTaskIds = new Set<string>();
  private readonly activeWorkerTaskIds = new Set<string>();
  private readonly recoveringRequests = new Map<
    string,
    { request: CreativeWorkbenchResumeRequest; order: number }
  >();
  private disposed = false;

  constructor(
    private readonly tasks: CreativeTaskPort,
    private readonly assets: CreativeAssetPort,
    options: CreativeWorkbenchRuntimeControllerOptions = {},
  ) {
    this.pollOptions = options.poll ?? {};
    this.onPendingTask = options.onPendingTask;
    this.onSettledTask = options.onSettledTask;
    this.onRecoveryFailure = options.onRecoveryFailure;
  }

  snapshot(): CreativeWorkbenchRuntimeSnapshot {
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

  subscribe(listener: CreativeWorkbenchRuntimeListener): () => void {
    // React StrictMode intentionally performs an effect cleanup/setup cycle.
    this.disposed = false;
    this.listeners.add(listener);
    listener(this.snapshot());
    return () => this.listeners.delete(listener);
  }

  private emit(next: Omit<CreativeWorkbenchRuntimeSnapshot, "state">): void {
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
    patch: Partial<Omit<CreativeWorkbenchRuntimeSnapshot, "state">>,
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

  private begin(
    submittingCount: number,
    recoveringCount: number,
  ): {
    generation: number;
    controller: AbortController;
  } {
    this.operationController?.abort();
    const controller = new AbortController();
    this.operationController = controller;
    this.cancelAllRequested = false;
    this.cancelRequestedIds.clear();
    this.cancelDispatchedIds.clear();
    this.terminalTaskIds.clear();
    this.activeWorkerTaskIds.clear();
    this.recoveringRequests.clear();
    this.generation += 1;
    this.emit({
      entries: [],
      submissionFailures: [],
      submittingCount,
      recoveringCount,
      requestError: null,
    });
    return { generation: this.generation, controller };
  }

  private upsert(entry: CreativeWorkbenchRuntimeEntry): void {
    const existing = this.current.entries.find(
      (candidate) => candidate.task.taskId === entry.task.taskId,
    );
    if (existing && existing.order !== entry.order) {
      throw new CreativeWorkbenchRuntimeError(
        "invalid_parameters",
        `Backend reused task id ${entry.task.taskId} for two workbench slots`,
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
      outputs: committedWorkbenchOutputs(task, outputKind, this.assets),
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

  private async notifySettled(
    task: CreativeTask,
    controller: AbortController,
  ): Promise<void> {
    if (!isTerminalCreativeTaskStatus(task.status) || !this.onSettledTask)
      return;
    await this.onSettledTask(cloneTask(task), controller.signal);
  }

  private async createWorker(
    generation: number,
    controller: AbortController,
    order: number,
    input: CreateCreativeTaskInput,
    outputKind: "image" | "video" | "audio",
  ): Promise<void> {
    let task: CreativeTask | null = null;
    let submissionReleased = false;
    try {
      await this.onPendingTask?.(pendingReference(input), controller.signal);
      task = await this.tasks.create(cloneInput(input), controller.signal);
      if (!this.isCurrent(generation, controller)) return;
      this.activeWorkerTaskIds.add(task.taskId);
      const previousFailure = this.current.submissionFailures.find(
        (failure) => failure.order === order,
      );
      if (previousFailure) {
        const submissionFailures = this.current.submissionFailures.filter(
          (failure) => failure.order !== order,
        );
        this.update({
          submissionFailures,
          requestError:
            this.current.requestError === previousFailure.error
              ? (submissionFailures.at(-1)?.error ?? null)
              : this.current.requestError,
        });
      }
      this.publishTask(generation, controller, order, task, input, outputKind);
      this.decrement("submittingCount");
      submissionReleased = true;
      if (isTerminalCreativeTaskStatus(task.status)) {
        await this.notifySettled(task, controller);
        return;
      }
      if (
        (this.cancelAllRequested || this.cancelRequestedIds.has(task.taskId)) &&
        !this.cancelDispatchedIds.has(task.taskId)
      ) {
        this.cancelDispatchedIds.add(task.taskId);
        task = await this.tasks.cancel(
          creativeTaskReference(task),
          controller.signal,
        );
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
      if (task) this.activeWorkerTaskIds.delete(task.taskId);
    }
  }

  private assertWorkersIdle(): void {
    if (
      this.current.submittingCount > 0 ||
      this.current.recoveringCount > 0 ||
      this.activeWorkerTaskIds.size > 0
    ) {
      throw new CreativeWorkbenchRuntimeError(
        "busy",
        "A workbench task is already active",
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
      throw new CreativeWorkbenchRuntimeError(
        "busy",
        "A workbench request has an unresolved outcome",
      );
    }
  }

  private assertUsable(): void {
    if (this.disposed) {
      throw new CreativeWorkbenchRuntimeError(
        "disposed",
        "This workbench runtime controller has been disposed",
      );
    }
  }

  private supplementalOperation(): {
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
  ): Promise<CreativeWorkbenchRuntimeSnapshot> {
    this.assertUsable();
    this.assertNotBusy();
    const { generation, controller } = this.begin(repeat, 0);
    const cloned = cloneInput(input);
    await Promise.all(
      Array.from({ length: repeat }, (_, order) =>
        this.createWorker(
          generation,
          controller,
          order,
          order === 0
            ? cloned
            : {
                ...cloneInput(cloned),
                idempotencyKey: createCreativeTaskIdempotencyKey(),
              },
          outputKind,
        ),
      ),
    );
    return this.snapshot();
  }

  async run(
    plan: PreparedCreativeWorkbenchRun,
  ): Promise<CreativeWorkbenchRuntimeSnapshot> {
    this.assertUsable();
    assertPreparedRun(plan);
    return this.runInput(plan.input, plan.repeat, plan.outputKind);
  }

  private async resumeWorker(
    generation: number,
    controller: AbortController,
    order: number,
    request: CreativeWorkbenchResumeRequest,
  ): Promise<void> {
    let task: CreativeTask | null = null;
    let firstResponse = true;
    const releaseRecovery = (): void => {
      if (this.recoveringRequests.delete(request.reference.taskId)) {
        this.decrement("recoveringCount");
      }
      firstResponse = false;
    };
    try {
      this.activeWorkerTaskIds.add(request.reference.taskId);
      task = await this.tasks.get(request.reference, controller.signal);
      assertCreativeTaskReference(task, request.reference);
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
        (this.cancelAllRequested || this.cancelRequestedIds.has(task.taskId)) &&
        !this.cancelDispatchedIds.has(task.taskId)
      ) {
        this.cancelDispatchedIds.add(task.taskId);
        task = await this.tasks.cancel(request.reference, controller.signal);
        assertCreativeTaskReference(task, request.reference);
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
      this.activeWorkerTaskIds.delete(request.reference.taskId);
      if (this.isCurrent(generation, controller) && firstResponse)
        releaseRecovery();
    }
  }

  async resume(
    requests: readonly CreativeWorkbenchResumeRequest[],
  ): Promise<CreativeWorkbenchRuntimeSnapshot> {
    this.assertUsable();
    if (requests.length === 0) return this.snapshot();
    this.assertWorkersIdle();
    const ids = requests.map((request) => request.reference.taskId);
    if (new Set(ids).size !== ids.length) {
      throw new CreativeWorkbenchRuntimeError(
        "invalid_parameters",
        "Resume requests contain duplicate task ids",
        "requests",
      );
    }
    for (const request of requests) {
      assertTaskCapabilityPair(
        request.reference.task,
        request.reference.capability,
      );
      if (expectedOutputKind(request.reference) !== request.outputKind) {
        throw new CreativeWorkbenchRuntimeError(
          "task_capability_mismatch",
          `Resume task ${request.reference.taskId} cannot project ${request.outputKind} output`,
          "outputKind",
        );
      }
      assertRetryIdentity(request);
    }
    const { generation, controller } = this.begin(0, requests.length);
    requests.forEach((request, order) => {
      this.recoveringRequests.set(request.reference.taskId, { request, order });
    });
    await Promise.all(
      requests.map((request, order) =>
        this.resumeWorker(generation, controller, order, request),
      ),
    );
    return this.snapshot();
  }

  async cancel(taskId?: string): Promise<CreativeWorkbenchRuntimeSnapshot> {
    this.assertUsable();
    if (taskId) this.cancelRequestedIds.add(taskId);
    else this.cancelAllRequested = true;
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
      throw new CreativeWorkbenchRuntimeError(
        "task_not_found",
        `No active workbench task ${taskId}`,
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

  async retry(taskId: string): Promise<CreativeWorkbenchRuntimeSnapshot> {
    this.assertUsable();
    const entry = this.current.entries.find(
      (candidate) => candidate.task.taskId === taskId,
    );
    if (!entry) {
      throw new CreativeWorkbenchRuntimeError(
        "task_not_found",
        `Unknown workbench task ${taskId}`,
        "taskId",
      );
    }
    if (
      entry.requestError &&
      !isTerminalCreativeTaskStatus(entry.task.status)
    ) {
      if (this.activeWorkerTaskIds.has(taskId)) {
        throw new CreativeWorkbenchRuntimeError(
          "busy",
          `Workbench task ${taskId} still has an active worker`,
          "taskId",
        );
      }
      const request: CreativeWorkbenchResumeRequest = {
        reference: creativeTaskReference(entry.task),
        outputKind: entry.outputKind,
        retryInput: entry.retryInput,
      };
      assertRetryIdentity(request, false);
      const { generation, controller } = this.supplementalOperation();
      this.cancelAllRequested = false;
      this.cancelRequestedIds.delete(taskId);
      this.cancelDispatchedIds.delete(taskId);
      this.recoveringRequests.set(taskId, { request, order: entry.order });
      this.update({ recoveringCount: this.current.recoveringCount + 1 });
      await this.resumeWorker(generation, controller, entry.order, request);
      return this.snapshot();
    }
    if (entry.task.status === "succeeded" && entry.requestError) {
      const { controller } = this.supplementalOperation();
      let replacement: CreativeWorkbenchRuntimeEntry;
      try {
        const outputs = committedWorkbenchOutputs(
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
      const { controller } = this.supplementalOperation();
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
      throw new CreativeWorkbenchRuntimeError(
        "task_not_retryable",
        `Workbench task ${taskId} cannot be retried`,
        "taskId",
      );
    }
    const { generation, controller } = this.supplementalOperation();
    const order =
      this.current.entries.reduce(
        (maximum, candidate) => Math.max(maximum, candidate.order),
        -1,
      ) + 1;
    const retryInput = {
      ...cloneInput(entry.retryInput),
      idempotencyKey: createCreativeTaskIdempotencyKey(),
    };
    this.cancelAllRequested = false;
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
  ): Promise<CreativeWorkbenchRuntimeSnapshot> {
    this.assertUsable();
    const failure = this.current.submissionFailures.find(
      (candidate) => candidate.order === order,
    );
    if (!failure) {
      throw new CreativeWorkbenchRuntimeError(
        "task_not_found",
        `Unknown workbench submission slot ${order}`,
        "order",
      );
    }
    const { generation, controller } = this.supplementalOperation();
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

  reset(): void {
    this.assertUsable();
    this.assertNotBusy();
    this.operationController?.abort();
    this.operationController = null;
    this.generation += 1;
    this.cancelAllRequested = false;
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
    this.cancelRequestedIds.clear();
    this.cancelDispatchedIds.clear();
    this.terminalTaskIds.clear();
    this.activeWorkerTaskIds.clear();
    this.recoveringRequests.clear();
    this.current = INITIAL_SNAPSHOT;
    this.listeners.clear();
  }
}
