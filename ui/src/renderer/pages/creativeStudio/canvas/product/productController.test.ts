/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAsset } from '../../assets';
import { createEmptyCreativeProjectDocument } from '../../domain';
import { canvasCommands, canvasReducer, createInitialCanvasState } from '../core';
import { createCreativeCanvasProductNode } from './nodeFactory';
import {
  clampCreativeCanvasRightPanelWidth,
  CREATIVE_CANVAS_SOURCE_LEFT_PANEL_WIDTH,
  canLeaveCreativeCanvasAfterFlush,
  creativeCanvasBlockedLeaveMessage,
  creativeCanvasProductPanelViews,
  creativeCanvasProductSelectionCapabilities,
  creativeCanvasSaveDisplayMessage,
  resolveCreativeNodeAssetPresentation,
  withCreativeCanvasBottomView,
  withCreativeCanvasLeftPanelOpen,
  withCreativeCanvasLeftView,
  withCreativeCanvasRightView,
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
  test('projects and updates canonical panel views without losing persisted dimensions', () => {
    const document = createEmptyCreativeProjectDocument(
      '019b0000-0000-7000-8000-000000000001'
    );
    const initial = structuredClone(document.panels);

    const left = withCreativeCanvasLeftView(initial, 'assets');
    const right = withCreativeCanvasRightView(left, 'properties');
    const bottom = withCreativeCanvasBottomView(right, 'timeline');
    const closed = withCreativeCanvasRightView(bottom, null);

    expect(creativeCanvasProductPanelViews(initial)).toEqual({
      left: 'canvas',
      right: null,
      bottom: null,
    });
    expect(creativeCanvasProductPanelViews(bottom)).toEqual({
      left: 'assets',
      right: 'properties',
      bottom: 'timeline',
    });
    expect(creativeCanvasProductPanelViews(closed).right).toBeNull();
    expect(closed.right.activeView).toBe('properties');
    expect(closed.left.width).toBe(CREATIVE_CANVAS_SOURCE_LEFT_PANEL_WIDTH);
    expect(closed.right.width).toBe(initial.right.width);
    expect(closed.bottom.height).toBe(initial.bottom.height);
    expect(initial).toEqual(document.panels);

    const staleGeometry = {
      ...initial,
      left: { ...initial.left, width: 216 },
      right: { ...initial.right, width: 340 },
    };
    const sourceLeft = withCreativeCanvasLeftView(staleGeometry, 'canvas');
    const collapsed = withCreativeCanvasLeftPanelOpen(sourceLeft, false);
    const assistant = withCreativeCanvasRightView(staleGeometry, 'assistant');
    expect(sourceLeft.left.width).toBe(CREATIVE_CANVAS_SOURCE_LEFT_PANEL_WIDTH);
    expect(collapsed.left.open).toBe(false);
    expect(collapsed.left.width).toBe(CREATIVE_CANVAS_SOURCE_LEFT_PANEL_WIDTH);
    expect(assistant.right.width).toBe(
      clampCreativeCanvasRightPanelWidth(staleGeometry.right.width)
    );
  });

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
    const conflict = {
      status: 'conflict' as const,
      revision: '7',
      error: new Error('internal revision conflict for project-id'),
    };
    const error = {
      status: 'error' as const,
      revision: '7',
      error: new Error('offline'),
    };
    expect(canLeaveCreativeCanvasAfterFlush(conflict)).toBe(false);
    expect(canLeaveCreativeCanvasAfterFlush(error)).toBe(false);
    expect(creativeCanvasBlockedLeaveMessage(conflict)).toBe(
      '远端画布已更新，本地更改未覆盖。请先重新载入远端版本。'
    );
    expect(creativeCanvasBlockedLeaveMessage(error)).toBe('offline');
    expect(
      creativeCanvasSaveDisplayMessage({
        status: 'conflict',
        revision: '7',
        hasPendingChanges: true,
        error: conflict.error,
      })
    ).toBe('远端画布已更新，本地更改未覆盖。');
    expect(
      creativeCanvasSaveDisplayMessage({
        status: 'saved',
        revision: '8',
        hasPendingChanges: false,
        error: null,
      })
    ).toBeUndefined();
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
      originalSrc: imageAsset.originalUrl,
      label: imageAsset.title,
      alt: imageAsset.title,
    });
    expect(resolveCreativeNodeAssetPresentation(connectedImage, new Map([
      [imageAsset.id, { ...imageAsset, deletedAt: 50 }],
    ]))).toEqual({ src: '', label: imageAsset.title, deleted: true });

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

  test('prefers a video node’s explicit image poster without changing its playable source', () => {
    const videoAsset = asset({
      id: 'asset-video',
      kind: 'video',
      mimeType: 'video/mp4',
      originalUrl: '/video.mp4',
      thumbnailUrl: '/video-thumbnail.jpg',
    });
    const posterAsset = asset({
      id: 'asset-poster',
      originalUrl: '/poster-original.png',
      thumbnailUrl: '/poster-thumbnail.jpg',
    });
    const node = createCreativeCanvasProductNode('video', createInitialCanvasState(), VIEWPORT);
    const videoNode = {
      ...node,
      data: { ...node.data, assetId: videoAsset.id, posterAssetId: posterAsset.id },
    };

    expect(resolveCreativeNodeAssetPresentation(videoNode, new Map([
      [videoAsset.id, videoAsset],
      [posterAsset.id, posterAsset],
    ]))).toEqual({
      src: videoAsset.originalUrl,
      posterSrc: posterAsset.originalUrl,
      label: videoAsset.title,
    });
  });

  test('falls back to the video thumbnail when its explicit poster is unavailable', () => {
    const videoAsset = asset({
      id: 'asset-video',
      kind: 'video',
      mimeType: 'video/mp4',
      originalUrl: '/video.mp4',
      thumbnailUrl: '/video-thumbnail.jpg',
    });
    const posterAsset = asset({ id: 'asset-poster', originalUrl: '/poster.png' });
    const node = createCreativeCanvasProductNode('video', createInitialCanvasState(), VIEWPORT);
    const videoNode = {
      ...node,
      data: { ...node.data, assetId: videoAsset.id, posterAssetId: posterAsset.id },
    };
    const invalidPosters: Array<CreativeAsset | null> = [
      null,
      { ...posterAsset, deletedAt: 50 },
      { ...posterAsset, kind: 'video', mimeType: 'video/mp4' },
      { ...posterAsset, originalUrl: '' },
      { ...posterAsset, originalUrl: '   ' },
    ];

    for (const invalidPoster of invalidPosters) {
      const assets = new Map([[videoAsset.id, videoAsset]]);
      if (invalidPoster) assets.set(invalidPoster.id, invalidPoster);
      expect(resolveCreativeNodeAssetPresentation(videoNode, assets)).toEqual({
        src: videoAsset.originalUrl,
        posterSrc: videoAsset.thumbnailUrl,
        label: videoAsset.title,
      });
    }

    expect(resolveCreativeNodeAssetPresentation(videoNode, new Map([
      [videoAsset.id, { ...videoAsset, thumbnailUrl: null }],
    ]))).toEqual({ src: videoAsset.originalUrl, label: videoAsset.title });
    expect(resolveCreativeNodeAssetPresentation(videoNode, new Map([
      [videoAsset.id, { ...videoAsset, deletedAt: 50 }],
      [posterAsset.id, posterAsset],
    ]))).toEqual({ src: '', label: videoAsset.title, deleted: true });
  });
});
