/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAsset } from '../../assets';
import type {
  CreativeCanvasNodeDataByKind,
  CreativeCanvasNodeKind,
} from '../../domain';
import type { PromptLibrarySelection } from '../../prompts';
import { createInitialCanvasState } from '../core';
import {
  CREATIVE_CANVAS_PRODUCT_NODE_SIZES,
  CREATIVE_CANVAS_PRODUCT_CASCADE_STEP,
  CreativeCanvasNodeFactoryError,
  createCreativeCanvasProductNode,
  creativeCanvasProductInsertionViewport,
  creativeNodeFromAsset,
  creativeTextNodeFromPrompt,
} from './nodeFactory';

const UUID_V7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const VIEWPORT_SIZE = { width: 1_000, height: 600 };

const expectedData: CreativeCanvasNodeDataByKind = {
  image: {
    assetId: null,
    caption: '',
    alt: '',
    fit: 'contain',
    naturalSize: null,
  },
  panorama: {
    assetId: null,
    projection: 'equirectangular',
    yaw: 0,
    pitch: 0,
    fieldOfView: 75,
  },
  text: {
    text: '',
    format: 'plain',
    fontSize: 14,
    textAlign: 'left',
  },
  config: {
    task: 'image_generation',
    capability: 'text-to-image',
    providerId: null,
    model: null,
    prompt: '',
    negativePrompt: '',
    parameters: {},
    inputAssetIds: [],
    taskId: null,
    resultAssetIds: [],
    status: 'idle',
    errorMessage: null,
  },
  video: {
    assetId: null,
    posterAssetId: null,
    autoplay: false,
    loop: false,
    muted: false,
    trimStartMs: 0,
    trimEndMs: null,
  },
  audio: {
    assetId: null,
    title: '',
    loop: false,
    volume: 1,
    trimStartMs: 0,
    trimEndMs: null,
  },
  director: {
    sceneId: null,
    cameraId: null,
    timelineMs: 0,
    durationMs: 0,
  },
  group: {
    title: '节点组',
    color: null,
    collapsed: false,
  },
};

const asset = (overrides: Partial<CreativeAsset> = {}): CreativeAsset => ({
  id: 'asset-real-1',
  kind: 'image',
  title: '真实素材',
  collection: null,
  tags: [],
  mimeType: 'image/png',
  width: 1_920,
  height: 1_080,
  bytes: 42_000,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: '/api/creative-studio/files/asset-real-1',
  thumbnailUrl: '/api/creative-studio/files/asset-real-1?thumb=1',
  createdAt: 1,
  updatedAt: 2,
  ...overrides,
});

describe('createCreativeCanvasProductNode', () => {
  test('builds all eight canonical payloads with independent bare UUIDv7 ids', () => {
    const state = createInitialCanvasState();
    const kinds: CreativeCanvasNodeKind[] = [
      'text',
      'image',
      'panorama',
      'video',
      'audio',
      'config',
      'director',
      'group',
    ];
    const nodes = kinds.map((kind) =>
      createCreativeCanvasProductNode(kind, state, VIEWPORT_SIZE, { cascadeIndex: 0 })
    );

    expect(new Set(nodes.map((node) => node.id)).size).toBe(8);
    for (const node of nodes) {
      expect(UUID_V7.test(node.id)).toBe(true);
      expect(node.id.includes('node-')).toBe(false);
      expect(node.data).toEqual(expectedData[node.type]);
      expect(node.groupId).toBeNull();
      expect(node.locked).toBe(false);
      expect(node.zIndex).toBe(0);
      expect(node.size).toEqual(CREATIVE_CANVAS_PRODUCT_NODE_SIZES[node.type]);
    }

    const firstConfig = nodes.find((node) => node.type === 'config');
    const secondConfig = createCreativeCanvasProductNode('config', state, VIEWPORT_SIZE);
    if (!firstConfig) throw new Error('config fixture missing');
    firstConfig.data.parameters.width = 1_024;
    expect(secondConfig.data.parameters).toEqual({});
  });

  test('centers in the visible viewport, cascades in client pixels, and appends z-index', () => {
    const empty = createInitialCanvasState({
      viewport: { x: -200, y: -100, zoom: 2 },
    });
    const centered = createCreativeCanvasProductNode('text', empty, VIEWPORT_SIZE, {
      cascadeIndex: 0,
    });
    const oneClientSlot = createCreativeCanvasProductNode('text', empty, VIEWPORT_SIZE, {
      cascadeIndex: 1,
    });
    expect(centered.position).toEqual({ x: 180, y: 80 });
    expect(oneClientSlot.position).toEqual({
      x: 180 + CREATIVE_CANVAS_PRODUCT_CASCADE_STEP / 2,
      y: 80 + CREATIVE_CANVAS_PRODUCT_CASCADE_STEP / 2,
    });

    const high = createCreativeCanvasProductNode('image', empty, VIEWPORT_SIZE, {
      cascadeIndex: 0,
      zIndex: 7,
    });
    const low = createCreativeCanvasProductNode('audio', empty, VIEWPORT_SIZE, {
      cascadeIndex: 0,
      zIndex: -3,
    });
    const populated = createInitialCanvasState({
      document: { nodes: [high, low], connections: [] },
      viewport: empty.viewport,
    });
    const cascaded = createCreativeCanvasProductNode('text', populated, VIEWPORT_SIZE);

    expect(cascaded.position).toEqual({
      x: 180 + CREATIVE_CANVAS_PRODUCT_CASCADE_STEP,
      y: 80 + CREATIVE_CANVAS_PRODUCT_CASCADE_STEP,
    });
    expect(cascaded.zIndex).toBe(8);
  });

  test('centers only the pristine server viewport before the first insertion', () => {
    const pristine = createInitialCanvasState();
    const centeredViewport = creativeCanvasProductInsertionViewport(
      pristine,
      VIEWPORT_SIZE
    );
    expect(centeredViewport).toEqual({ x: 500, y: 300, zoom: 1 });

    const centeredState = createInitialCanvasState({ viewport: centeredViewport });
    expect(
      createCreativeCanvasProductNode('text', centeredState, VIEWPORT_SIZE, {
        cascadeIndex: 0,
      }).position
    ).toEqual({ x: -170, y: -120 });

    const userPanned = createInitialCanvasState({
      viewport: { x: 12, y: -8, zoom: 1 },
    });
    expect(
      creativeCanvasProductInsertionViewport(userPanned, VIEWPORT_SIZE)
    ).toEqual(userPanned.viewport);
  });

  test('accepts explicit safe layout overrides without giving up generated identity', () => {
    const state = createInitialCanvasState();
    const node = createCreativeCanvasProductNode('director', state, VIEWPORT_SIZE, {
      position: { x: 12, y: 34 },
      size: { width: 480, height: 270 },
      zIndex: -4,
      locked: true,
    });

    expect(UUID_V7.test(node.id)).toBe(true);
    expect(node.position).toEqual({ x: 12, y: 34 });
    expect(node.size).toEqual({ width: 480, height: 270 });
    expect(node.zIndex).toBe(-4);
    expect(node.locked).toBe(true);
  });
});

describe('real library insertion helpers', () => {
  test('maps image, video, audio, and text assets without persisting presentation URLs', () => {
    const state = createInitialCanvasState();
    const image = creativeNodeFromAsset(asset(), state, VIEWPORT_SIZE, { cascadeIndex: 0 });
    const video = creativeNodeFromAsset(
      asset({ id: 'asset-video', kind: 'video', title: '原始视频', width: null, height: null }),
      state,
      VIEWPORT_SIZE,
      { cascadeIndex: 1 }
    );
    const audio = creativeNodeFromAsset(
      asset({ id: 'asset-audio', kind: 'audio', title: '环境声', width: null, height: null }),
      state,
      VIEWPORT_SIZE,
      { cascadeIndex: 2 }
    );
    const text = creativeNodeFromAsset(
      asset({
        id: 'asset-text',
        kind: 'text',
        title: '剧本片段',
        textContent: '第一幕\n雨夜。',
        width: null,
        height: null,
      }),
      state,
      VIEWPORT_SIZE,
      { cascadeIndex: 3 }
    );

    expect(image.type).toBe('image');
    if (image.type === 'image') {
      expect(image.data).toEqual({
        assetId: 'asset-real-1',
        caption: '真实素材',
        alt: '真实素材',
        fit: 'contain',
        naturalSize: { width: 1_920, height: 1_080 },
      });
    }
    expect(video.type === 'video' && video.data.posterAssetId).toBeNull();
    expect(audio.type === 'audio' && audio.data.title).toBe('环境声');
    expect(text.type === 'text' && text.data.text).toBe('第一幕\n雨夜。');

    for (const node of [image, video, audio, text]) {
      const persisted = JSON.stringify(node);
      expect(persisted.includes('/api/creative-studio/files/')).toBe(false);
      expect(persisted.includes('thumb=1')).toBe(false);
      expect(UUID_V7.test(node.id)).toBe(true);
    }
  });

  test('rejects missing stable identity or unavailable text instead of inventing content', () => {
    const state = createInitialCanvasState();
    const capture = (input: CreativeAsset) => {
      try {
        creativeNodeFromAsset(input, state, VIEWPORT_SIZE);
        return null;
      } catch (error) {
        return error;
      }
    };

    const missingId = capture(asset({ id: '   ' }));
    const missingText = capture(asset({ kind: 'text', textContent: null }));
    expect(missingId instanceof CreativeCanvasNodeFactoryError).toBe(true);
    expect((missingId as CreativeCanvasNodeFactoryError).code).toBe('asset-id-required');
    expect(missingText instanceof CreativeCanvasNodeFactoryError).toBe(true);
    expect((missingText as CreativeCanvasNodeFactoryError).code).toBe('asset-text-unavailable');
  });

  test('copies a validated prompt verbatim into a text node and no unsupported metadata', () => {
    const prompt: PromptLibrarySelection = {
      id: 'prompt-real-1',
      source: 'preset',
      title: '电影感雨夜',
      prompt: '保留真实材质。\n使用柔和侧光。',
      category: '摄影',
      tags: ['cinematic'],
      knowledgeBaseIds: ['kb-1'],
      coverUrl: null,
      sourceUrl: null,
      license: null,
      licenseUrl: null,
    };
    const original = structuredClone(prompt);
    const node = creativeTextNodeFromPrompt(
      prompt,
      createInitialCanvasState(),
      VIEWPORT_SIZE,
      { cascadeIndex: 0 }
    );

    expect(node.data.text).toBe(prompt.prompt);
    expect(node.data.format).toBe('plain');
    expect(JSON.stringify(node).includes(prompt.id)).toBe(false);
    expect(JSON.stringify(node).includes('kb-1')).toBe(false);
    expect(prompt).toEqual(original);
  });
});
