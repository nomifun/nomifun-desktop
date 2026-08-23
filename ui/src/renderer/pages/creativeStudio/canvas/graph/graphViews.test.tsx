/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';

import type { CreativeCanvasConnection, CreativeCanvasNode } from '../../domain/schema';
import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import CanvasEdgeLayer from './CanvasEdgeLayer';
import CanvasMiniMap from './CanvasMiniMap';
import {
  buildCanvasConnectionBezier,
  centerCanvasViewportAt,
  createCanvasMiniMapProjection,
  miniMapPointToWorld,
  visibleWorldRect,
  worldPointToMiniMap,
} from './geometry';

const sourceNode: CreativeCanvasNode = {
  id: 'source',
  type: 'image',
  position: { x: 100, y: 50 },
  size: { width: 200, height: 100 },
  groupId: null,
  zIndex: 2,
  locked: false,
  data: { assetId: null, caption: '', alt: '', fit: 'contain', naturalSize: null, composer: null },
};

const targetNode: CreativeCanvasNode = {
  id: 'target',
  type: 'text',
  position: { x: 500, y: 250 },
  size: { width: 160, height: 80 },
  groupId: null,
  zIndex: 3,
  locked: false,
  data: { text: '目标', format: 'plain', fontSize: 16, textAlign: 'left' },
};

const connection: CreativeCanvasConnection = {
  id: 'edge-1',
  sourceNodeId: sourceNode.id,
  targetNodeId: targetNode.id,
  sourceHandle: 'out-top',
  targetHandle: 'in-bottom',
};

describe('Creative Studio canonical graph views', () => {
  test('builds handle-aware bezier geometry in world coordinates', () => {
    const geometry = buildCanvasConnectionBezier(connection, sourceNode, targetNode, {
      source: { 'out-top': { x: 100, y: 0, side: 'top' } },
      target: { 'in-bottom': { x: 80, y: 80, side: 'bottom' } },
    });

    expect(geometry.source.point.x).toBe(200);
    expect(geometry.source.point.y).toBe(50);
    expect(geometry.target.point.x).toBe(580);
    expect(geometry.target.point.y).toBe(330);
    expect(geometry.sourceControl.y < geometry.source.point.y).toBe(true);
    expect(geometry.targetControl.y > geometry.target.point.y).toBe(true);
    expect(geometry.path.startsWith('M 200 50 C 200 ')).toBe(true);
  });

  test('falls back to right/left midpoints when canonical handle geometry is unavailable', () => {
    const fallback = buildCanvasConnectionBezier(
      { ...connection, sourceHandle: 'missing', targetHandle: null },
      sourceNode,
      targetNode
    );

    expect(fallback.source.point.x).toBe(300);
    expect(fallback.source.point.y).toBe(100);
    expect(fallback.source.side).toBe('right');
    expect(fallback.target.point.x).toBe(500);
    expect(fallback.target.point.y).toBe(290);
    expect(fallback.target.side).toBe('left');
  });

  test('renders visible and hit paths with controlled semantic edge states', () => {
    const missing: CreativeCanvasConnection = {
      ...connection,
      id: 'missing-edge',
      targetNodeId: 'missing-node',
    };
    const html = renderToStaticMarkup(
      withCanvasTestI18n(
        <CanvasEdgeLayer
          nodes={[sourceNode, targetNode]}
          connections={[connection, missing]}
          stateByConnectionId={{
            [connection.id]: { selected: true, upstream: true, error: true, highlighted: true },
          }}
          onSelectConnection={() => undefined}
        />
      )
    );

    expect(html.includes('data-canvas-edge-layer="true"')).toBe(true);
    expect(html.includes('data-connection-id="edge-1"')).toBe(true);
    expect(html.includes('data-connection-id="missing-edge"')).toBe(false);
    expect(html.includes('data-edge-selected="true"')).toBe(true);
    expect(html.includes('data-edge-upstream="true"')).toBe(true);
    expect(html.includes('data-edge-error="true"')).toBe(true);
    expect(html.includes('data-edge-highlighted="true"')).toBe(true);
    expect((html.match(/<path/g) ?? []).length).toBe(2);
    expect(html.includes('role="button"')).toBe(true);
  });

  test('projects canonical nodes and the visible viewport into a reversible minimap', () => {
    const viewport = { x: -200, y: -100, zoom: 2 };
    const viewportSize = { width: 1000, height: 600 };
    const projection = createCanvasMiniMapProjection(
      [sourceNode, targetNode],
      viewport,
      viewportSize,
      { width: 240, height: 160, padding: 10, worldPadding: 0 }
    );
    const visible = visibleWorldRect(viewport, viewportSize);
    const point = { x: 320, y: 180 };
    const projected = worldPointToMiniMap(point, projection);
    const roundTrip = miniMapPointToWorld(projected, projection);

    expect(visible.x).toBe(100);
    expect(visible.y).toBe(50);
    expect(visible.width).toBe(500);
    expect(visible.height).toBe(300);
    expect(Math.abs(roundTrip.x - point.x) < 0.0001).toBe(true);
    expect(Math.abs(roundTrip.y - point.y) < 0.0001).toBe(true);
    expect(projection.scale > 0).toBe(true);
  });

  test('renders real minimap node and viewport geometry without media placeholders', () => {
    const html = renderToStaticMarkup(
      withCanvasTestI18n(
        <CanvasMiniMap
          nodes={[sourceNode, targetNode]}
          viewport={{ x: 0, y: 0, zoom: 1 }}
          viewportSize={{ width: 1024, height: 768 }}
          selectedNodeIds={new Set([targetNode.id])}
          onNavigate={() => undefined}
        />
      )
    );

    expect(html.includes('data-canvas-minimap-renderer="true"')).toBe(true);
    expect(html.includes('data-minimap-node-id="source"')).toBe(true);
    expect(html.includes('data-minimap-node-id="target"')).toBe(true);
    expect(html.includes('data-minimap-node-type="image"')).toBe(true);
    expect(html.includes('data-minimap-node-selected="true"')).toBe(true);
    expect(html.includes('data-minimap-viewport="true"')).toBe(true);
    expect(html.includes('<image')).toBe(false);
  });

  test('returns a viewport centered on a requested minimap world point', () => {
    const viewport = centerCanvasViewportAt(
      { x: 100, y: 200 },
      { x: 12, y: 40, zoom: 2 },
      { width: 1000, height: 600 }
    );

    expect(viewport.x).toBe(300);
    expect(viewport.y).toBe(-100);
    expect(viewport.zoom).toBe(2);
  });

  test('remains a headless canonical geometry layer', () => {
    const geometrySource = readFileSync(new URL('./geometry.ts', import.meta.url), 'utf8');
    const edgeSource = readFileSync(new URL('./CanvasEdgeLayer.tsx', import.meta.url), 'utf8');
    const miniMapSource = readFileSync(new URL('./CanvasMiniMap.tsx', import.meta.url), 'utf8');
    const sources = `${geometrySource}\n${edgeSource}\n${miniMapSource}`;

    expect(geometrySource.includes("from '../../domain/schema'")).toBe(true);
    expect(sources.includes('useState')).toBe(false);
    expect(sources.includes('useReducer')).toBe(false);
    expect(sources.includes('localStorage')).toBe(false);
    expect(sources.includes('fetch(')).toBe(false);
    expect(sources.includes('CreativeProjectDocument')).toBe(false);
    expect(edgeSource.includes('<svg')).toBe(true);
    expect(edgeSource.includes('@icon-park')).toBe(false);
  });
});
