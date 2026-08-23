/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { directorCommands } from './commands';
import {
  createDirectorCamera,
  createDirectorCharacter,
  createDirectorLight,
  createDirectorObject,
  createDirectorState,
} from './model';
import { directorReducer } from './reducer';
import type { DirectorState, DirectorVectorTrack } from './types';

const PROJECT_ID = 'director-project-1';

function initialState(): DirectorState {
  return createDirectorState({
    projectId: PROJECT_ID,
    name: 'Director project',
    sceneName: 'Scene',
    durationSeconds: 10,
  });
}

function withCamera(state = initialState()): DirectorState {
  return directorReducer(
    state,
    directorCommands.addEntity(createDirectorCamera({ id: 'camera-1', name: 'Camera 1' }))
  );
}

describe('director reducer entity and presentation commands', () => {
  test('adds, selects, renames and deletes strongly typed entities', () => {
    let state = initialState();
    const camera = createDirectorCamera({ id: 'camera-1', name: 'Camera 1' });
    const character = createDirectorCharacter({
      id: 'character-1',
      name: 'Character 1',
      assetId: 'asset-character-1',
    });
    const object = createDirectorObject({ id: 'object-1', name: 'Object 1' });
    const light = createDirectorLight({ id: 'light-1', name: 'Key light' });

    state = directorReducer(state, directorCommands.addEntity(camera));
    state = directorReducer(state, directorCommands.addEntity(character));
    state = directorReducer(state, directorCommands.addEntity(object));
    state = directorReducer(state, directorCommands.addEntity(light));

    expect(state.cameras.length).toBe(1);
    expect(state.characters[0].asset).toEqual({ assetId: 'asset-character-1' });
    expect(state.objects.length).toBe(1);
    expect(state.lights.length).toBe(1);
    expect(state.activeCameraId).toBe('camera-1');
    expect(state.selection).toEqual({ kind: 'light', id: 'light-1' });

    state = directorReducer(
      state,
      directorCommands.renameEntity({ kind: 'character', id: 'character-1' }, '  Lead  ')
    );
    expect(state.characters[0].name).toBe('Lead');

    const duplicate = directorReducer(
      state,
      directorCommands.addEntity(createDirectorObject({ id: 'character-1', name: 'Duplicate' }))
    );
    expect(duplicate).toBe(state);

    state = directorReducer(
      state,
      directorCommands.select({ kind: 'character', id: 'character-1' })
    );
    state = directorReducer(
      state,
      directorCommands.deleteEntity({ kind: 'character', id: 'character-1' })
    );
    expect(state.characters.length).toBe(0);
    expect(state.selection).toBeNull();
  });

  test('detaches command payloads and refuses transform changes for locked entities', () => {
    let state = initialState();
    const character = createDirectorCharacter({ id: 'character-1', name: 'Original' });
    const add = directorCommands.addEntity(character);
    character.name = 'Mutated outside';
    state = directorReducer(state, add);
    expect(state.characters[0].name).toBe('Original');

    const reference = { kind: 'character', id: 'character-1' } as const;
    state = directorReducer(state, directorCommands.setEntityLocked(reference, true));
    const lockedState = state;
    state = directorReducer(
      state,
      directorCommands.setEntityTransform(reference, {
        position: { x: 5, y: 0, z: 0 },
        rotation: { x: 0, y: 0, z: 0 },
        scale: { x: 1, y: 1, z: 1 },
      })
    );
    expect(state).toBe(lockedState);

    state = directorReducer(state, directorCommands.setEntityLocked(reference, false));
    state = directorReducer(
      state,
      directorCommands.setEntityTransform(reference, {
        position: { x: 5, y: 0, z: 0 },
        rotation: { x: 0, y: 0, z: 0 },
        scale: { x: 1, y: 1, z: 1 },
      })
    );
    expect(state.characters[0].transform.position.x).toBe(5);
  });

  test('normalizes camera ratios, updates guides and controls view panels', () => {
    let state = initialState();
    const noCameraView = directorReducer(state, directorCommands.setViewMode('camera'));
    expect(noCameraView).toBe(state);
    state = withCamera(state);

    state = directorReducer(
      state,
      directorCommands.setCameraAspectRatio('camera-1', { width: 1920, height: 1080 })
    );
    expect(state.cameras[0].aspectRatio).toEqual({ width: 16, height: 9 });
    const legalRatioState = state;
    state = directorReducer(
      state,
      directorCommands.setCameraAspectRatio('camera-1', { width: -1, height: 9 })
    );
    expect(state).toBe(legalRatioState);

    state = directorReducer(
      state,
      directorCommands.setCameraGuides('camera-1', { thirds: true, safeArea: true })
    );
    expect(state.cameras[0].guides).toEqual({
      frame: true,
      center: false,
      thirds: true,
      safeArea: true,
    });

    state = directorReducer(state, directorCommands.setViewMode('camera'));
    state = directorReducer(state, directorCommands.togglePanel('leftSidebar'));
    state = directorReducer(state, directorCommands.setPanel('timeline', true));
    expect(state.viewMode).toBe('camera');
    expect(state.panels).toEqual({
      leftSidebarOpen: false,
      rightSidebarOpen: true,
      timelineOpen: true,
    });
  });

  test('configures scene presentation without retaining URLs', () => {
    let state = initialState();
    state = directorReducer(
      state,
      directorCommands.configureScene({
        skyColor: '#AABBCC',
        panorama: { assetId: 'panorama-asset-1' },
        panoramaYawDegrees: 90,
        panoramaRadius: 80,
        groundVisible: false,
        gridVisible: false,
        snapToGrid: true,
        characterLabelsVisible: false,
      })
    );
    expect(state.scene.environment).toEqual({
      skyColor: '#aabbcc',
      panorama: { assetId: 'panorama-asset-1' },
      panoramaYawDegrees: 90,
      panoramaRadius: 80,
      groundVisible: false,
      gridVisible: false,
      snapToGrid: true,
      characterLabelsVisible: false,
    });
    expect('url' in (state.scene.environment.panorama as object)).toBe(false);
  });
});

describe('director reducer timeline and capture commands', () => {
  test('clamps seeks, keyframes and playback to the timeline duration', () => {
    let state = directorReducer(
      initialState(),
      directorCommands.addEntity(createDirectorCharacter({ id: 'character-1', name: 'Character' }))
    );
    const track: DirectorVectorTrack = {
      id: 'track-position-1',
      target: { kind: 'character', id: 'character-1' },
      valueType: 'vector3',
      property: 'position',
      keyframes: [
        {
          id: 'keyframe-1',
          valueType: 'vector3',
          timeSeconds: -5,
          value: { x: 0, y: 0, z: 0 },
          interpolation: 'linear',
        },
        {
          id: 'keyframe-2',
          valueType: 'vector3',
          timeSeconds: 50,
          value: { x: 10, y: 0, z: 0 },
          interpolation: 'linear',
        },
      ],
    };
    state = directorReducer(state, directorCommands.upsertTimelineTrack(track));
    expect(state.timeline.tracks[0].keyframes.map((item) => item.timeSeconds)).toEqual([0, 10]);

    state = directorReducer(state, directorCommands.seekTimeline(99));
    expect(state.timeline.currentTimeSeconds).toBe(10);
    state = directorReducer(state, directorCommands.setTimelineDuration(4));
    expect(state.timeline.currentTimeSeconds).toBe(4);
    expect(state.timeline.tracks[0].keyframes.map((item) => item.timeSeconds)).toEqual([0, 4]);

    state = directorReducer(state, directorCommands.seekTimeline(3.5));
    state = directorReducer(state, directorCommands.setTimelinePlaying(true));
    state = directorReducer(state, directorCommands.tickTimeline(1));
    expect(state.timeline.currentTimeSeconds).toBe(4);
    expect(state.timeline.playing).toBe(false);

    state = directorReducer(state, directorCommands.setTimelineLoop(true));
    state = directorReducer(state, directorCommands.seekTimeline(3.5));
    state = directorReducer(state, directorCommands.setTimelinePlaying(true));
    state = directorReducer(state, directorCommands.tickTimeline(1));
    expect(state.timeline.currentTimeSeconds).toBe(0.5);
    expect(state.timeline.playing).toBe(true);
  });

  test('runs a capture lifecycle using only camera and asset IDs', () => {
    let state = withCamera();
    state = directorReducer(
      state,
      directorCommands.configureCapture({
        width: 1280,
        height: 720,
        imageFormat: 'jpeg',
        videoFramesPerSecond: 30,
      })
    );
    state = directorReducer(state, directorCommands.requestCapture('request-1', 'image'));
    expect(state.capture.operation).toEqual({
      status: 'queued',
      request: {
        requestId: 'request-1',
        kind: 'image',
        cameraId: 'camera-1',
        width: 1280,
        height: 720,
        format: 'jpeg',
      },
    });

    const queued = state;
    state = directorReducer(
      state,
      directorCommands.completeCapture('request-1', {
        captureId: 'capture-1',
        assetId: 'asset-output-1',
        capturedAt: 100,
      })
    );
    expect(state).toBe(queued);

    state = directorReducer(state, directorCommands.startCapture('request-1'));
    state = directorReducer(
      state,
      directorCommands.completeCapture('request-1', {
        captureId: 'capture-1',
        assetId: 'asset-output-1',
        capturedAt: 100,
      })
    );
    expect(state.capture.operation.status).toBe('completed');
    expect(state.capture.records).toEqual([
      {
        id: 'capture-1',
        kind: 'image',
        cameraId: 'camera-1',
        assetId: 'asset-output-1',
        capturedAt: 100,
        width: 1280,
        height: 720,
        format: 'jpeg',
      },
    ]);
    expect('url' in state.capture.records[0]).toBe(false);

    state = directorReducer(state, directorCommands.requestCapture('request-2', 'video'));
    state = directorReducer(state, directorCommands.startCapture('request-2'));
    state = directorReducer(state, directorCommands.failCapture('request-2', 'engine-unavailable'));
    expect(state.capture.operation.status).toBe('failed');
  });

  test('deleting a camera removes dependent tracks and captures', () => {
    let state = withCamera();
    const track: DirectorVectorTrack = {
      id: 'camera-track-1',
      target: { kind: 'camera', id: 'camera-1' },
      valueType: 'vector3',
      property: 'position',
      keyframes: [],
    };
    state = directorReducer(state, directorCommands.upsertTimelineTrack(track));
    state = directorReducer(state, directorCommands.requestCapture('request-1', 'image'));
    state = directorReducer(state, directorCommands.startCapture('request-1'));
    state = directorReducer(
      state,
      directorCommands.completeCapture('request-1', {
        captureId: 'capture-1',
        assetId: 'asset-output-1',
        capturedAt: 100,
      })
    );
    state = directorReducer(
      state,
      directorCommands.deleteEntity({ kind: 'camera', id: 'camera-1' })
    );
    expect(state.cameras.length).toBe(0);
    expect(state.activeCameraId).toBeNull();
    expect(state.timeline.tracks.length).toBe(0);
    expect(state.capture.records.length).toBe(0);
    expect(state.capture.operation).toEqual({ status: 'idle' });
    expect(state.viewMode).toBe('director');
  });
});
