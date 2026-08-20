/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ReactNode } from 'react';

export type DirectorViewMode = 'director' | 'camera';
export type DirectorTransformMode = 'translate' | 'rotate' | 'scale';
export type DirectorInspectorKind = 'environment' | 'camera' | 'character' | 'object';
export type DirectorInspectorTab = 'properties' | 'captures';
export type DirectorAspectRatio = 'free' | '1:1' | '4:3' | '3:4' | '16:9' | '9:16' | '21:9';
export type DirectorCapturePreset = 'current' | 'four' | 'twelve';

export interface DirectorVector3 {
  x: number;
  y: number;
  z: number;
}

export type DirectorSceneObjectKind = 'character' | 'camera' | 'object' | 'crowd';

export interface DirectorSceneObject {
  id: string;
  name: string;
  kind: DirectorSceneObjectKind;
  visible: boolean;
  locked: boolean;
  selected?: boolean;
  missingLocalAsset?: boolean;
}

export interface DirectorSceneGroup {
  id: string;
  label: string;
  objects: readonly DirectorSceneObject[];
}

/** A thumbnail is rendered only when the caller supplies a real asset URL. */
export interface DirectorModelLibraryItem {
  id: string;
  name: string;
  thumbnailUrl?: string;
  deletable?: boolean;
}

/** Captures always point to real renderer output supplied by the controller. */
export interface DirectorCapture {
  id: string;
  name: string;
  thumbnailUrl: string;
  imageUrl: string;
  cameraId?: string;
}

export interface DirectorPanoramaAsset {
  assetId: string;
  name: string;
  thumbnailUrl: string;
}

export interface DirectorEnvironmentInspectorValue {
  kind: 'environment';
  sceneScale: number;
  position: DirectorVector3;
  rotation: DirectorVector3;
  panorama: DirectorPanoramaAsset | null;
  skyColor: string;
  panoramaYaw: number;
  panoramaRadius: number;
  showLabels: boolean;
  snapToGrid: boolean;
  showGround: boolean;
  showGrid: boolean;
  groundHeight: number;
  groundOpacity: number;
}

export interface DirectorCameraInspectorValue {
  kind: 'camera';
  id: string;
  name: string;
  position: DirectorVector3;
  rotation: DirectorVector3;
  fov: number;
  targetLabel?: string;
  tab: DirectorInspectorTab;
  captures: readonly DirectorCapture[];
}

export interface DirectorCharacterInspectorValue {
  kind: 'character';
  id: string;
  name: string;
  bodyType: string;
  position: DirectorVector3;
  rotation: DirectorVector3;
  scale: number;
  color: string;
  posePresetId?: string | null;
}

export interface DirectorObjectInspectorValue {
  kind: 'object';
  id: string;
  name: string;
  modelLabel?: string;
  position: DirectorVector3;
  rotation: DirectorVector3;
  scale: number;
  color: string;
  localAssetMissing?: boolean;
}

export type DirectorInspectorValue =
  | DirectorEnvironmentInspectorValue
  | DirectorCameraInspectorValue
  | DirectorCharacterInspectorValue
  | DirectorObjectInspectorValue;

export interface DirectorOption {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface DirectorKeyframe {
  id: string;
  timeSeconds: number;
  selected?: boolean;
}

export type DirectorTimelineTrackKind = 'scene' | 'camera' | 'character' | 'object';

export interface DirectorTimelineTrack {
  id: string;
  label: string;
  kind: DirectorTimelineTrackKind;
  selected?: boolean;
  keyframes: readonly DirectorKeyframe[];
}

export interface DirectorTimelineState {
  open: boolean;
  height: number;
  currentTimeSeconds: number;
  durationSeconds: number;
  fps: number;
  playing: boolean;
  loop: boolean;
  autoKey: boolean;
  tracks: readonly DirectorTimelineTrack[];
  selectedTrackId?: string | null;
  selectedKeyframeId?: string | null;
}

export interface DirectorWorkbenchShellProps {
  title?: string;
  viewMode: DirectorViewMode;
  transformMode: DirectorTransformMode;
  viewportSlot: ReactNode;
  viewportOverlaySlot?: ReactNode;
  gizmoSlot?: ReactNode;
  headerActionsSlot?: ReactNode;
  sceneQuery: string;
  sceneGroups: readonly DirectorSceneGroup[];
  inspector: DirectorInspectorValue;
  bodyTypeOptions: readonly DirectorOption[];
  posePresetOptions: readonly DirectorOption[];
  modelLibraryOpen: boolean;
  modelLibraryItems: readonly DirectorModelLibraryItem[];
  aspectPickerOpen: boolean;
  aspectRatio: DirectorAspectRatio;
  showRuleOfThirds: boolean;
  panelsCollapsed: boolean;
  timeline: DirectorTimelineState;
  disabled?: boolean;
  captureBusy?: boolean;
  onClose?(): void;
  onViewModeChange(mode: DirectorViewMode): void;
  onTransformModeChange(mode: DirectorTransformMode): void;
  onSceneQueryChange(query: string): void;
  onSceneObjectSelect(objectId: string): void;
  onSceneObjectVisibilityChange(objectId: string, visible: boolean): void;
  onSceneObjectLockChange(objectId: string, locked: boolean): void;
  onInspectorChange(value: DirectorInspectorValue): void;
  onChoosePanorama?(): void;
  onRemovePanorama?(): void;
  onReimportObjectModel?(): void;
  onPosePresetSelect?(presetId: string): void;
  onCameraCapture?(): void;
  onCaptureView?(capture: DirectorCapture): void;
  onCaptureDelete?(captureId: string): void;
  onCaptureSendToCanvas?(capture: DirectorCapture): void;
  onCaptureClearAll?(): void;
  onCaptureSendAll?(): void;
  onAddCharacter?(): void;
  onImportPanorama?(): void;
  onImportModel?(): void;
  onAddCamera?(): void;
  onCaptureViewport?(preset: DirectorCapturePreset): void;
  onModelLibraryOpenChange(open: boolean): void;
  onModelLibraryAdd?(modelId: string): void;
  onModelLibraryDelete?(modelId: string): void;
  onAspectPickerOpenChange(open: boolean): void;
  onAspectRatioChange(ratio: DirectorAspectRatio): void;
  onRuleOfThirdsChange(enabled: boolean): void;
  onPanelsCollapsedChange(collapsed: boolean): void;
  onTimelineOpenChange(open: boolean): void;
  onTimelinePlayingChange(playing: boolean): void;
  onTimelineLoopChange(loop: boolean): void;
  onTimelineAutoKeyChange(autoKey: boolean): void;
  onTimelineTimeChange(timeSeconds: number): void;
  onTimelineDurationChange(durationSeconds: number): void;
  onTimelineTrackSelect(trackId: string): void;
  onKeyframeSelect(trackId: string, keyframeId: string): void;
  onKeyframeAdd?(trackId: string, timeSeconds: number): void;
  onKeyframeDelete?(trackId: string, keyframeId: string): void;
  onTimelineExport?(): void;
}

export const DIRECTOR_ASPECT_RATIO_OPTIONS: readonly {
  value: DirectorAspectRatio;
  label: string;
}[] = [
  { value: 'free', label: '自由' },
  { value: '1:1', label: '1:1' },
  { value: '4:3', label: '4:3' },
  { value: '3:4', label: '3:4' },
  { value: '16:9', label: '16:9' },
  { value: '9:16', label: '9:16' },
  { value: '21:9', label: '21:9' },
];
