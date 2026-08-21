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
  canonicalCreativePendingTaskIds,
  classifyCreativeCanvasLoadState,
  creativeStudioPanelStateEqual,
  fitCanvasViewport,
  projectDocumentFromCanvasState,
  projectDocumentWithAgentSessions,
  projectDocumentWithCanvasPanels,
  projectDocumentWithPendingTaskIds,
  shouldHydrateCreativeCanvasDetail,
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

  test('replaces an idle cached revision after mount without overwriting local work', () => {
    const incoming = detail();
    expect(
      shouldHydrateCreativeCanvasDetail({
        projectId: PROJECT_ID,
        loadedProjectId: null,
        loadedRevision: null,
        detail: incoming,
        save: { status: 'idle', hasPendingChanges: false },
      })
    ).toBe(true);
    expect(
      shouldHydrateCreativeCanvasDetail({
        projectId: PROJECT_ID,
        loadedProjectId: PROJECT_ID,
        loadedRevision: '6',
        detail: incoming,
        save: { status: 'idle', hasPendingChanges: false },
      })
    ).toBe(true);
    expect(
      shouldHydrateCreativeCanvasDetail({
        projectId: PROJECT_ID,
        loadedProjectId: PROJECT_ID,
        loadedRevision: '7',
        detail: incoming,
        save: { status: 'idle', hasPendingChanges: false },
      })
    ).toBe(false);
    for (const status of ['dirty', 'saving', 'saved', 'conflict', 'error'] as const) {
      expect(
        shouldHydrateCreativeCanvasDetail({
          projectId: PROJECT_ID,
          loadedProjectId: PROJECT_ID,
          loadedRevision: '6',
          detail: incoming,
          save: { status, hasPendingChanges: status !== 'saved' },
        })
      ).toBe(false);
    }
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

  test('round-trips canonical panel state without dropping reducer-owned canvas changes', () => {
    const base = createEmptyCreativeProjectDocument(PROJECT_ID);
    const node = testNode('text', 207, { x: 40, y: 60 });
    const state = createInitialCanvasState({
      document: { nodes: [node], connections: [] },
      viewport: { x: 18, y: 22, zoom: 1.25 },
    });
    const panels = {
      left: { ...base.panels.left, width: 216, activeView: 'assets' as const },
      right: { ...base.panels.right, open: true, activeView: 'properties' as const },
      bottom: { ...base.panels.bottom, open: true, activeView: 'history' as const },
    };

    const saved = projectDocumentWithCanvasPanels(base, state, panels);
    const reloaded = structuredClone(saved);

    expect(reloaded.panels).toEqual(panels);
    expect(reloaded.nodes).toEqual([node]);
    expect(reloaded.viewport).toEqual({ x: 18, y: 22, zoom: 1.25 });
    expect(creativeStudioPanelStateEqual(reloaded.panels, panels)).toBe(true);
    expect(creativeStudioPanelStateEqual(reloaded.panels, base.panels)).toBe(false);
    expect(base.panels).not.toEqual(panels);
  });

  test('persists a canonical unique pending-task recovery feed with current canvas state', () => {
    const base = createEmptyCreativeProjectDocument(PROJECT_ID);
    const firstTaskId = testUuid(208);
    const secondTaskId = testUuid(209);
    const node = testNode('config', 210);
    const state = createInitialCanvasState({
      document: { nodes: [node], connections: [] },
      viewport: { x: 3, y: 4, zoom: 1.1 },
    });

    const saved = projectDocumentWithPendingTaskIds(
      base,
      state,
      [firstTaskId, secondTaskId, firstTaskId]
    );

    expect(saved.pendingTaskIds).toEqual([firstTaskId, secondTaskId]);
    expect(saved.nodes).toEqual([node]);
    expect(saved.viewport).toEqual({ x: 3, y: 4, zoom: 1.1 });
    expect(canonicalCreativePendingTaskIds([firstTaskId, firstTaskId])).toEqual([
      firstTaskId,
    ]);
    let invalidTaskIdError: unknown;
    try {
      canonicalCreativePendingTaskIds(['not-a-task-id']);
    } catch (error) {
      invalidTaskIdError = error;
    }
    expect(invalidTaskIdError instanceof TypeError).toBe(true);
  });

  test('persists validated Agent references with the latest canvas state', () => {
    const base = createEmptyCreativeProjectDocument(PROJECT_ID);
    const sessionId = testUuid(211);
    const node = testNode('text', 212);
    const state = createInitialCanvasState({
      document: { nodes: [node], connections: [] },
      viewport: { x: 9, y: 11, zoom: 1.2 },
    });
    const sessions = [
      {
        id: sessionId,
        title: '海报创作',
        messageIds: [],
        model: { providerId: testUuid(213), model: 'nomi-chat' },
        pendingTurn: {
          idempotencyKey: testUuid(214),
          prompt: '继续制作海报',
          createdAt: 10,
        },
        createdAt: 1,
        updatedAt: 10,
      },
    ];

    const saved = projectDocumentWithAgentSessions(base, state, sessions, sessionId);

    expect(saved.chatSessions).toEqual(sessions);
    expect(saved.activeChatId).toBe(sessionId);
    expect(saved.nodes).toEqual([node]);
    expect(saved.viewport).toEqual({ x: 9, y: 11, zoom: 1.2 });
    expect(base.chatSessions).toEqual([]);

    let invalidActiveSession: unknown;
    try {
      projectDocumentWithAgentSessions(base, state, sessions, testUuid(215));
    } catch (error) {
      invalidActiveSession = error;
    }
    expect(invalidActiveSession instanceof Error).toBe(true);
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
