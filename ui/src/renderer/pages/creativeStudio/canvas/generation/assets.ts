/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAssetPort } from "../../assets";
import type { CreativeTask } from "../../tasks";
import { projectCreativeTaskOutput } from "../../tasks";
import type { CanvasGenerationCommittedOutput } from "./types";

/** Resolve URLs only after the task runtime has validated committed result ids. */
export function committedCanvasOutputs(
  task: CreativeTask,
  kind: "image" | "video" | "audio",
  assets: CreativeAssetPort,
): CanvasGenerationCommittedOutput[] {
  const output = projectCreativeTaskOutput(task);
  if (!output) return [];
  return output.assetIds.map((assetId) => ({
    assetId,
    kind,
    url: kind === "audio" ? null : assets.url(assetId, "original"),
  }));
}
