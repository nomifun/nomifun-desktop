/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeTimelineNodeData } from '../../domain';
import {
  appendTimelineClips,
  moveTimelineClip,
  resolveTimelineClipDuration,
  timelineClipAtTime,
  timelineDurationMs,
  timelineScaleDurationMs,
  timelineTickValues,
  trimTimelineClip,
} from './timelineModel';

const empty = (): CreativeTimelineNodeData => ({
  title: '时间线1',
  muted: false,
  clips: [],
});
describe('timeline model', () => {
  test('appends image and video assets in track order with stable clip identity', () => {
    let index = 0;
    const data = appendTimelineClips(
      empty(),
      [
        { id: 'asset-image', kind: 'image' },
        { id: 'asset-video', kind: 'video' },
      ],
      () => `clip-${++index}`
    );

    expect(data.clips).toEqual([
      {
        id: 'clip-1',
        assetId: 'asset-image',
        kind: 'image',
        startMs: 0,
        durationMs: 5_000,
        sourceStartMs: 0,
        sourceDurationMs: null,
      },
      {
        id: 'clip-2',
        assetId: 'asset-video',
        kind: 'video',
        startMs: 5_000,
        durationMs: 5_000,
        sourceStartMs: 0,
        sourceDurationMs: null,
      },
    ]);
    expect(timelineDurationMs(data.clips)).toBe(10_000);
    expect(timelineScaleDurationMs(data.clips)).toBe(60_000);
    expect(timelineTickValues(60_000)).toEqual([
      0, 5_000, 10_000, 15_000, 20_000, 25_000, 30_000,
      35_000, 40_000, 45_000, 50_000, 55_000, 60_000,
    ]);
  });

  test('moves, trims, and resolves video metadata without exceeding source bounds', () => {
    let data = appendTimelineClips(
      empty(),
      [{ id: 'asset-video', kind: 'video' }],
      () => 'clip-video'
    );
    data = resolveTimelineClipDuration(data, 'clip-video', 8_000);
    data = moveTimelineClip(data, 'clip-video', 2_000);
    data = trimTimelineClip(data, 'clip-video', 'start', 1_000);
    data = trimTimelineClip(data, 'clip-video', 'end', 10_000);

    expect(data.clips[0]).toEqual({
      id: 'clip-video',
      assetId: 'asset-video',
      kind: 'video',
      startMs: 3_000,
      durationMs: 7_000,
      sourceStartMs: 1_000,
      sourceDurationMs: 8_000,
    });
    expect(timelineClipAtTime(data.clips, 2_999)).toBeNull();
    expect(timelineClipAtTime(data.clips, 3_000)?.id).toBe('clip-video');
    expect(timelineClipAtTime(data.clips, 10_000)).toBeNull();
  });

  test('keeps trims above the usable half-second minimum', () => {
    const data = appendTimelineClips(
      empty(),
      [{ id: 'asset-image', kind: 'image' }],
      () => 'clip-image'
    );
    const trimmed = trimTimelineClip(data, 'clip-image', 'start', 10_000);
    expect(trimmed.clips[0]?.startMs).toBe(4_500);
    expect(trimmed.clips[0]?.durationMs).toBe(500);
  });
});
