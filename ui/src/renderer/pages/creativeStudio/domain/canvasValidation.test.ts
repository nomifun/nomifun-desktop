/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  CREATIVE_STUDIO_DOCUMENT_SCHEMA,
  CreativeStudioContractError,
  createEmptyCreativeCanvasDocument,
  creativeCanvasDocumentToLegacyProject,
  legacyProjectDocumentToCreativeCanvas,
  parseCreativeCanvasDetailResponse,
  parseCreativeCanvasDocument,
  parseCreativeCanvasListResponse,
  parseSaveCreativeCanvasRequest,
} from '.';

const CANVAS_ID = '0198f8bb-8424-7b3d-8f17-bc6a1676f112';

const summary = {
  canvasId: CANVAS_ID,
  title: '产品视觉探索',
  revision: '12',
  nodeCount: 0,
  connectionCount: 0,
  createdAt: 1_770_000_000_000,
  updatedAt: 1_770_000_100_000,
};

const captureError = (operation: () => unknown): unknown => {
  try {
    operation();
    return null;
  } catch (error) {
    return error;
  }
};

describe('Creative Canvas façade contract', () => {
  test('builds and validates the canonical canvasId document shape', () => {
    const document = createEmptyCreativeCanvasDocument(CANVAS_ID);

    expect(document).toMatchObject({
      schema: CREATIVE_STUDIO_DOCUMENT_SCHEMA,
      canvasId: CANVAS_ID,
      nodes: [],
      connections: [],
    });
    expect(Object.hasOwn(document, 'projectId')).toBe(false);
    expect(parseCreativeCanvasDocument(document, CANVAS_ID)).toEqual(document);
  });

  test('parses list and detail responses without admitting retired wire fields', () => {
    const document = createEmptyCreativeCanvasDocument(CANVAS_ID);

    expect(
      parseCreativeCanvasListResponse({ canvases: [summary] })
    ).toEqual({ canvases: [summary] });
    expect(
      parseCreativeCanvasDetailResponse({ canvas: summary, document })
    ).toEqual({ canvas: summary, document });

    for (const invalid of [
      { projects: [summary] },
      { canvases: [{ ...summary, projectId: CANVAS_ID }] },
      {
        canvas: { ...summary, projectId: CANVAS_ID },
        document,
      },
    ]) {
      expect(
        captureError(() => parseCreativeCanvasListResponse(invalid)) instanceof
          CreativeStudioContractError
      ).toBe(true);
    }
  });

  test('validates canonical CAS saves and keeps legacy conversion explicit', () => {
    const document = createEmptyCreativeCanvasDocument(CANVAS_ID);
    expect(
      parseSaveCreativeCanvasRequest(
        { expectedRevision: '12', document },
        CANVAS_ID
      )
    ).toEqual({ expectedRevision: '12', document });

    const legacy = creativeCanvasDocumentToLegacyProject(document);
    expect(legacy.projectId).toBe(CANVAS_ID);
    expect(Object.hasOwn(legacy, 'canvasId')).toBe(false);
    expect(legacyProjectDocumentToCreativeCanvas(legacy)).toEqual(document);
  });

  test('reports identity drift with Canvas vocabulary', () => {
    const otherCanvasId = '0198f8bb-8424-7b3d-8f17-bc6a1676f113';
    try {
      parseCreativeCanvasDocument(
        createEmptyCreativeCanvasDocument(otherCanvasId),
        CANVAS_ID
      );
      throw new Error('Expected identity mismatch');
    } catch (error) {
      expect(error).toMatchObject({
        name: 'CreativeStudioContractError',
        code: 'CANVAS_MISMATCH',
        path: '$.canvasId',
      });
      expect((error as Error).message.includes('project')).toBe(false);
    }
  });
});
