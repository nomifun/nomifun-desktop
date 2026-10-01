/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { CreativeCanvasNode } from '../../domain';
import { testNode, testUuid } from '../core/testFixtures';
import { canvasNodeGenerationTaskSummary, pendingCanvasNodeGenerationConfigs } from './canvasNodeGeneration';

type ConfigNode = Extract<CreativeCanvasNode, { type: 'config' }>;
const config = (id: number, source: number, status: ConfigNode['data']['status']): ConfigNode => {
  const base = testNode('config', id);
  return { ...base, data: {
    ...base.data, task: 'image_generation', capability: 't2i',
    operation: { kind: 'image-node-compose', sourceNodeId: testUuid(source), sourceAssetId: null },
    taskId: testUuid(id + 1000), status,
  } };
};

describe('canvas generation boundaries by authored node', () => {
  test('each queued or running task blocks only its source node', () => {
    const first = config(1, 10, 'queued');
    const second = config(2, 11, 'running');
    const document = { nodes: [first, second] };
    const pendingIds = [first.data.taskId!, second.data.taskId!];
    expect(pendingCanvasNodeGenerationConfigs(document, testUuid(10), pendingIds)).toEqual([first]);
    expect(pendingCanvasNodeGenerationConfigs(document, testUuid(11), pendingIds)).toEqual([second]);
    expect(canvasNodeGenerationTaskSummary(document, testUuid(12), null, pendingIds)).toMatchObject({ state: 'idle', pendingCount: 0 });
  });

  test('an older pending task still blocks its source when the latest config is terminal', () => {
    const older = config(1, 10, 'running');
    const latest = config(2, 10, 'succeeded');
    const document = { nodes: [older, latest] };
    expect(canvasNodeGenerationTaskSummary(document, testUuid(10), latest)).toMatchObject({ state: 'running', pendingCount: 1 });
    expect(canvasNodeGenerationTaskSummary(document, testUuid(11), null)).toMatchObject({ state: 'idle', pendingCount: 0 });
  });

  test.each(['succeeded', 'failed', 'canceled'] as const)('keeps %s settlement pending until durable cleanup completes', (status) => {
    const owner = config(1, 10, status);
    const document = { nodes: [owner] };
    expect(canvasNodeGenerationTaskSummary(document, testUuid(10), owner, [owner.data.taskId!])).toMatchObject({ state: status, pendingCount: 1 });
    expect(canvasNodeGenerationTaskSummary(document, testUuid(10), owner)).toMatchObject({ state: status, pendingCount: 0 });
    expect(canvasNodeGenerationTaskSummary(document, testUuid(10), owner, [], true)).toMatchObject({ state: status, pendingCount: 1 });
  });

  test('mask edits and inline image generation share the same source boundary', () => {
    const owner = config(1, 10, 'running');
    owner.data.task = 'image_edit';
    owner.data.capability = 'i2i';
    owner.data.operation = { kind: 'image-mask-edit', sourceNodeId: testUuid(10), sourceAssetId: testUuid(20), markedReferenceAssetId: testUuid(21) };
    expect(canvasNodeGenerationTaskSummary({ nodes: [owner] }, testUuid(10), null)).toMatchObject({ state: 'running', pendingCount: 1 });
    expect(canvasNodeGenerationTaskSummary({ nodes: [owner] }, testUuid(11), null)).toMatchObject({ state: 'idle', pendingCount: 0 });
  });
});
