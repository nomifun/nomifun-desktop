/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset, CreativeAssetKind, CreativeAssetAvailability } from "../../assets";
import type { CreativeModelOption } from "../../models";
import type {
  CreateCreativeTaskInput,
  CreativeCreationModelTask,
  CreativeTask,
  CreativeTaskCapability,
  CreativeTaskReference,
} from "../../tasks";

export type CanvasGenerationKind = "image" | "video" | "audio";
export type CanvasGenerationModelTask = Exclude<CreativeCreationModelTask, "music_generation" | "chat">;

export interface CanvasGenerationTaskOperation<
  TTask extends CreativeCreationModelTask = CreativeCreationModelTask,
  TCapability extends CreativeTaskCapability = CreativeTaskCapability,
> {
  task: TTask;
  capability: TCapability;
}

declare const preparedCanvasGenerationRunBrand: unique symbol;

export interface PreparedCanvasGenerationRun {
  /** Opaque: only the validated plan builders in this package can mint a run. */
  readonly [preparedCanvasGenerationRunBrand]: true;
  kind: CanvasGenerationKind;
  input: CreateCreativeTaskInput & { task: CanvasGenerationModelTask };
  /** Video can create several real backend tasks; image count remains one backend batch. */
  repeat: number;
  outputKind: Exclude<CreativeAssetKind, "text">;
  model: CreativeModelOption;
  references: readonly CreativeAsset[];
}

export interface CanvasGenerationCommittedOutput {
  assetId: string;
  kind: Exclude<CreativeAssetKind, "text">;
  /** Audio uses asset callbacks and intentionally does not require a URL. */
  url: string | null;
  availability?: CreativeAssetAvailability;
}

export interface CanvasGenerationRuntimeEntry {
  hasDeletedInputs?: boolean;
  order: number;
  task: CreativeTask;
  outputs: readonly CanvasGenerationCommittedOutput[];
  requestError: Error | null;
  /** Exact request retained for a user-requested retry; never mutated or widened. */
  retryInput: CreateCreativeTaskInput | null;
  outputKind: Exclude<CreativeAssetKind, "text">;
}

/** A create request can fail before the backend allocates a task id. */
export interface CanvasGenerationSubmissionFailure {
  order: number;
  input: CreateCreativeTaskInput;
  outputKind: Exclude<CreativeAssetKind, "text">;
  error: Error;
}

export type CanvasGenerationRuntimeState =
  | "idle"
  | "submitting"
  | "recovering"
  | "queued"
  | "running"
  | "succeeded"
  | "failed"
  | "canceled"
  | "mixed"
  | "request_error";

export interface CanvasGenerationRuntimeSnapshot {
  state: CanvasGenerationRuntimeState;
  entries: readonly CanvasGenerationRuntimeEntry[];
  /** Retryable request slots; these are deliberately not represented as tasks. */
  submissionFailures: readonly CanvasGenerationSubmissionFailure[];
  submittingCount: number;
  recoveringCount: number;
  requestError: Error | null;
}

export interface CanvasGenerationResumeRequest {
  reference: CreativeTaskReference;
  outputKind: Exclude<CreativeAssetKind, "text">;
  retryInput?: CreateCreativeTaskInput | null;
  /** Concrete selected assets required when retryInput carries references. */
  retryReferences?: readonly CreativeAsset[];
}


export type { GenerationReferences, GenerationReferenceBinding } from "@renderer/creation/references";
export { GenerationError } from "@renderer/creation/generationError";
