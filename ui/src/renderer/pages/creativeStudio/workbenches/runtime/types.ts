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
  CreativeTaskInputRole,
  CreativeTaskReference,
} from "../../tasks";

export type CreativeWorkbenchKind = "image" | "video" | "audio";

export interface CreativeWorkbenchReferenceBinding {
  assetId: string;
  kind: CreativeAssetKind;
  role: CreativeTaskInputRole;
}

/**
 * Reference assets must be concrete objects selected from the existing asset
 * port. Asset origin is provenance, not an ownership proof.
 */
export interface CreativeWorkbenchReferences {
  bindings: readonly CreativeWorkbenchReferenceBinding[];
  assets: readonly CreativeAsset[];
}

export interface CreativeWorkbenchTaskOperation<
  TTask extends CreativeCreationModelTask = CreativeCreationModelTask,
  TCapability extends CreativeTaskCapability = CreativeTaskCapability,
> {
  task: TTask;
  capability: TCapability;
}

declare const preparedCreativeWorkbenchRunBrand: unique symbol;

export interface PreparedCreativeWorkbenchRun {
  /** Opaque: only the validated plan builders in this package can mint a run. */
  readonly [preparedCreativeWorkbenchRunBrand]: true;
  kind: CreativeWorkbenchKind;
  input: CreateCreativeTaskInput;
  /** Video can create several real backend tasks; image count remains one backend batch. */
  repeat: number;
  outputKind: Exclude<CreativeAssetKind, "text">;
  model: CreativeModelOption;
  references: readonly CreativeAsset[];
}

export interface CreativeWorkbenchCommittedOutput {
  assetId: string;
  kind: Exclude<CreativeAssetKind, "text">;
  /** Audio uses asset callbacks and intentionally does not require a URL. */
  url: string | null;
  availability?: CreativeAssetAvailability;
}

export interface CreativeWorkbenchRuntimeEntry {
  hasDeletedInputs?: boolean;
  order: number;
  task: CreativeTask;
  outputs: readonly CreativeWorkbenchCommittedOutput[];
  requestError: Error | null;
  /** Exact request retained for a user-requested retry; never mutated or widened. */
  retryInput: CreateCreativeTaskInput | null;
  outputKind: Exclude<CreativeAssetKind, "text">;
}

/** A create request can fail before the backend allocates a task id. */
export interface CreativeWorkbenchSubmissionFailure {
  order: number;
  input: CreateCreativeTaskInput;
  outputKind: Exclude<CreativeAssetKind, "text">;
  error: Error;
}

export type CreativeWorkbenchRuntimeState =
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

export interface CreativeWorkbenchRuntimeSnapshot {
  state: CreativeWorkbenchRuntimeState;
  entries: readonly CreativeWorkbenchRuntimeEntry[];
  /** Retryable request slots; these are deliberately not represented as tasks. */
  submissionFailures: readonly CreativeWorkbenchSubmissionFailure[];
  submittingCount: number;
  recoveringCount: number;
  requestError: Error | null;
}

export interface CreativeWorkbenchResumeRequest {
  reference: CreativeTaskReference;
  outputKind: Exclude<CreativeAssetKind, "text">;
  retryInput?: CreateCreativeTaskInput | null;
  /** Concrete selected assets required when retryInput carries references. */
  retryReferences?: readonly CreativeAsset[];
}

export type CreativeWorkbenchRuntimeErrorCode =
  | "catalog_loading"
  | "catalog_error"
  | "model_required"
  | "model_not_compatible"
  | "task_capability_mismatch"
  | "invalid_parameters"
  | "reference_not_owned"
  | "reference_kind_mismatch"
  | "reference_contract_mismatch"
  | "busy"
  | "task_not_found"
  | "task_not_retryable"
  | "disposed"
  | "presentation_state_unsupported";

export class CreativeWorkbenchRuntimeError extends Error {
  readonly code: CreativeWorkbenchRuntimeErrorCode;
  readonly field: string | null;

  constructor(
    code: CreativeWorkbenchRuntimeErrorCode,
    message: string,
    field: string | null = null,
  ) {
    super(message);
    this.name = "CreativeWorkbenchRuntimeError";
    this.code = code;
    this.field = field;
  }
}
