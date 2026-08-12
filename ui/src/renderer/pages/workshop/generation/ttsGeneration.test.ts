/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ReactFlowInstance } from '@xyflow/react';
import { parseAssetId, parseWorkshopNodeId, type AssetId } from '@/common/types/ids';
import type { WorkshopFlowEdge, WorkshopFlowNode } from '../canvas/model';
import { buildTaskParams, DEFAULT_TTS_PARAMS, readTtsParams } from './genConstants';
import { buildRunPlan } from './pipeline';
import { spawnResultNodes } from './spawn';
import { generationModeForTask } from './useGenerationRun';

const asset = (label: string): AssetId => {
  const suffix = Array.from(label)
    .map((char) => char.charCodeAt(0).toString(16).padStart(2, '0'))
    .join('')
    .slice(0, 12)
    .padEnd(12, '0');
  return parseAssetId(`019b0000-0000-7000-8000-${suffix}`);
};

const cardId = parseWorkshopNodeId('019b0000-0000-7000-8000-000000000001');

const ttsCard = (): WorkshopFlowNode =>
  ({
    id: cardId,
    type: 'generator',
    position: { x: 0, y: 0 },
    width: 344,
    data: { mode: 'tts', prompt: 'hello there', params: { voice: 'nova' }, mentions: [], status: 'idle', resultAssetIds: [] },
  }) as WorkshopFlowNode;

describe('workshop tts mode', () => {
  test('voice params tolerate missing/blank values and pass custom ids through', () => {
    expect(readTtsParams({})).toEqual(DEFAULT_TTS_PARAMS);
    expect(readTtsParams({ voice: '  ' })).toEqual(DEFAULT_TTS_PARAMS);
    expect(readTtsParams({ voice: 'my-custom-voice' })).toEqual({ voice: 'my-custom-voice' });
    expect(DEFAULT_TTS_PARAMS.voice).toBe('');
  });

  test('buildTaskParams sends exactly {prompt, voice} for tts', () => {
    expect(buildTaskParams('tts', { voice: 'shimmer', width: 512 }, 'say hi')).toEqual({
      prompt: 'say hi',
      voice: 'shimmer',
    });
  });

  test('run plan derives capability tts with no reference inputs', async () => {
    const plan = await buildRunPlan({
      node: ttsCard(),
      nodes: [ttsCard()],
      edges: [],
      mode: 'tts',
      mentions: [],
      basePrompt: 'hello there',
    });

    expect(plan.capability).toBe('tts');
    expect(plan.inputs).toEqual([]);
    expect(plan.referenceCount).toBe(0);
    expect(plan.prompt).toBe('hello there');
  });

  test('the persisted tts capability owns the result mode', () => {
    expect(generationModeForTask({ capability: 'tts' })).toBe('tts');
  });

  test('tts results never fan out canvas nodes (no audio node kind)', async () => {
    const added: WorkshopFlowNode[] = [];
    const rf = {
      getNode: () => undefined,
      addNodes: (input: WorkshopFlowNode | WorkshopFlowNode[]) => {
        added.push(...(Array.isArray(input) ? input : [input]));
      },
      addEdges: () => {},
    } as unknown as ReactFlowInstance<WorkshopFlowNode, WorkshopFlowEdge>;

    await spawnResultNodes(rf, ttsCard(), 'tts', [asset('audio_a'), asset('audio_b')]);

    expect(added).toEqual([]);
  });
});
