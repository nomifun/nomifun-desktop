/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  DIRECTOR_LIMITS,
  cloneDirectorEntity,
  cloneDirectorTrack,
  cloneDirectorTransform,
  cloneDirectorVector3,
  isDirectorId,
  isDirectorTrackCompatible,
  isDirectorVector3,
} from './model';
import type {
  DirectorBooleanKeyframe,
  DirectorEvaluatedFrame,
  DirectorKeyframe,
  DirectorNumberKeyframe,
  DirectorState,
  DirectorTimelineTrack,
  DirectorVector3,
  DirectorVectorKeyframe,
} from './types';

export function clampDirectorTime(timeSeconds: number, durationSeconds: number): number {
  if (!Number.isFinite(durationSeconds) || durationSeconds <= 0) return 0;
  if (!Number.isFinite(timeSeconds)) return 0;
  return Math.min(durationSeconds, Math.max(0, timeSeconds));
}

export function interpolateDirectorNumber(
  from: number,
  to: number,
  progress: number,
  interpolation: 'linear' | 'step' | 'ease-in-out' = 'linear'
): number {
  const amount = Math.min(1, Math.max(0, progress));
  if (interpolation === 'step') return amount >= 1 ? to : from;
  const eased = interpolation === 'ease-in-out' ? amount * amount * (3 - 2 * amount) : amount;
  return from + (to - from) * eased;
}

export function interpolateDirectorAngleDegrees(
  from: number,
  to: number,
  progress: number,
  interpolation: 'linear' | 'step' | 'ease-in-out' = 'linear'
): number {
  if (interpolation === 'step') return progress >= 1 ? to : from;
  const shortestDelta = ((to - from + 540) % 360) - 180;
  return interpolateDirectorNumber(from, from + shortestDelta, progress, interpolation);
}

export function interpolateDirectorVector3(
  from: DirectorVector3,
  to: DirectorVector3,
  progress: number,
  interpolation: 'linear' | 'step' | 'ease-in-out' = 'linear'
): DirectorVector3 {
  return {
    x: interpolateDirectorNumber(from.x, to.x, progress, interpolation),
    y: interpolateDirectorNumber(from.y, to.y, progress, interpolation),
    z: interpolateDirectorNumber(from.z, to.z, progress, interpolation),
  };
}

function interpolateDirectorRotation(
  from: DirectorVector3,
  to: DirectorVector3,
  progress: number,
  interpolation: 'linear' | 'step' | 'ease-in-out'
): DirectorVector3 {
  return {
    x: interpolateDirectorAngleDegrees(from.x, to.x, progress, interpolation),
    y: interpolateDirectorAngleDegrees(from.y, to.y, progress, interpolation),
    z: interpolateDirectorAngleDegrees(from.z, to.z, progress, interpolation),
  };
}

function segment<Keyframe extends DirectorKeyframe>(
  keyframes: readonly Keyframe[],
  timeSeconds: number
): { left: Keyframe; right: Keyframe; progress: number } | null {
  if (keyframes.length === 0) return null;
  if (timeSeconds <= keyframes[0].timeSeconds) {
    return { left: keyframes[0], right: keyframes[0], progress: 0 };
  }
  const last = keyframes[keyframes.length - 1];
  if (timeSeconds >= last.timeSeconds) return { left: last, right: last, progress: 0 };
  for (let index = 1; index < keyframes.length; index += 1) {
    const right = keyframes[index];
    if (timeSeconds > right.timeSeconds) continue;
    const left = keyframes[index - 1];
    if (timeSeconds === right.timeSeconds) return { left: right, right, progress: 0 };
    return {
      left,
      right,
      progress: (timeSeconds - left.timeSeconds) / (right.timeSeconds - left.timeSeconds),
    };
  }
  return { left: last, right: last, progress: 0 };
}

export function sampleDirectorTrack(
  track: DirectorTimelineTrack,
  timeSeconds: number
): number | boolean | DirectorVector3 | null {
  const sampleTime = Number.isFinite(timeSeconds) ? timeSeconds : 0;
  if (track.valueType === 'number') {
    const current = segment(track.keyframes, sampleTime);
    if (!current) return null;
    if (current.left === current.right) return current.left.value;
    return interpolateDirectorNumber(
      current.left.value,
      current.right.value,
      current.progress,
      current.left.interpolation
    );
  }
  if (track.valueType === 'boolean') {
    const current = segment(track.keyframes, sampleTime);
    if (!current) return null;
    return current.left.value;
  }
  const current = segment(track.keyframes, sampleTime);
  if (!current) return null;
  if (current.left === current.right) return cloneDirectorVector3(current.left.value);
  return track.property === 'rotation'
    ? interpolateDirectorRotation(
        current.left.value,
        current.right.value,
        current.progress,
        current.left.interpolation
      )
    : interpolateDirectorVector3(
        current.left.value,
        current.right.value,
        current.progress,
        current.left.interpolation
      );
}

function keyframeValueIsValid(track: DirectorTimelineTrack, keyframe: DirectorKeyframe): boolean {
  if (track.valueType !== keyframe.valueType) return false;
  if (!isDirectorId(keyframe.id) || !Number.isFinite(keyframe.timeSeconds)) return false;
  if (track.valueType === 'boolean') {
    return (
      keyframe.valueType === 'boolean' &&
      typeof keyframe.value === 'boolean' &&
      keyframe.interpolation === 'step'
    );
  }
  if (!['linear', 'step', 'ease-in-out'].includes(keyframe.interpolation)) return false;
  if (track.valueType === 'number') {
    if (keyframe.valueType !== 'number' || !Number.isFinite(keyframe.value)) return false;
    return track.property === 'focalLengthMm'
      ? keyframe.value >= 1 && keyframe.value <= 1_000
      : keyframe.value >= 0 && keyframe.value <= DIRECTOR_LIMITS.maxIntensity;
  }
  if (keyframe.valueType !== 'vector3') return false;
  return isDirectorVector3(keyframe.value, track.property === 'scale');
}

export function isDirectorKeyframeForTrack(
  track: DirectorTimelineTrack,
  keyframe: DirectorKeyframe
): boolean {
  return keyframeValueIsValid(track, keyframe);
}

export function isDirectorTimelineTrackShape(track: DirectorTimelineTrack): boolean {
  if (!isDirectorId(track.id) || !isDirectorTrackCompatible(track)) return false;
  if (track.target.kind !== 'scene' && !isDirectorId(track.target.id)) return false;
  if (track.keyframes.length > DIRECTOR_LIMITS.maxKeyframesPerTrack) return false;
  if (
    (track.valueType === 'vector3' &&
      !['position', 'rotation', 'scale'].includes(track.property)) ||
    (track.valueType === 'boolean' && track.property !== 'visible') ||
    (track.valueType === 'number' &&
      !['focalLengthMm', 'intensity'].includes(track.property))
  ) {
    return false;
  }
  return track.keyframes.every((keyframe) => keyframeValueIsValid(track, keyframe));
}

export function canonicalizeDirectorTrack(
  track: DirectorTimelineTrack,
  durationSeconds: number
): DirectorTimelineTrack {
  const duration = Math.max(0, durationSeconds);
  const byTime = new Map<number, DirectorKeyframe>();
  for (const keyframe of track.keyframes) {
    const cloned = {
      ...keyframe,
      timeSeconds: clampDirectorTime(keyframe.timeSeconds, duration),
      value:
        keyframe.valueType === 'vector3'
          ? cloneDirectorVector3(keyframe.value)
          : keyframe.value,
    } as DirectorKeyframe;
    byTime.set(cloned.timeSeconds, cloned);
  }
  const keyframes = [...byTime.values()].sort(
    (left, right) => left.timeSeconds - right.timeSeconds
  );
  return { ...cloneDirectorTrack(track), keyframes } as DirectorTimelineTrack;
}

export function upsertDirectorKeyframe(
  track: DirectorTimelineTrack,
  keyframe: DirectorKeyframe,
  durationSeconds: number
): DirectorTimelineTrack | null {
  if (!isDirectorKeyframeForTrack(track, keyframe)) return null;
  const keyframes = track.keyframes.filter(
    (current) => current.id !== keyframe.id && current.timeSeconds !== keyframe.timeSeconds
  );
  keyframes.push(
    keyframe.valueType === 'vector3'
      ? ({ ...keyframe, value: cloneDirectorVector3(keyframe.value) } as DirectorVectorKeyframe)
      : ({ ...keyframe } as DirectorNumberKeyframe | DirectorBooleanKeyframe)
  );
  return canonicalizeDirectorTrack(
    { ...track, keyframes } as DirectorTimelineTrack,
    durationSeconds
  );
}

function findFrameEntity(frame: DirectorEvaluatedFrame, kind: string, id: string) {
  switch (kind) {
    case 'camera':
      return frame.cameras.find((item) => item.id === id);
    case 'character':
      return frame.characters.find((item) => item.id === id);
    case 'object':
      return frame.objects.find((item) => item.id === id);
    case 'light':
      return frame.lights.find((item) => item.id === id);
    default:
      return undefined;
  }
}

export function evaluateDirectorFrame(
  state: DirectorState,
  timeSeconds = state.timeline.currentTimeSeconds
): DirectorEvaluatedFrame {
  const frame: DirectorEvaluatedFrame = {
    timeSeconds: clampDirectorTime(timeSeconds, state.timeline.durationSeconds),
    scene: {
      ...state.scene,
      transform: cloneDirectorTransform(state.scene.transform),
      environment: {
        ...state.scene.environment,
        panorama: state.scene.environment.panorama
          ? { assetId: state.scene.environment.panorama.assetId }
          : null,
      },
    },
    cameras: state.cameras.map(cloneDirectorEntity),
    characters: state.characters.map(cloneDirectorEntity),
    objects: state.objects.map(cloneDirectorEntity),
    lights: state.lights.map(cloneDirectorEntity),
  };

  for (const track of state.timeline.tracks) {
    const value = sampleDirectorTrack(track, frame.timeSeconds);
    if (value === null) continue;
    if (track.target.kind === 'scene') {
      if (track.valueType === 'vector3') {
        frame.scene.transform = {
          ...frame.scene.transform,
          [track.property]: cloneDirectorVector3(value as DirectorVector3),
        };
      }
      continue;
    }
    const entity = findFrameEntity(frame, track.target.kind, track.target.id);
    if (!entity) continue;
    if (track.valueType === 'vector3') {
      entity.transform = {
        ...entity.transform,
        [track.property]: cloneDirectorVector3(value as DirectorVector3),
      };
    } else if (track.valueType === 'boolean') {
      entity.visible = value as boolean;
    } else if (track.property === 'focalLengthMm' && entity.kind === 'camera') {
      entity.focalLengthMm = value as number;
    } else if (track.property === 'intensity' && entity.kind === 'light') {
      entity.intensity = value as number;
    }
  }
  return frame;
}
