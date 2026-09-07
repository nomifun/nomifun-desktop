/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import type { CreativeCanvasNode, CreativeCanvasNodeKind } from '../../domain/schema';
import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import {
  CREATIVE_NODE_VIEW_KINDS,
  CreativeImageNode,
  CreativeNodeView,
} from './CreativeNodeViews';

const base = {
  position: { x: 48, y: -24 },
  size: { width: 280, height: 190 },
  groupId: null,
  zIndex: 3,
  locked: false,
} as const;

const nodes: CreativeCanvasNode[] = [
  {
    ...base,
    id: 'image-1',
    type: 'image',
    locked: true,
    data: { assetId: 'asset-image', caption: '城市夜景', alt: '城市夜景', fit: 'cover', naturalSize: { width: 1920, height: 1080 }, composer: null },
  },
  {
    ...base,
    id: 'panorama-1',
    type: 'panorama',
    data: { assetId: null, projection: 'equirectangular', yaw: 14, pitch: -5, fieldOfView: 82 },
  },
  {
    ...base,
    id: 'text-1',
    type: 'text',
    data: { text: '第一幕：雨夜', format: 'markdown', fontSize: 18, textAlign: 'left' },
  },
  {
    ...base,
    id: 'config-1',
    type: 'config',
    data: {
      task: 'image_generation',
      capability: 'text-to-image',
      providerId: 'provider-1',
      model: 'model-1',
      prompt: '电影感雨夜街道',
      negativePrompt: '',
      operation: null,
      parameters: { width: 1024, height: 1024 },
      inputAssetIds: [],
      taskId: 'task-1',
      resultAssetIds: [],
      status: 'failed',
      errorMessage: '服务暂时不可用',
    },
  },
  {
    ...base,
    id: 'video-1',
    type: 'video',
    data: { assetId: null, posterAssetId: null, autoplay: false, loop: false, muted: true, trimStartMs: 0, trimEndMs: null, composer: null },
  },
  {
    ...base,
    id: 'audio-1',
    type: 'audio',
    data: { assetId: null, title: '环境声', loop: false, volume: 0.8, trimStartMs: 0, trimEndMs: 12_000, composer: null },
  },
  {
    ...base,
    id: 'director-1',
    type: 'director',
    data: { sceneId: 'scene-1', cameraId: 'camera-a', timelineMs: 2_400, durationMs: 8_000 },
  },
  {
    ...base,
    id: 'group-1',
    type: 'group',
    data: { title: '第一幕素材', color: '#7583b2', collapsed: false },
  },
];

const renderCanvas = (content: React.ReactNode) =>
  renderToStaticMarkup(withCanvasTestI18n(content));

describe('Creative Studio canonical node views', () => {
  test('renders deleted media nodes without their original image, video, or audio URLs', () => {
    for (const node of nodes.filter((node) => ['image', 'panorama', 'video', 'audio'].includes(node.type))) {
      const html = renderToStaticMarkup(withCanvasTestI18n(
        <CreativeNodeView node={node} asset={{ src: '', label: 'Deleted original', deleted: true }} />
      ));
      expect(html.includes('素材已删除')).toBe(true);
      expect(html.includes('<img')).toBe(false);
      expect(html.includes('<video')).toBe(false);
      expect(html.includes('<audio')).toBe(false);
    }
  });
  test('renders the seven user-facing kinds and keeps config task records headless', () => {
    const html = renderCanvas(
      <>{nodes.map((node) => <CreativeNodeView key={node.id} node={node} selected={node.id === 'text-1'} />)}</>
    );

    expect(CREATIVE_NODE_VIEW_KINDS.length).toBe(7);
    for (const kind of CREATIVE_NODE_VIEW_KINDS) {
      expect(html.includes(`data-node-type="${kind}"`)).toBe(true);
    }
    expect(html.includes('data-node-type="config"')).toBe(false);
    expect(html.includes('left:48px')).toBe(true);
    expect(html.includes('top:-24px')).toBe(true);
    expect(html.includes('data-node-selected="true"')).toBe(true);
    expect(html.includes('data-node-locked="true"')).toBe(true);
  });

  test('shows honest empty media states without exposing task-record status', () => {
    const html = renderCanvas(<>{nodes.map((node) => <CreativeNodeView key={node.id} node={node} />)}</>);

    expect(html.includes('data-node-empty-media="true"')).toBe(true);
    expect(
      html.includes('creativeStudio.canvas.nodes.video.empty')
    ).toBe(true);
    expect(
      html.includes('creativeStudio.canvas.nodes.audio.empty')
    ).toBe(true);
    expect(html.includes('0:00 – ∞ · 80%')).toBe(false);
    expect(html.includes('data-node-status="failed"')).toBe(false);
    expect(html.includes('服务暂时不可用')).toBe(false);
    expect(html.includes('<img')).toBe(false);
    expect(html.includes('<video')).toBe(false);
  });

  test('renders only an explicitly resolved asset URL and controlled runtime progress', () => {
    const imageNode = nodes.find((node): node is Extract<CreativeCanvasNode, { type: 'image' }> => node.type === 'image');
    if (!imageNode) throw new Error('image fixture is missing');
    const html = renderCanvas(
      <CreativeNodeView
        node={imageNode}
        asset={{ src: 'asset://resolved/image-1', alt: '已解析图片' }}
        runtime={{ status: 'running', progress: 42, label: '生成中' }}
      />
    );

    expect(html.includes('src="asset://resolved/image-1"')).toBe(true);
    expect(html.includes('alt="已解析图片"')).toBe(true);
    expect(html.includes('data-node-status="running"')).toBe(true);
    expect(html.includes('aria-valuenow="42"')).toBe(true);
    expect(html.includes('width:42%')).toBe(true);
  });

  test('shows video playback without the obsolete trim-range badge', () => {
    const videoNode = nodes.find((node): node is Extract<CreativeCanvasNode, { type: 'video' }> => node.type === 'video');
    if (!videoNode) throw new Error('video fixture is missing');
    for (const trimEndMs of [null, 12_000]) {
      const html = renderCanvas(
        <CreativeNodeView
          node={{ ...videoNode, data: { ...videoNode.data, assetId: 'video-asset', trimEndMs } }}
          asset={{ src: '/clip.mp4', posterSrc: '/poster.jpg' }}
        />
      );

      expect(html.includes('<video')).toBe(true);
      expect(html.includes('data-creative-video-player')).toBe(true);
      expect(html.includes('0:00 – ∞')).toBe(false);
      expect(html.includes('0:00 – 0:12')).toBe(false);
      expect(html.includes('controls=""')).toBe(false);
      expect(html.toLowerCase().includes('disablepictureinpicture=""')).toBe(true);
    }
  });

  test('keeps node names accessible without rendering descriptions above cards', () => {
    const imageNode = nodes.find((node): node is Extract<CreativeCanvasNode, { type: 'image' }> => node.type === 'image');
    if (!imageNode) throw new Error('image fixture is missing');
    const accessibleName = 'Image node accessible name';
    const html = renderCanvas(
      <CreativeImageNode
        node={imageNode}
        title={accessibleName}
        runtime={{ status: 'running', progress: 42 }}
      />
    );

    expect(html.includes(`aria-label="${accessibleName}"`)).toBe(true);
    expect(html.includes(`>${accessibleName}<`)).toBe(false);
  });

  test('stays headless and imports the canonical schema instead of defining document fields', () => {
    const typesSource = readFileSync(new URL('./types.ts', import.meta.url), 'utf8');
    const viewsSource = readFileSync(new URL('./CreativeNodeViews.tsx', import.meta.url), 'utf8');
    const frameSource = readFileSync(new URL('./CreativeNodeFrame.tsx', import.meta.url), 'utf8');
    const sources = `${typesSource}\n${viewsSource}\n${frameSource}`;

    expect(typesSource.includes("from '../../domain/schema'")).toBe(true);
    expect(sources.includes('useState')).toBe(false);
    expect(sources.includes('useReducer')).toBe(false);
    expect(sources.includes('<svg')).toBe(false);
    expect(sources.includes('localStorage')).toBe(false);
    expect(sources.includes('fetch(')).toBe(false);
    expect(sources.includes('CreativeProjectDocument')).toBe(false);
    expect((CREATIVE_NODE_VIEW_KINDS as readonly CreativeCanvasNodeKind[]).includes('group')).toBe(true);
  });
});
