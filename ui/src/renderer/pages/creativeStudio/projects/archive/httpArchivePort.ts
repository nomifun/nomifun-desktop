/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CREATIVE_STUDIO_CANVAS_ARCHIVE_IMPORT_ENDPOINT,
  CREATIVE_STUDIO_CANVAS_ARCHIVE_MIME,
  createCreativeStudioHttpCanvasArchivePort,
  type CreativeStudioCanvasArchiveFetch,
  type CreativeStudioCanvasArchiveSave,
} from '../../canvases/archive';
import {
  creativeCanvasDetailToLegacyProject,
  creativeCanvasSummaryToLegacyProject,
  legacyProjectDetailToCreativeCanvas,
} from '../../domain';
import type { CreativeStudioProjectArchivePort } from '../projectServiceAdapter';

/** @deprecated Canonical archive transport uses Canvas endpoints. */
export const CREATIVE_STUDIO_ARCHIVE_MIME =
  CREATIVE_STUDIO_CANVAS_ARCHIVE_MIME;
/** @deprecated Canonical archive transport uses Canvas endpoints. */
export const CREATIVE_STUDIO_ARCHIVE_IMPORT_ENDPOINT =
  CREATIVE_STUDIO_CANVAS_ARCHIVE_IMPORT_ENDPOINT;

export type CreativeStudioArchiveFetch = CreativeStudioCanvasArchiveFetch;
export type CreativeStudioArchiveSave = CreativeStudioCanvasArchiveSave;

/**
 * @deprecated Historical method-name adapter over canonical Canvas archive IO.
 */
export function createCreativeStudioHttpArchivePort(
  archiveFetch?: CreativeStudioArchiveFetch,
  save?: CreativeStudioArchiveSave
): Required<CreativeStudioProjectArchivePort> {
  const canvasPort = createCreativeStudioHttpCanvasArchivePort(
    archiveFetch,
    save
  );
  return {
    async importCanvasArchive(file) {
      return canvasPort.importCanvasArchive(file);
    },
    async exportCanvasArchive(canvases) {
      await canvasPort.exportCanvasArchive(canvases);
    },
  };
}

/** @deprecated Use creativeStudioHttpCanvasArchivePort. */
export const creativeStudioHttpArchivePort =
  createCreativeStudioHttpArchivePort();

// Keep named conversions reachable for older test utilities without exposing a
// second transport implementation.
export const legacyArchiveCanvasToProject =
  creativeCanvasSummaryToLegacyProject;
export const legacyArchiveDetailToProject =
  creativeCanvasDetailToLegacyProject;
export const legacyArchiveProjectToCanvas =
  legacyProjectDetailToCreativeCanvas;
