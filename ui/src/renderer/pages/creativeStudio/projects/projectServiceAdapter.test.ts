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
  createCreativeStudioProjectsService,
  mapCreativeProjectSummary,
} from './projectServiceAdapter';

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
  const calls = { loads: [] as string[], removes: [] as string[] };
  const repository: CreativeCanvasRepository = {
    list: async () => [CANVAS],
    create: async (request = {}) => ({
      ...CANVAS,
      title: request.title ?? CANVAS.title,
    }),
    load: async (canvasId) => {
      calls.loads.push(canvasId);
      return {
        canvas: CANVAS,
        document: createEmptyCreativeCanvasDocument(canvasId),
      };
    },
    save: async () => CANVAS,
    rename: async (canvasId, title) => ({
      ...CANVAS,
      canvasId,
      title,
    }),
    remove: async (canvasId) => {
      calls.removes.push(canvasId);
    },
  };
  return { repository, calls };
};

describe('legacy project-named Canvas service adapter', () => {
  test('maps Canvas identity without issuing a second persistence model', async () => {
    const fixture = repositoryFixture();
    const service = createCreativeStudioProjectsService(fixture.repository);

    expect(mapCreativeProjectSummary(CANVAS)).toEqual({
      id: CANVAS.canvasId,
      title: CANVAS.title,
      createdAt: CANVAS.createdAt,
      updatedAt: CANVAS.updatedAt,
      nodeCount: CANVAS.nodeCount,
      connectionCount: CANVAS.connectionCount,
    });
    expect(await service.listProjects()).toEqual([
      mapCreativeProjectSummary(CANVAS),
    ]);
    expect((await service.createProject('无限画布 2')).title).toBe('无限画布 2');
    expect(
      (await service.renameProject(CANVAS.canvasId, '品牌概念')).title
    ).toBe('品牌概念');

    await service.deleteProjects([CANVAS.canvasId]);
    expect(fixture.calls.removes).toEqual([CANVAS.canvasId]);
  });

  test('adapts canonical archive capabilities through legacy method names', async () => {
    const fixture = repositoryFixture();
    const exported: CreativeCanvasDetail[][] = [];
    const service = createCreativeStudioProjectsService(fixture.repository, {
      importCanvasArchive: async () => [CANVAS],
      exportCanvasArchive: async (canvases) => {
        exported.push([...canvases]);
      },
    });

    expect(
      await service.importProjectArchive(new File([], 'canvas.zip'))
    ).toEqual([mapCreativeProjectSummary(CANVAS)]);
    await service.exportProjects([CANVAS.canvasId]);

    expect(fixture.calls.loads).toEqual([CANVAS.canvasId]);
    expect(exported[0]?.[0]?.canvas.canvasId).toBe(CANVAS.canvasId);
  });
});
