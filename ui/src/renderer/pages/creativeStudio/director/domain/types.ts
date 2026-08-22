/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type DirectorEntityKind = 'camera' | 'character' | 'object' | 'light';
export type DirectorViewMode = 'director' | 'camera';

export interface DirectorVector3 {
  x: number;
  y: number;
  z: number;
}

export interface DirectorTransform3D {
  position: DirectorVector3;
  /** Euler angles in degrees. */
  rotation: DirectorVector3;
  scale: DirectorVector3;
}

export interface DirectorAssetBinding {
  /** Stable NomiFun asset identifier. URL resolution belongs to the host. */
  assetId: string;
}

export interface DirectorSceneEnvironment {
  skyColor: string;
  panorama: DirectorAssetBinding | null;
  panoramaYawDegrees: number;
  panoramaRadius: number;
  groundVisible: boolean;
  gridVisible: boolean;
  snapToGrid: boolean;
  characterLabelsVisible: boolean;
}

export interface DirectorScene {
  name: string;
  transform: DirectorTransform3D;
  environment: DirectorSceneEnvironment;
}

export interface DirectorEntityBase {
  id: string;
  name: string;
  transform: DirectorTransform3D;
  visible: boolean;
  locked: boolean;
}

export interface DirectorCharacter extends DirectorEntityBase {
  kind: 'character';
  asset: DirectorAssetBinding | null;
}

export interface DirectorObject3D extends DirectorEntityBase {
  kind: 'object';
  asset: DirectorAssetBinding | null;
}

export interface DirectorCameraAspectRatio {
  width: number;
  height: number;
}

export interface DirectorCameraGuides {
  frame: boolean;
  center: boolean;
  thirds: boolean;
  safeArea: boolean;
}

export interface DirectorCamera extends DirectorEntityBase {
  kind: 'camera';
  projection: 'perspective' | 'orthographic';
  focalLengthMm: number;
  orthographicSize: number;
  nearClip: number;
  farClip: number;
  aspectRatio: DirectorCameraAspectRatio;
  guides: DirectorCameraGuides;
}

export interface DirectorLight extends DirectorEntityBase {
  kind: 'light';
  lightType: 'ambient' | 'directional' | 'point' | 'spot';
  color: string;
  intensity: number;
  range: number;
  coneAngleDegrees: number;
}

export type DirectorEntity =
  | DirectorCamera
  | DirectorCharacter
  | DirectorObject3D
  | DirectorLight;

export type DirectorEntityRef = {
  [Kind in DirectorEntityKind]: { kind: Kind; id: string };
}[DirectorEntityKind];

export type DirectorSelection = { kind: 'scene' } | DirectorEntityRef;
export type DirectorTrackTarget = { kind: 'scene' } | DirectorEntityRef;

export type DirectorContinuousInterpolation = 'linear' | 'step' | 'ease-in-out';

export interface DirectorNumberKeyframe {
  id: string;
  valueType: 'number';
  timeSeconds: number;
  value: number;
  interpolation: DirectorContinuousInterpolation;
}

export interface DirectorVectorKeyframe {
  id: string;
  valueType: 'vector3';
  timeSeconds: number;
  value: DirectorVector3;
  interpolation: DirectorContinuousInterpolation;
}

export interface DirectorBooleanKeyframe {
  id: string;
  valueType: 'boolean';
  timeSeconds: number;
  value: boolean;
  interpolation: 'step';
}

export type DirectorKeyframe =
  | DirectorNumberKeyframe
  | DirectorVectorKeyframe
  | DirectorBooleanKeyframe;

export interface DirectorVectorTrack {
  id: string;
  target: DirectorTrackTarget;
  valueType: 'vector3';
  property: 'position' | 'rotation' | 'scale';
  keyframes: DirectorVectorKeyframe[];
}

export interface DirectorNumberTrack {
  id: string;
  target: DirectorEntityRef;
  valueType: 'number';
  property: 'focalLengthMm' | 'intensity';
  keyframes: DirectorNumberKeyframe[];
}

export interface DirectorBooleanTrack {
  id: string;
  target: DirectorEntityRef;
  valueType: 'boolean';
  property: 'visible';
  keyframes: DirectorBooleanKeyframe[];
}

export type DirectorTimelineTrack =
  | DirectorVectorTrack
  | DirectorNumberTrack
  | DirectorBooleanTrack;

export interface DirectorTimelineState {
  durationSeconds: number;
  currentTimeSeconds: number;
  framesPerSecond: number;
  playing: boolean;
  loop: boolean;
  tracks: DirectorTimelineTrack[];
}

export interface DirectorCaptureSettings {
  width: number;
  height: number;
  imageFormat: 'png' | 'jpeg';
  videoFramesPerSecond: number;
}

export type DirectorCaptureRequest =
  | {
      requestId: string;
      kind: 'image';
      cameraId: string;
      width: number;
      height: number;
      format: 'png' | 'jpeg';
    }
  | {
      requestId: string;
      kind: 'video';
      cameraId: string;
      width: number;
      height: number;
      format: 'mp4';
      framesPerSecond: number;
      durationSeconds: number;
    };

export type DirectorCaptureRecord =
  | {
      id: string;
      kind: 'image';
      cameraId: string;
      assetId: string;
      capturedAt: number;
      width: number;
      height: number;
      format: 'png' | 'jpeg';
    }
  | {
      id: string;
      kind: 'video';
      cameraId: string;
      assetId: string;
      capturedAt: number;
      width: number;
      height: number;
      format: 'mp4';
      framesPerSecond: number;
      durationSeconds: number;
    };

export type DirectorCaptureOperation =
  | { status: 'idle' }
  | { status: 'queued'; request: DirectorCaptureRequest }
  | { status: 'capturing'; request: DirectorCaptureRequest }
  | {
      status: 'completed';
      request: DirectorCaptureRequest;
      captureId: string;
      assetId: string;
    }
  | { status: 'failed'; request: DirectorCaptureRequest; code: string };

export interface DirectorCaptureState {
  settings: DirectorCaptureSettings;
  operation: DirectorCaptureOperation;
  records: DirectorCaptureRecord[];
}

export interface DirectorPanelState {
  leftSidebarOpen: boolean;
  rightSidebarOpen: boolean;
  timelineOpen: boolean;
}

export interface DirectorState {
  projectId: string;
  name: string;
  scene: DirectorScene;
  cameras: DirectorCamera[];
  characters: DirectorCharacter[];
  objects: DirectorObject3D[];
  lights: DirectorLight[];
  activeCameraId: string | null;
  selection: DirectorSelection | null;
  viewMode: DirectorViewMode;
  panels: DirectorPanelState;
  timeline: DirectorTimelineState;
  capture: DirectorCaptureState;
}

export interface DirectorEvaluatedFrame {
  timeSeconds: number;
  scene: DirectorScene;
  cameras: DirectorCamera[];
  characters: DirectorCharacter[];
  objects: DirectorObject3D[];
  lights: DirectorLight[];
}

export interface DirectorPersistentTimeline {
  durationSeconds: number;
  currentTimeSeconds: number;
  framesPerSecond: number;
  loop: boolean;
  tracks: DirectorTimelineTrack[];
}

export interface DirectorPersistentCapture {
  settings: DirectorCaptureSettings;
  records: DirectorCaptureRecord[];
}

export interface DirectorProjectSnapshotV1 {
  projectId: string;
  name: string;
  scene: DirectorScene;
  cameras: DirectorCamera[];
  characters: DirectorCharacter[];
  objects: DirectorObject3D[];
  lights: DirectorLight[];
  activeCameraId: string | null;
  selection: DirectorSelection | null;
  viewMode: DirectorViewMode;
  panels: DirectorPanelState;
  timeline: DirectorPersistentTimeline;
  capture: DirectorPersistentCapture;
}

export interface DirectorProjectDocumentV1 {
  kind: 'nomifun.director.project';
  version: 1;
  project: DirectorProjectSnapshotV1;
}

export type DirectorImportErrorCode =
  | 'invalid-json'
  | 'invalid-envelope'
  | 'unsupported-version'
  | 'invalid-value'
  | 'limit-exceeded'
  | 'duplicate-id'
  | 'broken-reference';

export interface DirectorImportError {
  code: DirectorImportErrorCode;
  path: string;
  message: string;
}

export type DirectorImportResult =
  | { ok: true; state: DirectorState }
  | { ok: false; error: DirectorImportError };

export type DirectorExportResult =
  | { ok: true; json: string; document: DirectorProjectDocumentV1 }
  | { ok: false; error: DirectorImportError };
