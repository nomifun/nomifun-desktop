/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeProjectDocument } from "../../domain/schema";
import {
  pendingCreativeTaskReferences,
  type CreativeCreationModelTask,
  type CreativeTaskReference,
} from "../../tasks";
import type { CanvasGenerationResumeRequest } from "./types";
import { GenerationError } from "./types";

function canvasOutputKindForTask(
  task: CreativeCreationModelTask,
): "image" | "video" | "audio" {
  if (task === "image_generation" || task === "image_edit") return "image";
  if (task === "video_generation") return "video";
  if (task === "speech_synthesis") return "audio";
  throw new GenerationError(
    "task_capability_mismatch",
    `Creative canvas generation cannot recover ${task} output`,
    "task",
  );
}

export function canvasResumeRequests(
  references: readonly CreativeTaskReference[],
): CanvasGenerationResumeRequest[] {
  return references.flatMap((reference) =>
    reference.owner.kind !== "canvas_node" || reference.task === "chat"
      ? []
      : [
          {
            reference,
            outputKind: canvasOutputKindForTask(reference.task),
          },
        ],
  );
}

/** Resolve canonical pendingTaskIds without importing a page or editor store. */
export function canvasResumeRequestsFromDocument(
  document: Pick<
    CreativeProjectDocument,
    "projectId" | "pendingTaskIds" | "nodes"
  >,
): CanvasGenerationResumeRequest[] {
  return canvasResumeRequests(pendingCreativeTaskReferences(document));
}
