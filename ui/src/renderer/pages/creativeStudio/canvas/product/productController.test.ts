/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAsset } from '../../assets';
import { canvasCommands, canvasReducer, createInitialCanvasState } from '../core';
import { createCreativeCanvasProductNode } from './nodeFactory';
import {
  canLeaveCreativeCanvasAfterFlush,
  creativeCanvasProductSelectionCapabilities,
  resolveCreativeNodeAssetPresentation,
} from './productController';

const VIEWPORT = { width: 1200, height: 800 };

const asset = (overrides: Partial<CreativeAsset> = {}): CreativeAsset => ({
  id: 'asset-image',
  kind: 'image',
  title: '真实图片',
  collection: null,
  tags: [],
  mimeType: 'image/png',
  width: 1280,
  height: 720,
  bytes: 1024,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: 'nomifun://asset/asset-image/original',
  thumbnailUrl: 'nomifun://asset/asset-image/thumbnail',
  createdAt: 1,
  updatedAt: 2,
  ...overrides,
});

describe('Creative Canvas product controller helpers', () => {
  test('derives grouping and deletion affordances from canonical selection', () => {
    let state = createInitialCanvasState();
    const first = createCreativeCanvasProductNode('text', state, VIEWPORT);
    state = canvasReducer(state, canvasCommands.addNode(first, { at: 1 }));
    const second = createCreativeCanvasProductNode('image', state, VIEWPORT);
    state = canvasReducer(state, canvasCommands.addNode(second, { at: 2 }));
    state = canvasReducer(state, canvasCommands.setSelection([first.id, second.id]));

    expect(creativeCanvasProductSelectionCapabilities(state)).toEqual({
      hasSelection: true,
      canGroup: true,
      groupIds: [],
    });

    state = canvasReducer(
      state,
      canvasCommands.groupNodes({
        nodeIds: [first.id, second.id],
        at: 3,
      })
    );
    const capabilities = creativeCanvasProductSelectionCapabilities(state);
    expect(capabilities.hasSelection).toBe(true);
    expect(capabilities.canGroup).toBe(false);
    expect(capabilities.groupIds.length).toBe(1);
  });

  test('allows route leave only after a non-lossy flush result', () => {
    expect(canLeaveCreativeCanvasAfterFlush({ status: 'noop', revision: '7' })).toBe(true);
    expect(canLeaveCreativeCanvasAfterFlush({ status: 'saved', revision: '8' })).toBe(true);
    expect(
      canLeaveCreativeCanvasAfterFlush({
        status: 'conflict',
        revision: '7',
        error: new Error('conflict'),
      })
    ).toBe(false);
    expect(
      canLeaveCreativeCanvasAfterFlush({
        status: 'error',
        revision: '7',
        error: new Error('offline'),
      })
    ).toBe(false);
  });

  test('resolves only real, type-compatible asset URLs', () => {
    const imageAsset = asset();
    const state = createInitialCanvasState();
    const imageNode = createCreativeCanvasProductNode('image', state, VIEWPORT);
    const connectedImage = {
      ...imageNode,
      data: { ...imageNode.data, assetId: imageAsset.id },
    };
    const assets = new Map([[imageAsset.id, imageAsset]]);

    expect(resolveCreativeNodeAssetPresentation(connectedImage, assets)).toEqual({
      src: imageAsset.thumbnailUrl,
      label: imageAsset.title,
      alt: imageAsset.title,
    });

    const audioAsset = asset({ kind: 'audio', mimeType: 'audio/wav' });
    expect(
      resolveCreativeNodeAssetPresentation(
        connectedImage,
        new Map([[audioAsset.id, audioAsset]])
      )
    ).toBeNull();
    expect(
      resolveCreativeNodeAssetPresentation(
        { ...connectedImage, data: { ...connectedImage.data, assetId: 'missing' } },
        assets
      )
    ).toBeNull();
  });
});
