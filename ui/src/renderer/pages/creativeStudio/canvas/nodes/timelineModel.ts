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
const TIMELINE_DEFAULT_SCALE_MS = 60_000;
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
): CreativeTimelineNodeData => {
  const clip = data.clips.find((item) => item.id === clipId);
  if (!clip) return data;

  const requestedStart = clamp(startMs, 0, TIMELINE_MAX_DURATION_MS - clip.durationMs);
  const neighbors = data.clips
    .filter((item) => item.id !== clipId)
    .sort((left, right) => left.startMs - right.startMs);
  let resolvedStart = clip.startMs;
  let nearestDistance = Number.POSITIVE_INFINITY;
  const considerGap = (start: number, end: number) => {
    if (end - start < clip.durationMs) return;
    const candidate = clamp(requestedStart, start, end - clip.durationMs);
    const distance = Math.abs(candidate - requestedStart);
    // Prefer the original side of a collision when both edges are equally close.
    if (
      distance < nearestDistance ||
      (distance === nearestDistance &&
        Math.abs(candidate - clip.startMs) < Math.abs(resolvedStart - clip.startMs))
    ) {
      resolvedStart = candidate;
      nearestDistance = distance;
    }
  };

  // Only place the whole clip in free time; other clips keep their authored positions.
  let gapStart = 0;
  for (const neighbor of neighbors) {
    considerGap(gapStart, neighbor.startMs);
    gapStart = Math.max(gapStart, neighbor.startMs + neighbor.durationMs);
  }
  considerGap(gapStart, TIMELINE_MAX_DURATION_MS);

  if (resolvedStart === clip.startMs) return data;
  return {
    ...data,
    clips: data.clips.map((item) =>
      item.id === clipId ? { ...item, startMs: resolvedStart } : item
    ),
  };
};

export const reorderTimelineClip = (
  data: CreativeTimelineNodeData,
  clipId: string,
  startMs: number,
  insertionTimeMs: number
): CreativeTimelineNodeData => {
  const ordered = [...data.clips].sort((left, right) => left.startMs - right.startMs);
  const originalIndex = ordered.findIndex((clip) => clip.id === clipId);
  const clip = ordered[originalIndex];
  if (!clip) return data;

  const requestedStart = clamp(startMs, 0, TIMELINE_MAX_DURATION_MS - clip.durationMs);
  const neighbors = ordered.filter((item) => item.id !== clipId);
  const overlaps = neighbors.some((item) =>
    requestedStart < item.startMs + item.durationMs &&
    requestedStart + clip.durationMs > item.startMs
  );
  if (!overlaps) return moveTimelineClip(data, clipId, requestedStart);

  // The pointer chooses the insertion boundary, regardless of where the clip was grabbed.
  const targetTime = finite(insertionTimeMs, requestedStart);
  const nextIndex = neighbors.filter((item) =>
    targetTime >= item.startMs + item.durationMs / 2
  ).length;
  if (nextIndex === originalIndex) return moveTimelineClip(data, clipId, requestedStart);

  const gaps = ordered.map((item, index) => {
    const previous = ordered[index - 1];
    return Math.max(0, item.startMs - (previous ? previous.startMs + previous.durationMs : 0));
  });
  neighbors.splice(nextIndex, 0, clip);
  const starts = new Map<string, number>();
  let cursor = 0;
  for (const [index, item] of neighbors.entries()) {
    // Preserve the existing gaps and shift only the clips between the old and new slots.
    const nextStart = cursor + gaps[index]!;
    starts.set(item.id, nextStart);
    cursor = nextStart + item.durationMs;
  }
  if (cursor > TIMELINE_MAX_DURATION_MS) return moveTimelineClip(data, clipId, requestedStart);

  return {
    ...data,
    clips: data.clips.map((item) => {
      const nextStart = starts.get(item.id)!;
      return nextStart === item.startMs ? item : { ...item, startMs: nextStart };
    }),
  };
};

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
