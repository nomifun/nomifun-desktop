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
import type { CreativeWorkbenchResumeRequest } from "./types";
import { CreativeWorkbenchRuntimeError } from "./types";

export function workbenchOutputKindForTask(
  task: CreativeCreationModelTask,
): "image" | "video" | "audio" {
  if (task === "image_generation" || task === "image_edit") return "image";
  if (task === "video_generation") return "video";
  if (task === "speech_synthesis") return "audio";
  throw new CreativeWorkbenchRuntimeError(
    "task_capability_mismatch",
    `Creative workbench cannot recover ${task} output`,
    "task",
  );
}

export function workbenchResumeRequests(
  references: readonly CreativeTaskReference[],
): CreativeWorkbenchResumeRequest[] {
  return references.flatMap((reference) =>
    reference.task === "chat"
      ? []
      : [
          {
            reference,
            outputKind: workbenchOutputKindForTask(reference.task),
          },
        ],
  );
}

/** Resolve canonical pendingTaskIds without importing a page or editor store. */
export function workbenchResumeRequestsFromDocument(
  document: Pick<
    CreativeProjectDocument,
    "projectId" | "pendingTaskIds" | "nodes"
  >,
): CreativeWorkbenchResumeRequest[] {
  return workbenchResumeRequests(pendingCreativeTaskReferences(document));
}
