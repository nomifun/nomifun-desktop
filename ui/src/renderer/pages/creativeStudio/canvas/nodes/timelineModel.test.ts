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
  reorderTimelineClip,
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

const withClips = (
  intervals: ReadonlyArray<readonly [startMs: number, durationMs: number]>
): CreativeTimelineNodeData => ({
  ...empty(),
  clips: intervals.map(([startMs, durationMs], index) => ({
    id: `clip-${index}`,
    assetId: `asset-${index}`,
    kind: 'image',
    startMs,
    durationMs,
    sourceStartMs: 0,
    sourceDurationMs: null,
  })),
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

  test('snaps a moved clip to either neighbor without changing media or other clips', () => {
    const data = withClips([[0, 5_000], [10_000, 5_000], [20_000, 5_000]]);
    data.clips[1] = {
      ...data.clips[1]!, kind: 'video', sourceStartMs: 2_000, sourceDurationMs: 8_000,
    };
    for (const [requested, expected] of [[2_000, 5_000], [17_000, 15_000]] as const) {
      const moved = moveTimelineClip(data, 'clip-1', requested);
      expect(moved.clips).toEqual([
        data.clips[0]!,
        { ...data.clips[1]!, startMs: expected },
        data.clips[2]!,
      ]);
    }
    expect(data.clips[1]?.startMs).toBe(10_000);
  });

  test('allows free placement and crossing clips into a new gap', () => {
    const data = withClips([[0, 5_000], [10_000, 5_000], [20_000, 5_000]]);
    expect(moveTimelineClip(data, 'clip-1', 7_000).clips[1]?.startMs).toBe(7_000);
    expect(moveTimelineClip(data, 'clip-1', 26_000).clips[1]?.startMs).toBe(26_000);
    expect(moveTimelineClip(data, 'clip-0', 12_000).clips[0]?.startMs).toBe(15_000);
  });

  test('keeps touching clips in place during small moves and skips gaps that are too short', () => {
    const touching = withClips([[0, 5_000], [5_000, 5_000], [10_000, 5_000]]);
    expect(moveTimelineClip(touching, 'clip-1', 5_100).clips[1]?.startMs).toBe(5_000);
    expect(moveTimelineClip(touching, 'clip-1', 10_000).clips[1]?.startMs).toBe(5_000);
    expect(moveTimelineClip(touching, 'clip-1', 14_000).clips[1]?.startMs).toBe(15_000);

    const shortGaps = withClips([[30_000, 5_000], [14_000, 5_000], [0, 5_000], [7_000, 5_000]]);
    expect(moveTimelineClip(shortGaps, 'clip-0', 6_000).clips[0]?.startMs).toBe(19_000);
  });

  test('moves an already overlapping clip out of occupied time', () => {
    const data = withClips([[7_500, 5_000], [0, 10_000], [12_000, 5_000]]);
    expect(moveTimelineClip(data, 'clip-0', 8_000).clips[0]?.startMs).toBe(17_000);
  });

  test('never overlaps any neighbor while moving across an unsorted timeline', () => {
    const data = withClips([[0, 5_000], [20_000, 8_000], [8_000, 4_000], [15_000, 3_000]]);
    for (let requested = -1_000; requested <= 35_000; requested += 250) {
      const moved = moveTimelineClip(data, 'clip-0', requested);
      const clip = moved.clips[0]!;
      expect(clip.startMs).toBeGreaterThanOrEqual(0);
      expect(clip.durationMs).toBe(5_000);
      for (const neighbor of moved.clips.slice(1)) {
        expect(
          clip.startMs + clip.durationMs <= neighbor.startMs ||
          clip.startMs >= neighbor.startMs + neighbor.durationMs
        ).toBe(true);
      }
    }
  });

  test('respects timeline bounds and leaves clips unchanged when there is no available space', () => {
    const data = withClips([[10_000, 5_000], [86_390_000, 10_000]]);
    expect(moveTimelineClip(data, 'clip-0', -1_000).clips[0]?.startMs).toBe(0);
    expect(moveTimelineClip(data, 'clip-0', 86_400_000).clips[0]?.startMs).toBe(86_385_000);
    const full = withClips([[10_000, 5_000], [0, 86_400_000]]);
    expect(moveTimelineClip(full, 'clip-0', 20_000)).toEqual(full);
    expect(moveTimelineClip(data, 'missing', 20_000)).toEqual(data);
  });

  test('inserts the last clip between touching clips with different durations', () => {
    const data = withClips([[0, 4_000], [4_000, 5_000], [9_000, 5_000], [14_000, 7_000]]);
    data.clips[3] = {
      ...data.clips[3]!, kind: 'video', sourceStartMs: 2_000, sourceDurationMs: 9_000,
    };
    const moved = reorderTimelineClip(data, 'clip-3', 5_500, 9_000);
    expect(moved.clips).toEqual(data.clips.map((clip, index) => ({
      ...clip, startMs: [0, 4_000, 16_000, 9_000][index],
    })));
    expect(timelineDurationMs(moved.clips)).toBe(21_000);
    expect(timelineClipAtTime(moved.clips, 9_000)?.id).toBe('clip-3');
    expect(timelineClipAtTime(moved.clips, 16_000)?.id).toBe('clip-2');
    expect(data.clips.map((clip) => clip.startMs)).toEqual([0, 4_000, 9_000, 14_000]);
  });

  test('moves earlier clips later and supports insertion at both ends', () => {
    const data = withClips([[0, 4_000], [4_000, 5_000], [9_000, 5_000], [14_000, 7_000]]);
    expect(reorderTimelineClip(data, 'clip-0', 12_000, 14_000).clips.map((clip) => clip.startMs))
      .toEqual([10_000, 0, 5_000, 14_000]);
    expect(reorderTimelineClip(data, 'clip-3', -3_500, 0).clips.map((clip) => clip.startMs))
      .toEqual([7_000, 11_000, 16_000, 0]);
    expect(reorderTimelineClip(data, 'clip-0', 18_000, 20_000).clips.map((clip) => clip.startMs))
      .toEqual([17_000, 0, 5_000, 10_000]);
  });

  test('uses pointer position for insertion even when a clip is grabbed near an edge', () => {
    const data = withClips([[0, 5_000], [5_000, 5_000], [10_000, 5_000], [15_000, 7_000]]);
    for (const requestedStart of [9_500, 3_500]) {
      expect(reorderTimelineClip(data, 'clip-3', requestedStart, 10_000).clips.map((clip) => clip.startMs))
        .toEqual([0, 5_000, 17_000, 10_000]);
    }
  });

  test('preserves existing gaps during insertion and allows free placement in empty time', () => {
    const data = withClips([[2_000, 5_000], [10_000, 5_000], [20_000, 7_000]]);
    expect(reorderTimelineClip(data, 'clip-2', 6_500, 10_000).clips.map((clip) => clip.startMs))
      .toEqual([2_000, 22_000, 10_000]);
    expect(reorderTimelineClip(data, 'clip-2', 30_000, 33_500).clips.map((clip) => clip.startMs))
      .toEqual([2_000, 10_000, 30_000]);
    const gap = withClips([[0, 5_000], [10_000, 5_000], [25_000, 7_000]]);
    expect(reorderTimelineClip(gap, 'clip-2', 16_000, 19_500).clips.map((clip) => clip.startMs))
      .toEqual([0, 10_000, 16_000]);
  });

  test('keeps all clips separate and preserves media through repeated reordering', () => {
    let data = withClips([[0, 4_000], [4_000, 5_000], [9_000, 5_000], [14_000, 7_000]]);
    const media = data.clips.map(({ startMs: _startMs, ...clip }) => clip);
    for (const id of ['clip-3', 'clip-0', 'clip-1', 'clip-2']) {
      for (let pointerTime = 0; pointerTime <= 24_000; pointerTime += 1_000) {
        const clip = data.clips.find((item) => item.id === id)!;
        data = reorderTimelineClip(data, id, pointerTime - clip.durationMs / 2, pointerTime);
        const ordered = [...data.clips].sort((a, b) => a.startMs - b.startMs);
        expect(ordered[0]!.startMs).toBeGreaterThanOrEqual(0);
        for (let index = 1; index < ordered.length; index += 1) {
          expect(ordered[index]!.startMs)
            .toBeGreaterThanOrEqual(ordered[index - 1]!.startMs + ordered[index - 1]!.durationMs);
        }
        expect(data.clips.map(({ startMs: _startMs, ...item }) => item)).toEqual(media);
      }
    }
  });

  test('does not reorder at the original position or exceed the timeline limit', () => {
    const data = withClips([[0, 5_000], [5_000, 5_000], [10_000, 5_000]]);
    expect(reorderTimelineClip(data, 'clip-1', 5_000, 7_500)).toEqual(data);
    expect(reorderTimelineClip(data, 'missing', 0, 0)).toEqual(data);
    const full = withClips([[0, 5_000], [5_000, 86_395_000]]);
    const moved = reorderTimelineClip(full, 'clip-0', 86_395_000, 86_400_000);
    expect(moved.clips.map((clip) => clip.startMs)).toEqual([86_395_000, 0]);
    expect(timelineDurationMs(moved.clips)).toBe(86_400_000);
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
