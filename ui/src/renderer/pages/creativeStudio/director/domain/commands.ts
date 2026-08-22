/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  cloneDirectorEntity,
  cloneDirectorTrack,
  cloneDirectorTransform,
  cloneDirectorVector3,
  directorEntityRef,
} from './model';
import type {
  DirectorAssetBinding,
  DirectorCameraAspectRatio,
  DirectorCameraGuides,
  DirectorCaptureSettings,
  DirectorEntity,
  DirectorEntityRef,
  DirectorKeyframe,
  DirectorLight,
  DirectorPanelState,
  DirectorSceneEnvironment,
  DirectorSelection,
  DirectorTimelineTrack,
  DirectorTransform3D,
  DirectorViewMode,
} from './types';

export type DirectorPanel = 'leftSidebar' | 'rightSidebar' | 'timeline';

export type DirectorCommand =
  | { type: 'project/rename'; name: string }
  | { type: 'scene/rename'; name: string }
  | { type: 'scene/set-transform'; transform: DirectorTransform3D }
  | { type: 'scene/configure'; patch: Partial<DirectorSceneEnvironment> }
  | { type: 'entity/add'; entity: DirectorEntity; select: boolean }
  | { type: 'entity/delete'; reference: DirectorEntityRef }
  | { type: 'entity/rename'; reference: DirectorEntityRef; name: string }
  | { type: 'entity/set-transform'; reference: DirectorEntityRef; transform: DirectorTransform3D }
  | { type: 'entity/set-visible'; reference: DirectorEntityRef; visible: boolean }
  | { type: 'entity/set-locked'; reference: DirectorEntityRef; locked: boolean }
  | {
      type: 'entity/set-asset';
      reference: Extract<DirectorEntityRef, { kind: 'character' | 'object' }>;
      asset: DirectorAssetBinding | null;
    }
  | { type: 'selection/set'; selection: DirectorSelection | null }
  | { type: 'camera/set-active'; cameraId: string | null }
  | { type: 'camera/set-aspect-ratio'; cameraId: string; aspectRatio: DirectorCameraAspectRatio }
  | { type: 'camera/set-guides'; cameraId: string; guides: Partial<DirectorCameraGuides> }
  | {
      type: 'camera/configure';
      cameraId: string;
      patch: {
        projection?: 'perspective' | 'orthographic';
        focalLengthMm?: number;
        orthographicSize?: number;
        nearClip?: number;
        farClip?: number;
      };
    }
  | {
      type: 'light/configure';
      lightId: string;
      patch: Partial<Pick<DirectorLight, 'lightType' | 'color' | 'intensity' | 'range' | 'coneAngleDegrees'>>;
    }
  | { type: 'view/set-mode'; mode: DirectorViewMode }
  | { type: 'panel/set'; panel: DirectorPanel; open: boolean }
  | { type: 'panel/toggle'; panel: DirectorPanel }
  | { type: 'timeline/set-duration'; durationSeconds: number }
  | { type: 'timeline/set-time'; timeSeconds: number }
  | { type: 'timeline/set-playing'; playing: boolean }
  | { type: 'timeline/set-loop'; loop: boolean }
  | { type: 'timeline/set-frame-rate'; framesPerSecond: number }
  | { type: 'timeline/tick'; deltaSeconds: number }
  | { type: 'timeline/upsert-track'; track: DirectorTimelineTrack }
  | { type: 'timeline/delete-track'; trackId: string }
  | { type: 'timeline/upsert-keyframe'; trackId: string; keyframe: DirectorKeyframe }
  | { type: 'timeline/delete-keyframe'; trackId: string; keyframeId: string }
  | { type: 'capture/configure'; patch: Partial<DirectorCaptureSettings> }
  | {
      type: 'capture/request';
      requestId: string;
      kind: 'image' | 'video';
      cameraId?: string;
    }
  | { type: 'capture/start'; requestId: string }
  | {
      type: 'capture/complete';
      requestId: string;
      captureId: string;
      assetId: string;
      capturedAt: number;
    }
  | { type: 'capture/fail'; requestId: string; code: string }
  | { type: 'capture/clear-operation' }
  | { type: 'capture/delete-record'; captureId: string };

function cloneEnvironmentPatch(
  patch: Partial<DirectorSceneEnvironment>
): Partial<DirectorSceneEnvironment> {
  return {
    ...patch,
    panorama:
      patch.panorama === undefined
        ? undefined
        : patch.panorama
          ? { assetId: patch.panorama.assetId }
          : null,
  };
}

function cloneKeyframe(keyframe: DirectorKeyframe): DirectorKeyframe {
  return {
    ...keyframe,
    value:
      keyframe.valueType === 'vector3'
        ? cloneDirectorVector3(keyframe.value)
        : keyframe.value,
  } as DirectorKeyframe;
}

export const directorCommands = {
  renameProject(name: string): DirectorCommand {
    return { type: 'project/rename', name };
  },

  renameScene(name: string): DirectorCommand {
    return { type: 'scene/rename', name };
  },

  setSceneTransform(transform: DirectorTransform3D): DirectorCommand {
    return { type: 'scene/set-transform', transform: cloneDirectorTransform(transform) };
  },

  configureScene(patch: Partial<DirectorSceneEnvironment>): DirectorCommand {
    return { type: 'scene/configure', patch: cloneEnvironmentPatch(patch) };
  },

  addEntity(entity: DirectorEntity, options: { select?: boolean } = {}): DirectorCommand {
    return {
      type: 'entity/add',
      entity: cloneDirectorEntity(entity),
      select: options.select ?? true,
    };
  },

  deleteEntity(entity: DirectorEntity | DirectorEntityRef): DirectorCommand {
    const reference = 'transform' in entity ? directorEntityRef(entity) : entity;
    return { type: 'entity/delete', reference: { ...reference } as DirectorEntityRef };
  },

  renameEntity(reference: DirectorEntityRef, name: string): DirectorCommand {
    return { type: 'entity/rename', reference: { ...reference } as DirectorEntityRef, name };
  },

  setEntityTransform(reference: DirectorEntityRef, transform: DirectorTransform3D): DirectorCommand {
    return {
      type: 'entity/set-transform',
      reference: { ...reference } as DirectorEntityRef,
      transform: cloneDirectorTransform(transform),
    };
  },

  setEntityVisible(reference: DirectorEntityRef, visible: boolean): DirectorCommand {
    return { type: 'entity/set-visible', reference: { ...reference } as DirectorEntityRef, visible };
  },

  setEntityLocked(reference: DirectorEntityRef, locked: boolean): DirectorCommand {
    return { type: 'entity/set-locked', reference: { ...reference } as DirectorEntityRef, locked };
  },

  setEntityAsset(
    reference: Extract<DirectorEntityRef, { kind: 'character' | 'object' }>,
    assetId: string | null
  ): DirectorCommand {
    return {
      type: 'entity/set-asset',
      reference: { ...reference },
      asset: assetId === null ? null : { assetId },
    };
  },

  select(selection: DirectorSelection | null): DirectorCommand {
    return { type: 'selection/set', selection: selection ? { ...selection } : null };
  },

  setActiveCamera(cameraId: string | null): DirectorCommand {
    return { type: 'camera/set-active', cameraId };
  },

  setCameraAspectRatio(
    cameraId: string,
    aspectRatio: DirectorCameraAspectRatio
  ): DirectorCommand {
    return { type: 'camera/set-aspect-ratio', cameraId, aspectRatio: { ...aspectRatio } };
  },

  setCameraGuides(cameraId: string, guides: Partial<DirectorCameraGuides>): DirectorCommand {
    return { type: 'camera/set-guides', cameraId, guides: { ...guides } };
  },

  configureCamera(
    cameraId: string,
    patch: Extract<DirectorCommand, { type: 'camera/configure' }>['patch']
  ): DirectorCommand {
    return { type: 'camera/configure', cameraId, patch: { ...patch } };
  },

  configureLight(
    lightId: string,
    patch: Extract<DirectorCommand, { type: 'light/configure' }>['patch']
  ): DirectorCommand {
    return { type: 'light/configure', lightId, patch: { ...patch } };
  },

  setViewMode(mode: DirectorViewMode): DirectorCommand {
    return { type: 'view/set-mode', mode };
  },

  setPanel(panel: DirectorPanel, open: boolean): DirectorCommand {
    return { type: 'panel/set', panel, open };
  },

  togglePanel(panel: DirectorPanel): DirectorCommand {
    return { type: 'panel/toggle', panel };
  },

  setTimelineDuration(durationSeconds: number): DirectorCommand {
    return { type: 'timeline/set-duration', durationSeconds };
  },

  seekTimeline(timeSeconds: number): DirectorCommand {
    return { type: 'timeline/set-time', timeSeconds };
  },

  setTimelinePlaying(playing: boolean): DirectorCommand {
    return { type: 'timeline/set-playing', playing };
  },

  setTimelineLoop(loop: boolean): DirectorCommand {
    return { type: 'timeline/set-loop', loop };
  },

  setTimelineFrameRate(framesPerSecond: number): DirectorCommand {
    return { type: 'timeline/set-frame-rate', framesPerSecond };
  },

  tickTimeline(deltaSeconds: number): DirectorCommand {
    return { type: 'timeline/tick', deltaSeconds };
  },

  upsertTimelineTrack(track: DirectorTimelineTrack): DirectorCommand {
    return { type: 'timeline/upsert-track', track: cloneDirectorTrack(track) };
  },

  deleteTimelineTrack(trackId: string): DirectorCommand {
    return { type: 'timeline/delete-track', trackId };
  },

  upsertKeyframe(trackId: string, keyframe: DirectorKeyframe): DirectorCommand {
    return { type: 'timeline/upsert-keyframe', trackId, keyframe: cloneKeyframe(keyframe) };
  },

  deleteKeyframe(trackId: string, keyframeId: string): DirectorCommand {
    return { type: 'timeline/delete-keyframe', trackId, keyframeId };
  },

  configureCapture(patch: Partial<DirectorCaptureSettings>): DirectorCommand {
    return { type: 'capture/configure', patch: { ...patch } };
  },

  requestCapture(
    requestId: string,
    kind: 'image' | 'video',
    cameraId?: string
  ): DirectorCommand {
    return { type: 'capture/request', requestId, kind, cameraId };
  },

  startCapture(requestId: string): DirectorCommand {
    return { type: 'capture/start', requestId };
  },

  completeCapture(
    requestId: string,
    output: { captureId: string; assetId: string; capturedAt: number }
  ): DirectorCommand {
    return { type: 'capture/complete', requestId, ...output };
  },

  failCapture(requestId: string, code: string): DirectorCommand {
    return { type: 'capture/fail', requestId, code };
  },

  clearCaptureOperation(): DirectorCommand {
    return { type: 'capture/clear-operation' };
  },

  deleteCaptureRecord(captureId: string): DirectorCommand {
    return { type: 'capture/delete-record', captureId };
  },
} as const;

export function directorPanelKey(panel: DirectorPanel): keyof DirectorPanelState {
  switch (panel) {
    case 'leftSidebar':
      return 'leftSidebarOpen';
    case 'rightSidebar':
      return 'rightSidebarOpen';
    case 'timeline':
      return 'timelineOpen';
  }
}
