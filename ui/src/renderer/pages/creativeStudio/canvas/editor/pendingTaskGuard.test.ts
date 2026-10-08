/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { canvasCommands, createInitialCanvasState } from '../core';
import { testDocument, testEdge, testNode, testUuid } from '../core/testFixtures';
import {
  pendingTaskCommandGuard,
} from './pendingTaskGuard';

const pendingConfig = () => {
  const node = testNode('config', 1);
  return {
    ...node,
    data: {
      ...node.data,
      providerId: testUuid(20),
      model: 'image-edit-model',
      task: 'image_edit' as const,
      capability: 'i2i',
      taskId: testUuid(9),
      status: 'running' as const,
    },
  };
};

const pendingImageCompose = () => {
  const source = testNode('image', 2);
  const owner = pendingConfig();
  return {
    source,
    owner: {
      ...owner,
      data: {
        ...owner.data,
        task: 'image_generation' as const,
        capability: 't2i',
        operation: {
          kind: 'image-node-compose' as const,
          sourceNodeId: source.id,
          sourceAssetId: null,
        },
        parameters: {},
      },
    },
  };
};

describe('canvas pending task owner guard', () => {
  test('blocks delete, owner mutation, and undo that would orphan a pending task', () => {
    const owner = pendingConfig();
    const state = createInitialCanvasState({
      document: testDocument([owner]),
    });
    state.history.past = [testDocument()];

    expect(
      pendingTaskCommandGuard(state, canvasCommands.deleteSelection({ nodeIds: [owner.id] }), [owner.data.taskId as string]).allowed,
    ).toBe(false);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.updateNode({
          ...owner,
          data: { ...owner.data, taskId: null },
        }), [owner.data.taskId as string]).allowed,
    ).toBe(false);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.undo(), [owner.data.taskId as string]).allowed,
    ).toBe(false);
  });

  test('allows unrelated edits and authoritative owner reconciliation', () => {
    const owner = pendingConfig();
    const text = testNode('text', 2);
    const state = createInitialCanvasState({
      document: testDocument([owner, text]),
    });
    const taskIds = [owner.data.taskId as string];

    expect(
      pendingTaskCommandGuard(state, canvasCommands.moveNodes({ x: 10, y: 4 }, { nodeIds: [text.id] }), taskIds).allowed,
    ).toBe(true);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.reconcileRuntimeNode({
          ...owner,
          data: { ...owner.data, status: 'succeeded' },
        }), taskIds).allowed,
    ).toBe(true);
  });

  test('reports the exact pending task ids orphaned by a command', () => {
    const owner = pendingConfig();
    const state = createInitialCanvasState({ document: testDocument([owner]) });

    expect(
      pendingTaskCommandGuard(
        state,
        canvasCommands.deleteSelection({ nodeIds: [owner.id] }),
        [owner.data.taskId as string, testUuid(99)]
      )
    ).toEqual({
      allowed: false,
      orphanedTaskIds: [owner.data.taskId, testUuid(99)],
    });
  });

  test('protects a pending image compose source but allows authoritative settlement', () => {
    const { owner, source } = pendingImageCompose();
    const state = createInitialCanvasState({
      document: testDocument([owner, source]),
    });
    const taskIds = [owner.data.taskId as string];

    expect(
      pendingTaskCommandGuard(state, canvasCommands.deleteSelection({ nodeIds: [source.id] }), taskIds).allowed
    ).toBe(false);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.updateNode({
          ...source,
          data: { ...source.data, assetId: testUuid(40) },
        }), taskIds).allowed
    ).toBe(false);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.moveNodes({ x: 10, y: 4 }, { nodeIds: [source.id] }), taskIds).allowed
    ).toBe(true);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.reconcileRuntimeNode({
          ...source,
          data: { ...source.data, assetId: testUuid(40) },
        }), taskIds).allowed
    ).toBe(true);
  });

  test('protects connected multi-image inputs and their lineage while pending', () => {
    const { owner: baseOwner, source } = pendingImageCompose();
    const reference = testNode('image', 42);
    reference.data.assetId = testUuid(43);
    const owner = {
      ...baseOwner,
      data: {
        ...baseOwner.data,
        task: 'image_edit' as const,
        capability: 'i2i',
        inputAssetIds: [reference.data.assetId],
      },
    };
    const edge = testEdge(44, reference.id, source.id);
    const state = createInitialCanvasState({
      document: testDocument([owner, source, reference], [edge]),
    });
    const taskIds = [owner.data.taskId as string];

    expect(
      pendingTaskCommandGuard(state, canvasCommands.deleteSelection({ nodeIds: [reference.id] }), taskIds).allowed
    ).toBe(false);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.deleteEdges([edge.id]), taskIds).allowed
    ).toBe(false);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.moveNodes({ x: 8, y: 4 }, { nodeIds: [reference.id] }), taskIds).allowed
    ).toBe(true);
  });

  test('protects a pending video compose source and rejects operation mutation', () => {
    const source = testNode('video', 3);
    const config = pendingConfig();
    const owner = {
      ...config,
      data: {
        ...config.data,
        task: 'video_generation' as const,
        capability: 't2v',
        operation: {
          kind: 'video-node-compose' as const,
          sourceNodeId: source.id,
          sourceAssetId: null,
        },
      },
    };
    const state = createInitialCanvasState({ document: testDocument([owner, source]) });
    const taskIds = [owner.data.taskId as string];

    expect(
      pendingTaskCommandGuard(state, canvasCommands.deleteSelection({ nodeIds: [source.id] }), taskIds).allowed
    ).toBe(false);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.updateNode({
          ...owner,
          data: { ...owner.data, operation: null },
        }), taskIds).allowed
    ).toBe(false);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.updateNode({
          ...source,
          data: { ...source.data, assetId: testUuid(41) },
        }), taskIds).allowed
    ).toBe(false);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.reconcileRuntimeNode({
          ...source,
          data: { ...source.data, assetId: testUuid(41) },
        }), taskIds).allowed
    ).toBe(true);
  });

  test('protects a pending audio compose source and allows authoritative settlement', () => {
    const source = testNode('audio', 4);
    const config = pendingConfig();
    const owner = {
      ...config,
      data: {
        ...config.data,
        task: 'speech_synthesis' as const,
        capability: 'tts',
        operation: {
          kind: 'audio-node-compose' as const,
          sourceNodeId: source.id,
          sourceAssetId: null,
        },
      },
    };
    const state = createInitialCanvasState({
      document: testDocument([owner, source]),
    });
    const taskIds = [owner.data.taskId as string];

    expect(
      pendingTaskCommandGuard(state, canvasCommands.deleteSelection({ nodeIds: [source.id] }), taskIds).allowed
    ).toBe(false);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.updateNode({
          ...source,
          data: { ...source.data, assetId: testUuid(42) },
        }), taskIds).allowed
    ).toBe(false);
    expect(
      pendingTaskCommandGuard(state, canvasCommands.reconcileRuntimeNode({
          ...source,
          data: { ...source.data, assetId: testUuid(42) },
        }), taskIds).allowed
    ).toBe(true);
  });
});
