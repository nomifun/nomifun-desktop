/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { canvasCommands, createInitialCanvasState } from '../core';
import { testDocument, testNode, testUuid } from '../core/testFixtures';
import {
  canvasCommandPreservesPendingTaskOwners,
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
      canvasCommandPreservesPendingTaskOwners(
        state,
        [owner.data.taskId as string],
        canvasCommands.deleteSelection({ nodeIds: [owner.id] }),
      ),
    ).toBe(false);
    expect(
      canvasCommandPreservesPendingTaskOwners(
        state,
        [owner.data.taskId as string],
        canvasCommands.updateNode({
          ...owner,
          data: { ...owner.data, taskId: null },
        }),
      ),
    ).toBe(false);
    expect(
      canvasCommandPreservesPendingTaskOwners(
        state,
        [owner.data.taskId as string],
        canvasCommands.undo(),
      ),
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
      canvasCommandPreservesPendingTaskOwners(
        state,
        taskIds,
        canvasCommands.moveNodes({ x: 10, y: 4 }, { nodeIds: [text.id] }),
      ),
    ).toBe(true);
    expect(
      canvasCommandPreservesPendingTaskOwners(
        state,
        taskIds,
        canvasCommands.reconcileRuntimeNode({
          ...owner,
          data: { ...owner.data, status: 'succeeded' },
        }),
      ),
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
      canvasCommandPreservesPendingTaskOwners(
        state,
        taskIds,
        canvasCommands.deleteSelection({ nodeIds: [source.id] })
      )
    ).toBe(false);
    expect(
      canvasCommandPreservesPendingTaskOwners(
        state,
        taskIds,
        canvasCommands.updateNode({
          ...source,
          data: { ...source.data, assetId: testUuid(40) },
        })
      )
    ).toBe(false);
    expect(
      canvasCommandPreservesPendingTaskOwners(
        state,
        taskIds,
        canvasCommands.moveNodes({ x: 10, y: 4 }, { nodeIds: [source.id] })
      )
    ).toBe(true);
    expect(
      canvasCommandPreservesPendingTaskOwners(
        state,
        taskIds,
        canvasCommands.reconcileRuntimeNode({
          ...source,
          data: { ...source.data, assetId: testUuid(40) },
        })
      )
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
      canvasCommandPreservesPendingTaskOwners(
        state,
        taskIds,
        canvasCommands.deleteSelection({ nodeIds: [source.id] })
      )
    ).toBe(false);
    expect(
      canvasCommandPreservesPendingTaskOwners(
        state,
        taskIds,
        canvasCommands.updateNode({
          ...owner,
          data: { ...owner.data, operation: null },
        })
      )
    ).toBe(false);
    expect(
      canvasCommandPreservesPendingTaskOwners(
        state,
        taskIds,
        canvasCommands.updateNode({
          ...source,
          data: { ...source.data, assetId: testUuid(41) },
        })
      )
    ).toBe(false);
    expect(
      canvasCommandPreservesPendingTaskOwners(
        state,
        taskIds,
        canvasCommands.reconcileRuntimeNode({
          ...source,
          data: { ...source.data, assetId: testUuid(41) },
        })
      )
    ).toBe(true);
  });
});
