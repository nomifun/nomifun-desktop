/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeCanvasSummary } from '../domain';

export const createCreativeStudioCanvasFixture = (
  overrides: Partial<CreativeCanvasSummary> = {}
): CreativeCanvasSummary => ({
  canvasId: 'canvas-brand-film',
  title: '品牌短片概念',
  revision: '1',
  createdAt: Date.parse('2026-08-17T03:10:00.000Z'),
  updatedAt: Date.parse('2026-08-20T01:35:00.000Z'),
  nodeCount: 18,
  connectionCount: 12,
  ...overrides,
});

export const CREATIVE_STUDIO_CANVAS_FIXTURES: readonly CreativeCanvasSummary[] = [
  createCreativeStudioCanvasFixture(),
  createCreativeStudioCanvasFixture({
    canvasId: 'canvas-product-stills',
    title: '秋季产品静物',
    updatedAt: Date.parse('2026-08-19T09:12:00.000Z'),
    nodeCount: 9,
    connectionCount: 6,
  }),
  createCreativeStudioCanvasFixture({
    canvasId: 'canvas-character-study',
    title: '角色风格探索',
    updatedAt: Date.parse('2026-08-18T14:46:00.000Z'),
    nodeCount: 24,
    connectionCount: 19,
  }),
];
