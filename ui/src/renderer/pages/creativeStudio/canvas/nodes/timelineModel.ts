/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeTimelineClip,
  CreativeTimelineNodeData,
} from '../../domain';

const TIMELINE_DEFAULT_CLIP_DURATION_MS = 5_000;
const TIMELINE_MIN_CLIP_DURATION_MS = 500;
const TIMELINE_DEFAULT_SCALE_MS = 30_000;
const TIMELINE_MAX_DURATION_MS = 86_400_000;

export interface TimelineInsertAsset {
  id: string;
  kind: 'image' | 'video';
}

const finite = (value: number, fallback = 0): number =>
  Number.isFinite(value) ? value : fallback;

const clamp = (value: number, minimum: number, maximum: number): number =>
  Math.min(maximum, Math.max(minimum, finite(value, minimum)));

export const timelineDurationMs = (
  clips: readonly CreativeTimelineClip[]
): number =>
  clips.reduce(
    (duration, clip) =>
      Math.max(duration, finite(clip.startMs) + Math.max(0, finite(clip.durationMs))),
    0
  );

export const timelineScaleDurationMs = (
  clips: readonly CreativeTimelineClip[]
): number => {
  const duration = timelineDurationMs(clips);
  return Math.max(
    TIMELINE_DEFAULT_SCALE_MS,
    Math.ceil(duration / 5_000) * 5_000
  );
};

export const timelineTickValues = (scaleDurationMs: number): number[] => {
  const safeDuration = Math.max(
    TIMELINE_DEFAULT_SCALE_MS,
    finite(scaleDurationMs, TIMELINE_DEFAULT_SCALE_MS)
  );
  const values: number[] = [];
  for (let value = 0; value <= safeDuration; value += 5_000) values.push(value);
  return values;
};

export const appendTimelineClips = (
  data: CreativeTimelineNodeData,
  assets: readonly TimelineInsertAsset[],
  createId: () => string
): CreativeTimelineNodeData => {
  let cursor = timelineDurationMs(data.clips);
  const clips = [...data.clips];
  for (const asset of assets) {
    const clip: CreativeTimelineClip = {
      id: createId(),
      assetId: asset.id,
      kind: asset.kind,
      startMs: cursor,
      durationMs: TIMELINE_DEFAULT_CLIP_DURATION_MS,
      sourceStartMs: 0,
      sourceDurationMs: null,
    };
    clips.push(clip);
    cursor += clip.durationMs;
  }
  return { ...data, clips };
};

export const moveTimelineClip = (
  data: CreativeTimelineNodeData,
  clipId: string,
  startMs: number
): CreativeTimelineNodeData => ({
  ...data,
  clips: data.clips.map((clip) =>
    clip.id === clipId
      ? {
          ...clip,
          startMs: clamp(startMs, 0, TIMELINE_MAX_DURATION_MS - clip.durationMs),
        }
      : clip
  ),
});

export const trimTimelineClip = (
  data: CreativeTimelineNodeData,
  clipId: string,
  edge: 'start' | 'end',
  deltaMs: number
): CreativeTimelineNodeData => ({
  ...data,
  clips: data.clips.map((clip) => {
    if (clip.id !== clipId) return clip;
    if (edge === 'end') {
      const sourceMaximum = clip.sourceDurationMs === null
        ? TIMELINE_MAX_DURATION_MS - clip.startMs
        : Math.max(
            TIMELINE_MIN_CLIP_DURATION_MS,
            clip.sourceDurationMs - clip.sourceStartMs
          );
      return {
        ...clip,
        durationMs: clamp(
          clip.durationMs + deltaMs,
          TIMELINE_MIN_CLIP_DURATION_MS,
          sourceMaximum
        ),
      };
    }

    const maximumForward = clip.durationMs - TIMELINE_MIN_CLIP_DURATION_MS;
    const maximumBackward = Math.min(clip.startMs, clip.sourceStartMs);
    const applied = clamp(deltaMs, -maximumBackward, maximumForward);
    return {
      ...clip,
      startMs: clip.startMs + applied,
      sourceStartMs: clip.sourceStartMs + applied,
      durationMs: clip.durationMs - applied,
    };
  }),
});

export const resolveTimelineClipDuration = (
  data: CreativeTimelineNodeData,
  clipId: string,
  sourceDurationMs: number
): CreativeTimelineNodeData => {
  const normalized = clamp(sourceDurationMs, TIMELINE_MIN_CLIP_DURATION_MS, TIMELINE_MAX_DURATION_MS);
  return {
    ...data,
    clips: data.clips.map((clip) => {
      if (clip.id !== clipId || clip.kind !== 'video') return clip;
      const available = Math.max(
        TIMELINE_MIN_CLIP_DURATION_MS,
        normalized - clip.sourceStartMs
      );
      return {
        ...clip,
        sourceDurationMs: normalized,
        durationMs: Math.min(clip.durationMs, available),
      };
    }),
  };
};

export const removeTimelineClip = (
  data: CreativeTimelineNodeData,
  clipId: string
): CreativeTimelineNodeData => ({
  ...data,
  clips: data.clips.filter((clip) => clip.id !== clipId),
});

export const timelineClipAtTime = (
  clips: readonly CreativeTimelineClip[],
  timeMs: number
): CreativeTimelineClip | null => {
  const ordered = [...clips].sort((left, right) => left.startMs - right.startMs);
  for (let index = ordered.length - 1; index >= 0; index -= 1) {
    const clip = ordered[index];
    if (clip && timeMs >= clip.startMs && timeMs < clip.startMs + clip.durationMs) {
      return clip;
    }
  }
  return null;
};
