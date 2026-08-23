/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  createEmptyCreativeCanvasDocument,
  type CreativeCanvasDetail,
  type CreativeCanvasSummary,
} from '../domain';
import type { CreativeCanvasRepository } from '../services/canvasRepository';
import {
  CreativeStudioCanvasArchiveUnavailableError,
  createCreativeStudioCanvasesService,
} from './canvasServiceAdapter';

const CANVAS: CreativeCanvasSummary = {
  canvasId: '0190f5fe-7c00-7a00-8abc-000000000801',
  title: '无限画布 1',
  revision: '7',
  nodeCount: 4,
  connectionCount: 3,
  createdAt: 1_776_600_000_000,
  updatedAt: 1_776_600_030_000,
};

const repositoryFixture = () => {
  const calls = {
    creates: [] as Array<{ title?: string }>,
    loads: [] as string[],
    renames: [] as Array<{ canvasId: string; title: string }>,
    removes: [] as string[],
  };
  const repository: CreativeCanvasRepository = {
    list: async () => [CANVAS],
    create: async (request = {}) => {
      calls.creates.push(request);
      return { ...CANVAS, title: request.title ?? CANVAS.title };
    },
    load: async (canvasId) => {
      calls.loads.push(canvasId);
      return {
        canvas: CANVAS,
        document: createEmptyCreativeCanvasDocument(canvasId),
      };
    },
    save: async () => CANVAS,
    rename: async (canvasId, title) => {
      calls.renames.push({ canvasId, title });
      return { ...CANVAS, canvasId, title };
    },
    remove: async (canvasId) => {
      calls.removes.push(canvasId);
    },
  };
  return { repository, calls };
};

const captureError = async (operation: () => Promise<unknown>) => {
  try {
    await operation();
    return null;
  } catch (error) {
    return error;
  }
};

describe('Creative Studio Canvas service adapter', () => {
  test('passes canonical summaries without identity remapping', async () => {
    const fixture = repositoryFixture();
    const service = createCreativeStudioCanvasesService(fixture.repository);

    expect(await service.listCanvases()).toEqual([CANVAS]);
    expect((await service.createCanvas('无限画布 2')).title).toBe('无限画布 2');
    expect(
      (await service.renameCanvas(CANVAS.canvasId, '品牌概念')).title
    ).toBe('品牌概念');
    await service.deleteCanvases([CANVAS.canvasId, 'canvas-2']);

    expect(fixture.calls.creates).toEqual([{ title: '无限画布 2' }]);
    expect(fixture.calls.renames).toEqual([
      { canvasId: CANVAS.canvasId, title: '品牌概念' },
    ]);
    expect(fixture.calls.removes).toEqual([CANVAS.canvasId, 'canvas-2']);
  });

  test('fails explicitly without an archive port and loads exact details when connected', async () => {
    const fixture = repositoryFixture();
    const unavailable = createCreativeStudioCanvasesService(
      fixture.repository
    );

    expect(
      (await captureError(() =>
        unavailable.importCanvasArchive(new File([], 'canvas.zip'))
      )) instanceof CreativeStudioCanvasArchiveUnavailableError
    ).toBe(true);

    const exported: CreativeCanvasDetail[][] = [];
    const connected = createCreativeStudioCanvasesService(fixture.repository, {
      importCanvasArchive: async () => [CANVAS],
      exportCanvasArchive: async (canvases) => {
        exported.push([...canvases]);
      },
    });
    expect(
      await connected.importCanvasArchive(new File([], 'canvas.zip'))
    ).toEqual([CANVAS]);
    await connected.exportCanvases([CANVAS.canvasId]);

    expect(fixture.calls.loads).toEqual([CANVAS.canvasId]);
    expect(exported[0]?.[0]?.document.canvasId).toBe(CANVAS.canvasId);
  });
});
