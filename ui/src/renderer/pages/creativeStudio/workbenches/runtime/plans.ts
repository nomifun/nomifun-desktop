/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeJsonObject } from "../../domain/schema";
import type { CreativeAsset } from "../../assets";
import type { CreativeModelCatalogSnapshot } from "../../models";
import {
  assertTaskCapabilityPair,
  createCreativeTaskIdempotencyKey,
} from "../../tasks";
import type { CreativeTaskInput, CreativeTaskInputRole } from "../../tasks";
import type { AudioWorkbenchFieldSupport, AudioWorkbenchValue } from "../audio";
import type {
  ImageWorkbenchInterfaceMode,
  ImageWorkbenchQuality,
} from "../image";
import { resolveExactWorkbenchModel } from "./catalog";
import type { CreativeWorkbenchModelSelection } from "./catalog";
import { validateWorkbenchReferences } from "./assets";
import type {
  CreativeWorkbenchReferences,
  CreativeWorkbenchTaskOperation,
  PreparedCreativeWorkbenchRun,
} from "./types";
import { CreativeWorkbenchRuntimeError } from "./types";

const preparedRuns = new WeakSet<object>();

function markPreparedRun(
  run: Omit<PreparedCreativeWorkbenchRun, symbol>,
): PreparedCreativeWorkbenchRun {
  preparedRuns.add(run);
  return run as PreparedCreativeWorkbenchRun;
}

/** Runtime counterpart to the opaque PreparedCreativeWorkbenchRun type. */
export function isPreparedCreativeWorkbenchRun(
  value: PreparedCreativeWorkbenchRun,
): boolean {
  return typeof value === "object" && value !== null && preparedRuns.has(value);
}

interface WorkbenchPlanBase {
  catalog: CreativeModelCatalogSnapshot;
  projectId: string;
  nodeId: string;
  model: CreativeWorkbenchModelSelection | null;
  references: CreativeWorkbenchReferences;
  extraParameters?: CreativeJsonObject;
}

export type ImageWorkbenchOperation =
  | CreativeWorkbenchTaskOperation<"image_generation", "t2i">
  | CreativeWorkbenchTaskOperation<"image_edit", "i2i" | "inpaint">;

export interface PrepareImageWorkbenchRunInput extends WorkbenchPlanBase {
  operation: ImageWorkbenchOperation;
  prompt: string;
  interfaceMode: ImageWorkbenchInterfaceMode;
  quality: ImageWorkbenchQuality;
  width: number | null;
  height: number | null;
  aspectRatio: string;
  count: number;
}

export type VideoWorkbenchOperation = CreativeWorkbenchTaskOperation<
  "video_generation",
  "t2v" | "i2v" | "v2v"
>;

export interface PrepareVideoWorkbenchRunInput extends WorkbenchPlanBase {
  operation: VideoWorkbenchOperation;
  prompt: string;
  resolution: string;
  aspectRatio: string;
  seconds: number;
  width: number | null;
  height: number | null;
  taskCount: number;
}

export interface PrepareAudioWorkbenchRunInput extends WorkbenchPlanBase {
  value: AudioWorkbenchValue;
  fieldSupport: AudioWorkbenchFieldSupport;
  maxTextLength?: number;
}

function requirePrompt(
  value: string,
  field: string,
  maxLength?: number,
): string {
  if (!value.trim()) {
    throw new CreativeWorkbenchRuntimeError(
      "invalid_parameters",
      `${field} must not be blank`,
      field,
    );
  }
  if (maxLength !== undefined && Array.from(value).length > maxLength) {
    throw new CreativeWorkbenchRuntimeError(
      "invalid_parameters",
      `${field} exceeds the ${maxLength} character limit`,
      field,
    );
  }
  return value;
}

function requireInteger(
  value: number,
  field: string,
  minimum: number,
  maximum: number,
): number {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new CreativeWorkbenchRuntimeError(
      "invalid_parameters",
      `${field} must be an integer between ${minimum} and ${maximum}`,
      field,
    );
  }
  return value;
}

function requireFiniteNumber(
  value: number,
  field: string,
  minimum: number,
  maximum: number,
): number {
  if (!Number.isFinite(value) || value < minimum || value > maximum) {
    throw new CreativeWorkbenchRuntimeError(
      "invalid_parameters",
      `${field} must be between ${minimum} and ${maximum}`,
      field,
    );
  }
  return value;
}

function dimensions(
  width: number | null,
  height: number | null,
  maximum: number,
): CreativeJsonObject {
  if ((width === null) !== (height === null)) {
    throw new CreativeWorkbenchRuntimeError(
      "invalid_parameters",
      "width and height must both be set or both be automatic",
      "dimensions",
    );
  }
  if (width === null || height === null) return {};
  return {
    width: requireInteger(width, "width", 1, maximum),
    height: requireInteger(height, "height", 1, maximum),
  };
}

function mergeExtraParameters(
  base: CreativeJsonObject,
  extra: CreativeJsonObject | undefined,
  reserved: readonly string[],
): CreativeJsonObject {
  if (!extra) return base;
  const collision = reserved.find((field) =>
    Object.prototype.hasOwnProperty.call(extra, field),
  );
  if (collision) {
    throw new CreativeWorkbenchRuntimeError(
      "invalid_parameters",
      `extraParameters must not override ${collision}`,
      `extraParameters.${collision}`,
    );
  }
  return { ...base, ...extra };
}

function taskInputs(
  references: CreativeWorkbenchReferences,
): CreativeTaskInput[] {
  return references.bindings.map((binding) => ({
    assetId: binding.assetId,
    role: binding.role,
  }));
}

function roles(
  references: CreativeWorkbenchReferences,
): CreativeTaskInputRole[] {
  return references.bindings.map((binding) => binding.role);
}

function requireAllKinds(
  assets: readonly CreativeAsset[],
  kind: CreativeAsset["kind"],
  field = "references",
): void {
  if (assets.some((asset) => asset.kind !== kind)) {
    throw new CreativeWorkbenchRuntimeError(
      "reference_kind_mismatch",
      `All ${field} must be ${kind} assets`,
      field,
    );
  }
}

function assertNoReferences(
  references: CreativeWorkbenchReferences,
  capability: string,
): void {
  if (references.bindings.length > 0) {
    throw new CreativeWorkbenchRuntimeError(
      "reference_contract_mismatch",
      `${capability} does not consume reference assets`,
      "references",
    );
  }
}

export function prepareImageWorkbenchRun(
  input: PrepareImageWorkbenchRunInput,
): PreparedCreativeWorkbenchRun {
  assertTaskCapabilityPair(input.operation.task, input.operation.capability);
  const model = resolveExactWorkbenchModel(
    input.catalog,
    input.model,
    input.operation.task,
  );
  const assets = validateWorkbenchReferences(input.references);
  requireAllKinds(assets, "image");
  const inputRoles = roles(input.references);

  if (input.operation.capability === "t2i") {
    assertNoReferences(input.references, "t2i");
  } else if (input.operation.capability === "i2i") {
    if (
      assets.length === 0 ||
      inputRoles.some((role) => role !== "reference")
    ) {
      throw new CreativeWorkbenchRuntimeError(
        "reference_contract_mismatch",
        "i2i requires at least one image with role reference",
        "references",
      );
    }
  } else {
    const referenceCount = inputRoles.filter(
      (role) => role === "reference",
    ).length;
    const maskCount = inputRoles.filter((role) => role === "mask").length;
    if (
      referenceCount < 1 ||
      maskCount !== 1 ||
      inputRoles.some((role) => role !== "reference" && role !== "mask")
    ) {
      throw new CreativeWorkbenchRuntimeError(
        "reference_contract_mismatch",
        "inpaint requires image references and exactly one mask",
        "references",
      );
    }
  }

  const parameters = mergeExtraParameters(
    {
      prompt: requirePrompt(input.prompt, "prompt"),
      interface_mode: input.interfaceMode,
      quality: input.quality,
      aspect: requirePrompt(input.aspectRatio, "aspectRatio"),
      count: requireInteger(input.count, "count", 1, 10),
      ...dimensions(input.width, input.height, 8_192),
    },
    input.extraParameters,
    [
      "prompt",
      "interface_mode",
      "quality",
      "aspect",
      "count",
      "width",
      "height",
      "n",
      "size",
    ],
  );

  return markPreparedRun({
    kind: "image",
    repeat: 1,
    outputKind: "image",
    model,
    references: assets,
    input: {
      idempotencyKey: createCreativeTaskIdempotencyKey(),
      owner: { kind: "canvas_node", projectId: input.projectId, nodeId: input.nodeId },
      providerId: model.providerId,
      model: model.model,
      task: input.operation.task,
      capability: input.operation.capability,
      parameters,
      inputs: taskInputs(input.references),
    },
  });
}

export function prepareVideoWorkbenchRun(
  input: PrepareVideoWorkbenchRunInput,
): PreparedCreativeWorkbenchRun {
  assertTaskCapabilityPair(input.operation.task, input.operation.capability);
  const model = resolveExactWorkbenchModel(
    input.catalog,
    input.model,
    input.operation.task,
  );
  const assets = validateWorkbenchReferences(input.references);
  const inputRoles = roles(input.references);

  if (input.operation.capability === "t2v") {
    assertNoReferences(input.references, "t2v");
  } else if (input.operation.capability === "i2v") {
    requireAllKinds(assets, "image");
    if (
      assets.length === 0 ||
      inputRoles.some(
        (role) =>
          role !== "reference" &&
          role !== "first_frame" &&
          role !== "last_frame",
      )
    ) {
      throw new CreativeWorkbenchRuntimeError(
        "reference_contract_mismatch",
        "i2v requires image references with reference/first_frame/last_frame roles",
        "references",
      );
    }
  } else {
    requireAllKinds(assets, "video");
    if (assets.length === 0 || inputRoles.some((role) => role !== "video")) {
      throw new CreativeWorkbenchRuntimeError(
        "reference_contract_mismatch",
        "v2v requires video assets with role video",
        "references",
      );
    }
  }

  const parameters = mergeExtraParameters(
    {
      prompt: requirePrompt(input.prompt, "prompt"),
      resolution: requirePrompt(input.resolution, "resolution"),
      aspect: requirePrompt(input.aspectRatio, "aspectRatio"),
      seconds: requireInteger(input.seconds, "seconds", 1, 3_600),
      ...dimensions(input.width, input.height, 8_192),
    },
    input.extraParameters,
    ["prompt", "resolution", "aspect", "seconds", "width", "height", "size"],
  );

  return markPreparedRun({
    kind: "video",
    repeat: requireInteger(input.taskCount, "taskCount", 1, 6),
    outputKind: "video",
    model,
    references: assets,
    input: {
      idempotencyKey: createCreativeTaskIdempotencyKey(),
      owner: { kind: "canvas_node", projectId: input.projectId, nodeId: input.nodeId },
      providerId: model.providerId,
      model: model.model,
      task: "video_generation",
      capability: input.operation.capability,
      parameters,
      inputs: taskInputs(input.references),
    },
  });
}

export function prepareAudioWorkbenchRun(
  input: PrepareAudioWorkbenchRunInput,
): PreparedCreativeWorkbenchRun {
  const task = "speech_synthesis" as const;
  const capability = "tts" as const;
  assertTaskCapabilityPair(task, capability);
  const selected = input.value.model ?? input.model;
  if (
    input.value.model &&
    input.model &&
    (input.value.model.providerId !== input.model.providerId ||
      input.value.model.model !== input.model.model)
  ) {
    throw new CreativeWorkbenchRuntimeError(
      "model_not_compatible",
      "Audio value.model and runtime model selection disagree",
      "model",
    );
  }
  const model = resolveExactWorkbenchModel(input.catalog, selected, task);
  validateWorkbenchReferences(input.references);
  assertNoReferences(input.references, "tts");
  if (input.fieldSupport.references) {
    throw new CreativeWorkbenchRuntimeError(
      "reference_contract_mismatch",
      "The current creation TTS adapter does not consume reference audio",
      "fieldSupport.references",
    );
  }
  if (!input.fieldSupport.instructions && input.value.instructions.trim()) {
    throw new CreativeWorkbenchRuntimeError(
      "invalid_parameters",
      "instructions are present but not enabled by the selected protocol contract",
      "instructions",
    );
  }
  if (!input.fieldSupport.speed && input.value.speed !== 1) {
    throw new CreativeWorkbenchRuntimeError(
      "invalid_parameters",
      "speed is present but not enabled by the selected protocol contract",
      "speed",
    );
  }
  const base: CreativeJsonObject = {
    prompt: requirePrompt(
      input.value.text,
      "text",
      input.maxTextLength ?? 4_096,
    ),
    ...(input.fieldSupport.voice && input.value.voice.trim()
      ? { voice: input.value.voice.trim() }
      : {}),
    ...(input.fieldSupport.format && input.value.format.trim()
      ? { format: input.value.format.trim() }
      : {}),
    ...(input.fieldSupport.speed
      ? { speed: requireFiniteNumber(input.value.speed, "speed", 0.25, 4) }
      : {}),
    ...(input.fieldSupport.instructions && input.value.instructions.trim()
      ? { instructions: input.value.instructions }
      : {}),
  };
  const parameters = mergeExtraParameters(base, input.extraParameters, [
    "prompt",
    "voice",
    "format",
    "speed",
    "instructions",
  ]);

  return markPreparedRun({
    kind: "audio",
    repeat: 1,
    outputKind: "audio",
    model,
    references: [],
    input: {
      idempotencyKey: createCreativeTaskIdempotencyKey(),
      owner: { kind: "canvas_node", projectId: input.projectId, nodeId: input.nodeId },
      providerId: model.providerId,
      model: model.model,
      task,
      capability,
      parameters,
      inputs: [],
    },
  });
}
