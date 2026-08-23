/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  createEmptyCreativeCanvasDocument,
  createEmptyCreativeProjectDocument,
} from '../domain';
import {
  createCreativeStudioProjectApi,
  type CreativeStudioHttpRequest,
} from './projectApi';

const CANVAS_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f112';
const summary = {
  canvasId: CANVAS_ID,
  title: 'Untitled canvas',
  revision: '1',
  nodeCount: 0,
  connectionCount: 0,
  createdAt: 1_770_000_000_000,
  updatedAt: 1_770_000_000_000,
};

describe('legacy Creative Studio persistence adapter', () => {
  test('uses only Canvas endpoints and converts the response for older editor modules', async () => {
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    const request: CreativeStudioHttpRequest = async (method, path, body) => {
      calls.push({ method, path, ...(body === undefined ? {} : { body }) });
      if (method === 'GET' && path.endsWith('/canvases')) {
        return { canvases: [summary] };
      }
      if (method === 'GET') {
        return {
          canvas: summary,
          document: createEmptyCreativeCanvasDocument(CANVAS_ID),
        };
      }
      if (method === 'DELETE') return undefined;
      return { canvas: summary };
    };
    const api = createCreativeStudioProjectApi(request);

    expect(await api.listProjects()).toEqual([
      { ...summary, projectId: CANVAS_ID, canvasId: undefined },
    ].map(({ canvasId: _canvasId, ...legacy }) => legacy));
    expect(await api.getProject(CANVAS_ID)).toEqual({
      project: {
        title: summary.title,
        revision: summary.revision,
        nodeCount: 0,
        connectionCount: 0,
        createdAt: summary.createdAt,
        updatedAt: summary.updatedAt,
        projectId: CANVAS_ID,
      },
      document: createEmptyCreativeProjectDocument(CANVAS_ID),
    });
    await api.saveProject(CANVAS_ID, {
      expectedRevision: '1',
      document: createEmptyCreativeProjectDocument(CANVAS_ID),
    });

    expect(calls.map((call) => call.path)).toEqual([
      '/api/creative-studio/canvases',
      `/api/creative-studio/canvases/${CANVAS_ID}`,
      `/api/creative-studio/canvases/${CANVAS_ID}/document`,
    ]);
    expect(calls[2].body).toEqual({
      expectedRevision: '1',
      document: createEmptyCreativeCanvasDocument(CANVAS_ID),
    });
    expect(calls.some((call) => call.path.includes('/projects'))).toBe(false);
  });
});
