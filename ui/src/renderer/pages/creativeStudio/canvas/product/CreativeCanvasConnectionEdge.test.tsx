/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import type {
  CreativeCanvasConnection,
  CreativeCanvasNode,
} from '../../domain/schema';
import CreativeCanvasConnectionEdge from './CreativeCanvasConnectionEdge';

const source: CreativeCanvasNode = {
  id: 'source-node',
  type: 'image',
  position: { x: 100, y: 50 },
  size: { width: 200, height: 100 },
  groupId: null,
  zIndex: 1,
  locked: false,
  data: { assetId: null, caption: '', alt: '', fit: 'contain', naturalSize: null },
};

const target: CreativeCanvasNode = {
  id: 'target-node',
  type: 'text',
  position: { x: 500, y: 250 },
  size: { width: 160, height: 80 },
  groupId: null,
  zIndex: 2,
  locked: false,
  data: { text: '目标', format: 'plain', fontSize: 16, textAlign: 'left' },
};

const connection: CreativeCanvasConnection = {
  id: 'connection-1',
  sourceNodeId: source.id,
  targetNodeId: target.id,
  sourceHandle: null,
  targetHandle: null,
};

describe('CreativeCanvasConnectionEdge', () => {
  test('renders exactly one canonical bezier with a separate accessible hit target', () => {
    const html = renderToStaticMarkup(
      <CreativeCanvasConnectionEdge
        connection={connection}
        source={source}
        target={target}
        selected
        onActivate={() => undefined}
      />
    );

    expect(html.includes('data-canvas-product-edge="true"')).toBe(true);
    expect(html.includes('data-connection-id="connection-1"')).toBe(true);
    expect(html.includes('data-edge-selected="true"')).toBe(true);
    expect(html.includes('M 300 100 C')).toBe(true);
    expect((html.match(/<svg/g) ?? []).length).toBe(1);
    expect((html.match(/<path/g) ?? []).length).toBe(2);
    expect(html.includes('role="button"')).toBe(true);
    expect(html.includes('tabindex="0"')).toBe(true);
    expect(html.includes('aria-pressed="true"')).toBe(true);
    expect(html.includes('连接 source-node 至 target-node')).toBe(true);
  });

  test('keeps geometry and activation canonical instead of owning graph state', () => {
    const sourceText = readFileSync(
      new URL('./CreativeCanvasConnectionEdge.tsx', import.meta.url),
      'utf8'
    );

    expect(sourceText.includes('buildCanvasConnectionBezier(connection, source, target)')).toBe(true);
    expect(sourceText.includes("event.key !== 'Enter' && event.key !== ' '")).toBe(true);
    expect(sourceText.includes('event.preventDefault()')).toBe(true);
    expect(sourceText.includes('event.stopPropagation()')).toBe(true);
    expect(sourceText.includes('useState')).toBe(false);
    expect(sourceText.includes('useReducer')).toBe(false);
    expect(sourceText.includes('fetch(')).toBe(false);
    expect(sourceText.includes('localStorage')).toBe(false);
  });
});
