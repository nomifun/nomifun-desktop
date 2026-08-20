/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { directorCommands } from './commands';
import { createDirectorCamera, createDirectorCharacter, createDirectorLight, createDirectorState } from './model';
import { directorReducer } from './reducer';
import {
  clampDirectorTime,
  evaluateDirectorFrame,
  interpolateDirectorAngleDegrees,
  interpolateDirectorNumber,
  interpolateDirectorVector3,
  sampleDirectorTrack,
} from './timeline';
import type {
  DirectorBooleanTrack,
  DirectorNumberTrack,
  DirectorVectorTrack,
} from './types';

describe('director keyframe interpolation', () => {
  test('clamps time and interpolates scalar/vector easing deterministically', () => {
    expect(clampDirectorTime(-1, 10)).toBe(0);
    expect(clampDirectorTime(12, 10)).toBe(10);
    expect(clampDirectorTime(Number.NaN, 10)).toBe(0);
    expect(interpolateDirectorNumber(0, 8, 0.25, 'linear')).toBe(2);
    expect(interpolateDirectorNumber(0, 8, 0.25, 'ease-in-out')).toBe(1.25);
    expect(interpolateDirectorNumber(0, 8, 0.75, 'step')).toBe(0);
    expect(interpolateDirectorNumber(0, 8, 1, 'step')).toBe(8);
    expect(
      interpolateDirectorVector3(
        { x: 0, y: 10, z: -10 },
        { x: 10, y: 20, z: 0 },
        0.5
      )
    ).toEqual({ x: 5, y: 15, z: -5 });
  });

  test('uses the shortest rotation arc', () => {
    expect(interpolateDirectorAngleDegrees(350, 10, 0.5)).toBe(360);
    expect(interpolateDirectorAngleDegrees(10, 350, 0.5)).toBe(0);
  });

  test('samples exact boundaries, continuous segments and boolean steps', () => {
    const focalTrack: DirectorNumberTrack = {
      id: 'track-focal',
      target: { kind: 'camera', id: 'camera-1' },
      valueType: 'number',
      property: 'focalLengthMm',
      keyframes: [
        { id: 'kf-focal-1', valueType: 'number', timeSeconds: 0, value: 20, interpolation: 'linear' },
        { id: 'kf-focal-2', valueType: 'number', timeSeconds: 4, value: 60, interpolation: 'linear' },
      ],
    };
    const visibilityTrack: DirectorBooleanTrack = {
      id: 'track-visible',
      target: { kind: 'character', id: 'character-1' },
      valueType: 'boolean',
      property: 'visible',
      keyframes: [
        { id: 'kf-visible-1', valueType: 'boolean', timeSeconds: 0, value: true, interpolation: 'step' },
        { id: 'kf-visible-2', valueType: 'boolean', timeSeconds: 2, value: false, interpolation: 'step' },
      ],
    };
    expect(sampleDirectorTrack(focalTrack, -1)).toBe(20);
    expect(sampleDirectorTrack(focalTrack, 2)).toBe(40);
    expect(sampleDirectorTrack(focalTrack, 4)).toBe(60);
    expect(sampleDirectorTrack(visibilityTrack, 1.99)).toBe(true);
    expect(sampleDirectorTrack(visibilityTrack, 2)).toBe(false);
  });
});

describe('director evaluated frames', () => {
  test('applies tracks to a detached frame without mutating project state', () => {
    let state = createDirectorState({ projectId: 'project-1', name: 'Project', durationSeconds: 4 });
    state = directorReducer(
      state,
      directorCommands.addEntity(createDirectorCamera({ id: 'camera-1', name: 'Camera' }))
    );
    state = directorReducer(
      state,
      directorCommands.addEntity(createDirectorCharacter({ id: 'character-1', name: 'Character' }))
    );
    state = directorReducer(
      state,
      directorCommands.addEntity(createDirectorLight({ id: 'light-1', name: 'Light' }))
    );
    const position: DirectorVectorTrack = {
      id: 'track-position',
      target: { kind: 'character', id: 'character-1' },
      valueType: 'vector3',
      property: 'position',
      keyframes: [
        { id: 'kf-position-1', valueType: 'vector3', timeSeconds: 0, value: { x: 0, y: 0, z: 0 }, interpolation: 'linear' },
        { id: 'kf-position-2', valueType: 'vector3', timeSeconds: 4, value: { x: 8, y: 4, z: 0 }, interpolation: 'linear' },
      ],
    };
    const focal: DirectorNumberTrack = {
      id: 'track-focal',
      target: { kind: 'camera', id: 'camera-1' },
      valueType: 'number',
      property: 'focalLengthMm',
      keyframes: [
        { id: 'kf-focal-1', valueType: 'number', timeSeconds: 0, value: 20, interpolation: 'linear' },
        { id: 'kf-focal-2', valueType: 'number', timeSeconds: 4, value: 60, interpolation: 'linear' },
      ],
    };
    const intensity: DirectorNumberTrack = {
      id: 'track-intensity',
      target: { kind: 'light', id: 'light-1' },
      valueType: 'number',
      property: 'intensity',
      keyframes: [
        { id: 'kf-light-1', valueType: 'number', timeSeconds: 0, value: 0, interpolation: 'linear' },
        { id: 'kf-light-2', valueType: 'number', timeSeconds: 4, value: 2, interpolation: 'linear' },
      ],
    };
    state = directorReducer(state, directorCommands.upsertTimelineTrack(position));
    state = directorReducer(state, directorCommands.upsertTimelineTrack(focal));
    state = directorReducer(state, directorCommands.upsertTimelineTrack(intensity));

    const before = JSON.stringify(state);
    const frame = evaluateDirectorFrame(state, 2);
    expect(frame.timeSeconds).toBe(2);
    expect(frame.characters[0].transform.position).toEqual({ x: 4, y: 2, z: 0 });
    expect(frame.cameras[0].focalLengthMm).toBe(40);
    expect(frame.lights[0].intensity).toBe(1);
    expect(JSON.stringify(state)).toBe(before);

    frame.characters[0].transform.position.x = 999;
    expect(state.characters[0].transform.position.x).toBe(0);
  });
});
