/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  CreativeStudioContractError,
  createEmptyCreativeCanvasDocument,
} from '../domain';
import {
  createCreativeStudioCanvasApi,
  type CreativeStudioHttpRequest,
} from './canvasApi';

const CANVAS_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f112';
const PROVIDER_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f118';
const summary = {
  canvasId: CANVAS_ID,
  title: 'Untitled canvas',
  revision: '1',
  nodeCount: 0,
  connectionCount: 0,
  createdAt: 1_770_000_000_000,
  updatedAt: 1_770_000_000_000,
};

const captureError = async (operation: () => Promise<unknown>) => {
  try {
    await operation();
    return null;
  } catch (error) {
    return error;
  }
};

describe('Creative Studio Canvas API client', () => {
  test('uses canonical Canvas endpoints and validates every response', async () => {
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
    const api = createCreativeStudioCanvasApi(request);

    await api.listCanvases();
    await api.createCanvas({
      title: 'Untitled canvas',
      agentKickoff: {
        prompt: 'Plan a launch poster',
        model: { providerId: PROVIDER_ID, model: 'gpt-5' },
      },
    });
    await api.getCanvas(CANVAS_ID);
    await api.renameCanvas(CANVAS_ID, { title: 'Renamed' });
    await api.saveCanvas(CANVAS_ID, {
      expectedRevision: '1',
      document: createEmptyCreativeCanvasDocument(CANVAS_ID),
    });
    await api.deleteCanvas(CANVAS_ID);

    expect(calls).toEqual([
      { method: 'GET', path: '/api/creative-studio/canvases' },
      {
        method: 'POST',
        path: '/api/creative-studio/canvases',
        body: {
          title: 'Untitled canvas',
          agentKickoff: {
            prompt: 'Plan a launch poster',
            model: { providerId: PROVIDER_ID, model: 'gpt-5' },
          },
        },
      },
      {
        method: 'GET',
        path: `/api/creative-studio/canvases/${CANVAS_ID}`,
      },
      {
        method: 'PATCH',
        path: `/api/creative-studio/canvases/${CANVAS_ID}`,
        body: { title: 'Renamed' },
      },
      {
        method: 'PUT',
        path: `/api/creative-studio/canvases/${CANVAS_ID}/document`,
        body: {
          expectedRevision: '1',
          document: createEmptyCreativeCanvasDocument(CANVAS_ID),
        },
      },
      {
        method: 'DELETE',
        path: `/api/creative-studio/canvases/${CANVAS_ID}`,
      },
    ]);
  });

  test('rejects server identity drift and retired response fields', async () => {
    const otherId = '0198f8bb-8424-7b3d-8f17-bc6a1676f113';
    const drift = createCreativeStudioCanvasApi(async () => ({
      canvas: { ...summary, canvasId: otherId },
      document: createEmptyCreativeCanvasDocument(otherId),
    }));
    expect(
      (await captureError(() => drift.getCanvas(CANVAS_ID))) instanceof
        CreativeStudioContractError
    ).toBe(true);

    const retired = createCreativeStudioCanvasApi(async () => ({
      projects: [{ ...summary, projectId: CANVAS_ID }],
    }));
    expect(
      (await captureError(() => retired.listCanvases())) instanceof
        CreativeStudioContractError
    ).toBe(true);
  });
});
