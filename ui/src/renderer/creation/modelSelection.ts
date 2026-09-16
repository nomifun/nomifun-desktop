import type { ImageGenerationModelOption } from './parameters/image';
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { parseProviderId } from "@/common/types/ids";

import {
  buildCreativeModelGroups,
  findCreativeModelOption,
  flattenCreativeModelGroups,
} from "@renderer/pages/creativeStudio/models";
import type {
  CreativeModelCatalogSnapshot,
  CreativeModelOption,
  CreativeModelSelectionRef,
} from "@renderer/pages/creativeStudio/models";
import type { CreativeCreationModelTask } from "@renderer/pages/creativeStudio/tasks";
import { GenerationError } from "./generationError";

export interface GenerationModelSelection {
  providerId: string;
  model: string;
}

function normalizedSelection(
  selection: GenerationModelSelection,
): CreativeModelSelectionRef {
  if (!selection.model || selection.model.trim() !== selection.model) {
    throw new GenerationError(
      "model_required",
      "A normalized model id is required",
      "model",
    );
  }
  return {
    providerId: parseProviderId(selection.providerId),
    model: selection.model,
  };
}

/** Resolve only inside the exact ModelTask pool; neighbouring tasks are never consulted. */
export function resolveExactGenerationModel(
  catalog: CreativeModelCatalogSnapshot,
  selection: GenerationModelSelection | null,
  task: CreativeCreationModelTask,
): CreativeModelOption {
  if (catalog.status === "loading") {
    throw new GenerationError(
      "catalog_loading",
      `The ${task} model catalog is still loading`,
    );
  }
  if (catalog.status === "error") {
    throw new GenerationError(
      "catalog_error",
      catalog.error?.message || `The ${task} model catalog failed to load`,
    );
  }
  if (!selection) {
    throw new GenerationError(
      "model_required",
      `Select an exact ${task} model`,
    );
  }
  const groups = buildCreativeModelGroups(catalog.providers, {
    capability: "task",
    task,
  });
  const model = findCreativeModelOption(groups, normalizedSelection(selection));
  if (!model || model.task !== task) {
    throw new GenerationError(
      "model_not_compatible",
      `Model ${selection.providerId}/${selection.model} is not enabled for ${task}`,
      "model",
    );
  }
  return model;
}

export function exactGenerationModelOptions(
  catalog: CreativeModelCatalogSnapshot,
  task: CreativeCreationModelTask,
): CreativeModelOption[] {
  if (catalog.status !== "ready") return [];
  return flattenCreativeModelGroups(
    buildCreativeModelGroups(catalog.providers, { capability: "task", task }),
  );
}

export function imageGenerationModelOptions(
  catalog: CreativeModelCatalogSnapshot,
  task: Extract<CreativeCreationModelTask, "image_generation" | "image_edit">,
): ImageGenerationModelOption[] {
  return exactGenerationModelOptions(catalog, task).map((option) => ({
    providerId: option.providerId,
    model: option.model,
    label: option.displayName ?? option.model,
    ...(option.rawModelId ? { rawModelId: option.rawModelId } : {}),
    providerLabel: option.providerName,
    platform: option.platform,
    protocol: option.protocol,
  }));
}
