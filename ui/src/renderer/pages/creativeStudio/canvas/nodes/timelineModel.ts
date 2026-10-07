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

const orderTimelineClips = (
  clips: readonly CreativeTimelineClip[]
): CreativeTimelineClip[] => clips
  .map((clip, index) => ({ clip, index }))
  .sort(
    (left, right) =>
      left.clip.startMs - right.clip.startMs || left.index - right.index
  )
  .map(({ clip }) => clip);

const placeTimelineClipsContinuously = (
  data: CreativeTimelineNodeData,
  ordered: readonly CreativeTimelineClip[]
): CreativeTimelineNodeData => {
  const starts = new Map<string, number>();
  let cursor = 0;
  for (const clip of ordered) {
    starts.set(clip.id, cursor);
    cursor += clip.durationMs;
  }
  if (cursor > TIMELINE_MAX_DURATION_MS) return data;

  let changed = false;
  const clips = data.clips.map((clip) => {
    const startMs = starts.get(clip.id);
    if (startMs === undefined || startMs === clip.startMs) return clip;
    changed = true;
    return { ...clip, startMs };
  });
  return changed ? { ...data, clips } : data;
};

/** Packs clips from zero in their current timeline order without changing media or duration. */
export const compactTimelineClips = (
  data: CreativeTimelineNodeData
): CreativeTimelineNodeData =>
  placeTimelineClipsContinuously(data, orderTimelineClips(data.clips));

const resizeTimelineClipEnd = (
  data: CreativeTimelineNodeData,
  clipId: string,
  requestedDurationMs: number,
  sourceMaximumMs: number
): CreativeTimelineNodeData => {
  const compacted = compactTimelineClips(data);
  const ordered = orderTimelineClips(compacted.clips);
  const clipIndex = ordered.findIndex((clip) => clip.id === clipId);
  const clip = ordered[clipIndex];
  if (!clip) return compacted;

  const followers = ordered.slice(clipIndex + 1);
  const affectedEndMs = followers.reduce(
    (endMs, item) => Math.max(endMs, item.startMs + item.durationMs),
    clip.startMs + clip.durationMs
  );
  const maximumGrowthMs = Math.max(0, TIMELINE_MAX_DURATION_MS - affectedEndMs);
  const maximumDurationMs = Math.min(
    Math.max(TIMELINE_MIN_CLIP_DURATION_MS, sourceMaximumMs),
    clip.durationMs + maximumGrowthMs
  );
  const durationMs = clamp(
    requestedDurationMs,
    TIMELINE_MIN_CLIP_DURATION_MS,
    maximumDurationMs
  );
  const durationDeltaMs = durationMs - clip.durationMs;
  if (durationDeltaMs === 0) return compacted;

  // Ripple the applied trim through the tail so every edit boundary stays closed.
  const followerIds = new Set(followers.map((item) => item.id));
  return {
    ...compacted,
    clips: compacted.clips.map((item) => {
      if (item.id === clipId) return { ...item, durationMs };
      if (!followerIds.has(item.id)) return item;
      return { ...item, startMs: item.startMs + durationDeltaMs };
    }),
  };
};

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
  const compacted = compactTimelineClips(data);
  let cursor = timelineDurationMs(compacted.clips);
  const clips = [...compacted.clips];
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
  return { ...compacted, clips };
};

export const moveTimelineClip = (
  data: CreativeTimelineNodeData,
  clipId: string,
  startMs: number
): CreativeTimelineNodeData => {
  const compacted = compactTimelineClips(data);
  const clip = compacted.clips.find((item) => item.id === clipId);
  if (!clip) return compacted;

  const requestedStart = clamp(startMs, 0, TIMELINE_MAX_DURATION_MS - clip.durationMs);
  const neighbors = compacted.clips
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

  // Resolve a collision-free intended order; compaction below closes all blank time.
  let gapStart = 0;
  for (const neighbor of neighbors) {
    considerGap(gapStart, neighbor.startMs);
    gapStart = Math.max(gapStart, neighbor.startMs + neighbor.durationMs);
  }
  considerGap(gapStart, TIMELINE_MAX_DURATION_MS);

  if (resolvedStart === clip.startMs) return compacted;
  return compactTimelineClips({
    ...compacted,
    clips: compacted.clips.map((item) =>
      item.id === clipId ? { ...item, startMs: resolvedStart } : item
    ),
  });
};

export const reorderTimelineClip = (
  data: CreativeTimelineNodeData,
  clipId: string,
  startMs: number,
  insertionTimeMs: number
): CreativeTimelineNodeData => {
  const compacted = compactTimelineClips(data);
  const ordered = orderTimelineClips(compacted.clips);
  const originalIndex = ordered.findIndex((clip) => clip.id === clipId);
  const clip = ordered[originalIndex];
  if (!clip) return compacted;

  const requestedStart = clamp(startMs, 0, TIMELINE_MAX_DURATION_MS - clip.durationMs);
  const neighbors = ordered.filter((item) => item.id !== clipId);
  const overlaps = neighbors.some((item) =>
    requestedStart < item.startMs + item.durationMs &&
    requestedStart + clip.durationMs > item.startMs
  );
  if (!overlaps) return moveTimelineClip(compacted, clipId, requestedStart);

  // The pointer chooses the insertion boundary, regardless of where the clip was grabbed.
  const targetTime = finite(insertionTimeMs, requestedStart);
  const nextIndex = neighbors.filter((item) =>
    targetTime >= item.startMs + item.durationMs / 2
  ).length;
  if (nextIndex === originalIndex) return compacted;

  neighbors.splice(nextIndex, 0, clip);
  return placeTimelineClipsContinuously(compacted, neighbors);
};

export const trimTimelineClip = (
  data: CreativeTimelineNodeData,
  clipId: string,
  edge: 'start' | 'end',
  deltaMs: number
): CreativeTimelineNodeData => {
  const compacted = compactTimelineClips(data);
  const clip = compacted.clips.find((item) => item.id === clipId);
  if (!clip) return compacted;
  if (edge === 'end') {
    const sourceMaximum = clip.sourceDurationMs === null
      ? TIMELINE_MAX_DURATION_MS - clip.startMs
      : Math.max(
          TIMELINE_MIN_CLIP_DURATION_MS,
          clip.sourceDurationMs - clip.sourceStartMs
        );
    return resizeTimelineClipEnd(
      compacted,
      clipId,
      clip.durationMs + deltaMs,
      sourceMaximum
    );
  }

  const maximumForward = clip.durationMs - TIMELINE_MIN_CLIP_DURATION_MS;
  const maximumBackward = Math.min(clip.startMs, clip.sourceStartMs);
  const applied = clamp(deltaMs, -maximumBackward, maximumForward);
  if (applied === 0) return compacted;
  return compactTimelineClips({
    ...compacted,
    clips: compacted.clips.map((item) =>
      item.id === clipId
        ? {
            ...item,
            startMs: item.startMs + applied,
            sourceStartMs: item.sourceStartMs + applied,
            durationMs: item.durationMs - applied,
          }
        : item
    ),
  });
};

export const resolveTimelineClipDuration = (
  data: CreativeTimelineNodeData,
  clipId: string,
  sourceDurationMs: number
): CreativeTimelineNodeData => {
  const compacted = compactTimelineClips(data);
  const normalized = clamp(sourceDurationMs, TIMELINE_MIN_CLIP_DURATION_MS, TIMELINE_MAX_DURATION_MS);
  const clip = compacted.clips.find((item) => item.id === clipId);
  if (!clip || clip.kind !== 'video') return compacted;
  const available = Math.max(
    TIMELINE_MIN_CLIP_DURATION_MS,
    normalized - clip.sourceStartMs
  );
  const withMetadata = {
    ...compacted,
    clips: compacted.clips.map((item) =>
      item.id === clipId
        ? {
            ...item,
            sourceDurationMs: normalized,
          }
        : item
    ),
  };
  return resizeTimelineClipEnd(
    withMetadata,
    clipId,
    Math.min(clip.durationMs, available),
    available
  );
};

export const removeTimelineClip = (
  data: CreativeTimelineNodeData,
  clipId: string
): CreativeTimelineNodeData => compactTimelineClips({
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
