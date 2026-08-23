/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { parseAssetId } from "@/common/types/ids";

import type { CreativeAsset, CreativeAssetPort } from "../../assets";
import type { CreativeTask } from "../../tasks";
import { projectCreativeTaskOutput } from "../../tasks";
import type {
  CreativeWorkbenchCommittedOutput,
  CreativeWorkbenchReferences,
} from "./types";
import { CreativeWorkbenchRuntimeError } from "./types";

/**
 * Validate references against concrete objects selected from the current-user
 * asset port. We deliberately do not treat optional origin fields as ACLs.
 */
export function validateWorkbenchReferences(
  references: CreativeWorkbenchReferences,
): CreativeAsset[] {
  const suppliedIds = references.assets.map((asset) => asset.id);
  if (
    new Set(suppliedIds).size !== suppliedIds.length ||
    references.assets.length !== references.bindings.length
  ) {
    throw new CreativeWorkbenchRuntimeError(
      "reference_contract_mismatch",
      "Reference assets must match bindings one-to-one without duplicates",
      "assets",
    );
  }
  const assets = new Map(references.assets.map((asset) => [asset.id, asset]));
  const seen = new Set<string>();
  return references.bindings.map((binding, index) => {
    const assetId = String(parseAssetId(binding.assetId));
    if (seen.has(assetId)) {
      throw new CreativeWorkbenchRuntimeError(
        "reference_contract_mismatch",
        `Reference asset ${assetId} is duplicated`,
        `bindings[${index}].assetId`,
      );
    }
    seen.add(assetId);
    const asset = assets.get(assetId);
    if (!asset) {
      throw new CreativeWorkbenchRuntimeError(
        "reference_not_owned",
        `Reference asset ${assetId} was not supplied by the asset selection boundary`,
        `bindings[${index}].assetId`,
      );
    }
    if (asset.kind !== binding.kind) {
      throw new CreativeWorkbenchRuntimeError(
        "reference_kind_mismatch",
        `Reference asset ${assetId} is ${asset.kind}, not ${binding.kind}`,
        `bindings[${index}].kind`,
      );
    }
    return asset;
  });
}

/** Resolve URLs only after the task runtime has validated committed result ids. */
export function committedWorkbenchOutputs(
  task: CreativeTask,
  kind: "image" | "video" | "audio",
  assets: CreativeAssetPort,
): CreativeWorkbenchCommittedOutput[] {
  const output = projectCreativeTaskOutput(task);
  if (!output) return [];
  return output.assetIds.map((assetId) => ({
    assetId,
    kind,
    url: kind === "audio" ? null : assets.url(assetId, "original"),
  }));
}
