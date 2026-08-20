/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  DIRECTOR_LIMITS,
  cloneDirectorAsset,
  cloneDirectorEntity,
  cloneDirectorTrack,
  cloneDirectorTransform,
  cloneDirectorVector3,
  directorTargetKey,
  directorTrackKey,
  isDirectorAssetId,
  isDirectorHexColor,
  isDirectorId,
  normalizeDirectorAspectRatio,
  normalizeDirectorName,
} from './model';
import type {
  DirectorAssetBinding,
  DirectorBooleanKeyframe,
  DirectorBooleanTrack,
  DirectorCamera,
  DirectorCameraAspectRatio,
  DirectorCameraGuides,
  DirectorCaptureRecord,
  DirectorCaptureSettings,
  DirectorCharacter,
  DirectorContinuousInterpolation,
  DirectorEntityRef,
  DirectorExportResult,
  DirectorImportError,
  DirectorImportErrorCode,
  DirectorImportResult,
  DirectorLight,
  DirectorNumberKeyframe,
  DirectorNumberTrack,
  DirectorObject3D,
  DirectorPanelState,
  DirectorProjectDocumentV1,
  DirectorProjectSnapshotV1,
  DirectorScene,
  DirectorSceneEnvironment,
  DirectorSelection,
  DirectorState,
  DirectorTimelineTrack,
  DirectorTrackTarget,
  DirectorTransform3D,
  DirectorVector3,
  DirectorVectorKeyframe,
  DirectorVectorTrack,
  DirectorViewMode,
} from './types';

type UnknownRecord = Record<string, unknown>;

class DirectorDecodeFailure extends Error {
  readonly detail: DirectorImportError;

  constructor(code: DirectorImportErrorCode, path: string, message: string) {
    super(message);
    this.name = 'DirectorDecodeFailure';
    this.detail = { code, path, message };
  }
}

function fail(code: DirectorImportErrorCode, path: string, message: string): never {
  throw new DirectorDecodeFailure(code, path, message);
}

function exactRecord(
  value: unknown,
  path: string,
  keys: readonly string[],
  code: DirectorImportErrorCode = 'invalid-value'
): UnknownRecord {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    fail(code, path, 'Expected an object');
  }
  const record = value as UnknownRecord;
  const allowed = new Set(keys);
  for (const key of Object.keys(record)) {
    if (!allowed.has(key)) fail(code, `${path}.${key}`, 'Unexpected field');
  }
  for (const key of keys) {
    if (!Object.prototype.hasOwnProperty.call(record, key)) {
      fail(code, `${path}.${key}`, 'Required field is missing');
    }
  }
  return record;
}

function arrayOf<T>(
  value: unknown,
  path: string,
  limit: number,
  parse: (item: unknown, itemPath: string) => T
): T[] {
  if (!Array.isArray(value)) fail('invalid-value', path, 'Expected an array');
  if (value.length > limit) fail('limit-exceeded', path, 'Array exceeds the v1 limit');
  return value.map((item, index) => parse(item, `${path}[${index}]`));
}

function enumValue<const Values extends readonly string[]>(
  value: unknown,
  values: Values,
  path: string
): Values[number] {
  if (typeof value !== 'string' || !values.includes(value)) {
    fail('invalid-value', path, `Expected one of: ${values.join(', ')}`);
  }
  return value as Values[number];
}

function booleanValue(value: unknown, path: string): boolean {
  if (typeof value !== 'boolean') fail('invalid-value', path, 'Expected a boolean');
  return value;
}

function numberValue(
  value: unknown,
  path: string,
  minimum: number,
  maximum: number
): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < minimum || value > maximum) {
    fail('invalid-value', path, 'Expected a bounded finite number');
  }
  return value;
}

function integerValue(
  value: unknown,
  path: string,
  minimum: number,
  maximum: number
): number {
  const result = numberValue(value, path, minimum, maximum);
  if (!Number.isSafeInteger(result)) fail('invalid-value', path, 'Expected a safe integer');
  return result;
}

function idValue(value: unknown, path: string): string {
  if (!isDirectorId(value)) fail('invalid-value', path, 'Expected a stable ID');
  return value;
}

function assetIdValue(value: unknown, path: string): string {
  if (!isDirectorAssetId(value)) {
    fail('invalid-value', path, 'Expected an asset ID; URLs are not part of the director schema');
  }
  return value;
}

function nameValue(value: unknown, path: string): string {
  const name = normalizeDirectorName(value);
  if (!name) fail('invalid-value', path, 'Expected a displayable name');
  return name;
}

function colorValue(value: unknown, path: string): string {
  if (!isDirectorHexColor(value)) fail('invalid-value', path, 'Expected a six-digit hex color');
  return value.toLowerCase();
}

function parseVector3(value: unknown, path: string, scale = false): DirectorVector3 {
  const record = exactRecord(value, path, ['x', 'y', 'z']);
  const minimum = scale ? 0.000001 : -DIRECTOR_LIMITS.maxCoordinate;
  const maximum = scale ? DIRECTOR_LIMITS.maxScale : DIRECTOR_LIMITS.maxCoordinate;
  return {
    x: numberValue(record.x, `${path}.x`, minimum, maximum),
    y: numberValue(record.y, `${path}.y`, minimum, maximum),
    z: numberValue(record.z, `${path}.z`, minimum, maximum),
  };
}

function parseTransform(value: unknown, path: string): DirectorTransform3D {
  const record = exactRecord(value, path, ['position', 'rotation', 'scale']);
  return {
    position: parseVector3(record.position, `${path}.position`),
    rotation: parseVector3(record.rotation, `${path}.rotation`),
    scale: parseVector3(record.scale, `${path}.scale`, true),
  };
}

function parseAsset(value: unknown, path: string): DirectorAssetBinding | null {
  if (value === null) return null;
  const record = exactRecord(value, path, ['assetId']);
  return { assetId: assetIdValue(record.assetId, `${path}.assetId`) };
}

function parseEnvironment(value: unknown, path: string): DirectorSceneEnvironment {
  const record = exactRecord(value, path, [
    'skyColor',
    'panorama',
    'panoramaYawDegrees',
    'panoramaRadius',
    'groundVisible',
    'gridVisible',
    'snapToGrid',
    'characterLabelsVisible',
  ]);
  return {
    skyColor: colorValue(record.skyColor, `${path}.skyColor`),
    panorama: parseAsset(record.panorama, `${path}.panorama`),
    panoramaYawDegrees: numberValue(
      record.panoramaYawDegrees,
      `${path}.panoramaYawDegrees`,
      -DIRECTOR_LIMITS.maxCoordinate,
      DIRECTOR_LIMITS.maxCoordinate
    ),
    panoramaRadius: numberValue(
      record.panoramaRadius,
      `${path}.panoramaRadius`,
      0.000001,
      DIRECTOR_LIMITS.maxCoordinate
    ),
    groundVisible: booleanValue(record.groundVisible, `${path}.groundVisible`),
    gridVisible: booleanValue(record.gridVisible, `${path}.gridVisible`),
    snapToGrid: booleanValue(record.snapToGrid, `${path}.snapToGrid`),
    characterLabelsVisible: booleanValue(
      record.characterLabelsVisible,
      `${path}.characterLabelsVisible`
    ),
  };
}

function parseScene(value: unknown, path: string): DirectorScene {
  const record = exactRecord(value, path, ['name', 'transform', 'environment']);
  return {
    name: nameValue(record.name, `${path}.name`),
    transform: parseTransform(record.transform, `${path}.transform`),
    environment: parseEnvironment(record.environment, `${path}.environment`),
  };
}

const ENTITY_BASE_KEYS = ['kind', 'id', 'name', 'transform', 'visible', 'locked'] as const;

function parseEntityBase(record: UnknownRecord, path: string) {
  return {
    id: idValue(record.id, `${path}.id`),
    name: nameValue(record.name, `${path}.name`),
    transform: parseTransform(record.transform, `${path}.transform`),
    visible: booleanValue(record.visible, `${path}.visible`),
    locked: booleanValue(record.locked, `${path}.locked`),
  };
}

function parseCharacter(value: unknown, path: string): DirectorCharacter {
  const record = exactRecord(value, path, [...ENTITY_BASE_KEYS, 'asset']);
  if (record.kind !== 'character') fail('invalid-value', `${path}.kind`, 'Expected character');
  return { kind: 'character', ...parseEntityBase(record, path), asset: parseAsset(record.asset, `${path}.asset`) };
}

function parseObject(value: unknown, path: string): DirectorObject3D {
  const record = exactRecord(value, path, [...ENTITY_BASE_KEYS, 'asset']);
  if (record.kind !== 'object') fail('invalid-value', `${path}.kind`, 'Expected object');
  return { kind: 'object', ...parseEntityBase(record, path), asset: parseAsset(record.asset, `${path}.asset`) };
}

function parseAspectRatio(value: unknown, path: string): DirectorCameraAspectRatio {
  const record = exactRecord(value, path, ['width', 'height']);
  const aspectRatio = normalizeDirectorAspectRatio({
    width: integerValue(record.width, `${path}.width`, 1, 10_000),
    height: integerValue(record.height, `${path}.height`, 1, 10_000),
  });
  if (!aspectRatio) fail('invalid-value', path, 'Expected a positive aspect ratio');
  return aspectRatio;
}

function parseGuides(value: unknown, path: string): DirectorCameraGuides {
  const record = exactRecord(value, path, ['frame', 'center', 'thirds', 'safeArea']);
  return {
    frame: booleanValue(record.frame, `${path}.frame`),
    center: booleanValue(record.center, `${path}.center`),
    thirds: booleanValue(record.thirds, `${path}.thirds`),
    safeArea: booleanValue(record.safeArea, `${path}.safeArea`),
  };
}

function parseCamera(value: unknown, path: string): DirectorCamera {
  const record = exactRecord(value, path, [
    ...ENTITY_BASE_KEYS,
    'projection',
    'focalLengthMm',
    'orthographicSize',
    'nearClip',
    'farClip',
    'aspectRatio',
    'guides',
  ]);
  if (record.kind !== 'camera') fail('invalid-value', `${path}.kind`, 'Expected camera');
  const nearClip = numberValue(
    record.nearClip,
    `${path}.nearClip`,
    0.000001,
    DIRECTOR_LIMITS.maxCoordinate
  );
  return {
    kind: 'camera',
    ...parseEntityBase(record, path),
    projection: enumValue(record.projection, ['perspective', 'orthographic'] as const, `${path}.projection`),
    focalLengthMm: numberValue(record.focalLengthMm, `${path}.focalLengthMm`, 1, 1_000),
    orthographicSize: numberValue(
      record.orthographicSize,
      `${path}.orthographicSize`,
      0.000001,
      DIRECTOR_LIMITS.maxScale
    ),
    nearClip,
    farClip: numberValue(
      record.farClip,
      `${path}.farClip`,
      nearClip + 0.000001,
      DIRECTOR_LIMITS.maxCoordinate
    ),
    aspectRatio: parseAspectRatio(record.aspectRatio, `${path}.aspectRatio`),
    guides: parseGuides(record.guides, `${path}.guides`),
  };
}

function parseLight(value: unknown, path: string): DirectorLight {
  const record = exactRecord(value, path, [
    ...ENTITY_BASE_KEYS,
    'lightType',
    'color',
    'intensity',
    'range',
    'coneAngleDegrees',
  ]);
  if (record.kind !== 'light') fail('invalid-value', `${path}.kind`, 'Expected light');
  return {
    kind: 'light',
    ...parseEntityBase(record, path),
    lightType: enumValue(
      record.lightType,
      ['ambient', 'directional', 'point', 'spot'] as const,
      `${path}.lightType`
    ),
    color: colorValue(record.color, `${path}.color`),
    intensity: numberValue(
      record.intensity,
      `${path}.intensity`,
      0,
      DIRECTOR_LIMITS.maxIntensity
    ),
    range: numberValue(record.range, `${path}.range`, 0, DIRECTOR_LIMITS.maxCoordinate),
    coneAngleDegrees: numberValue(record.coneAngleDegrees, `${path}.coneAngleDegrees`, 0, 180),
  };
}

function parseEntityRef(value: unknown, path: string): DirectorEntityRef {
  const record = exactRecord(value, path, ['kind', 'id']);
  const kind = enumValue(record.kind, ['camera', 'character', 'object', 'light'] as const, `${path}.kind`);
  return { kind, id: idValue(record.id, `${path}.id`) } as DirectorEntityRef;
}

function parseTrackTarget(value: unknown, path: string): DirectorTrackTarget {
  if (value && typeof value === 'object' && !Array.isArray(value) && (value as UnknownRecord).kind === 'scene') {
    exactRecord(value, path, ['kind']);
    return { kind: 'scene' };
  }
  return parseEntityRef(value, path);
}

function parseSelection(value: unknown, path: string): DirectorSelection | null {
  if (value === null) return null;
  if (value && typeof value === 'object' && !Array.isArray(value) && (value as UnknownRecord).kind === 'scene') {
    exactRecord(value, path, ['kind']);
    return { kind: 'scene' };
  }
  return parseEntityRef(value, path);
}

function parseInterpolation(value: unknown, path: string): DirectorContinuousInterpolation {
  return enumValue(value, ['linear', 'step', 'ease-in-out'] as const, path);
}

function parseNumberKeyframe(
  value: unknown,
  path: string,
  property: DirectorNumberTrack['property'],
  durationSeconds: number
): DirectorNumberKeyframe {
  const record = exactRecord(value, path, ['id', 'valueType', 'timeSeconds', 'value', 'interpolation']);
  if (record.valueType !== 'number') fail('invalid-value', `${path}.valueType`, 'Expected number');
  const maximum = property === 'focalLengthMm' ? 1_000 : DIRECTOR_LIMITS.maxIntensity;
  const minimum = property === 'focalLengthMm' ? 1 : 0;
  return {
    id: idValue(record.id, `${path}.id`),
    valueType: 'number',
    timeSeconds: numberValue(record.timeSeconds, `${path}.timeSeconds`, 0, durationSeconds),
    value: numberValue(record.value, `${path}.value`, minimum, maximum),
    interpolation: parseInterpolation(record.interpolation, `${path}.interpolation`),
  };
}

function parseVectorKeyframe(
  value: unknown,
  path: string,
  property: DirectorVectorTrack['property'],
  durationSeconds: number
): DirectorVectorKeyframe {
  const record = exactRecord(value, path, ['id', 'valueType', 'timeSeconds', 'value', 'interpolation']);
  if (record.valueType !== 'vector3') fail('invalid-value', `${path}.valueType`, 'Expected vector3');
  return {
    id: idValue(record.id, `${path}.id`),
    valueType: 'vector3',
    timeSeconds: numberValue(record.timeSeconds, `${path}.timeSeconds`, 0, durationSeconds),
    value: parseVector3(record.value, `${path}.value`, property === 'scale'),
    interpolation: parseInterpolation(record.interpolation, `${path}.interpolation`),
  };
}

function parseBooleanKeyframe(
  value: unknown,
  path: string,
  durationSeconds: number
): DirectorBooleanKeyframe {
  const record = exactRecord(value, path, ['id', 'valueType', 'timeSeconds', 'value', 'interpolation']);
  if (record.valueType !== 'boolean') fail('invalid-value', `${path}.valueType`, 'Expected boolean');
  if (record.interpolation !== 'step') fail('invalid-value', `${path}.interpolation`, 'Boolean tracks use step interpolation');
  return {
    id: idValue(record.id, `${path}.id`),
    valueType: 'boolean',
    timeSeconds: numberValue(record.timeSeconds, `${path}.timeSeconds`, 0, durationSeconds),
    value: booleanValue(record.value, `${path}.value`),
    interpolation: 'step',
  };
}

function assertStrictKeyframeOrder(
  keyframes: readonly { id: string; timeSeconds: number }[],
  path: string
): void {
  const ids = new Set<string>();
  keyframes.forEach((keyframe, index) => {
    if (ids.has(keyframe.id)) fail('duplicate-id', `${path}[${index}].id`, 'Duplicate keyframe ID');
    ids.add(keyframe.id);
    if (index > 0 && keyframes[index - 1].timeSeconds >= keyframe.timeSeconds) {
      fail('invalid-value', `${path}[${index}].timeSeconds`, 'Keyframe times must be strictly increasing');
    }
  });
}

function parseTrack(
  value: unknown,
  path: string,
  durationSeconds: number
): DirectorTimelineTrack {
  const record = exactRecord(value, path, ['id', 'target', 'valueType', 'property', 'keyframes']);
  const id = idValue(record.id, `${path}.id`);
  const target = parseTrackTarget(record.target, `${path}.target`);
  const valueType = enumValue(record.valueType, ['number', 'vector3', 'boolean'] as const, `${path}.valueType`);
  if (valueType === 'vector3') {
    const property = enumValue(record.property, ['position', 'rotation', 'scale'] as const, `${path}.property`);
    const keyframes = arrayOf(
      record.keyframes,
      `${path}.keyframes`,
      DIRECTOR_LIMITS.maxKeyframesPerTrack,
      (item, itemPath) => parseVectorKeyframe(item, itemPath, property, durationSeconds)
    );
    assertStrictKeyframeOrder(keyframes, `${path}.keyframes`);
    return { id, target, valueType, property, keyframes };
  }
  if (target.kind === 'scene') fail('invalid-value', `${path}.target`, 'Scene only supports transform tracks');
  if (valueType === 'number') {
    const property = enumValue(record.property, ['focalLengthMm', 'intensity'] as const, `${path}.property`);
    if (
      (property === 'focalLengthMm' && target.kind !== 'camera') ||
      (property === 'intensity' && target.kind !== 'light')
    ) {
      fail('invalid-value', `${path}.property`, 'Track property is incompatible with its target');
    }
    const keyframes = arrayOf(
      record.keyframes,
      `${path}.keyframes`,
      DIRECTOR_LIMITS.maxKeyframesPerTrack,
      (item, itemPath) => parseNumberKeyframe(item, itemPath, property, durationSeconds)
    );
    assertStrictKeyframeOrder(keyframes, `${path}.keyframes`);
    return { id, target, valueType, property, keyframes };
  }
  if (record.property !== 'visible') fail('invalid-value', `${path}.property`, 'Boolean tracks animate visibility');
  const keyframes = arrayOf(
    record.keyframes,
    `${path}.keyframes`,
    DIRECTOR_LIMITS.maxKeyframesPerTrack,
    (item, itemPath) => parseBooleanKeyframe(item, itemPath, durationSeconds)
  );
  assertStrictKeyframeOrder(keyframes, `${path}.keyframes`);
  return { id, target, valueType, property: 'visible', keyframes } as DirectorBooleanTrack;
}

function parsePanels(value: unknown, path: string): DirectorPanelState {
  const record = exactRecord(value, path, ['leftSidebarOpen', 'rightSidebarOpen', 'timelineOpen']);
  return {
    leftSidebarOpen: booleanValue(record.leftSidebarOpen, `${path}.leftSidebarOpen`),
    rightSidebarOpen: booleanValue(record.rightSidebarOpen, `${path}.rightSidebarOpen`),
    timelineOpen: booleanValue(record.timelineOpen, `${path}.timelineOpen`),
  };
}

function parseCaptureSettings(value: unknown, path: string): DirectorCaptureSettings {
  const record = exactRecord(value, path, ['width', 'height', 'imageFormat', 'videoFramesPerSecond']);
  return {
    width: integerValue(record.width, `${path}.width`, 1, DIRECTOR_LIMITS.maxCaptureDimension),
    height: integerValue(record.height, `${path}.height`, 1, DIRECTOR_LIMITS.maxCaptureDimension),
    imageFormat: enumValue(record.imageFormat, ['png', 'jpeg'] as const, `${path}.imageFormat`),
    videoFramesPerSecond: integerValue(record.videoFramesPerSecond, `${path}.videoFramesPerSecond`, 1, 240),
  };
}

function parseCaptureRecord(value: unknown, path: string): DirectorCaptureRecord {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    fail('invalid-value', path, 'Expected a capture record');
  }
  const kind = (value as UnknownRecord).kind;
  const commonKeys = ['id', 'kind', 'cameraId', 'assetId', 'capturedAt', 'width', 'height', 'format'];
  const record = exactRecord(
    value,
    path,
    kind === 'video' ? [...commonKeys, 'framesPerSecond', 'durationSeconds'] : commonKeys
  );
  const common = {
    id: idValue(record.id, `${path}.id`),
    cameraId: idValue(record.cameraId, `${path}.cameraId`),
    assetId: assetIdValue(record.assetId, `${path}.assetId`),
    capturedAt: integerValue(record.capturedAt, `${path}.capturedAt`, 0, Number.MAX_SAFE_INTEGER),
    width: integerValue(record.width, `${path}.width`, 1, DIRECTOR_LIMITS.maxCaptureDimension),
    height: integerValue(record.height, `${path}.height`, 1, DIRECTOR_LIMITS.maxCaptureDimension),
  };
  if (kind === 'image') {
    return {
      ...common,
      kind,
      format: enumValue(record.format, ['png', 'jpeg'] as const, `${path}.format`),
    };
  }
  if (kind !== 'video') fail('invalid-value', `${path}.kind`, 'Expected image or video');
  if (record.format !== 'mp4') fail('invalid-value', `${path}.format`, 'Video capture format must be mp4');
  return {
    ...common,
    kind,
    format: 'mp4',
    framesPerSecond: integerValue(record.framesPerSecond, `${path}.framesPerSecond`, 1, 240),
    durationSeconds: numberValue(
      record.durationSeconds,
      `${path}.durationSeconds`,
      0.000001,
      DIRECTOR_LIMITS.maxDurationSeconds
    ),
  };
}

function ensureUniqueIds(values: readonly { id: string }[], path: string, shared?: Set<string>): Set<string> {
  const ids = shared ?? new Set<string>();
  values.forEach((value, index) => {
    if (ids.has(value.id)) fail('duplicate-id', `${path}[${index}].id`, 'Duplicate ID');
    ids.add(value.id);
  });
  return ids;
}

function parseProject(value: unknown, path: string): DirectorState {
  const record = exactRecord(value, path, [
    'projectId',
    'name',
    'scene',
    'cameras',
    'characters',
    'objects',
    'lights',
    'activeCameraId',
    'selection',
    'viewMode',
    'panels',
    'timeline',
    'capture',
  ]);
  const cameras = arrayOf(record.cameras, `${path}.cameras`, DIRECTOR_LIMITS.maxEntitiesPerKind, parseCamera);
  const characters = arrayOf(record.characters, `${path}.characters`, DIRECTOR_LIMITS.maxEntitiesPerKind, parseCharacter);
  const objects = arrayOf(record.objects, `${path}.objects`, DIRECTOR_LIMITS.maxEntitiesPerKind, parseObject);
  const lights = arrayOf(record.lights, `${path}.lights`, DIRECTOR_LIMITS.maxEntitiesPerKind, parseLight);
  const entityIds = ensureUniqueIds(cameras, `${path}.cameras`);
  ensureUniqueIds(characters, `${path}.characters`, entityIds);
  ensureUniqueIds(objects, `${path}.objects`, entityIds);
  ensureUniqueIds(lights, `${path}.lights`, entityIds);

  const timelineRecord = exactRecord(record.timeline, `${path}.timeline`, [
    'durationSeconds',
    'currentTimeSeconds',
    'framesPerSecond',
    'loop',
    'tracks',
  ]);
  const durationSeconds = numberValue(
    timelineRecord.durationSeconds,
    `${path}.timeline.durationSeconds`,
    0,
    DIRECTOR_LIMITS.maxDurationSeconds
  );
  const tracks = arrayOf(
    timelineRecord.tracks,
    `${path}.timeline.tracks`,
    DIRECTOR_LIMITS.maxTracks,
    (item, itemPath) => parseTrack(item, itemPath, durationSeconds)
  );
  ensureUniqueIds(tracks, `${path}.timeline.tracks`);
  const trackKeys = new Set<string>();
  const keyframeIds = new Set<string>();
  tracks.forEach((track, trackIndex) => {
    const key = directorTrackKey(track);
    if (trackKeys.has(key)) {
      fail('duplicate-id', `${path}.timeline.tracks[${trackIndex}]`, 'Duplicate target property track');
    }
    trackKeys.add(key);
    track.keyframes.forEach((keyframe, keyframeIndex) => {
      if (keyframeIds.has(keyframe.id)) {
        fail(
          'duplicate-id',
          `${path}.timeline.tracks[${trackIndex}].keyframes[${keyframeIndex}].id`,
          'Duplicate keyframe ID'
        );
      }
      keyframeIds.add(keyframe.id);
    });
  });

  const captureRecord = exactRecord(record.capture, `${path}.capture`, ['settings', 'records']);
  const captures = arrayOf(
    captureRecord.records,
    `${path}.capture.records`,
    DIRECTOR_LIMITS.maxCaptures,
    parseCaptureRecord
  );
  ensureUniqueIds(captures, `${path}.capture.records`);

  const state: DirectorState = {
    projectId: idValue(record.projectId, `${path}.projectId`),
    name: nameValue(record.name, `${path}.name`),
    scene: parseScene(record.scene, `${path}.scene`),
    cameras,
    characters,
    objects,
    lights,
    activeCameraId:
      record.activeCameraId === null
        ? null
        : idValue(record.activeCameraId, `${path}.activeCameraId`),
    selection: parseSelection(record.selection, `${path}.selection`),
    viewMode: enumValue(record.viewMode, ['director', 'camera'] as const, `${path}.viewMode`) as DirectorViewMode,
    panels: parsePanels(record.panels, `${path}.panels`),
    timeline: {
      durationSeconds,
      currentTimeSeconds: numberValue(
        timelineRecord.currentTimeSeconds,
        `${path}.timeline.currentTimeSeconds`,
        0,
        durationSeconds
      ),
      framesPerSecond: integerValue(
        timelineRecord.framesPerSecond,
        `${path}.timeline.framesPerSecond`,
        1,
        240
      ),
      playing: false,
      loop: booleanValue(timelineRecord.loop, `${path}.timeline.loop`),
      tracks,
    },
    capture: {
      settings: parseCaptureSettings(captureRecord.settings, `${path}.capture.settings`),
      operation: { status: 'idle' },
      records: captures,
    },
  };

  if (state.activeCameraId !== null && !cameras.some((camera) => camera.id === state.activeCameraId)) {
    fail('broken-reference', `${path}.activeCameraId`, 'Active camera does not exist');
  }
  if (state.viewMode === 'camera' && state.activeCameraId === null) {
    fail('broken-reference', `${path}.viewMode`, 'Camera view requires an active camera');
  }
  if (state.selection && state.selection.kind !== 'scene') {
    const key = `${state.selection.kind}:${state.selection.id}`;
    if (!entityIds.has(state.selection.id) || ![
      ...cameras,
      ...characters,
      ...objects,
      ...lights,
    ].some((entity) => `${entity.kind}:${entity.id}` === key)) {
      fail('broken-reference', `${path}.selection`, 'Selection target does not exist');
    }
  }
  const existingTargets = new Set([
    'scene',
    ...cameras.map((entity) => `camera:${entity.id}`),
    ...characters.map((entity) => `character:${entity.id}`),
    ...objects.map((entity) => `object:${entity.id}`),
    ...lights.map((entity) => `light:${entity.id}`),
  ]);
  tracks.forEach((track, index) => {
    if (!existingTargets.has(directorTargetKey(track.target))) {
      fail('broken-reference', `${path}.timeline.tracks[${index}].target`, 'Track target does not exist');
    }
  });
  captures.forEach((capture, index) => {
    if (!cameras.some((camera) => camera.id === capture.cameraId)) {
      fail('broken-reference', `${path}.capture.records[${index}].cameraId`, 'Capture camera does not exist');
    }
  });
  return state;
}

function documentFromState(state: DirectorState): DirectorProjectDocumentV1 {
  const snapshot: DirectorProjectSnapshotV1 = {
    projectId: state.projectId,
    name: state.name,
    scene: {
      ...state.scene,
      transform: cloneDirectorTransform(state.scene.transform),
      environment: {
        ...state.scene.environment,
        panorama: cloneDirectorAsset(state.scene.environment.panorama),
      },
    },
    cameras: state.cameras.map(cloneDirectorEntity),
    characters: state.characters.map(cloneDirectorEntity),
    objects: state.objects.map(cloneDirectorEntity),
    lights: state.lights.map(cloneDirectorEntity),
    activeCameraId: state.activeCameraId,
    selection: state.selection ? { ...state.selection } : null,
    viewMode: state.viewMode,
    panels: { ...state.panels },
    timeline: {
      durationSeconds: state.timeline.durationSeconds,
      currentTimeSeconds: state.timeline.currentTimeSeconds,
      framesPerSecond: state.timeline.framesPerSecond,
      loop: state.timeline.loop,
      tracks: state.timeline.tracks.map(cloneDirectorTrack),
    },
    capture: {
      settings: { ...state.capture.settings },
      records: state.capture.records.map((record) => ({ ...record })),
    },
  };
  return { kind: 'nomifun.director.project', version: 1, project: snapshot };
}

function decodeDocument(value: unknown): DirectorState {
  const envelope = exactRecord(
    value,
    '$',
    ['kind', 'version', 'project'],
    'invalid-envelope'
  );
  if (envelope.kind !== 'nomifun.director.project') {
    fail('invalid-envelope', '$.kind', 'Not a NomiFun director project');
  }
  if (envelope.version !== 1) {
    fail('unsupported-version', '$.version', 'Only director project version 1 is supported');
  }
  return parseProject(envelope.project, '$.project');
}

export function importDirectorProjectV1(json: string): DirectorImportResult {
  if (
    typeof json !== 'string' ||
    json.length > DIRECTOR_LIMITS.maxJsonBytes ||
    new TextEncoder().encode(json).byteLength > DIRECTOR_LIMITS.maxJsonBytes
  ) {
    return {
      ok: false,
      error: {
        code: 'limit-exceeded',
        path: '$',
        message: 'Director project JSON exceeds the v1 size limit',
      },
    };
  }
  let value: unknown;
  try {
    value = JSON.parse(json) as unknown;
  } catch {
    return {
      ok: false,
      error: { code: 'invalid-json', path: '$', message: 'Director project is not valid JSON' },
    };
  }
  try {
    return { ok: true, state: decodeDocument(value) };
  } catch (reason) {
    if (reason instanceof DirectorDecodeFailure) return { ok: false, error: reason.detail };
    return {
      ok: false,
      error: { code: 'invalid-value', path: '$', message: 'Director project validation failed' },
    };
  }
}

export function exportDirectorProjectV1(state: DirectorState): DirectorExportResult {
  try {
    const candidate = documentFromState(state);
    const validation = importDirectorProjectV1(JSON.stringify(candidate));
    if (!validation.ok) return validation;
    const document = documentFromState(validation.state);
    return { ok: true, document, json: JSON.stringify(document, null, 2) };
  } catch {
    return {
      ok: false,
      error: { code: 'invalid-value', path: '$', message: 'Director state cannot be exported' },
    };
  }
}
