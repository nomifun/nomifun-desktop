/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  parseAssetId,
  parseWorkshopEdgeId,
  parseWorkshopNodeId,
} from '@/common/types/ids';
import type { WorkshopFlowEdge, WorkshopFlowNode } from '../canvas/model';
import {
  buildRunPlan,
  imageGeneratorTaskForInputs,
  mentionRefForAsset,
} from './pipeline';
import {
  exactGeneratorTaskPool,
  generatorTaskForMode,
  type GeneratorTaskPools,
} from './useGeneratorModels';

const cardId = parseWorkshopNodeId('019b0000-0000-7000-8000-000000000001');
const imageNodeId = parseWorkshopNodeId('019b0000-0000-7000-8000-000000000002');
const imageAssetId = parseAssetId('019b0000-0000-7000-8000-000000000003');
const maskAssetId = parseAssetId('019b0000-0000-7000-8000-000000000004');

const card = (): WorkshopFlowNode =>
  ({
    id: cardId,
    type: 'generator',
    position: { x: 200, y: 0 },
    data: {
      mode: 'image',
      prompt: 'paint it',
      params: {},
      mentions: [],
      status: 'idle',
      resultAssetIds: [],
    },
  }) as WorkshopFlowNode;

const imageNode = (): WorkshopFlowNode =>
  ({
    id: imageNodeId,
    type: 'image',
    position: { x: 0, y: 0 },
    data: { assetId: imageAssetId },
  }) as WorkshopFlowNode;

const imageEdge = (): WorkshopFlowEdge => ({
  id: parseWorkshopEdgeId('019b0000-0000-7000-8000-000000000005'),
  source: imageNodeId,
  target: cardId,
});

describe('workshop image model task follows the live run inputs', () => {
  test('adding and removing an upstream image switches generation -> edit -> generation', async () => {
    const nodes = [imageNode(), card()];
    const noReference = {
      nodeId: cardId,
      nodes,
      edges: [] as WorkshopFlowEdge[],
      mentions: [] as string[],
    };
    expect(imageGeneratorTaskForInputs(noReference)).toBe('image_generation');
    expect(
      imageGeneratorTaskForInputs({ ...noReference, edges: [imageEdge()] })
    ).toBe('image_edit');
    expect(imageGeneratorTaskForInputs({ ...noReference, edges: [] })).toBe(
      'image_generation'
    );

    const t2iPlan = await buildRunPlan({
      node: card(),
      nodes,
      edges: [],
      mode: 'image',
      mentions: [],
      basePrompt: 'paint it',
    });
    const i2iPlan = await buildRunPlan({
      node: card(),
      nodes,
      edges: [imageEdge()],
      mode: 'image',
      mentions: [],
      basePrompt: 'paint it',
    });
    expect(t2iPlan.capability).toBe('t2i');
    expect(i2iPlan.capability).toBe('i2i');
  });

  test('an image mention or mask requires image_edit, while clearing both restores generation', async () => {
    const nodes = [card()];
    const mention = mentionRefForAsset('image', imageAssetId);
    expect(
      imageGeneratorTaskForInputs({ nodeId: cardId, nodes, edges: [], mentions: [mention] })
    ).toBe('image_edit');
    expect(
      imageGeneratorTaskForInputs({
        nodeId: cardId,
        nodes,
        edges: [],
        mentions: [],
        maskAssetId,
      })
    ).toBe('image_edit');
    expect(
      imageGeneratorTaskForInputs({ nodeId: cardId, nodes, edges: [], mentions: [] })
    ).toBe('image_generation');

    const inpaintPlan = await buildRunPlan({
      node: card(),
      nodes,
      edges: [],
      mode: 'image',
      mentions: [],
      maskAssetId,
      basePrompt: 'paint it',
    });
    expect(inpaintPlan.capability).toBe('inpaint');
  });

  test('the picker selects one exact task pool and never unions or falls back', () => {
    const pools: GeneratorTaskPools<string> = {
      chat: ['chat'],
      speech_synthesis: ['tts'],
      video_generation: ['video'],
      image_generation: ['generation-only'],
      image_edit: ['edit-only'],
    };
    expect(generatorTaskForMode('image', 'image_generation')).toBe('image_generation');
    expect(generatorTaskForMode('image', 'image_edit')).toBe('image_edit');
    expect(exactGeneratorTaskPool('image', 'image_generation', pools)).toEqual([
      'generation-only',
    ]);
    expect(exactGeneratorTaskPool('image', 'image_edit', pools)).toEqual(['edit-only']);
  });
});

