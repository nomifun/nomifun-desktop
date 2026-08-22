/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { directorPanelKey } from './commands';
import type { DirectorCommand } from './commands';
import {
  DIRECTOR_LIMITS,
  cloneDirectorAsset,
  cloneDirectorEntity,
  cloneDirectorTransform,
  directorEntityRef,
  directorSelectionExists,
  directorTargetExists,
  directorTrackKey,
  entityCollectionKey,
  findDirectorEntity,
  hasDirectorEntityId,
  isDirectorAssetId,
  isDirectorErrorCode,
  isDirectorHexColor,
  isDirectorId,
  isDirectorTransform,
  isFiniteDirectorNumber,
  normalizeDirectorAspectRatio,
  normalizeDirectorName,
} from './model';
import {
  canonicalizeDirectorTrack,
  clampDirectorTime,
  isDirectorTimelineTrackShape,
  upsertDirectorKeyframe,
} from './timeline';
import type {
  DirectorCamera,
  DirectorCameraGuides,
  DirectorCaptureRecord,
  DirectorCaptureRequest,
  DirectorEntity,
  DirectorEntityRef,
  DirectorLight,
  DirectorSceneEnvironment,
  DirectorState,
  DirectorTimelineTrack,
} from './types';

function hasOnlyKeys(value: object, allowed: readonly string[]): boolean {
  const keys = new Set(allowed);
  return Object.keys(value).every((key) => keys.has(key));
}

function entityIsValid(entity: DirectorEntity): boolean {
  if (
    !isDirectorId(entity.id) ||
    !normalizeDirectorName(entity.name) ||
    !isDirectorTransform(entity.transform) ||
    typeof entity.visible !== 'boolean' ||
    typeof entity.locked !== 'boolean'
  ) {
    return false;
  }
  if (entity.kind === 'character' || entity.kind === 'object') {
    return entity.asset === null || isDirectorAssetId(entity.asset.assetId);
  }
  if (entity.kind === 'camera') {
    return (
      (entity.projection === 'perspective' || entity.projection === 'orthographic') &&
      isFiniteDirectorNumber(entity.focalLengthMm, 1, 1_000) &&
      isFiniteDirectorNumber(entity.orthographicSize, 0.000001, DIRECTOR_LIMITS.maxScale) &&
      isFiniteDirectorNumber(entity.nearClip, 0.000001, DIRECTOR_LIMITS.maxCoordinate) &&
      isFiniteDirectorNumber(
        entity.farClip,
        entity.nearClip + 0.000001,
        DIRECTOR_LIMITS.maxCoordinate
      ) &&
      normalizeDirectorAspectRatio(entity.aspectRatio) !== null &&
      Object.values(entity.guides).every((value) => typeof value === 'boolean')
    );
  }
  return (
    ['ambient', 'directional', 'point', 'spot'].includes(entity.lightType) &&
    isDirectorHexColor(entity.color) &&
    isFiniteDirectorNumber(entity.intensity, 0, DIRECTOR_LIMITS.maxIntensity) &&
    isFiniteDirectorNumber(entity.range, 0, DIRECTOR_LIMITS.maxCoordinate) &&
    isFiniteDirectorNumber(entity.coneAngleDegrees, 0, 180)
  );
}

function replaceEntity(
  state: DirectorState,
  reference: DirectorEntityRef,
  update: (entity: DirectorEntity) => DirectorEntity
): DirectorState {
  const current = findDirectorEntity(state, reference);
  if (!current) return state;
  const next = update(current);
  if (next === current) return state;
  switch (reference.kind) {
    case 'camera':
      return {
        ...state,
        cameras: state.cameras.map((item) =>
          item.id === reference.id ? (next as DirectorCamera) : item
        ),
      };
    case 'character':
      return {
        ...state,
        characters: state.characters.map((item) =>
          item.id === reference.id ? (next as typeof item) : item
        ),
      };
    case 'object':
      return {
        ...state,
        objects: state.objects.map((item) =>
          item.id === reference.id ? (next as typeof item) : item
        ),
      };
    case 'light':
      return {
        ...state,
        lights: state.lights.map((item) =>
          item.id === reference.id ? (next as DirectorLight) : item
        ),
      };
  }
}

function addEntity(
  state: DirectorState,
  entity: DirectorEntity,
  select: boolean
): DirectorState {
  if (!entityIsValid(entity) || hasDirectorEntityId(state, entity.id)) return state;
  const collection = entityCollectionKey(entity.kind);
  if (state[collection].length >= DIRECTOR_LIMITS.maxEntitiesPerKind) return state;
  const cloned = cloneDirectorEntity(entity);
  const selection = select ? directorEntityRef(cloned) : state.selection;
  switch (cloned.kind) {
    case 'camera':
      return {
        ...state,
        cameras: [...state.cameras, cloned],
        activeCameraId: state.activeCameraId ?? cloned.id,
        selection,
      };
    case 'character':
      return { ...state, characters: [...state.characters, cloned], selection };
    case 'object':
      return { ...state, objects: [...state.objects, cloned], selection };
    case 'light':
      return { ...state, lights: [...state.lights, cloned], selection };
  }
}

function deleteEntity(state: DirectorState, reference: DirectorEntityRef): DirectorState {
  if (!findDirectorEntity(state, reference)) return state;
  const selection =
    state.selection?.kind === reference.kind &&
    state.selection.id === reference.id
      ? null
      : state.selection;
  const tracks = state.timeline.tracks.filter(
    (track) =>
      !(
        track.target.kind === reference.kind &&
        track.target.id === reference.id
      )
  );
  const base: DirectorState = {
    ...state,
    selection,
    timeline: { ...state.timeline, tracks },
  };
  switch (reference.kind) {
    case 'camera': {
      const cameras = state.cameras.filter((item) => item.id !== reference.id);
      const activeCameraId =
        state.activeCameraId === reference.id ? (cameras[0]?.id ?? null) : state.activeCameraId;
      const operation =
        state.capture.operation.status !== 'idle' &&
        state.capture.operation.request.cameraId === reference.id
          ? ({ status: 'idle' } as const)
          : state.capture.operation;
      return {
        ...base,
        cameras,
        activeCameraId,
        viewMode: activeCameraId === null ? 'director' : state.viewMode,
        capture: {
          ...state.capture,
          operation,
          records: state.capture.records.filter((record) => record.cameraId !== reference.id),
        },
      };
    }
    case 'character':
      return {
        ...base,
        characters: state.characters.filter((item) => item.id !== reference.id),
      };
    case 'object':
      return { ...base, objects: state.objects.filter((item) => item.id !== reference.id) };
    case 'light':
      return { ...base, lights: state.lights.filter((item) => item.id !== reference.id) };
  }
}

function configureScene(
  state: DirectorState,
  patch: Partial<DirectorSceneEnvironment>
): DirectorState {
  if (
    !hasOnlyKeys(patch, [
      'skyColor',
      'panorama',
      'panoramaYawDegrees',
      'panoramaRadius',
      'groundVisible',
      'gridVisible',
      'snapToGrid',
      'characterLabelsVisible',
    ])
  ) {
    return state;
  }
  if (patch.skyColor !== undefined && !isDirectorHexColor(patch.skyColor)) return state;
  if (
    patch.panorama !== undefined &&
    patch.panorama !== null &&
    !isDirectorAssetId(patch.panorama.assetId)
  ) {
    return state;
  }
  if (
    patch.panoramaYawDegrees !== undefined &&
    !isFiniteDirectorNumber(
      patch.panoramaYawDegrees,
      -DIRECTOR_LIMITS.maxCoordinate,
      DIRECTOR_LIMITS.maxCoordinate
    )
  ) {
    return state;
  }
  if (
    patch.panoramaRadius !== undefined &&
    !isFiniteDirectorNumber(patch.panoramaRadius, 0.000001, DIRECTOR_LIMITS.maxCoordinate)
  ) {
    return state;
  }
  const booleans: Array<keyof DirectorSceneEnvironment> = [
    'groundVisible',
    'gridVisible',
    'snapToGrid',
    'characterLabelsVisible',
  ];
  if (booleans.some((key) => patch[key] !== undefined && typeof patch[key] !== 'boolean')) {
    return state;
  }
  return {
    ...state,
    scene: {
      ...state.scene,
      environment: {
        ...state.scene.environment,
        ...patch,
        skyColor: patch.skyColor?.toLowerCase() ?? state.scene.environment.skyColor,
        panorama:
          patch.panorama === undefined
            ? state.scene.environment.panorama
            : cloneDirectorAsset(patch.panorama),
      },
    },
  };
}

function setCameraGuides(
  state: DirectorState,
  cameraId: string,
  guides: Partial<DirectorCameraGuides>
): DirectorState {
  if (
    !hasOnlyKeys(guides, ['frame', 'center', 'thirds', 'safeArea']) ||
    Object.values(guides).some((value) => typeof value !== 'boolean')
  ) {
    return state;
  }
  return replaceEntity(state, { kind: 'camera', id: cameraId }, (entity) => {
    if (entity.kind !== 'camera') return entity;
    return { ...entity, guides: { ...entity.guides, ...guides } };
  });
}

function configureCamera(
  state: DirectorState,
  cameraId: string,
  patch: Extract<DirectorCommand, { type: 'camera/configure' }>['patch']
): DirectorState {
  if (
    !hasOnlyKeys(patch, [
      'projection',
      'focalLengthMm',
      'orthographicSize',
      'nearClip',
      'farClip',
    ])
  ) {
    return state;
  }
  const camera = state.cameras.find((item) => item.id === cameraId);
  if (!camera) return state;
  const next = { ...camera, ...patch };
  if (
    !['perspective', 'orthographic'].includes(next.projection) ||
    !isFiniteDirectorNumber(next.focalLengthMm, 1, 1_000) ||
    !isFiniteDirectorNumber(next.orthographicSize, 0.000001, DIRECTOR_LIMITS.maxScale) ||
    !isFiniteDirectorNumber(next.nearClip, 0.000001, DIRECTOR_LIMITS.maxCoordinate) ||
    !isFiniteDirectorNumber(
      next.farClip,
      next.nearClip + 0.000001,
      DIRECTOR_LIMITS.maxCoordinate
    )
  ) {
    return state;
  }
  return replaceEntity(state, { kind: 'camera', id: cameraId }, () => next);
}

function configureLight(
  state: DirectorState,
  lightId: string,
  patch: Extract<DirectorCommand, { type: 'light/configure' }>['patch']
): DirectorState {
  if (!hasOnlyKeys(patch, ['lightType', 'color', 'intensity', 'range', 'coneAngleDegrees'])) {
    return state;
  }
  const light = state.lights.find((item) => item.id === lightId);
  if (!light) return state;
  const next = { ...light, ...patch };
  if (
    !['ambient', 'directional', 'point', 'spot'].includes(next.lightType) ||
    !isDirectorHexColor(next.color) ||
    !isFiniteDirectorNumber(next.intensity, 0, DIRECTOR_LIMITS.maxIntensity) ||
    !isFiniteDirectorNumber(next.range, 0, DIRECTOR_LIMITS.maxCoordinate) ||
    !isFiniteDirectorNumber(next.coneAngleDegrees, 0, 180)
  ) {
    return state;
  }
  return replaceEntity(state, { kind: 'light', id: lightId }, () => ({
    ...next,
    color: next.color.toLowerCase(),
  }));
}

function setTimelineDuration(state: DirectorState, durationSeconds: number): DirectorState {
  if (!isFiniteDirectorNumber(durationSeconds, 0, DIRECTOR_LIMITS.maxDurationSeconds)) return state;
  return {
    ...state,
    timeline: {
      ...state.timeline,
      durationSeconds,
      currentTimeSeconds: clampDirectorTime(state.timeline.currentTimeSeconds, durationSeconds),
      playing: durationSeconds > 0 ? state.timeline.playing : false,
      tracks: state.timeline.tracks.map((track) =>
        canonicalizeDirectorTrack(track, durationSeconds)
      ),
    },
  };
}

function tickTimeline(state: DirectorState, deltaSeconds: number): DirectorState {
  if (!state.timeline.playing || !isFiniteDirectorNumber(deltaSeconds, 0)) return state;
  const duration = state.timeline.durationSeconds;
  if (duration <= 0) {
    return { ...state, timeline: { ...state.timeline, currentTimeSeconds: 0, playing: false } };
  }
  const elapsed = state.timeline.currentTimeSeconds + deltaSeconds;
  if (state.timeline.loop) {
    return {
      ...state,
      timeline: { ...state.timeline, currentTimeSeconds: elapsed % duration },
    };
  }
  if (elapsed >= duration) {
    return {
      ...state,
      timeline: { ...state.timeline, currentTimeSeconds: duration, playing: false },
    };
  }
  return { ...state, timeline: { ...state.timeline, currentTimeSeconds: elapsed } };
}

function upsertTrack(state: DirectorState, track: DirectorTimelineTrack): DirectorState {
  if (
    !isDirectorTimelineTrackShape(track) ||
    !directorTargetExists(state, track.target) ||
    state.timeline.tracks.length >= DIRECTOR_LIMITS.maxTracks &&
      !state.timeline.tracks.some((item) => item.id === track.id)
  ) {
    return state;
  }
  const keyframeIds = new Set<string>();
  if (track.keyframes.some((keyframe) => keyframeIds.size === keyframeIds.add(keyframe.id).size)) {
    return state;
  }
  const otherTracks = state.timeline.tracks.filter((item) => item.id !== track.id);
  if (
    otherTracks.some(
      (item) =>
        directorTrackKey(item) === directorTrackKey(track) ||
        item.keyframes.some((keyframe) => keyframeIds.has(keyframe.id))
    )
  ) {
    return state;
  }
  const normalized = canonicalizeDirectorTrack(track, state.timeline.durationSeconds);
  const existingIndex = state.timeline.tracks.findIndex((item) => item.id === track.id);
  const tracks = [...state.timeline.tracks];
  if (existingIndex >= 0) tracks[existingIndex] = normalized;
  else tracks.push(normalized);
  return { ...state, timeline: { ...state.timeline, tracks } };
}

function configureCapture(
  state: DirectorState,
  patch: Partial<DirectorState['capture']['settings']>
): DirectorState {
  if (!hasOnlyKeys(patch, ['width', 'height', 'imageFormat', 'videoFramesPerSecond'])) {
    return state;
  }
  const settings = { ...state.capture.settings, ...patch };
  if (
    !Number.isSafeInteger(settings.width) ||
    !Number.isSafeInteger(settings.height) ||
    settings.width <= 0 ||
    settings.height <= 0 ||
    settings.width > DIRECTOR_LIMITS.maxCaptureDimension ||
    settings.height > DIRECTOR_LIMITS.maxCaptureDimension ||
    !['png', 'jpeg'].includes(settings.imageFormat) ||
    !Number.isSafeInteger(settings.videoFramesPerSecond) ||
    settings.videoFramesPerSecond < 1 ||
    settings.videoFramesPerSecond > 240
  ) {
    return state;
  }
  return { ...state, capture: { ...state.capture, settings } };
}

function requestCapture(
  state: DirectorState,
  command: Extract<DirectorCommand, { type: 'capture/request' }>
): DirectorState {
  if (
    !isDirectorId(command.requestId) ||
    state.capture.operation.status === 'queued' ||
    state.capture.operation.status === 'capturing'
  ) {
    return state;
  }
  const cameraId = command.cameraId ?? state.activeCameraId;
  if (!cameraId || !state.cameras.some((camera) => camera.id === cameraId)) return state;
  const settings = state.capture.settings;
  const request: DirectorCaptureRequest =
    command.kind === 'image'
      ? {
          requestId: command.requestId,
          kind: 'image',
          cameraId,
          width: settings.width,
          height: settings.height,
          format: settings.imageFormat,
        }
      : {
          requestId: command.requestId,
          kind: 'video',
          cameraId,
          width: settings.width,
          height: settings.height,
          format: 'mp4',
          framesPerSecond: settings.videoFramesPerSecond,
          durationSeconds: state.timeline.durationSeconds,
        };
  if (request.kind === 'video' && request.durationSeconds <= 0) return state;
  return { ...state, capture: { ...state.capture, operation: { status: 'queued', request } } };
}

function completeCapture(
  state: DirectorState,
  command: Extract<DirectorCommand, { type: 'capture/complete' }>
): DirectorState {
  const operation = state.capture.operation;
  if (
    operation.status !== 'capturing' ||
    operation.request.requestId !== command.requestId ||
    !isDirectorId(command.captureId) ||
    !isDirectorAssetId(command.assetId) ||
    !Number.isSafeInteger(command.capturedAt) ||
    command.capturedAt < 0 ||
    state.capture.records.some((record) => record.id === command.captureId) ||
    state.capture.records.length >= DIRECTOR_LIMITS.maxCaptures
  ) {
    return state;
  }
  const request = operation.request;
  const record: DirectorCaptureRecord =
    request.kind === 'image'
      ? {
          id: command.captureId,
          kind: 'image',
          cameraId: request.cameraId,
          assetId: command.assetId,
          capturedAt: command.capturedAt,
          width: request.width,
          height: request.height,
          format: request.format,
        }
      : {
          id: command.captureId,
          kind: 'video',
          cameraId: request.cameraId,
          assetId: command.assetId,
          capturedAt: command.capturedAt,
          width: request.width,
          height: request.height,
          format: request.format,
          framesPerSecond: request.framesPerSecond,
          durationSeconds: request.durationSeconds,
        };
  return {
    ...state,
    capture: {
      ...state.capture,
      operation: {
        status: 'completed',
        request,
        captureId: command.captureId,
        assetId: command.assetId,
      },
      records: [...state.capture.records, record],
    },
  };
}

export function directorReducer(state: DirectorState, command: DirectorCommand): DirectorState {
  switch (command.type) {
    case 'project/rename': {
      const name = normalizeDirectorName(command.name);
      return name && name !== state.name ? { ...state, name } : state;
    }
    case 'scene/rename': {
      const name = normalizeDirectorName(command.name);
      return name && name !== state.scene.name
        ? { ...state, scene: { ...state.scene, name } }
        : state;
    }
    case 'scene/set-transform':
      return isDirectorTransform(command.transform)
        ? {
            ...state,
            scene: { ...state.scene, transform: cloneDirectorTransform(command.transform) },
          }
        : state;
    case 'scene/configure':
      return configureScene(state, command.patch);
    case 'entity/add':
      return addEntity(state, command.entity, command.select);
    case 'entity/delete':
      return deleteEntity(state, command.reference);
    case 'entity/rename': {
      const name = normalizeDirectorName(command.name);
      return name
        ? replaceEntity(state, command.reference, (entity) =>
            entity.name === name ? entity : { ...entity, name }
          )
        : state;
    }
    case 'entity/set-transform':
      return isDirectorTransform(command.transform)
        ? replaceEntity(state, command.reference, (entity) =>
            entity.locked
              ? entity
              : { ...entity, transform: cloneDirectorTransform(command.transform) }
          )
        : state;
    case 'entity/set-visible':
      return replaceEntity(state, command.reference, (entity) => ({
        ...entity,
        visible: command.visible,
      }));
    case 'entity/set-locked':
      return replaceEntity(state, command.reference, (entity) => ({
        ...entity,
        locked: command.locked,
      }));
    case 'entity/set-asset':
      return command.asset === null || isDirectorAssetId(command.asset.assetId)
        ? replaceEntity(state, command.reference, (entity) =>
            entity.kind === 'character' || entity.kind === 'object'
              ? { ...entity, asset: cloneDirectorAsset(command.asset) }
              : entity
          )
        : state;
    case 'selection/set':
      return command.selection === null || directorSelectionExists(state, command.selection)
        ? { ...state, selection: command.selection ? { ...command.selection } : null }
        : state;
    case 'camera/set-active':
      return command.cameraId === null || state.cameras.some((camera) => camera.id === command.cameraId)
        ? {
            ...state,
            activeCameraId: command.cameraId,
            viewMode: command.cameraId === null ? 'director' : state.viewMode,
          }
        : state;
    case 'camera/set-aspect-ratio': {
      const aspectRatio = normalizeDirectorAspectRatio(command.aspectRatio);
      return aspectRatio
        ? replaceEntity(state, { kind: 'camera', id: command.cameraId }, (entity) =>
            entity.kind === 'camera' ? { ...entity, aspectRatio } : entity
          )
        : state;
    }
    case 'camera/set-guides':
      return setCameraGuides(state, command.cameraId, command.guides);
    case 'camera/configure':
      return configureCamera(state, command.cameraId, command.patch);
    case 'light/configure':
      return configureLight(state, command.lightId, command.patch);
    case 'view/set-mode':
      return command.mode === 'camera' && state.activeCameraId === null
        ? state
        : { ...state, viewMode: command.mode };
    case 'panel/set': {
      const key = directorPanelKey(command.panel);
      return { ...state, panels: { ...state.panels, [key]: command.open } };
    }
    case 'panel/toggle': {
      const key = directorPanelKey(command.panel);
      return { ...state, panels: { ...state.panels, [key]: !state.panels[key] } };
    }
    case 'timeline/set-duration':
      return setTimelineDuration(state, command.durationSeconds);
    case 'timeline/set-time':
      return Number.isFinite(command.timeSeconds)
        ? {
            ...state,
            timeline: {
              ...state.timeline,
              currentTimeSeconds: clampDirectorTime(
                command.timeSeconds,
                state.timeline.durationSeconds
              ),
            },
          }
        : state;
    case 'timeline/set-playing':
      return {
        ...state,
        timeline: {
          ...state.timeline,
          playing: command.playing && state.timeline.durationSeconds > 0,
        },
      };
    case 'timeline/set-loop':
      return { ...state, timeline: { ...state.timeline, loop: command.loop } };
    case 'timeline/set-frame-rate':
      return Number.isSafeInteger(command.framesPerSecond) &&
        command.framesPerSecond >= 1 &&
        command.framesPerSecond <= 240
        ? {
            ...state,
            timeline: { ...state.timeline, framesPerSecond: command.framesPerSecond },
          }
        : state;
    case 'timeline/tick':
      return tickTimeline(state, command.deltaSeconds);
    case 'timeline/upsert-track':
      return upsertTrack(state, command.track);
    case 'timeline/delete-track': {
      const tracks = state.timeline.tracks.filter((track) => track.id !== command.trackId);
      return tracks.length === state.timeline.tracks.length
        ? state
        : { ...state, timeline: { ...state.timeline, tracks } };
    }
    case 'timeline/upsert-keyframe': {
      if (!isDirectorId(command.keyframe.id)) return state;
      const trackIndex = state.timeline.tracks.findIndex((track) => track.id === command.trackId);
      if (trackIndex < 0) return state;
      if (
        state.timeline.tracks.some(
          (track, index) =>
            index !== trackIndex &&
            track.keyframes.some((keyframe) => keyframe.id === command.keyframe.id)
        )
      ) {
        return state;
      }
      const updated = upsertDirectorKeyframe(
        state.timeline.tracks[trackIndex],
        command.keyframe,
        state.timeline.durationSeconds
      );
      if (!updated) return state;
      const tracks = [...state.timeline.tracks];
      tracks[trackIndex] = updated;
      return { ...state, timeline: { ...state.timeline, tracks } };
    }
    case 'timeline/delete-keyframe': {
      const trackIndex = state.timeline.tracks.findIndex((track) => track.id === command.trackId);
      if (trackIndex < 0) return state;
      const track = state.timeline.tracks[trackIndex];
      const keyframes = track.keyframes.filter((keyframe) => keyframe.id !== command.keyframeId);
      if (keyframes.length === track.keyframes.length) return state;
      const tracks = [...state.timeline.tracks];
      tracks[trackIndex] = { ...track, keyframes } as DirectorTimelineTrack;
      return { ...state, timeline: { ...state.timeline, tracks } };
    }
    case 'capture/configure':
      return configureCapture(state, command.patch);
    case 'capture/request':
      return requestCapture(state, command);
    case 'capture/start': {
      const operation = state.capture.operation;
      return operation.status === 'queued' && operation.request.requestId === command.requestId
        ? {
            ...state,
            capture: {
              ...state.capture,
              operation: { status: 'capturing', request: operation.request },
            },
          }
        : state;
    }
    case 'capture/complete':
      return completeCapture(state, command);
    case 'capture/fail': {
      const operation = state.capture.operation;
      return (operation.status === 'queued' || operation.status === 'capturing') &&
        operation.request.requestId === command.requestId &&
        isDirectorErrorCode(command.code)
        ? {
            ...state,
            capture: {
              ...state.capture,
              operation: { status: 'failed', request: operation.request, code: command.code },
            },
          }
        : state;
    }
    case 'capture/clear-operation':
      return state.capture.operation.status === 'idle'
        ? state
        : { ...state, capture: { ...state.capture, operation: { status: 'idle' } } };
    case 'capture/delete-record': {
      const records = state.capture.records.filter((record) => record.id !== command.captureId);
      return records.length === state.capture.records.length
        ? state
        : { ...state, capture: { ...state.capture, records } };
    }
  }
}
