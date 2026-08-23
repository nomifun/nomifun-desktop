/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeCanvasDetail, CreativeCanvasSummary } from '../domain';
import {
  creativeCanvasRepository,
  type CreativeCanvasRepository,
} from '../services/canvasRepository';
import { creativeStudioHttpCanvasArchivePort } from './archive';
import type { CreativeStudioCanvasesService } from './types';

/** Archive IO stays separate from canonical Canvas persistence. */
export interface CreativeStudioCanvasArchivePort {
  importCanvasArchive?(file: File): Promise<readonly CreativeCanvasSummary[]>;
  exportCanvasArchive?(canvases: readonly CreativeCanvasDetail[]): Promise<void>;
}

export type CreativeStudioCanvasArchiveOperation = 'import' | 'export';

export class CreativeStudioCanvasArchiveUnavailableError extends Error {
  readonly operation: CreativeStudioCanvasArchiveOperation;

  constructor(operation: CreativeStudioCanvasArchiveOperation) {
    super(`Creative Studio canvas archive ${operation} is not connected`);
    this.name = 'CreativeStudioCanvasArchiveUnavailableError';
    this.operation = operation;
  }
}

const assertRequestActive = (signal?: AbortSignal) => {
  if (!signal?.aborted) return;
  if (signal.reason instanceof Error) throw signal.reason;
  throw new DOMException('Creative Studio canvas request was aborted', 'AbortError');
};

export const createCreativeStudioCanvasesService = (
  repository: CreativeCanvasRepository = creativeCanvasRepository,
  archivePort?: CreativeStudioCanvasArchivePort
): CreativeStudioCanvasesService => ({
  archiveCapabilities: {
    canImport: typeof archivePort?.importCanvasArchive === 'function',
    canExport: typeof archivePort?.exportCanvasArchive === 'function',
  },

  listCanvases: async (signal) => {
    assertRequestActive(signal);
    const canvases = await repository.list();
    assertRequestActive(signal);
    return canvases;
  },

  createCanvas: (title) => repository.create({ title }),

  importCanvasArchive: async (file) => {
    if (!archivePort?.importCanvasArchive) {
      throw new CreativeStudioCanvasArchiveUnavailableError('import');
    }
    return archivePort.importCanvasArchive(file);
  },

  renameCanvas: (canvasId, title) => repository.rename(canvasId, title),

  deleteCanvases: async (canvasIds) => {
    await Promise.all(canvasIds.map((canvasId) => repository.remove(canvasId)));
  },

  exportCanvases: async (canvasIds) => {
    if (!archivePort?.exportCanvasArchive) {
      throw new CreativeStudioCanvasArchiveUnavailableError('export');
    }
    const canvases = await Promise.all(
      canvasIds.map((canvasId) => repository.load(canvasId))
    );
    await archivePort.exportCanvasArchive(canvases);
  },
});

export const creativeStudioCanvasesService =
  createCreativeStudioCanvasesService(
    creativeCanvasRepository,
    creativeStudioHttpCanvasArchivePort
  );
