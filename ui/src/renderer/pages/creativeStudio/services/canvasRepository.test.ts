/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { BackendHttpError } from '@/common/adapter/httpBridge';
import { describe, expect, test } from 'bun:test';
import { createEmptyCreativeCanvasDocument } from '../domain';
import type { CreativeStudioCanvasApi } from './canvasApi';
import {
  CreativeCanvasRepositoryError,
  createCreativeCanvasRepository,
} from './canvasRepository';
import {
  sortCreativeCanvasSummaries,
  upsertCreativeCanvasSummary,
} from './useCreativeCanvases';

const CANVAS_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f112';
const canvas = {
  canvasId: CANVAS_ID,
  title: 'Canvas',
  revision: '1',
  nodeCount: 0,
  connectionCount: 0,
  createdAt: 100,
  updatedAt: 200,
};

const apiStub = (
  overrides: Partial<CreativeStudioCanvasApi> = {}
): CreativeStudioCanvasApi => ({
  listCanvases: async () => [canvas],
  createCanvas: async () => canvas,
  getCanvas: async () => ({
    canvas,
    document: createEmptyCreativeCanvasDocument(CANVAS_ID),
  }),
  renameCanvas: async () => canvas,
  deleteCanvas: async () => undefined,
  saveCanvas: async () => canvas,
  ...overrides,
});

const captureError = async (operation: () => Promise<unknown>) => {
  try {
    await operation();
    return null;
  } catch (error) {
    return error;
  }
};

describe('Creative Canvas repository', () => {
  test('presents the narrow Canvas persistence port', async () => {
    const calls: unknown[] = [];
    const repository = createCreativeCanvasRepository(
      apiStub({
        createCanvas: async (request) => {
          calls.push({ request });
          return canvas;
        },
        saveCanvas: async (canvasId, request) => {
          calls.push({ canvasId, request });
          return { ...canvas, revision: '2' };
        },
      })
    );
    const document = createEmptyCreativeCanvasDocument(CANVAS_ID);
    const kickoff = {
      title: 'Canvas',
      agentKickoff: {
        prompt: 'Design a launch poster',
        model: {
          providerId: '0198f8bb-8424-7b3d-8f17-bc6a1676f118',
          model: 'gpt-5',
        },
      },
    };

    expect(await repository.list()).toEqual([canvas]);
    expect(await repository.create(kickoff)).toEqual(canvas);
    expect(await repository.load(CANVAS_ID)).toEqual({ canvas, document });
    expect(await repository.save(CANVAS_ID, '1', document)).toMatchObject({
      revision: '2',
    });
    expect(calls).toEqual([
      { request: kickoff },
      {
        canvasId: CANVAS_ID,
        request: { expectedRevision: '1', document },
      },
    ]);
  });

  test('maps stale saves and missing canvases to stable errors', async () => {
    const conflict = new BackendHttpError({
      method: 'PUT',
      path: `/api/creative-studio/canvases/${CANVAS_ID}/document`,
      status: 409,
      body: { code: 'REVISION_CONFLICT', error: 'Canvas revision is stale' },
    });
    const repository = createCreativeCanvasRepository(
      apiStub({ saveCanvas: async () => Promise.reject(conflict) })
    );

    expect(
      await captureError(() =>
        repository.save(
        CANVAS_ID,
        '1',
        createEmptyCreativeCanvasDocument(CANVAS_ID)
      )
      )
    ).toMatchObject({
      name: 'CreativeCanvasRepositoryError',
      kind: 'revision-conflict',
      status: 409,
      backendCode: 'REVISION_CONFLICT',
    });

    const missing = createCreativeCanvasRepository(
      apiStub({
        getCanvas: async () =>
          Promise.reject(
            new BackendHttpError({
              method: 'GET',
              path: `/api/creative-studio/canvases/${CANVAS_ID}`,
              status: 404,
              body: { code: 'NOT_FOUND', error: 'Canvas not found' },
            })
          ),
      })
    );
    try {
      await missing.load(CANVAS_ID);
      throw new Error('Expected missing canvas failure');
    } catch (error) {
      expect(error instanceof CreativeCanvasRepositoryError).toBe(true);
      expect(error).toMatchObject({ kind: 'not-found', status: 404 });
    }
  });
});

describe('Creative Canvas hook cache helpers', () => {
  test('sorts newest first and upserts by authoritative canvas id', () => {
    const older = {
      ...canvas,
      canvasId: '0198f8bb-8424-7b3d-8f17-bc6a1676f113',
      updatedAt: 100,
    };
    expect(
      sortCreativeCanvasSummaries([older, canvas]).map((item) => item.canvasId)
    ).toEqual([CANVAS_ID, older.canvasId]);
    expect(
      upsertCreativeCanvasSummary(
        [older, canvas],
        { ...canvas, title: 'Renamed', updatedAt: 300 }
      )
    ).toEqual([{ ...canvas, title: 'Renamed', updatedAt: 300 }, older]);
  });
});
