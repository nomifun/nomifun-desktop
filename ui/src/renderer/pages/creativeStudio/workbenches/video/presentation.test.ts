/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  clampVideoProgress,
  normalizeVideoTaskCount,
  toggleAllVideoTasks,
  toggleVideoTaskSelection,
  videoWorkbenchDimensions,
  videoResultsState,
} from './presentation';
import type { VideoWorkbenchTask } from './types';

const taskBase = {
  prompt: '镜头缓慢推进',
  createdAtLabel: '08/20 14:30',
  model: { providerId: 'provider-a', model: 'video-model' },
  modelLabel: 'video-model',
  resolutionLabel: '1080P',
  sizeLabel: '16:9',
  durationLabel: '6s',
  taskCount: 1,
};

const running = (id: string): VideoWorkbenchTask => ({
  ...taskBase,
  id,
  taskId: `task-${id}`,
  status: 'running',
  progress: 42,
});

const succeeded = (id: string): VideoWorkbenchTask => ({
  ...taskBase,
  id,
  taskId: `task-${id}`,
  status: 'succeeded',
  assetId: `asset-${id}`,
  videoUrl: `https://example.invalid/${id}.mp4`,
});

const failed = (id: string): VideoWorkbenchTask => ({
  ...taskBase,
  id,
  taskId: `task-${id}`,
  status: 'failed',
  error: 'generation failed',
});

const queued = (id: string): VideoWorkbenchTask => ({
  ...taskBase,
  id,
  taskId: `task-${id}`,
  status: 'queued',
});

const canceled = (id: string): VideoWorkbenchTask => ({
  ...taskBase,
  id,
  taskId: `task-${id}`,
  status: 'canceled',
  message: 'canceled by user',
});

describe('videoResultsState', () => {
  test('keeps every backend status and mixed state explicit', () => {
    expect(videoResultsState([])).toBe('empty');
    expect(videoResultsState([queued('a')])).toBe('queued');
    expect(videoResultsState([running('a'), running('b')])).toBe('running');
    expect(videoResultsState([succeeded('a')])).toBe('succeeded');
    expect(videoResultsState([failed('a')])).toBe('failed');
    expect(videoResultsState([canceled('a')])).toBe('canceled');
    expect(videoResultsState([running('a'), succeeded('b'), failed('c')])).toBe('mixed');
  });
});

describe('video workbench value guards', () => {
  test('clamps progress without fabricating an unknown value', () => {
    expect(clampVideoProgress(undefined)).toBeNull();
    expect(clampVideoProgress(Number.NaN)).toBeNull();
    expect(clampVideoProgress(-2)).toBe(0);
    expect(clampVideoProgress(42.8)).toBe(42);
    expect(clampVideoProgress(109)).toBe(100);
  });

  test('normalizes provider-agnostic batch count to the product limit', () => {
    expect(normalizeVideoTaskCount(0)).toBe(1);
    expect(normalizeVideoTaskCount(2.9)).toBe(2);
    expect(normalizeVideoTaskCount(20)).toBe(6);
    expect(normalizeVideoTaskCount(Number.NaN)).toBe(1);
  });

  test('derives final pixels independently from the selected aspect ratio', () => {
    expect(videoWorkbenchDimensions('1080p', '16:9')).toEqual({
      width: 1920,
      height: 1080,
    });
    expect(videoWorkbenchDimensions('720p', '9:16')).toEqual({
      width: 720,
      height: 1280,
    });
  });
});

describe('video task selection', () => {
  test('adds and removes one task without duplicating ids', () => {
    expect(toggleVideoTaskSelection(['a'], 'b', true)).toEqual(['a', 'b']);
    expect(toggleVideoTaskSelection(['a', 'b'], 'b', true)).toEqual(['a', 'b']);
    expect(toggleVideoTaskSelection(['a', 'b'], 'a', false)).toEqual(['b']);
  });

  test('select-all preserves out-of-view ids and deselects only visible tasks', () => {
    expect(toggleAllVideoTasks(['a', 'b'], ['outside'])).toEqual(['outside', 'a', 'b']);
    expect(toggleAllVideoTasks(['a', 'b'], ['outside', 'a', 'b'])).toEqual(['outside']);
  });
});
