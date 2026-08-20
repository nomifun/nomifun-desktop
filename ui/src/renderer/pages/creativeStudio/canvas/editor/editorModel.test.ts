/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  createEmptyCreativeProjectDocument,
  type CreativeProjectDetail,
} from '../../domain';
import { CreativeProjectRepositoryError } from '../../services';
import { createInitialCanvasState } from '../core';
import { testEdge, testNode, testUuid } from '../core/testFixtures';
import {
  canvasStateFromProjectDocument,
  canvasSurfaceBackground,
  classifyCreativeCanvasLoadState,
  fitCanvasViewport,
  projectDocumentFromCanvasState,
} from './editorModel';

const PROJECT_ID = testUuid(200);

const detail = (): CreativeProjectDetail => ({
  project: {
    projectId: PROJECT_ID,
    title: 'Editor contract',
    revision: '7',
    nodeCount: 0,
    connectionCount: 0,
    createdAt: 1,
    updatedAt: 2,
  },
  document: createEmptyCreativeProjectDocument(PROJECT_ID),
});

describe('creative canvas editor model', () => {
  test('classifies loading, canonical ready, missing, and repository errors', () => {
    expect(
      classifyCreativeCanvasLoadState({
        projectId: PROJECT_ID,
        detail: undefined,
        isLoading: true,
        error: undefined,
      })
    ).toBe('loading');
    expect(
      classifyCreativeCanvasLoadState({
        projectId: PROJECT_ID,
        detail: detail(),
        isLoading: false,
        error: undefined,
      })
    ).toBe('ready');
    expect(
      classifyCreativeCanvasLoadState({
        projectId: PROJECT_ID,
        detail: undefined,
        isLoading: false,
        error: undefined,
      })
    ).toBe('not-found');
    expect(
      classifyCreativeCanvasLoadState({
        projectId: PROJECT_ID,
        detail: undefined,
        isLoading: false,
        error: new CreativeProjectRepositoryError({
          kind: 'not-found',
          message: 'missing',
        }),
      })
    ).toBe('not-found');
    expect(
      classifyCreativeCanvasLoadState({
        projectId: PROJECT_ID,
        detail: undefined,
        isLoading: false,
        error: new CreativeProjectRepositoryError({
          kind: 'transport',
          message: 'offline',
        }),
      })
    ).toBe('error');
  });

  test('round-trips only the canonical canvas fields and preserves project metadata', () => {
    const first = testNode('image', 201, { x: 10, y: 20 });
    const second = testNode('director', 202, { x: 300, y: 120 });
    const connection = testEdge(203, first.id, second.id);
    const base = {
      ...createEmptyCreativeProjectDocument(PROJECT_ID),
      background: 'lines' as const,
      nodes: [first, second],
      connections: [connection],
      pendingTaskIds: [testUuid(204)],
    };
    const state = canvasStateFromProjectDocument(base);
    const moved = createInitialCanvasState({
      document: {
        nodes: [{ ...first, position: { x: 40, y: 50 } }, second],
        connections: [connection],
      },
      viewport: { x: 12, y: 24, zoom: 1.5 },
    });
    const result = projectDocumentFromCanvasState(base, moved);

    expect(state.document.nodes).toEqual(base.nodes);
    expect(state.document.connections).toEqual(base.connections);
    expect(result.viewport).toEqual({ x: 12, y: 24, zoom: 1.5 });
    expect(result.nodes[0]?.position).toEqual({ x: 40, y: 50 });
    expect(result.connections).toEqual([connection]);
    expect(result.background).toBe('lines');
    expect(result.pendingTaskIds).toEqual([testUuid(204)]);
    expect(result.schema).toBe(base.schema);
    expect(result.projectId).toBe(PROJECT_ID);
  });

  test('passes the three canonical background modes directly to the surface', () => {
    expect(
      (['dots', 'lines', 'blank'] as const).map((background) =>
        canvasSurfaceBackground(background)
      )
    ).toEqual(['dots', 'lines', 'blank']);
  });

  test('fits graph bounds with padding and centers an empty canvas', () => {
    const state = createInitialCanvasState({
      document: {
        nodes: [
          testNode('image', 205, { x: 100, y: 50, width: 200, height: 100 }),
          testNode('text', 206, { x: 300, y: 150, width: 200, height: 100 }),
        ],
        connections: [],
      },
    });

    expect(fitCanvasViewport(state, { width: 1_000, height: 600 }, 100)).toEqual({
      x: -100,
      y: 0,
      zoom: 2,
    });
    expect(
      fitCanvasViewport(createInitialCanvasState(), { width: 1_000, height: 600 })
    ).toEqual({ x: 500, y: 300, zoom: 1 });
  });
});
