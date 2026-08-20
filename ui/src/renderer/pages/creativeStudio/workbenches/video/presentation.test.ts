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
  videoResultsState,
} from './presentation';
import type { VideoWorkbenchTask } from './types';

const taskBase = {
  prompt: '镜头缓慢推进',
  createdAtLabel: '08/20 14:30',
  modelLabel: 'video-model',
  resolutionLabel: '1080P',
  sizeLabel: '16:9',
  durationLabel: '6s',
  taskCount: 1,
};

const running = (id: string): VideoWorkbenchTask => ({
  ...taskBase,
  id,
  status: 'running',
  progress: 42,
});

const success = (id: string): VideoWorkbenchTask => ({
  ...taskBase,
  id,
  status: 'success',
  videoUrl: `https://example.invalid/${id}.mp4`,
});

const failed = (id: string): VideoWorkbenchTask => ({
  ...taskBase,
  id,
  status: 'failed',
  error: 'generation failed',
});

describe('videoResultsState', () => {
  test('keeps empty, running, success, failed and mixed states explicit', () => {
    expect(videoResultsState([])).toBe('empty');
    expect(videoResultsState([running('a'), running('b')])).toBe('running');
    expect(videoResultsState([success('a')])).toBe('success');
    expect(videoResultsState([failed('a')])).toBe('failed');
    expect(videoResultsState([running('a'), success('b'), failed('c')])).toBe('mixed');
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
