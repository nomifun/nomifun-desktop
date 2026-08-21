/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(
  new URL('./CanvasAudioTaskRuntimeBridge.tsx', import.meta.url),
  'utf8'
);

describe('Canvas audio task runtime bridge structure', () => {
  test('routes only canonical canvas audio composition through audio settlement', () => {
    for (const token of [
      'canvasAudioComposeConfigForReference',
      'canvasAudioComposeResumeRequests',
      'persistCanvasAudioComposePendingTask',
      'reconcileCanvasAudioComposeTask',
      'settleCanvasAudioComposeTask',
      'orphanCanvasAudioComposeTask',
      "scopeKey: `${props.projectId}:canvas-audio-tasks`",
      "outputKind: 'audio'",
    ]) {
      expect(source.includes(token)).toBe(true);
    }
    expect(source.includes('canvasImageCompose')).toBe(false);
    expect(source.includes('canvasVideoCompose')).toBe(false);
    expect(source.includes('settleCanvasImageMaskEditTask')).toBe(false);
    expect(source.includes("outputKind: 'image'")).toBe(false);
    expect(source.includes("outputKind: 'video'")).toBe(false);
  });

  test('preserves pending-before-POST admission, retry and cancel contracts', () => {
    expect(source.includes('onPendingTask')).toBe(true);
    expect(source.includes('waitForCanvasImageMaskEditAdmission')).toBe(true);
    expect(source.includes('runtime.controller.run(plan)')).toBe(true);
    expect(source.includes('runtime.controller.retrySubmission(order)')).toBe(
      true
    );
    expect(source.includes('runtime.controller.retry(taskId)')).toBe(true);
    expect(source.includes('runtime.controller.cancel(taskId)')).toBe(true);
    expect(source.includes('canvasAudioTaskReferenceFromPlan')).toBe(true);
    expect(source.includes('creativeTaskReferenceFromInput(plan.input)')).toBe(
      true
    );
  });

  test('cleans only authoritative 404 recovery orphans', () => {
    expect(source.includes('isCanvasImageMaskEditTaskNotFound')).toBe(true);
    expect(source.includes('if (!isCanvasImageMaskEditTaskNotFound(error)) return false')).toBe(
      true
    );
    expect(
      source.includes(
        '服务器未找到遗留的音频创作任务，已只清理该任务的恢复标记。'
      )
    ).toBe(true);
    expect(source.includes("message.includes('404')")).toBe(false);
  });
});
