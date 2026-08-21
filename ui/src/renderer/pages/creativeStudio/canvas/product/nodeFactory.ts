/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from '../../assets';
import type {
  CreativeCanvasNode,
  CreativeCanvasNodeDataByKind,
  CreativeCanvasNodeKind,
  CreativePoint,
  CreativeSize,
} from '../../domain';
import type { PromptLibrarySelection } from '../../prompts';
import {
  clientToCanvas,
  makeCanvasNode,
  normalizeCanvasViewport,
  type CanvasState,
  type CanvasViewport,
} from '../core';

export const CREATIVE_CANVAS_PRODUCT_NODE_SIZES = {
  text: { width: 340, height: 240 },
  image: { width: 340, height: 240 },
  panorama: { width: 340, height: 170 },
  video: { width: 420, height: 236 },
  audio: { width: 340, height: 160 },
  config: { width: 440, height: 240 },
  director: { width: 360, height: 320 },
  group: { width: 760, height: 480 },
} as const satisfies Record<CreativeCanvasNodeKind, CreativeSize>;

/** Repeated insertions move by this many client pixels, independent of zoom. */
export const CREATIVE_CANVAS_PRODUCT_CASCADE_STEP = 28;
export const CREATIVE_CANVAS_PRODUCT_CASCADE_SLOTS = 8;

const DEFAULT_NODE_DATA: CreativeCanvasNodeDataByKind = {
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
    operation: null,
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
    composer: null,
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

export type CreativeCanvasProductState = Pick<CanvasState, 'document' | 'viewport'>;

export interface CreativeCanvasProductNodeOverrides {
  /** Defaults to the current node count and wraps through eight visible slots. */
  cascadeIndex?: number;
  /** Explicit world position. When present it replaces center/cascade placement. */
  position?: CreativePoint;
  size?: CreativeSize;
  zIndex?: number;
  locked?: boolean;
}

export type CreativeCanvasNodeFactoryErrorCode =
  | 'asset-id-required'
  | 'asset-text-unavailable';

export class CreativeCanvasNodeFactoryError extends Error {
  readonly code: CreativeCanvasNodeFactoryErrorCode;

  constructor(code: CreativeCanvasNodeFactoryErrorCode, message: string) {
    super(message);
    this.name = 'CreativeCanvasNodeFactoryError';
    this.code = code;
  }
}

const positiveFinite = (value: number, fallback: number): number =>
  Number.isFinite(value) && value > 0 ? value : fallback;

const finite = (value: number, fallback: number): number =>
  Number.isFinite(value) ? value : fallback;

const normalizeSize = (value: CreativeSize | undefined, fallback: CreativeSize): CreativeSize => ({
  width: positiveFinite(value?.width ?? fallback.width, fallback.width),
  height: positiveFinite(value?.height ?? fallback.height, fallback.height),
});

/**
 * The reference canvas keeps world origin at the visible center. A new
 * server-side project cannot know its eventual client dimensions, so its
 * canonical viewport starts at 0/0. Normalize that one pristine state at the
 * first centered insertion; user panning or zooming an empty canvas is always
 * preserved.
 */
export function creativeCanvasProductInsertionViewport(
  state: CreativeCanvasProductState,
  viewportSize: CreativeSize
): CanvasViewport {
  const viewport = normalizeCanvasViewport(state.viewport);
  const isPristine =
    state.document.nodes.length === 0 &&
    viewport.x === 0 &&
    viewport.y === 0 &&
    viewport.zoom === 1;
  if (!isPristine) return viewport;
  return {
    x: positiveFinite(viewportSize.width, 1) / 2,
    y: positiveFinite(viewportSize.height, 1) / 2,
    zoom: 1,
  };
}

const nextCanvasZIndex = (state: CreativeCanvasProductState): number =>
  state.document.nodes.reduce(
    (highest, node) => Math.max(highest, Number.isInteger(node.zIndex) ? node.zIndex : -1),
    -1
  ) + 1;

const cascadeSlot = (requested: number): number => {
  if (!Number.isFinite(requested)) return 0;
  return Math.max(0, Math.trunc(requested)) % CREATIVE_CANVAS_PRODUCT_CASCADE_SLOTS;
};

/**
 * Place a new node around the visible client-space center. Cascade offsets are
 * expressed in client pixels, so zoom never turns a 28px insertion offset into
 * an unexpectedly large or tiny screen jump.
 */
export function creativeCanvasProductNodePosition(
  state: CreativeCanvasProductState,
  viewportSize: CreativeSize,
  nodeSize: CreativeSize,
  cascadeIndex = state.document.nodes.length
): CreativePoint {
  const width = positiveFinite(viewportSize.width, 1);
  const height = positiveFinite(viewportSize.height, 1);
  const offset = cascadeSlot(cascadeIndex) * CREATIVE_CANVAS_PRODUCT_CASCADE_STEP;
  const worldCenter = clientToCanvas(
    { x: width / 2 + offset, y: height / 2 + offset },
    state.viewport
  );
  return {
    x: worldCenter.x - nodeSize.width / 2,
    y: worldCenter.y - nodeSize.height / 2,
  };
}

const defaultDataFor = <K extends CreativeCanvasNodeKind>(
  kind: K
): CreativeCanvasNodeDataByKind[K] => structuredClone(DEFAULT_NODE_DATA[kind]);

const createNodeWithData = <K extends CreativeCanvasNodeKind>(
  kind: K,
  data: CreativeCanvasNodeDataByKind[K],
  state: CreativeCanvasProductState,
  viewportSize: CreativeSize,
  overrides: CreativeCanvasProductNodeOverrides = {}
): Extract<CreativeCanvasNode, { type: K }> => {
  const defaultSize = CREATIVE_CANVAS_PRODUCT_NODE_SIZES[kind];
  const size = normalizeSize(overrides.size, defaultSize);
  const centeredPosition = creativeCanvasProductNodePosition(
    state,
    viewportSize,
    size,
    overrides.cascadeIndex
  );
  const position = overrides.position
    ? {
        x: finite(overrides.position.x, centeredPosition.x),
        y: finite(overrides.position.y, centeredPosition.y),
      }
    : centeredPosition;
  const automaticZIndex = nextCanvasZIndex(state);
  const zIndex = Number.isInteger(overrides.zIndex)
    ? (overrides.zIndex as number)
    : automaticZIndex;

  return makeCanvasNode({
    type: kind,
    position,
    size,
    zIndex,
    locked: overrides.locked ?? false,
    data: structuredClone(data),
  });
};

/** Build one of the eight canonical empty product nodes with a fresh UUIDv7. */
export function createCreativeCanvasProductNode<K extends CreativeCanvasNodeKind>(
  kind: K,
  state: CreativeCanvasProductState,
  viewportSize: CreativeSize,
  overrides: CreativeCanvasProductNodeOverrides = {}
): Extract<CreativeCanvasNode, { type: K }> {
  return createNodeWithData(
    kind,
    defaultDataFor(kind),
    state,
    viewportSize,
    overrides
  );
}

const requireAssetId = (asset: CreativeAsset): string => {
  if (!asset.id.trim()) {
    throw new CreativeCanvasNodeFactoryError(
      'asset-id-required',
      'A real asset id is required before inserting an asset into the canvas.'
    );
  }
  return asset.id;
};

const naturalImageSize = (asset: CreativeAsset): CreativeSize | null =>
  asset.width !== null &&
  asset.height !== null &&
  Number.isFinite(asset.width) &&
  Number.isFinite(asset.height) &&
  asset.width > 0 &&
  asset.height > 0
    ? { width: asset.width, height: asset.height }
    : null;

/**
 * Convert a validated NomiFun asset to its canonical canvas node. Only the
 * stable asset id and real metadata/content are persisted; original and
 * thumbnail URLs remain presentation concerns.
 */
export function creativeNodeFromAsset(
  asset: CreativeAsset,
  state: CreativeCanvasProductState,
  viewportSize: CreativeSize,
  overrides: CreativeCanvasProductNodeOverrides = {}
): CreativeCanvasNode {
  const assetId = requireAssetId(asset);
  switch (asset.kind) {
    case 'image':
      return createNodeWithData(
        'image',
        {
          assetId,
          caption: asset.title,
          alt: asset.title,
          fit: 'contain',
          naturalSize: naturalImageSize(asset),
          composer: null,
        },
        state,
        viewportSize,
        overrides
      );
    case 'video':
      return createNodeWithData(
        'video',
        {
          assetId,
          // A thumbnail URL is not a durable poster asset id.
          posterAssetId: null,
          autoplay: false,
          loop: false,
          muted: false,
          trimStartMs: 0,
          trimEndMs: null,
          composer: null,
        },
        state,
        viewportSize,
        overrides
      );
    case 'audio':
      return createNodeWithData(
        'audio',
        {
          assetId,
          title: asset.title,
          loop: false,
          volume: 1,
          trimStartMs: 0,
          trimEndMs: null,
        },
        state,
        viewportSize,
        overrides
      );
    case 'text':
      if (asset.textContent === null) {
        throw new CreativeCanvasNodeFactoryError(
          'asset-text-unavailable',
          `Text asset ${assetId} has no real text content to insert.`
        );
      }
      return createNodeWithData(
        'text',
        {
          text: asset.textContent,
          format: 'plain',
          fontSize: 14,
          textAlign: 'left',
        },
        state,
        viewportSize,
        overrides
      );
  }
}

/** Insert the validated prompt verbatim as a canonical text node. */
export function creativeTextNodeFromPrompt(
  prompt: PromptLibrarySelection,
  state: CreativeCanvasProductState,
  viewportSize: CreativeSize,
  overrides: CreativeCanvasProductNodeOverrides = {}
): Extract<CreativeCanvasNode, { type: 'text' }> {
  return createNodeWithData(
    'text',
    {
      text: prompt.prompt,
      format: 'plain',
      fontSize: 14,
      textAlign: 'left',
    },
    state,
    viewportSize,
    overrides
  );
}
