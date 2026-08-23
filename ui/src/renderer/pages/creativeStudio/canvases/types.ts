/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeCanvasSummary } from '../domain';

export type CreativeStudioCanvasesLoadState = 'loading' | 'ready' | 'error';

export interface CreativeStudioCanvasesSnapshot {
  status: CreativeStudioCanvasesLoadState;
  canvases: readonly CreativeCanvasSummary[];
  error?: string;
}

export interface CreativeStudioCanvasArchiveCapabilities {
  canImport: boolean;
  canExport: boolean;
}

/** Product-facing Canvas-library persistence and archive port. */
export interface CreativeStudioCanvasesService {
  readonly archiveCapabilities: CreativeStudioCanvasArchiveCapabilities;
  listCanvases(signal?: AbortSignal): Promise<readonly CreativeCanvasSummary[]>;
  createCanvas(title: string): Promise<CreativeCanvasSummary>;
  importCanvasArchive(file: File): Promise<readonly CreativeCanvasSummary[]>;
  renameCanvas(canvasId: string, title: string): Promise<CreativeCanvasSummary>;
  deleteCanvases(canvasIds: readonly string[]): Promise<void>;
  exportCanvases(canvasIds: readonly string[]): Promise<void>;
}
