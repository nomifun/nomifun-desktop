/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeCanvasSummary } from '../domain';
import {
  creativeStudioCanvasesService,
  type CreativeStudioCanvasArchivePort,
  createCreativeStudioCanvasesService,
} from '../canvases/canvasServiceAdapter';
import {
  creativeCanvasRepository,
  type CreativeCanvasRepository,
} from '../services/canvasRepository';
import type {
  CreativeStudioProjectSummary,
  CreativeStudioProjectsService,
} from './types';

/** @deprecated Map canonical Canvas summaries to the historical list shape. */
export const mapCreativeProjectSummary = (
  canvas: CreativeCanvasSummary
): CreativeStudioProjectSummary => ({
  id: canvas.canvasId,
  title: canvas.title,
  createdAt: canvas.createdAt,
  updatedAt: canvas.updatedAt,
  nodeCount: canvas.nodeCount,
  connectionCount: canvas.connectionCount,
});

export type CreativeStudioProjectArchivePort = CreativeStudioCanvasArchivePort;
export type CreativeStudioProjectArchiveOperation = 'import' | 'export';

/** @deprecated Use CreativeStudioCanvasArchiveUnavailableError. */
export class CreativeStudioProjectArchiveUnavailableError extends Error {
  readonly operation: CreativeStudioProjectArchiveOperation;

  constructor(operation: CreativeStudioProjectArchiveOperation) {
    super(`Creative Studio legacy archive adapter ${operation} is not connected`);
    this.name = 'CreativeStudioProjectArchiveUnavailableError';
    this.operation = operation;
  }
}

/**
 * @deprecated Compatibility adapter only. New product code must consume
 * createCreativeStudioCanvasesService directly.
 */
export const createCreativeStudioProjectsService = (
  repository?: CreativeCanvasRepository,
  archivePort?: CreativeStudioCanvasArchivePort
): CreativeStudioProjectsService => {
  const service = repository || archivePort
    ? createCreativeStudioCanvasesService(
        repository ?? creativeCanvasRepository,
        archivePort
      )
    : creativeStudioCanvasesService;

  return {
    archiveCapabilities: service.archiveCapabilities,
    listProjects: async (signal) =>
      (await service.listCanvases(signal)).map(mapCreativeProjectSummary),
    createProject: async (title) =>
      mapCreativeProjectSummary(await service.createCanvas(title)),
    importProjectArchive: async (file) =>
      (await service.importCanvasArchive(file)).map(mapCreativeProjectSummary),
    renameProject: async (id, title) =>
      mapCreativeProjectSummary(await service.renameCanvas(id, title)),
    deleteProjects: (ids) => service.deleteCanvases(ids),
    exportProjects: (ids) => service.exportCanvases(ids),
  };
};

/** @deprecated Use creativeStudioCanvasesService. */
export const creativeStudioProjectsService =
  createCreativeStudioProjectsService();
