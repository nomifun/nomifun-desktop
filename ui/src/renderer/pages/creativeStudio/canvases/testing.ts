/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeCanvasSummary } from '../domain';
import type { CreativeStudioCanvasesService } from './types';

export const createCreativeStudioCanvasFixture = (
  overrides: Partial<CreativeCanvasSummary> = {}
): CreativeCanvasSummary => ({
  canvasId: 'canvas-brand-film',
  title: '品牌短片概念',
  revision: '1',
  createdAt: Date.parse('2026-08-17T03:10:00.000Z'),
  updatedAt: Date.parse('2026-08-20T01:35:00.000Z'),
  nodeCount: 18,
  connectionCount: 12,
  ...overrides,
});

export const CREATIVE_STUDIO_CANVAS_FIXTURES: readonly CreativeCanvasSummary[] = [
  createCreativeStudioCanvasFixture(),
  createCreativeStudioCanvasFixture({
    canvasId: 'canvas-product-stills',
    title: '秋季产品静物',
    updatedAt: Date.parse('2026-08-19T09:12:00.000Z'),
    nodeCount: 9,
    connectionCount: 6,
  }),
  createCreativeStudioCanvasFixture({
    canvasId: 'canvas-character-study',
    title: '角色风格探索',
    updatedAt: Date.parse('2026-08-18T14:46:00.000Z'),
    nodeCount: 24,
    connectionCount: 19,
  }),
];

export interface MockCreativeStudioCanvasesService
  extends CreativeStudioCanvasesService {
  calls: {
    list: number;
    create: number;
    imports: File[];
    renames: Array<{ canvasId: string; title: string }>;
    deletes: string[][];
    exports: string[][];
  };
  snapshot(): CreativeCanvasSummary[];
}

export const createMockCreativeStudioCanvasesService = (
  seed: readonly CreativeCanvasSummary[] = CREATIVE_STUDIO_CANVAS_FIXTURES
): MockCreativeStudioCanvasesService => {
  let canvases = seed.map((canvas) => ({ ...canvas }));
  const calls: MockCreativeStudioCanvasesService['calls'] = {
    list: 0,
    create: 0,
    imports: [],
    renames: [],
    deletes: [],
    exports: [],
  };

  return {
    archiveCapabilities: { canImport: true, canExport: true },
    calls,
    snapshot: () => canvases.map((canvas) => ({ ...canvas })),
    listCanvases: async () => {
      calls.list += 1;
      return canvases.map((canvas) => ({ ...canvas }));
    },
    createCanvas: async (title) => {
      calls.create += 1;
      const created = createCreativeStudioCanvasFixture({
        canvasId: `canvas-created-${calls.create}`,
        title,
        createdAt: Date.parse('2026-08-20T02:00:00.000Z'),
        updatedAt: Date.parse('2026-08-20T02:00:00.000Z'),
        nodeCount: 0,
        connectionCount: 0,
      });
      canvases = [created, ...canvases];
      return { ...created };
    },
    importCanvasArchive: async (file) => {
      calls.imports.push(file);
      const imported = createCreativeStudioCanvasFixture({
        canvasId: `canvas-imported-${calls.imports.length}`,
        title: file.name.replace(/\.zip$/i, '') || '导入画布',
      });
      canvases = [imported, ...canvases];
      return [{ ...imported }];
    },
    renameCanvas: async (canvasId, title) => {
      calls.renames.push({ canvasId, title });
      const existing = canvases.find((canvas) => canvas.canvasId === canvasId);
      if (!existing) throw new Error(`Canvas ${canvasId} was not found`);
      const renamed = {
        ...existing,
        title,
        updatedAt: Date.parse('2026-08-20T02:20:00.000Z'),
      };
      canvases = canvases.map((canvas) =>
        canvas.canvasId === canvasId ? renamed : canvas
      );
      return { ...renamed };
    },
    deleteCanvases: async (canvasIds) => {
      calls.deletes.push([...canvasIds]);
      const removed = new Set(canvasIds);
      canvases = canvases.filter((canvas) => !removed.has(canvas.canvasId));
    },
    exportCanvases: async (canvasIds) => {
      calls.exports.push([...canvasIds]);
    },
  };
};
