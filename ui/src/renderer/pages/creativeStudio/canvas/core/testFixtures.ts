/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeCanvasNodeDataByKind } from '../../domain';
import { makeCanvasNode } from './document';
import type {
  CanvasDocument,
  CanvasEdge,
  CanvasIdFactory,
  CanvasNode,
  CanvasNodeType,
} from './types';

export function testUuid(index: number): string {
  return `019b0000-0000-7000-8000-${index.toString(16).padStart(12, '0')}`;
}

export function sequentialTestIdFactory(start = 1_000): CanvasIdFactory {
  let current = start;
  return () => testUuid(current++);
}

const TEST_DATA: CreativeCanvasNodeDataByKind = {
  image: {
    assetId: null,
    caption: '',
    alt: '',
    fit: 'contain',
    naturalSize: null,
    composer: null,
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
    title: 'Group',
    color: null,
    collapsed: false,
  },
};

export function testNode<K extends CanvasNodeType>(
  type: K,
  index: number,
  options: {
    x?: number;
    y?: number;
    width?: number;
    height?: number;
    groupId?: string | null;
    locked?: boolean;
    zIndex?: number;
  } = {}
): Extract<CanvasNode, { type: K }> {
  return makeCanvasNode({
    id: testUuid(index),
    type,
    position: { x: options.x ?? 0, y: options.y ?? 0 },
    size: { width: options.width ?? 100, height: options.height ?? 80 },
    groupId: options.groupId,
    locked: options.locked,
    zIndex: options.zIndex,
    data: structuredClone(TEST_DATA[type]),
  });
}

export function testEdge(
  index: number,
  sourceNodeId: string,
  targetNodeId: string
): CanvasEdge {
  return {
    id: testUuid(index),
    sourceNodeId,
    targetNodeId,
    sourceHandle: null,
    targetHandle: null,
  };
}

export function testDocument(
  nodes: CanvasNode[] = [],
  connections: CanvasEdge[] = []
): CanvasDocument {
  return { nodes, connections };
}
