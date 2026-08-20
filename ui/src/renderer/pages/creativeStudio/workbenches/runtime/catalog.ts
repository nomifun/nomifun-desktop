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
} from "../../models";
import type {
  CreativeModelCatalogSnapshot,
  CreativeModelOption,
  CreativeModelSelectionRef,
} from "../../models";
import type { CreativeCreationModelTask } from "../../tasks";
import { CreativeWorkbenchRuntimeError } from "./types";

export interface CreativeWorkbenchModelSelection {
  providerId: string;
  model: string;
}

function normalizedSelection(
  selection: CreativeWorkbenchModelSelection,
): CreativeModelSelectionRef {
  if (!selection.model || selection.model.trim() !== selection.model) {
    throw new CreativeWorkbenchRuntimeError(
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
export function resolveExactWorkbenchModel(
  catalog: CreativeModelCatalogSnapshot,
  selection: CreativeWorkbenchModelSelection | null,
  task: CreativeCreationModelTask,
): CreativeModelOption {
  if (catalog.status === "loading") {
    throw new CreativeWorkbenchRuntimeError(
      "catalog_loading",
      `The ${task} model catalog is still loading`,
    );
  }
  if (catalog.status === "error") {
    throw new CreativeWorkbenchRuntimeError(
      "catalog_error",
      catalog.error?.message || `The ${task} model catalog failed to load`,
    );
  }
  if (!selection) {
    throw new CreativeWorkbenchRuntimeError(
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
    throw new CreativeWorkbenchRuntimeError(
      "model_not_compatible",
      `Model ${selection.providerId}/${selection.model} is not enabled for ${task}`,
      "model",
    );
  }
  return model;
}

export function exactWorkbenchModelOptions(
  catalog: CreativeModelCatalogSnapshot,
  task: CreativeCreationModelTask,
): CreativeModelOption[] {
  if (catalog.status !== "ready") return [];
  return flattenCreativeModelGroups(
    buildCreativeModelGroups(catalog.providers, { capability: "task", task }),
  );
}
