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
import type {
  CreativeTaskInput,
  CreativeTaskInputRole,
  CreativeTaskOwner,
} from "../../tasks";
import type { SpeechGenerationFieldSupport, SpeechGenerationValue } from "@renderer/creation/parameters/speech";
import type {
  ImageGenerationInterfaceMode,
  ImageGenerationQuality,
} from "@renderer/creation/parameters/image";
import { imageGenerationSizePolicyForModel } from "@renderer/creation/parameters/image";
import { validateGenerationReferences } from "@renderer/creation/references";
import { resolveExactGenerationModel } from "@renderer/creation/modelSelection";
import type { GenerationModelSelection } from "@renderer/creation/modelSelection";
import type {
  GenerationReferences,
  CanvasGenerationTaskOperation,
  PreparedCanvasGenerationRun,
} from "./types";
import { GenerationError } from "./types";

const preparedRuns = new WeakSet<object>();

function markPreparedRun(
  run: Omit<PreparedCanvasGenerationRun, symbol>,
): PreparedCanvasGenerationRun {
  preparedRuns.add(run);
  return run as PreparedCanvasGenerationRun;
}

/** Runtime counterpart to the opaque PreparedCanvasGenerationRun type. */
export function isPreparedCanvasGenerationRun(
  value: PreparedCanvasGenerationRun,
): boolean {
  return typeof value === "object" && value !== null && preparedRuns.has(value);
}

interface CanvasPlanBase {
  catalog: CreativeModelCatalogSnapshot;
  /** Persistent canvas identity; every submitted task belongs to one node. */
  canvasId?: string;
  nodeId?: string;
  owner?: CreativeTaskOwner;
  model: GenerationModelSelection | null;
  references: GenerationReferences;
  extraParameters?: CreativeJsonObject;
}

function canvasTaskOwner(input: CanvasPlanBase): CreativeTaskOwner {
  if (input.owner) {
    if (input.nodeId || input.owner.kind !== "canvas_node" || input.owner.canvasId !== input.canvasId) {
      throw new GenerationError("invalid_parameters", "Canvas generation requires one matching canvas node owner", "owner");
    }
    return { ...input.owner };
  }
  if (!input.canvasId || !input.nodeId) {
    throw new GenerationError("invalid_parameters", "Canvas generation requires canvasId and nodeId", "owner");
  }
  return { kind: "canvas_node", canvasId: input.canvasId, nodeId: input.nodeId };
}

type CanvasImageOperation =
  | CanvasGenerationTaskOperation<"image_generation", "t2i">
  | CanvasGenerationTaskOperation<"image_edit", "i2i" | "inpaint">;

export interface PrepareCanvasImageRunInput extends CanvasPlanBase {
  operation: CanvasImageOperation;
  prompt: string;
  interfaceMode: ImageGenerationInterfaceMode;
  quality: ImageGenerationQuality;
  width: number | null;
  height: number | null;
  /** Provider-native size string, kept separate from display dimensions. */
  size?: string | null;
  aspectRatio: string;
  count: number;
}

export type CanvasVideoOperation = CanvasGenerationTaskOperation<
  "video_generation",
  "t2v" | "i2v" | "v2v"
>;

export interface PrepareCanvasVideoRunInput extends CanvasPlanBase {
  operation: CanvasVideoOperation;
  prompt: string;
  seconds: number;
  /** Product-level metadata retained for history; adapters receive only size. */
  resolution?: string;
  aspectRatio?: string;
  width: number | null;
  height: number | null;
  taskCount: number;
}

export interface PrepareCanvasAudioRunInput extends CanvasPlanBase {
  value: SpeechGenerationValue;
  fieldSupport: SpeechGenerationFieldSupport;
  maxTextLength?: number;
}

function requirePrompt(
  value: string,
  field: string,
  maxLength?: number,
): string {
  if (!value.trim()) {
    throw new GenerationError(
      "invalid_parameters",
      `${field} must not be blank`,
      field,
    );
  }
  if (maxLength !== undefined && Array.from(value).length > maxLength) {
    throw new GenerationError(
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
    throw new GenerationError(
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
    throw new GenerationError(
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
    throw new GenerationError(
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
    throw new GenerationError(
      "invalid_parameters",
      `extraParameters must not override ${collision}`,
      `extraParameters.${collision}`,
    );
  }
  return { ...base, ...extra };
}

function taskInputs(
  references: GenerationReferences,
): CreativeTaskInput[] {
  return references.bindings.map((binding) => ({
    assetId: binding.assetId,
    kind: binding.kind,
    role: binding.role,
  }));
}

function roles(
  references: GenerationReferences,
): CreativeTaskInputRole[] {
  return references.bindings.map((binding) => binding.role);
}

function requireAllKinds(
  assets: readonly CreativeAsset[],
  kind: CreativeAsset["kind"],
  field = "references",
): void {
  if (assets.some((asset) => asset.kind !== kind)) {
    throw new GenerationError(
      "reference_kind_mismatch",
      `All ${field} must be ${kind} assets`,
      field,
    );
  }
}

function assertNoReferences(
  references: GenerationReferences,
  capability: string,
): void {
  if (references.bindings.length > 0) {
    throw new GenerationError(
      "reference_contract_mismatch",
      `${capability} does not consume reference assets`,
      "references",
    );
  }
}

export function prepareCanvasImageRun(
  input: PrepareCanvasImageRunInput,
): PreparedCanvasGenerationRun {
  assertTaskCapabilityPair(input.operation.task, input.operation.capability);
  const model = resolveExactGenerationModel(
    input.catalog,
    input.model,
    input.operation.task,
  );
  const assets = validateGenerationReferences(input.references);
  requireAllKinds(assets, "image");
  const inputRoles = roles(input.references);

  if (input.operation.capability === "t2i") {
    assertNoReferences(input.references, "t2i");
  } else if (input.operation.capability === "i2i") {
    if (
      assets.length === 0 ||
      inputRoles.some((role) => role !== "reference")
    ) {
      throw new GenerationError(
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
      throw new GenerationError(
        "reference_contract_mismatch",
        "inpaint requires image references and exactly one mask",
        "references",
      );
    }
  }

  const sizePolicy = imageGenerationSizePolicyForModel(model);
  if (input.count > sizePolicy.maxCount) {
    throw new GenerationError(
      "invalid_parameters",
      `The selected image model supports at most ${sizePolicy.maxCount} output image`,
      "count",
    );
  }
  const matchingSizeOption = sizePolicy.options.find(
    (option) =>
      !option.disabled &&
      option.width === input.width &&
      option.height === input.height,
  );
  const isTextToImage = input.operation.capability === "t2i";
  if (
    isTextToImage &&
    !sizePolicy.allowCustomDimensions &&
    !matchingSizeOption
  ) {
    throw new GenerationError(
      "invalid_parameters",
      "The selected image model does not support the requested dimensions",
      "dimensions",
    );
  }
  const explicitProviderSize = input.size?.trim();
  if (
    isTextToImage &&
    explicitProviderSize &&
    matchingSizeOption?.requestSize &&
    explicitProviderSize !== matchingSizeOption.requestSize
  ) {
    throw new GenerationError(
      "invalid_parameters",
      "Provider-native size does not match the selected dimensions",
      "size",
    );
  }
  const providerSize = isTextToImage
    ? explicitProviderSize || matchingSizeOption?.requestSize
    : explicitProviderSize;
  const parameters = mergeExtraParameters(
    {
      prompt: requirePrompt(input.prompt, "prompt"),
      interface_mode: input.interfaceMode,
      quality: input.quality,
      aspect: requirePrompt(input.aspectRatio, "aspectRatio"),
      count: requireInteger(input.count, "count", 1, 10),
      ...(providerSize ? { size: providerSize } : {}),
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
      "size",
      "n",
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
      owner: canvasTaskOwner(input),
      providerId: model.providerId,
      model: model.model,
      task: input.operation.task,
      capability: input.operation.capability,
      parameters,
      inputs: taskInputs(input.references),
    },
  });
}

export function prepareCanvasVideoRun(
  input: PrepareCanvasVideoRunInput,
): PreparedCanvasGenerationRun {
  assertTaskCapabilityPair(input.operation.task, input.operation.capability);
  const model = resolveExactGenerationModel(
    input.catalog,
    input.model,
    input.operation.task,
  );
  const assets = validateGenerationReferences(input.references);
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
      throw new GenerationError(
        "reference_contract_mismatch",
        "i2v requires image references with reference/first_frame/last_frame roles",
        "references",
      );
    }
  } else {
    requireAllKinds(assets, "video");
    if (assets.length === 0 || inputRoles.some((role) => role !== "video")) {
      throw new GenerationError(
        "reference_contract_mismatch",
        "v2v requires video assets with role video",
        "references",
      );
    }
  }

  const parameters = mergeExtraParameters(
    {
      prompt: requirePrompt(input.prompt, "prompt"),
      seconds: requireInteger(input.seconds, "seconds", 1, 3_600),
      ...(input.resolution
        ? { resolution: requirePrompt(input.resolution, "resolution") }
        : {}),
      ...(input.aspectRatio
        ? { aspect: requirePrompt(input.aspectRatio, "aspectRatio") }
        : {}),
      ...dimensions(input.width, input.height, 8_192),
    },
    input.extraParameters,
    ["prompt", "seconds", "width", "height", "size", "resolution", "aspect"],
  );

  return markPreparedRun({
    kind: "video",
    repeat: requireInteger(input.taskCount, "taskCount", 1, 6),
    outputKind: "video",
    model,
    references: assets,
    input: {
      idempotencyKey: createCreativeTaskIdempotencyKey(),
      owner: canvasTaskOwner(input),
      providerId: model.providerId,
      model: model.model,
      task: "video_generation",
      capability: input.operation.capability,
      parameters,
      inputs: taskInputs(input.references),
    },
  });
}

export function prepareCanvasAudioRun(
  input: PrepareCanvasAudioRunInput,
): PreparedCanvasGenerationRun {
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
    throw new GenerationError(
      "model_not_compatible",
      "Audio value.model and runtime model selection disagree",
      "model",
    );
  }
  const model = resolveExactGenerationModel(input.catalog, selected, task);
  validateGenerationReferences(input.references);
  assertNoReferences(input.references, "tts");
  if (input.fieldSupport.references) {
    throw new GenerationError(
      "reference_contract_mismatch",
      "The current creation TTS adapter does not consume reference audio",
      "fieldSupport.references",
    );
  }
  if (!input.fieldSupport.instructions && input.value.instructions.trim()) {
    throw new GenerationError(
      "invalid_parameters",
      "instructions are present but not enabled by the selected protocol contract",
      "instructions",
    );
  }
  if (!input.fieldSupport.speed && input.value.speed !== 1) {
    throw new GenerationError(
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
      owner: canvasTaskOwner(input),
      providerId: model.providerId,
      model: model.model,
      task,
      capability,
      parameters,
      inputs: [],
    },
  });
}
