/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from "../../assets";
import type { CreativeModelCatalogSnapshot } from "../../models";
import type { CreativeCreationModelTask, CreativeTask } from "../../tasks";
import type {
  AudioWorkbenchProps,
  AudioWorkbenchReference,
  AudioWorkbenchResult,
  AudioWorkbenchTaskSummary,
  AudioWorkbenchValue,
} from "../audio";
import type {
  ImageWorkbenchModelOption,
  ImageWorkbenchProps,
  ImageWorkbenchReference,
  ImageWorkbenchResult,
  ImageWorkbenchTaskSummary,
} from "../image";
import type {
  VideoWorkbenchProps,
  VideoWorkbenchReference,
  VideoWorkbenchTask,
} from "../video";
import { exactWorkbenchModelOptions } from "./catalog";
import type {
  CreativeWorkbenchCommittedOutput,
  CreativeWorkbenchRuntimeEntry,
  CreativeWorkbenchRuntimeSnapshot,
} from "./types";
import { CreativeWorkbenchRuntimeError } from "./types";

type RuntimeAction = () => unknown | Promise<unknown>;

export interface CreativeWorkbenchPresentationFormatters {
  modelLabel(task: CreativeTask): string;
  createdAtLabel(task: CreativeTask): string;
  durationLabel(task: CreativeTask): string | undefined;
}

export interface ImageWorkbenchOutputPresentation {
  alt?: string;
  width?: number;
  height?: number;
  sizeLabel?: string;
}

export interface VideoWorkbenchOutputPresentation {
  posterUrl?: string;
  mediaMetaLabel?: string;
}

export interface AudioWorkbenchOutputPresentation {
  title?: string;
  durationMs?: number;
  sizeBytes?: number;
}

const defaultFormatters: CreativeWorkbenchPresentationFormatters = {
  modelLabel: (task) => `${task.providerId} / ${task.model}`,
  createdAtLabel: (task) => new Date(task.submittedAt).toISOString(),
  durationLabel: (task) => {
    if (task.startedAt === null || task.finishedAt === null) return undefined;
    return `${Math.max(0, task.finishedAt - task.startedAt)} ms`;
  },
};

function invoke(
  action: RuntimeAction,
  onActionError: (error: unknown) => void,
): void {
  try {
    void Promise.resolve(action()).catch(onActionError);
  } catch (error) {
    onActionError(error);
  }
}

function parameterString(task: CreativeTask, key: string): string | undefined {
  const value = task.parameters[key];
  return typeof value === "string" ? value : undefined;
}

function parameterNumber(task: CreativeTask, key: string): number | undefined {
  const value = task.parameters[key];
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function promptOf(task: CreativeTask): string {
  return parameterString(task, "prompt") ?? "";
}

function exactModelLabel(
  catalog: CreativeModelCatalogSnapshot | undefined,
  task: CreativeTask,
  formatters: CreativeWorkbenchPresentationFormatters,
): string {
  const option = catalog
    ? exactWorkbenchModelOptions(catalog, task.task).find(
        (candidate) =>
          candidate.providerId === task.providerId &&
          candidate.model === task.model,
      )
    : undefined;
  return option
    ? `${option.providerName} · ${option.model}`
    : formatters.modelLabel(task);
}

function requireFailedMessage(entry: CreativeWorkbenchRuntimeEntry): string {
  if (entry.task.status !== "failed" || !entry.task.error?.message) {
    throw new CreativeWorkbenchRuntimeError(
      "presentation_state_unsupported",
      `Failed task ${entry.task.taskId} has no authoritative error message`,
      "task.error",
    );
  }
  return entry.task.error.message;
}

function requireSucceededOutputs(
  entry: CreativeWorkbenchRuntimeEntry,
): readonly CreativeWorkbenchCommittedOutput[] {
  if (entry.task.status !== "succeeded" || entry.outputs.length === 0) {
    throw new CreativeWorkbenchRuntimeError(
      "presentation_state_unsupported",
      `Succeeded task ${entry.task.taskId} has no committed presentation outputs`,
      "outputs",
    );
  }
  return entry.outputs;
}

function isDeterministicImageParameterFailure(task: CreativeTask): boolean {
  if (task.status !== "failed") return false;
  const message = task.error?.message ?? "";
  return (
    task.error?.kind === "invalid_params" ||
    /(?:size.*(?:unsupported|not support|不支持)|(?:unsupported|not support|不支持).*size)/i.test(
      message,
    )
  );
}

function requireMediaUrl(output: CreativeWorkbenchCommittedOutput): string {
  if (!output.url) {
    throw new CreativeWorkbenchRuntimeError(
      "presentation_state_unsupported",
      `Committed ${output.kind} asset ${output.assetId} has no resolved URL`,
      "outputs.url",
    );
  }
  return output.url;
}

function singleEntry(
  snapshot: CreativeWorkbenchRuntimeSnapshot,
  kind: "image" | "audio",
): CreativeWorkbenchRuntimeEntry | undefined {
  if (snapshot.entries.length > 1) {
    throw new CreativeWorkbenchRuntimeError(
      "presentation_state_unsupported",
      `${kind} workbench cannot summarize ${snapshot.entries.length} backend tasks`,
      "entries",
    );
  }
  return snapshot.entries[0];
}

function isBusy(snapshot: CreativeWorkbenchRuntimeSnapshot): boolean {
  return (
    snapshot.submittingCount > 0 ||
    snapshot.recoveringCount > 0 ||
    snapshot.entries.some(
      (entry) =>
        entry.task.status === "queued" || entry.task.status === "running",
    )
  );
}

export interface CreativeWorkbenchRequestPresentation {
  state: "idle" | "submitting" | "recovering" | "request_error";
  message: string | null;
  retryableSubmissionOrders: readonly number[];
}

/** Request-layer state rendered by the page shell, never disguised as a backend task. */
export function workbenchRuntimeRequestPresentation(
  snapshot: CreativeWorkbenchRuntimeSnapshot,
): CreativeWorkbenchRequestPresentation {
  if (snapshot.submittingCount > 0) {
    return {
      state: "submitting",
      message: null,
      retryableSubmissionOrders: [],
    };
  }
  if (snapshot.recoveringCount > 0) {
    return {
      state: "recovering",
      message: null,
      retryableSubmissionOrders: [],
    };
  }
  const error =
    snapshot.requestError ??
    snapshot.entries.find((entry) => entry.requestError)?.requestError;
  if (error) {
    return {
      state: "request_error",
      message: error.message,
      retryableSubmissionOrders: snapshot.submissionFailures.map(
        (failure) => failure.order,
      ),
    };
  }
  return { state: "idle", message: null, retryableSubmissionOrders: [] };
}

function taskMessage(
  snapshot: CreativeWorkbenchRuntimeSnapshot,
  entry: CreativeWorkbenchRuntimeEntry | undefined,
): string | undefined {
  if (entry?.requestError) return entry.requestError.message;
  if (snapshot.requestError) return snapshot.requestError.message;
  return undefined;
}

export function imageWorkbenchReferencesFromAssets(
  assets: readonly CreativeAsset[],
): ImageWorkbenchReference[] {
  return assets.map((asset) => {
    if (asset.kind !== "image") {
      throw new CreativeWorkbenchRuntimeError(
        "reference_kind_mismatch",
        `Image workbench reference ${asset.id} is ${asset.kind}`,
        "assets",
      );
    }
    return {
      id: asset.id,
      name: asset.title,
      previewUrl: asset.thumbnailUrl ?? asset.originalUrl,
      originalUrl: asset.originalUrl,
    };
  });
}

export function videoWorkbenchReferencesFromAssets(
  assets: readonly CreativeAsset[],
): VideoWorkbenchReference[] {
  return assets.map((asset) => {
    if (asset.kind === "text") {
      throw new CreativeWorkbenchRuntimeError(
        "reference_kind_mismatch",
        `Video workbench cannot present text reference ${asset.id}`,
        "assets",
      );
    }
    return {
      id: asset.id,
      kind: asset.kind,
      name: asset.title,
      previewUrl: asset.kind === "image"
        ? asset.thumbnailUrl ?? asset.originalUrl
        : asset.kind === "video"
          ? asset.thumbnailUrl ?? undefined
          : undefined,
      originalUrl: asset.kind === "audio" ? undefined : asset.originalUrl,
    };
  });
}

export function audioWorkbenchReferencesFromAssets(
  assets: readonly CreativeAsset[],
): AudioWorkbenchReference[] {
  return assets.map((asset) => {
    if (asset.kind !== "audio") {
      throw new CreativeWorkbenchRuntimeError(
        "reference_kind_mismatch",
        `Audio workbench reference ${asset.id} is ${asset.kind}`,
        "assets",
      );
    }
    return {
      assetId: asset.id,
      name: asset.title,
      mimeType: asset.mimeType ?? undefined,
      sizeBytes: asset.bytes ?? undefined,
    };
  });
}

export function imageWorkbenchModelOptions(
  catalog: CreativeModelCatalogSnapshot,
  task: Extract<CreativeCreationModelTask, "image_generation" | "image_edit">,
): ImageWorkbenchModelOption[] {
  return exactWorkbenchModelOptions(catalog, task).map((option) => ({
    providerId: option.providerId,
    model: option.model,
    label: option.displayName ?? option.model,
    ...(option.rawModelId ? { rawModelId: option.rawModelId } : {}),
    providerLabel: option.providerName,
    platform: option.platform,
    protocol: option.protocol,
  }));
}

export interface MapImageWorkbenchRuntimeOptions {
  catalog?: CreativeModelCatalogSnapshot;
  formatters?: CreativeWorkbenchPresentationFormatters;
  outputPresentation?: (
    output: CreativeWorkbenchCommittedOutput,
    task: CreativeTask,
  ) => ImageWorkbenchOutputPresentation;
}

export function mapImageWorkbenchRuntimeResults(
  snapshot: CreativeWorkbenchRuntimeSnapshot,
  options: MapImageWorkbenchRuntimeOptions = {},
): ImageWorkbenchResult[] {
  const formatters = options.formatters ?? defaultFormatters;
  return snapshot.entries.map((entry): ImageWorkbenchResult => {
    const task = entry.task;
    const base = {
      ...(entry.hasDeletedInputs !== undefined ? { hasDeletedInputs: entry.hasDeletedInputs } : {}),
      taskId: task.taskId,
      prompt: promptOf(task),
      model: { providerId: task.providerId, model: task.model },
      modelLabel: exactModelLabel(options.catalog, task, formatters),
      createdAtLabel: formatters.createdAtLabel(task),
      durationLabel: formatters.durationLabel(task),
      retryable:
        !entry.hasDeletedInputs &&
        !isDeterministicImageParameterFailure(task) &&
        task.inputs !== null &&
        (!options.catalog ||
          exactWorkbenchModelOptions(options.catalog, task.task).some(
            (option) =>
              option.providerId === task.providerId && option.model === task.model,
          )),
      deletable:
        task.status === "succeeded" ||
        task.status === "failed" ||
        task.status === "canceled",
    };
    if (task.status === "succeeded") {
      const outputs = requireSucceededOutputs(entry).map((output) => {
        const presentation = options.outputPresentation?.(output, task) ?? {};
        return {
          assetId: output.assetId,
          imageUrl: requireMediaUrl(output),
          ...(output.availability ? { availability: output.availability } : {}),
          alt: presentation.alt ?? promptOf(task),
          width: presentation.width,
          height: presentation.height,
          sizeLabel: presentation.sizeLabel,
        };
      });
      return {
        ...base,
        id: task.taskId,
        status: "succeeded",
        outputs,
      };
    }
    if (task.status === "failed") {
      return {
        ...base,
        id: task.taskId,
        status: "failed",
        errorMessage: requireFailedMessage(entry),
        errorDetail: entry.requestError?.message,
      };
    }
    if (task.status === "canceled") {
      return { ...base, id: task.taskId, status: "canceled" };
    }
    return { ...base, id: task.taskId, status: task.status };
  });
}

export function imageWorkbenchTaskSummary(
  snapshot: CreativeWorkbenchRuntimeSnapshot,
): ImageWorkbenchTaskSummary {
  const entry = snapshot.entries.find(
    (candidate) =>
      candidate.task.status === "queued" || candidate.task.status === "running",
  );
  const activeCount = snapshot.entries.filter(
    (candidate) =>
      candidate.task.status === "queued" || candidate.task.status === "running",
  ).length;
  return {
    state: entry?.task.status ?? "idle",
    pendingCount:
      snapshot.submittingCount +
      Math.max(snapshot.recoveringCount, activeCount),
    message: taskMessage(snapshot, entry),
  };
}

export interface CreateImageWorkbenchRuntimePropsInput {
  base: Omit<
    ImageWorkbenchProps,
    | "modelOptions"
    | "results"
    | "task"
    | "disabled"
    | "onGenerate"
    | "onRetryResult"
  >;
  runtime: CreativeWorkbenchRuntimeSnapshot;
  catalog: CreativeModelCatalogSnapshot;
  task: Extract<CreativeCreationModelTask, "image_generation" | "image_edit">;
  disabled?: boolean;
  onGenerate: RuntimeAction;
  onRetryTask?: (taskId: string) => unknown | Promise<unknown>;
  onActionError(error: unknown): void;
  presentation?: MapImageWorkbenchRuntimeOptions;
}

export function createImageWorkbenchRuntimeProps(
  input: CreateImageWorkbenchRuntimePropsInput,
): ImageWorkbenchProps {
  const results = mapImageWorkbenchRuntimeResults(input.runtime, {
    ...input.presentation,
    catalog: input.catalog,
  });
  const taskIdsByResultId = new Map(
    results.map((result) => [result.id, result.taskId]),
  );
  return {
    ...input.base,
    modelOptions: imageWorkbenchModelOptions(input.catalog, input.task),
    results,
    task: imageWorkbenchTaskSummary(input.runtime),
    disabled: Boolean(input.disabled) || isBusy(input.runtime),
    onGenerate: () => invoke(input.onGenerate, input.onActionError),
    onRetryResult: input.onRetryTask
      ? (resultId) => {
          const taskId = taskIdsByResultId.get(resultId);
          if (!taskId) {
            input.onActionError(
              new CreativeWorkbenchRuntimeError(
                "task_not_found",
                `No runtime task owns image result ${resultId}`,
                "resultId",
              ),
            );
            return;
          }
          invoke(() => input.onRetryTask?.(taskId), input.onActionError);
        }
      : undefined,
  };
}

function videoLabels(task: CreativeTask): {
  resolutionLabel: string;
  sizeLabel: string;
  durationLabel: string;
} {
  const width = parameterNumber(task, "width");
  const height = parameterNumber(task, "height");
  const resolution = parameterString(task, "resolution");
  const aspect = parameterString(task, "aspect");
  const seconds = parameterNumber(task, "seconds");
  const divisor = (left: number, right: number): number => {
    let a = Math.abs(Math.round(left));
    let b = Math.abs(Math.round(right));
    while (b > 0) [a, b] = [b, a % b];
    return a || 1;
  };
  const derivedAspect =
    width !== undefined && height !== undefined
      ? `${width / divisor(width, height)}:${height / divisor(width, height)}`
      : "";
  return {
    resolutionLabel:
      resolution ??
      (width !== undefined && height !== undefined ? `${width}×${height}` : ""),
    sizeLabel: aspect ?? derivedAspect,
    durationLabel: seconds === undefined ? "" : `${seconds}s`,
  };
}

export interface MapVideoWorkbenchRuntimeOptions {
  catalog?: CreativeModelCatalogSnapshot;
  formatters?: CreativeWorkbenchPresentationFormatters;
  outputPresentation?: (
    output: CreativeWorkbenchCommittedOutput,
    task: CreativeTask,
  ) => VideoWorkbenchOutputPresentation;
}

export function mapVideoWorkbenchRuntimeTasks(
  snapshot: CreativeWorkbenchRuntimeSnapshot,
  options: MapVideoWorkbenchRuntimeOptions = {},
): VideoWorkbenchTask[] {
  const formatters = options.formatters ?? defaultFormatters;
  return snapshot.entries.map((entry): VideoWorkbenchTask => {
    const task = entry.task;
    const labels = videoLabels(task);
    const base = {
      ...(entry.hasDeletedInputs !== undefined ? { hasDeletedInputs: entry.hasDeletedInputs } : {}),
      id: task.taskId,
      taskId: task.taskId,
      prompt: promptOf(task),
      createdAtLabel: formatters.createdAtLabel(task),
      model: { providerId: task.providerId, model: task.model },
      modelLabel: exactModelLabel(options.catalog, task, formatters),
      ...labels,
      taskCount: 1,
      retryable:
        !entry.hasDeletedInputs &&
        task.inputs !== null &&
        (!options.catalog ||
          exactWorkbenchModelOptions(options.catalog, task.task).some(
            (option) =>
              option.providerId === task.providerId && option.model === task.model,
          )),
      deletable:
        task.status === "succeeded" ||
        task.status === "failed" ||
        task.status === "canceled",
    };
    if (task.status === "succeeded") {
      const outputs = requireSucceededOutputs(entry);
      if (outputs.length !== 1) {
        throw new CreativeWorkbenchRuntimeError(
          "presentation_state_unsupported",
          `Video task ${task.taskId} returned ${outputs.length} assets; the task card requires one`,
          "outputs",
        );
      }
      const output = outputs[0];
      if (!output) throw new Error("Unreachable empty video output");
      const presentation = options.outputPresentation?.(output, task) ?? {};
      return {
        ...base,
        status: "succeeded",
        assetId: output.assetId,
        videoUrl: requireMediaUrl(output),
        ...(output.availability ? { availability: output.availability } : {}),
        posterUrl: presentation.posterUrl,
        mediaMetaLabel: presentation.mediaMetaLabel,
      };
    }
    if (task.status === "failed") {
      return {
        ...base,
        status: "failed",
        error: requireFailedMessage(entry),
        errorDetail: entry.requestError?.message,
      };
    }
    if (task.status === "canceled") return { ...base, status: "canceled" };
    return { ...base, status: task.status };
  });
}

export interface CreateVideoWorkbenchRuntimePropsInput {
  base: Omit<
    VideoWorkbenchProps,
    "tasks" | "generating" | "onGenerate" | "onRetryTask"
  >;
  runtime: CreativeWorkbenchRuntimeSnapshot;
  catalog?: CreativeModelCatalogSnapshot;
  onGenerate: RuntimeAction;
  onRetryTask?: (taskId: string) => unknown | Promise<unknown>;
  onActionError(error: unknown): void;
  presentation?: MapVideoWorkbenchRuntimeOptions;
}

export function createVideoWorkbenchRuntimeProps(
  input: CreateVideoWorkbenchRuntimePropsInput,
): VideoWorkbenchProps {
  const busy = isBusy(input.runtime);
  return {
    ...input.base,
    tasks: mapVideoWorkbenchRuntimeTasks(input.runtime, {
      ...input.presentation,
      catalog: input.catalog,
    }),
    generating: busy,
    submitDisabled: Boolean(input.base.submitDisabled) || busy,
    onGenerate: () => invoke(input.onGenerate, input.onActionError),
    onRetryTask: input.onRetryTask
      ? (taskId) =>
          invoke(() => input.onRetryTask?.(taskId), input.onActionError)
      : undefined,
  };
}

export interface MapAudioWorkbenchRuntimeOptions {
  catalog?: CreativeModelCatalogSnapshot;
  formatters?: CreativeWorkbenchPresentationFormatters;
  outputPresentation?: (
    output: CreativeWorkbenchCommittedOutput,
    task: CreativeTask,
  ) => AudioWorkbenchOutputPresentation;
}

export function mapAudioWorkbenchRuntimeResults(
  snapshot: CreativeWorkbenchRuntimeSnapshot,
  options: MapAudioWorkbenchRuntimeOptions = {},
): AudioWorkbenchResult[] {
  const formatters = options.formatters ?? defaultFormatters;
  return snapshot.entries.map((entry): AudioWorkbenchResult => {
    const task = entry.task;
    const text = promptOf(task);
    const base = {
      ...(entry.hasDeletedInputs !== undefined ? { hasDeletedInputs: entry.hasDeletedInputs } : {}),
      taskId: task.taskId,
      title: text || task.taskId,
      text,
      modelLabel: exactModelLabel(options.catalog, task, formatters),
      formatLabel: parameterString(task, "format"),
      createdAtLabel: formatters.createdAtLabel(task),
    };
    if (task.status === "succeeded") {
      const outputs = requireSucceededOutputs(entry);
      if (outputs.length !== 1) {
        throw new CreativeWorkbenchRuntimeError(
          "presentation_state_unsupported",
          `Audio task ${task.taskId} returned ${outputs.length} assets; the result row requires one`,
          "outputs",
        );
      }
      const output = outputs[0];
      if (!output) throw new Error("Unreachable empty audio output");
      const presentation = options.outputPresentation?.(output, task) ?? {};
      return {
        ...base,
        id: output.assetId,
        title: presentation.title ?? base.title,
        status: "succeeded",
        assetId: output.assetId,
        durationMs: presentation.durationMs,
        sizeBytes: presentation.sizeBytes,
      };
    }
    if (task.status === "failed") {
      return {
        ...base,
        id: task.taskId,
        status: "failed",
        errorMessage: requireFailedMessage(entry),
      };
    }
    if (task.status === "canceled") {
      return { ...base, id: task.taskId, status: "canceled" };
    }
    return { ...base, id: task.taskId, status: task.status };
  });
}

export function audioWorkbenchTaskSummary(
  snapshot: CreativeWorkbenchRuntimeSnapshot,
): AudioWorkbenchTaskSummary {
  const entry = singleEntry(snapshot, "audio");
  return {
    state: entry?.task.status ?? "idle",
    taskId: entry?.task.taskId,
    message: taskMessage(snapshot, entry),
    errorMessage:
      entry?.task.status === "failed" ? requireFailedMessage(entry) : undefined,
  };
}

export interface CreateAudioWorkbenchRuntimePropsInput {
  base: Omit<
    AudioWorkbenchProps,
    | "results"
    | "task"
    | "disabled"
    | "onGenerate"
    | "onCancel"
    | "onRetry"
    | "onRetryResult"
  >;
  runtime: CreativeWorkbenchRuntimeSnapshot;
  catalog?: CreativeModelCatalogSnapshot;
  disabled?: boolean;
  onGenerate(value: AudioWorkbenchValue): unknown | Promise<unknown>;
  onCancel?: RuntimeAction;
  onRetryTask?: (taskId: string) => unknown | Promise<unknown>;
  onActionError(error: unknown): void;
  presentation?: MapAudioWorkbenchRuntimeOptions;
}

export function createAudioWorkbenchRuntimeProps(
  input: CreateAudioWorkbenchRuntimePropsInput,
): AudioWorkbenchProps {
  const results = mapAudioWorkbenchRuntimeResults(input.runtime, {
    ...input.presentation,
    catalog: input.catalog,
  });
  const entry = singleEntry(input.runtime, "audio");
  const retryTask =
    entry &&
    (entry.task.status === "failed" || entry.task.status === "canceled")
      ? entry.task.taskId
      : undefined;
  return {
    ...input.base,
    results,
    task: audioWorkbenchTaskSummary(input.runtime),
    disabled:
      Boolean(input.disabled) ||
      input.runtime.submittingCount > 0 ||
      input.runtime.recoveringCount > 0,
    onGenerate: (value) =>
      invoke(() => input.onGenerate(value), input.onActionError),
    onCancel: input.onCancel
      ? () => invoke(input.onCancel as RuntimeAction, input.onActionError)
      : undefined,
    onRetry:
      input.onRetryTask && retryTask
        ? () =>
            invoke(() => input.onRetryTask?.(retryTask), input.onActionError)
        : undefined,
    onRetryResult: input.onRetryTask
      ? (result) =>
          invoke(() => input.onRetryTask?.(result.taskId), input.onActionError)
      : undefined,
  };
}
