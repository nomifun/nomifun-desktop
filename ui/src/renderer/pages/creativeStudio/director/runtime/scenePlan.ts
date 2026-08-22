/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { MathUtils, Object3D } from 'three';

import {
  evaluateDirectorFrame,
  type DirectorCamera,
  type DirectorEvaluatedFrame,
  type DirectorState,
  type DirectorTransform3D,
} from '../domain';

export interface DirectorRuntimeFramePlan {
  frame: DirectorEvaluatedFrame;
  activeCameraId: string | null;
  useActiveCamera: boolean;
}

/** Single deterministic bridge from the canonical timeline into Three.js. */
export function createDirectorRuntimeFramePlan(
  state: DirectorState,
  timeSeconds = state.timeline.currentTimeSeconds
): DirectorRuntimeFramePlan {
  const frame = evaluateDirectorFrame(state, timeSeconds);
  const activeCameraId = state.activeCameraId;
  return {
    frame,
    activeCameraId,
    useActiveCamera:
      state.viewMode === 'camera' &&
      activeCameraId !== null &&
      frame.cameras.some((camera) => camera.id === activeCameraId),
  };
}

export function applyDirectorTransform(
  target: Object3D,
  transform: DirectorTransform3D
): void {
  target.position.set(transform.position.x, transform.position.y, transform.position.z);
  target.rotation.set(
    MathUtils.degToRad(transform.rotation.x),
    MathUtils.degToRad(transform.rotation.y),
    MathUtils.degToRad(transform.rotation.z),
    'XYZ'
  );
  target.scale.set(transform.scale.x, transform.scale.y, transform.scale.z);
}

/** Three.js uses a vertical field of view; the domain stores physical focal length. */
export function directorVerticalFovDegrees(camera: DirectorCamera): number {
  const aspect = camera.aspectRatio.width / camera.aspectRatio.height;
  const filmHeight = 36 / Math.max(aspect, 1);
  return MathUtils.radToDeg(2 * Math.atan(filmHeight / (2 * camera.focalLengthMm)));
}
