/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(
  new URL('./CanvasVideoTaskRuntimeBridge.tsx', import.meta.url),
  'utf8'
);

describe('Canvas video task runtime bridge structure', () => {
  test('routes only canonical canvas video composition through video settlement', () => {
    for (const token of [
      'canvasVideoComposeConfigForReference',
      'canvasVideoComposeResumeRequests',
      'persistCanvasVideoComposePendingTask',
      'reconcileCanvasVideoComposeTask',
      'settleCanvasVideoComposeTask',
      'orphanCanvasVideoComposeTask',
      "scopeKey: `${props.projectId}:canvas-video-tasks`",
      "outputKind: 'video'",
    ]) {
      expect(source.includes(token)).toBe(true);
    }
    expect(source.includes('canvasImageCompose')).toBe(false);
    expect(source.includes('settleCanvasImageMaskEditTask')).toBe(false);
    expect(source.includes("outputKind: 'image'")).toBe(false);
  });

  test('preserves the shared admission and authoritative 404 contracts', () => {
    expect(source.includes('waitForCanvasImageMaskEditAdmission')).toBe(true);
    expect(source.includes('isCanvasImageMaskEditTaskNotFound')).toBe(true);
    expect(source.includes('canvasVideoTaskReferenceFromPlan')).toBe(true);
    expect(source.includes('creativeTaskReferenceFromInput(plan.input)')).toBe(
      true
    );
    expect(
      source.includes(
        '服务器未找到遗留的视频创作任务，已只清理该任务的恢复标记。'
      )
    ).toBe(true);
  });
});
