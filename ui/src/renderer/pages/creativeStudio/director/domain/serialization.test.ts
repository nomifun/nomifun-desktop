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
  createDirectorObject,
  createDirectorState,
} from './model';
import { directorReducer } from './reducer';
import { exportDirectorProjectV1, importDirectorProjectV1 } from './serialization';
import type { DirectorState, DirectorVectorTrack } from './types';

type JsonRecord = Record<string, any>;

function projectState(): DirectorState {
  let state = createDirectorState({
    projectId: 'project-1',
    name: 'Director project',
    sceneName: 'Scene',
    durationSeconds: 8,
  });
  state = directorReducer(
    state,
    directorCommands.addEntity(
      createDirectorCamera({
        id: 'camera-1',
        name: 'Camera',
        aspectRatio: { width: 4, height: 3 },
        guides: { thirds: true },
      })
    )
  );
  state = directorReducer(
    state,
    directorCommands.addEntity(
      createDirectorCharacter({
        id: 'character-1',
        name: 'Character',
        assetId: 'asset-character-1',
      })
    )
  );
  state = directorReducer(
    state,
    directorCommands.addEntity(createDirectorObject({ id: 'object-1', name: 'Prop' }))
  );
  const track: DirectorVectorTrack = {
    id: 'track-1',
    target: { kind: 'character', id: 'character-1' },
    valueType: 'vector3',
    property: 'position',
    keyframes: [
      {
        id: 'keyframe-1',
        valueType: 'vector3',
        timeSeconds: 0,
        value: { x: 0, y: 0, z: 0 },
        interpolation: 'linear',
      },
      {
        id: 'keyframe-2',
        valueType: 'vector3',
        timeSeconds: 8,
        value: { x: 8, y: 0, z: 0 },
        interpolation: 'ease-in-out',
      },
    ],
  };
  state = directorReducer(state, directorCommands.upsertTimelineTrack(track));
  state = directorReducer(state, directorCommands.seekTimeline(3));
  state = directorReducer(state, directorCommands.setTimelinePlaying(true));
  state = directorReducer(state, directorCommands.requestCapture('request-1', 'image'));
  state = directorReducer(state, directorCommands.startCapture('request-1'));
  state = directorReducer(
    state,
    directorCommands.completeCapture('request-1', {
      captureId: 'capture-1',
      assetId: 'asset-capture-1',
      capturedAt: 123,
    })
  );
  return state;
}

function exportJson(state = projectState()): { json: string; document: JsonRecord } {
  const result = exportDirectorProjectV1(state);
  if (!result.ok) throw new Error(`Unexpected export failure: ${result.error.path}`);
  return { json: result.json, document: JSON.parse(result.json) as JsonRecord };
}

function importMutated(mutate: (document: JsonRecord) => void) {
  const { document } = exportJson();
  mutate(document);
  return importDirectorProjectV1(JSON.stringify(document));
}

describe('director JSON v1 round-trip', () => {
  test('round-trips persistent state and resets runtime playback/capture operation', () => {
    const state = projectState();
    expect(state.timeline.playing).toBe(true);
    expect(state.capture.operation.status).toBe('completed');
    const exported = exportDirectorProjectV1(state);
    expect(exported.ok).toBe(true);
    if (!exported.ok) return;

    expect(exported.document.kind).toBe('nomifun.director.project');
    expect(exported.document.version).toBe(1);
    expect('playing' in (exported.document.project.timeline as object)).toBe(false);
    expect('operation' in (exported.document.project.capture as object)).toBe(false);
    expect(exported.document.project.characters[0].asset).toEqual({
      assetId: 'asset-character-1',
    });

    const imported = importDirectorProjectV1(exported.json);
    expect(imported.ok).toBe(true);
    if (!imported.ok) return;
    expect(imported.state.timeline.playing).toBe(false);
    expect(imported.state.capture.operation).toEqual({ status: 'idle' });
    expect(imported.state.capture.records[0].assetId).toBe('asset-capture-1');
    expect(imported.state.timeline.tracks[0].keyframes.map((item) => item.timeSeconds)).toEqual([
      0,
      8,
    ]);
  });

  test('produces canonical stable JSON for an imported v1 project', () => {
    const first = exportJson().json;
    const imported = importDirectorProjectV1(first);
    if (!imported.ok) throw new Error('Expected a valid v1 project');
    const second = exportDirectorProjectV1(imported.state);
    if (!second.ok) throw new Error('Expected a valid second export');
    expect(second.json).toBe(first);
  });
});

describe('director JSON v1 fail-closed validation', () => {
  test('rejects invalid JSON, alien envelopes and unsupported versions', () => {
    const invalidJson = importDirectorProjectV1('{');
    expect(invalidJson.ok).toBe(false);
    if (!invalidJson.ok) expect(invalidJson.error.code).toBe('invalid-json');

    const alien = importDirectorProjectV1(JSON.stringify({ version: 1, project: {} }));
    expect(alien.ok).toBe(false);
    if (!alien.ok) expect(alien.error.code).toBe('invalid-envelope');

    const unsupported = importMutated((document) => {
      document.version = 2;
    });
    expect(unsupported.ok).toBe(false);
    if (!unsupported.ok) expect(unsupported.error.code).toBe('unsupported-version');
  });

  test('rejects URL fields and URL-shaped asset identifiers', () => {
    const withUrl = importMutated((document) => {
      document.project.characters[0].asset.url = 'https://example.test/model.glb';
    });
    expect(withUrl.ok).toBe(false);
    if (!withUrl.ok) {
      expect(withUrl.error.code).toBe('invalid-value');
      expect(withUrl.error.path.endsWith('.url')).toBe(true);
    }

    const urlAsId = importMutated((document) => {
      document.project.characters[0].asset.assetId = 'https://example.test/model.glb';
    });
    expect(urlAsId.ok).toBe(false);
    if (!urlAsId.ok) expect(urlAsId.error.code).toBe('invalid-value');
  });

  test('rejects duplicate IDs and broken entity references', () => {
    const duplicate = importMutated((document) => {
      document.project.objects[0].id = document.project.cameras[0].id;
    });
    expect(duplicate.ok).toBe(false);
    if (!duplicate.ok) expect(duplicate.error.code).toBe('duplicate-id');

    const missingCamera = importMutated((document) => {
      document.project.activeCameraId = 'missing-camera';
    });
    expect(missingCamera.ok).toBe(false);
    if (!missingCamera.ok) expect(missingCamera.error.code).toBe('broken-reference');

    const missingTrackTarget = importMutated((document) => {
      document.project.timeline.tracks[0].target.id = 'missing-character';
    });
    expect(missingTrackTarget.ok).toBe(false);
    if (!missingTrackTarget.ok) expect(missingTrackTarget.error.code).toBe('broken-reference');
  });

  test('rejects ambiguous keyframe order and unknown fields instead of repairing them', () => {
    const unsorted = importMutated((document) => {
      document.project.timeline.tracks[0].keyframes.reverse();
    });
    expect(unsorted.ok).toBe(false);
    if (!unsorted.ok) expect(unsorted.error.code).toBe('invalid-value');

    const unknown = importMutated((document) => {
      document.project.legacyScene = {};
    });
    expect(unknown.ok).toBe(false);
    if (!unknown.ok) {
      expect(unknown.error.code).toBe('invalid-value');
      expect(unknown.error.path).toBe('$.project.legacyScene');
    }
  });

  test('refuses to export an invalid in-memory state', () => {
    const state = projectState();
    state.cameras[0].aspectRatio.width = 0;
    const result = exportDirectorProjectV1(state);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.path.includes('aspectRatio')).toBe(true);
  });
});
