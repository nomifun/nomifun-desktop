/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  canvasErrorMessage,
  formatCanvasTimestamp,
  mergeCanvases,
  pruneCanvasSelection,
} from './canvasList';
import {
  CREATIVE_STUDIO_CANVAS_FIXTURES,
  createCreativeStudioCanvasFixture,
} from './testing';

describe('Creative Studio Canvas list model', () => {
  test('merges server updates by Canvas identity without mutating the source list', () => {
    const current = CREATIVE_STUDIO_CANVAS_FIXTURES.slice(0, 2);
    const renamed = { ...current[0], title: '更新后的品牌短片' };
    const added = createCreativeStudioCanvasFixture({
      canvasId: 'canvas-new',
      title: '新画布',
    });
    const merged = mergeCanvases(current, [renamed, added]);

    expect(merged.map(({ canvasId }) => canvasId)).toEqual([
      current[0].canvasId,
      current[1].canvasId,
      'canvas-new',
    ]);
    expect(merged[0].title).toBe('更新后的品牌短片');
    expect(current[0].title).toBe('品牌短片概念');
  });

  test('prunes stale selection and keeps date/error fallbacks deterministic', () => {
    const selected = new Set([
      CREATIVE_STUDIO_CANVAS_FIXTURES[0].canvasId,
      'deleted-canvas',
    ]);
    expect([
      ...pruneCanvasSelection(selected, CREATIVE_STUDIO_CANVAS_FIXTURES),
    ]).toEqual([CREATIVE_STUDIO_CANVAS_FIXTURES[0].canvasId]);
    expect(formatCanvasTimestamp(Number.NaN, 'zh-CN')).toBe('—');
    expect(canvasErrorMessage(new Error('storage offline'))).toBe(
      'storage offline'
    );
    expect(canvasErrorMessage(null)).toBe('Unknown error');
  });
});
