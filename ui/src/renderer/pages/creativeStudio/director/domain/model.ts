/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  DirectorAssetBinding,
  DirectorCamera,
  DirectorCameraAspectRatio,
  DirectorCameraGuides,
  DirectorCaptureOperation,
  DirectorCaptureRecord,
  DirectorCharacter,
  DirectorEntity,
  DirectorEntityKind,
  DirectorEntityRef,
  DirectorLight,
  DirectorObject3D,
  DirectorScene,
  DirectorSelection,
  DirectorState,
  DirectorTimelineTrack,
  DirectorTrackTarget,
  DirectorTransform3D,
  DirectorVector3,
} from './types';

const SAFE_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]*$/u;
const HEX_COLOR = /^#[0-9a-fA-F]{6}$/u;
const UNSAFE_CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u;

export const DIRECTOR_LIMITS = {
  idLength: 128,
  assetIdLength: 255,
  nameLength: 120,
  errorCodeLength: 80,
  maxEntitiesPerKind: 1_000,
  maxTracks: 2_000,
  maxKeyframesPerTrack: 20_000,
  maxCaptures: 5_000,
  maxDurationSeconds: 86_400,
  maxCoordinate: 1_000_000,
  maxScale: 100_000,
  maxIntensity: 1_000_000,
  maxCaptureDimension: 16_384,
  maxJsonBytes: 16 * 1024 * 1024,
} as const;

export const DIRECTOR_DEFAULT_TRANSFORM: DirectorTransform3D = {
  position: { x: 0, y: 0, z: 0 },
  rotation: { x: 0, y: 0, z: 0 },
  scale: { x: 1, y: 1, z: 1 },
};

export const DIRECTOR_DEFAULT_CAMERA_GUIDES: DirectorCameraGuides = {
  frame: true,
  center: false,
  thirds: false,
  safeArea: false,
};

export function isDirectorId(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length > 0 &&
    value.length <= DIRECTOR_LIMITS.idLength &&
    value === value.trim() &&
    SAFE_ID.test(value)
  );
}

export function isDirectorAssetId(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length > 0 &&
    value.length <= DIRECTOR_LIMITS.assetIdLength &&
    value === value.trim() &&
    SAFE_ID.test(value)
  );
}

export function normalizeDirectorName(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const name = value.trim();
  if (!name || name.length > DIRECTOR_LIMITS.nameLength || UNSAFE_CONTROL.test(name)) return null;
  return name;
}

export function isDirectorErrorCode(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length > 0 &&
    value.length <= DIRECTOR_LIMITS.errorCodeLength &&
    value === value.trim() &&
    SAFE_ID.test(value)
  );
}

export function isDirectorHexColor(value: unknown): value is string {
  return typeof value === 'string' && HEX_COLOR.test(value);
}

export function isFiniteDirectorNumber(
  value: unknown,
  minimum = -Number.MAX_VALUE,
  maximum = Number.MAX_VALUE
): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= minimum && value <= maximum;
}

export function isDirectorVector3(value: unknown, scale = false): value is DirectorVector3 {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const vector = value as Partial<DirectorVector3>;
  const minimum = scale ? 0.000001 : -DIRECTOR_LIMITS.maxCoordinate;
  const maximum = scale ? DIRECTOR_LIMITS.maxScale : DIRECTOR_LIMITS.maxCoordinate;
  return (
    isFiniteDirectorNumber(vector.x, minimum, maximum) &&
    isFiniteDirectorNumber(vector.y, minimum, maximum) &&
    isFiniteDirectorNumber(vector.z, minimum, maximum)
  );
}

export function isDirectorTransform(value: unknown): value is DirectorTransform3D {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const transform = value as Partial<DirectorTransform3D>;
  return (
    isDirectorVector3(transform.position) &&
    isDirectorVector3(transform.rotation) &&
    isDirectorVector3(transform.scale, true)
  );
}

export function cloneDirectorVector3(value: DirectorVector3): DirectorVector3 {
  return { x: value.x, y: value.y, z: value.z };
}

export function cloneDirectorTransform(value: DirectorTransform3D): DirectorTransform3D {
  return {
    position: cloneDirectorVector3(value.position),
    rotation: cloneDirectorVector3(value.rotation),
    scale: cloneDirectorVector3(value.scale),
  };
}

export function cloneDirectorAsset(value: DirectorAssetBinding | null): DirectorAssetBinding | null {
  return value ? { assetId: value.assetId } : null;
}

export function normalizeDirectorAspectRatio(
  value: DirectorCameraAspectRatio
): DirectorCameraAspectRatio | null {
  if (
    !Number.isSafeInteger(value.width) ||
    !Number.isSafeInteger(value.height) ||
    value.width <= 0 ||
    value.height <= 0 ||
    value.width > 10_000 ||
    value.height > 10_000
  ) {
    return null;
  }
  const divisor = greatestCommonDivisor(value.width, value.height);
  return { width: value.width / divisor, height: value.height / divisor };
}

function greatestCommonDivisor(left: number, right: number): number {
  let a = Math.abs(left);
  let b = Math.abs(right);
  while (b > 0) {
    const next = a % b;
    a = b;
    b = next;
  }
  return a || 1;
}

function assertId(value: string, label: string): void {
  if (!isDirectorId(value)) throw new TypeError(`${label} must be a valid stable ID`);
}

function assertAssetId(value: string | null | undefined): DirectorAssetBinding | null {
  if (value === null || value === undefined) return null;
  if (!isDirectorAssetId(value)) throw new TypeError('assetId must be a stable ID, not a URL');
  return { assetId: value };
}

function assertName(value: string, label: string): string {
  const normalized = normalizeDirectorName(value);
  if (!normalized) throw new TypeError(`${label} must be a displayable name`);
  return normalized;
}

function entityBase(input: {
  id: string;
  name: string;
  transform?: DirectorTransform3D;
  visible?: boolean;
  locked?: boolean;
}): Omit<DirectorEntity, 'kind'> {
  assertId(input.id, 'entity id');
  const transform = input.transform ?? DIRECTOR_DEFAULT_TRANSFORM;
  if (!isDirectorTransform(transform)) throw new TypeError('entity transform is invalid');
  return {
    id: input.id,
    name: assertName(input.name, 'entity name'),
    transform: cloneDirectorTransform(transform),
    visible: input.visible ?? true,
    locked: input.locked ?? false,
  } as Omit<DirectorEntity, 'kind'>;
}

export interface CreateDirectorAssetEntityInput {
  id: string;
  name: string;
  assetId?: string | null;
  transform?: DirectorTransform3D;
  visible?: boolean;
  locked?: boolean;
}

export function createDirectorCharacter(
  input: CreateDirectorAssetEntityInput
): DirectorCharacter {
  return {
    ...entityBase(input),
    kind: 'character',
    asset: assertAssetId(input.assetId),
  } as DirectorCharacter;
}

export function createDirectorObject(input: CreateDirectorAssetEntityInput): DirectorObject3D {
  return {
    ...entityBase(input),
    kind: 'object',
    asset: assertAssetId(input.assetId),
  } as DirectorObject3D;
}

export interface CreateDirectorCameraInput {
  id: string;
  name: string;
  transform?: DirectorTransform3D;
  visible?: boolean;
  locked?: boolean;
  projection?: DirectorCamera['projection'];
  focalLengthMm?: number;
  orthographicSize?: number;
  nearClip?: number;
  farClip?: number;
  aspectRatio?: DirectorCameraAspectRatio;
  guides?: Partial<DirectorCameraGuides>;
}

export function createDirectorCamera(input: CreateDirectorCameraInput): DirectorCamera {
  const focalLengthMm = input.focalLengthMm ?? 50;
  const orthographicSize = input.orthographicSize ?? 10;
  const nearClip = input.nearClip ?? 0.1;
  const farClip = input.farClip ?? 10_000;
  const aspectRatio = normalizeDirectorAspectRatio(input.aspectRatio ?? { width: 16, height: 9 });
  if (!isFiniteDirectorNumber(focalLengthMm, 1, 1_000)) {
    throw new TypeError('camera focal length is invalid');
  }
  if (!isFiniteDirectorNumber(orthographicSize, 0.000001, DIRECTOR_LIMITS.maxScale)) {
    throw new TypeError('camera orthographic size is invalid');
  }
  if (
    !isFiniteDirectorNumber(nearClip, 0.000001, DIRECTOR_LIMITS.maxCoordinate) ||
    !isFiniteDirectorNumber(farClip, nearClip + 0.000001, DIRECTOR_LIMITS.maxCoordinate)
  ) {
    throw new TypeError('camera clipping range is invalid');
  }
  if (!aspectRatio) throw new TypeError('camera aspect ratio is invalid');
  return {
    ...entityBase(input),
    kind: 'camera',
    projection: input.projection ?? 'perspective',
    focalLengthMm,
    orthographicSize,
    nearClip,
    farClip,
    aspectRatio,
    guides: { ...DIRECTOR_DEFAULT_CAMERA_GUIDES, ...input.guides },
  } as DirectorCamera;
}

export interface CreateDirectorLightInput {
  id: string;
  name: string;
  transform?: DirectorTransform3D;
  visible?: boolean;
  locked?: boolean;
  lightType?: DirectorLight['lightType'];
  color?: string;
  intensity?: number;
  range?: number;
  coneAngleDegrees?: number;
}

export function createDirectorLight(input: CreateDirectorLightInput): DirectorLight {
  const color = input.color ?? '#ffffff';
  const intensity = input.intensity ?? 1;
  const range = input.range ?? 10;
  const coneAngleDegrees = input.coneAngleDegrees ?? 45;
  if (!isDirectorHexColor(color)) throw new TypeError('light color is invalid');
  if (!isFiniteDirectorNumber(intensity, 0, DIRECTOR_LIMITS.maxIntensity)) {
    throw new TypeError('light intensity is invalid');
  }
  if (!isFiniteDirectorNumber(range, 0, DIRECTOR_LIMITS.maxCoordinate)) {
    throw new TypeError('light range is invalid');
  }
  if (!isFiniteDirectorNumber(coneAngleDegrees, 0, 180)) {
    throw new TypeError('light cone angle is invalid');
  }
  return {
    ...entityBase(input),
    kind: 'light',
    lightType: input.lightType ?? 'directional',
    color: color.toLowerCase(),
    intensity,
    range,
    coneAngleDegrees,
  } as DirectorLight;
}

export interface CreateDirectorStateInput {
  projectId: string;
  name: string;
  sceneName?: string;
  durationSeconds?: number;
}

export function createDirectorState(input: CreateDirectorStateInput): DirectorState {
  assertId(input.projectId, 'project id');
  const durationSeconds = input.durationSeconds ?? 10;
  if (!isFiniteDirectorNumber(durationSeconds, 0, DIRECTOR_LIMITS.maxDurationSeconds)) {
    throw new TypeError('timeline duration is invalid');
  }
  const scene: DirectorScene = {
    name: assertName(input.sceneName ?? 'Scene', 'scene name'),
    transform: cloneDirectorTransform(DIRECTOR_DEFAULT_TRANSFORM),
    environment: {
      skyColor: '#000000',
      panorama: null,
      panoramaYawDegrees: 0,
      panoramaRadius: 60,
      groundVisible: true,
      gridVisible: true,
      snapToGrid: false,
      characterLabelsVisible: true,
    },
  };
  return {
    projectId: input.projectId,
    name: assertName(input.name, 'project name'),
    scene,
    cameras: [],
    characters: [],
    objects: [],
    lights: [],
    activeCameraId: null,
    selection: { kind: 'scene' },
    viewMode: 'director',
    panels: {
      leftSidebarOpen: true,
      rightSidebarOpen: true,
      timelineOpen: false,
    },
    timeline: {
      durationSeconds,
      currentTimeSeconds: 0,
      framesPerSecond: 24,
      playing: false,
      loop: false,
      tracks: [],
    },
    capture: {
      settings: {
        width: 1920,
        height: 1080,
        imageFormat: 'png',
        videoFramesPerSecond: 24,
      },
      operation: { status: 'idle' },
      records: [],
    },
  };
}

export function directorEntityRef(entity: DirectorEntity): DirectorEntityRef {
  return { kind: entity.kind, id: entity.id } as DirectorEntityRef;
}

export function directorTargetKey(target: DirectorTrackTarget): string {
  return target.kind === 'scene' ? 'scene' : `${target.kind}:${target.id}`;
}

export function directorTrackKey(track: DirectorTimelineTrack): string {
  return `${directorTargetKey(track.target)}:${track.property}`;
}

export function findDirectorEntity(
  state: DirectorState,
  reference: DirectorEntityRef
): DirectorEntity | undefined {
  switch (reference.kind) {
    case 'camera':
      return state.cameras.find((item) => item.id === reference.id);
    case 'character':
      return state.characters.find((item) => item.id === reference.id);
    case 'object':
      return state.objects.find((item) => item.id === reference.id);
    case 'light':
      return state.lights.find((item) => item.id === reference.id);
  }
}

export function hasDirectorEntityId(state: DirectorState, id: string): boolean {
  return (
    state.cameras.some((item) => item.id === id) ||
    state.characters.some((item) => item.id === id) ||
    state.objects.some((item) => item.id === id) ||
    state.lights.some((item) => item.id === id)
  );
}

export function directorSelectionExists(
  state: DirectorState,
  selection: DirectorSelection
): boolean {
  return selection.kind === 'scene' || Boolean(findDirectorEntity(state, selection));
}

export function directorTargetExists(state: DirectorState, target: DirectorTrackTarget): boolean {
  return target.kind === 'scene' || Boolean(findDirectorEntity(state, target));
}

export function isDirectorTrackCompatible(track: DirectorTimelineTrack): boolean {
  if (track.valueType === 'vector3') return true;
  if (track.valueType === 'boolean') return true;
  if (track.property === 'focalLengthMm') return track.target.kind === 'camera';
  return track.property === 'intensity' && track.target.kind === 'light';
}

export function cloneDirectorEntity<Entity extends DirectorEntity>(entity: Entity): Entity {
  const base = {
    ...entity,
    transform: cloneDirectorTransform(entity.transform),
  };
  if (entity.kind === 'character' || entity.kind === 'object') {
    return { ...base, asset: cloneDirectorAsset(entity.asset) } as Entity;
  }
  if (entity.kind === 'camera') {
    return {
      ...base,
      aspectRatio: { ...entity.aspectRatio },
      guides: { ...entity.guides },
    } as Entity;
  }
  return base as Entity;
}

export function cloneDirectorTrack<Track extends DirectorTimelineTrack>(track: Track): Track {
  return {
    ...track,
    target: { ...track.target },
    keyframes: track.keyframes.map((keyframe) => ({
      ...keyframe,
      value:
        keyframe.valueType === 'vector3'
          ? cloneDirectorVector3(keyframe.value)
          : keyframe.value,
    })),
  } as Track;
}

export function cloneDirectorCaptureRecord<RecordType extends DirectorCaptureRecord>(
  record: RecordType
): RecordType {
  return { ...record };
}

export function cloneDirectorCaptureOperation(
  operation: DirectorCaptureOperation
): DirectorCaptureOperation {
  if (operation.status === 'idle') return operation;
  return { ...operation, request: { ...operation.request } };
}

export function cloneDirectorState(state: DirectorState): DirectorState {
  return {
    ...state,
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
    selection: state.selection ? { ...state.selection } : null,
    panels: { ...state.panels },
    timeline: {
      ...state.timeline,
      tracks: state.timeline.tracks.map(cloneDirectorTrack),
    },
    capture: {
      settings: { ...state.capture.settings },
      operation: cloneDirectorCaptureOperation(state.capture.operation),
      records: state.capture.records.map(cloneDirectorCaptureRecord),
    },
  };
}

export function entityCollectionKey(
  kind: DirectorEntityKind
): 'cameras' | 'characters' | 'objects' | 'lights' {
  switch (kind) {
    case 'camera':
      return 'cameras';
    case 'character':
      return 'characters';
    case 'object':
      return 'objects';
    case 'light':
      return 'lights';
  }
}
