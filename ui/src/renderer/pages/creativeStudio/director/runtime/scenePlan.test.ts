/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { Group, MathUtils } from 'three';

import {
  createDirectorCamera,
  createDirectorCharacter,
  createDirectorLight,
  createDirectorObject,
  createDirectorState,
  type DirectorVectorTrack,
} from '../domain';
import {
  applyDirectorTransform,
  createDirectorRuntimeFramePlan,
  directorVerticalFovDegrees,
} from './scenePlan';

describe('Director runtime frame plan', () => {
  test('evaluates the canonical timeline and selects only an existing active camera', () => {
    const state = createDirectorState({
      projectId: 'project-1',
      name: 'Project',
      sceneName: 'Scene',
      durationSeconds: 4,
    });
    state.cameras.push(createDirectorCamera({ id: 'camera-1', name: 'Camera', focalLengthMm: 50 }));
    state.characters.push(
      createDirectorCharacter({ id: 'character-1', name: 'Character', assetId: 'asset-character' })
    );
    state.objects.push(createDirectorObject({ id: 'object-1', name: 'Object', assetId: 'asset-object' }));
    state.lights.push(createDirectorLight({ id: 'light-1', name: 'Key light' }));
    const positionTrack: DirectorVectorTrack = {
      id: 'track-position',
      target: { kind: 'character', id: 'character-1' },
      valueType: 'vector3',
      property: 'position',
      keyframes: [
        {
          id: 'key-1',
          valueType: 'vector3',
          timeSeconds: 0,
          value: { x: 0, y: 0, z: 0 },
          interpolation: 'linear',
        },
        {
          id: 'key-2',
          valueType: 'vector3',
          timeSeconds: 4,
          value: { x: 4, y: 8, z: 12 },
          interpolation: 'linear',
        },
      ],
    };
    state.timeline.tracks.push(positionTrack);
    state.activeCameraId = 'camera-1';
    state.viewMode = 'camera';

    const plan = createDirectorRuntimeFramePlan(state, 2);
    expect(plan.useActiveCamera).toBe(true);
    expect(plan.activeCameraId).toBe('camera-1');
    expect(plan.frame.characters[0].transform.position).toEqual({ x: 2, y: 4, z: 6 });
    expect(plan.frame.objects[0].asset?.assetId).toBe('asset-object');
    expect(plan.frame.lights[0].lightType).toBe('directional');
    expect(state.characters[0].transform.position).toEqual({ x: 0, y: 0, z: 0 });

    state.activeCameraId = 'missing-camera';
    expect(createDirectorRuntimeFramePlan(state, 2).useActiveCamera).toBe(false);
  });

  test('projects domain transforms into Three.js coordinates and degree rotations', () => {
    const group = new Group();
    applyDirectorTransform(group, {
      position: { x: 1, y: 2, z: 3 },
      rotation: { x: 90, y: -45, z: 180 },
      scale: { x: 2, y: 3, z: 4 },
    });
    expect(group.position.toArray()).toEqual([1, 2, 3]);
    expect(group.scale.toArray()).toEqual([2, 3, 4]);
    expect(group.rotation.x).toBeCloseTo(MathUtils.degToRad(90));
    expect(group.rotation.y).toBeCloseTo(MathUtils.degToRad(-45));
    expect(group.rotation.z).toBeCloseTo(MathUtils.degToRad(180));
  });

  test('derives a finite vertical field of view from physical focal length', () => {
    const camera = createDirectorCamera({
      id: 'camera-1',
      name: 'Camera',
      focalLengthMm: 50,
      aspectRatio: { width: 16, height: 9 },
    });
    expect(directorVerticalFovDegrees(camera)).toBeCloseTo(22.895, 2);
  });
});
