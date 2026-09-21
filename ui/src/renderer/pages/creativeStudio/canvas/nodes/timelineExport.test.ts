/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeTimelineNodeData } from '../../domain';
import {
  buildTimelineExportPlan,
  selectTimelineExportFormat,
  selectTimelineExportMimeType,
  timelineExportSize,
  TimelineExportError,
  type CreativeTimelineAssetPresentation,
} from './timelineExport';

const timeline = (): CreativeTimelineNodeData => ({
  title: '时间线1',
  muted: false,
  clips: [
    {
      id: 'clip-image',
      assetId: 'asset-image',
      kind: 'image',
      startMs: 0,
      durationMs: 5_000,
      sourceStartMs: 0,
      sourceDurationMs: null,
    },
    {
      id: 'clip-video',
      assetId: 'asset-video',
      kind: 'video',
      startMs: 7_000,
      durationMs: 3_000,
      sourceStartMs: 2_000,
      sourceDurationMs: 12_000,
    },
  ],
});

const assets = new Map<string, CreativeTimelineAssetPresentation>([
  ['asset-image', {
    assetId: 'asset-image',
    kind: 'image',
    title: '开场',
    src: '/assets/opening.png',
    width: 1920,
    height: 1080,
  }],
  ['asset-video', {
    assetId: 'asset-video',
    kind: 'video',
    title: '结尾',
    src: '/assets/ending.mp4',
    width: 3840,
    height: 2160,
  }],
]);

describe('timeline composition export model', () => {
  test('builds a real ten-second composition plan with its authored gap and trims', () => {
    const plan = buildTimelineExportPlan(timeline(), assets);
    expect(plan.durationMs).toBe(10_000);
    expect(plan.clips.map(({ clip }) => ({
      id: clip.id,
      startMs: clip.startMs,
      durationMs: clip.durationMs,
      sourceStartMs: clip.sourceStartMs,
    }))).toEqual([
      { id: 'clip-image', startMs: 0, durationMs: 5_000, sourceStartMs: 0 },
      { id: 'clip-video', startMs: 7_000, durationMs: 3_000, sourceStartMs: 2_000 },
    ]);
  });

  test('fails closed when any clip cannot resolve its real media asset', () => {
    const missing = new Map(assets);
    missing.delete('asset-video');
    expect(() => buildTimelineExportPlan(timeline(), missing)).toThrow(TimelineExportError);
    try {
      buildTimelineExportPlan(timeline(), missing);
    } catch (error) {
      expect((error as TimelineExportError).code).toBe('asset-unavailable');
    }
  });

  test('caps output at 1920 pixels and keeps encoder-safe even dimensions', () => {
    expect(timelineExportSize(3840, 2160)).toEqual({ width: 1920, height: 1080 });
    expect(timelineExportSize(1001, 777)).toEqual({ width: 1000, height: 776 });
    expect(timelineExportSize(0, Number.NaN)).toEqual({ width: 1280, height: 720 });
  });

  test('requires an audio-capable WebM type when video sound is included', () => {
    const supported = new Set([
      'video/webm;codecs=vp9',
      'video/webm;codecs=vp8,opus',
    ]);
    expect(selectTimelineExportMimeType((value) => supported.has(value))).toBe(
      'video/webm;codecs=vp8,opus'
    );
    expect(
      selectTimelineExportMimeType((value) => supported.has(value), true)
    ).toBe('video/webm;codecs=vp8,opus');
    expect(
      selectTimelineExportMimeType(
        (value) => value === 'video/webm;codecs=vp9',
        true
      )
    ).toBeNull();
  });

  test('prefers H.264/AAC MP4 and retains WebM as a real fallback', () => {
    const support = (value: string) => [
      'video/mp4;codecs=avc1.42E01E,mp4a.40.2',
      'video/mp4;codecs=avc1.42E01E',
      'video/webm;codecs=vp9,opus',
    ].includes(value);
    expect(selectTimelineExportFormat(support, true)).toEqual({
      mimeType: 'video/mp4;codecs=avc1.42E01E,mp4a.40.2',
      extension: 'mp4',
      audio: true,
    });
    expect(selectTimelineExportFormat(support, false)).toEqual({
      mimeType: 'video/mp4;codecs=avc1.42E01E,mp4a.40.2',
      extension: 'mp4',
      audio: true,
    });
  });
});
