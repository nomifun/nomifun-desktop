/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from "../../assets";
import type {
  DirectorAspectRatio,
  DirectorCapture,
  DirectorInspectorValue,
  DirectorSceneGroup,
  DirectorTimelineState as DirectorTimelinePresentation,
} from "../components";
import { directorVerticalFovDegrees } from "../runtime";
import {
  findDirectorEntity,
  type DirectorCameraAspectRatio,
  type DirectorEntity,
  type DirectorState,
  type DirectorTimelineTrack,
} from "../domain";

export type DirectorAssetLookup = (
  assetId: string,
) => CreativeAsset | undefined;
export type DirectorAssetUrl = (assetId: string) => string;

const averageScale = (entity: DirectorEntity): number =>
  (entity.transform.scale.x +
    entity.transform.scale.y +
    entity.transform.scale.z) /
  3;

const selectedEntity = (state: DirectorState): DirectorEntity | undefined =>
  state.selection && state.selection.kind !== "scene"
    ? findDirectorEntity(state, state.selection)
    : undefined;

export function directorSceneGroups(
  state: DirectorState,
): DirectorSceneGroup[] {
  const selected =
    state.selection && state.selection.kind !== "scene"
      ? `${state.selection.kind}:${state.selection.id}`
      : null;
  const object = (
    entity: DirectorEntity,
    kind: "camera" | "character" | "object",
  ) => ({
    id: entity.id,
    name: entity.name,
    kind,
    visible: entity.visible,
    locked: entity.locked,
    selected: selected === `${entity.kind}:${entity.id}`,
    missingLocalAsset:
      (entity.kind === "character" || entity.kind === "object") &&
      entity.asset === null,
  });
  return [
    {
      id: "cameras",
      label: "机位",
      objects: state.cameras.map((entity) => object(entity, "camera")),
    },
    {
      id: "characters",
      label: "角色",
      objects: state.characters.map((entity) => object(entity, "character")),
    },
    {
      id: "objects",
      label: "模型",
      objects: state.objects.map((entity) => object(entity, "object")),
    },
    {
      id: "lights",
      label: "灯光",
      objects: state.lights.map((entity) => object(entity, "object")),
    },
  ];
}

function capturePresentation(
  state: DirectorState,
  assetUrl: DirectorAssetUrl,
): DirectorCapture[] {
  return state.capture.records
    .filter((record) => record.kind === "image")
    .map((record, index) => ({
      id: record.id,
      assetId: record.assetId,
      name: `机位截图 ${index + 1}`,
      thumbnailUrl: assetUrl(record.assetId),
      imageUrl: assetUrl(record.assetId),
      cameraId: record.cameraId,
    }));
}

export function directorInspectorValue(
  state: DirectorState,
  assetLookup: DirectorAssetLookup,
  assetUrl: DirectorAssetUrl,
  cameraTab: "properties" | "captures",
): DirectorInspectorValue {
  const entity = selectedEntity(state);
  if (!entity) {
    const panoramaId = state.scene.environment.panorama?.assetId;
    const panorama = panoramaId ? assetLookup(panoramaId) : undefined;
    return {
      kind: "environment",
      sceneScale: state.scene.transform.scale.x,
      position: { ...state.scene.transform.position },
      rotation: { ...state.scene.transform.rotation },
      panorama: panoramaId
        ? {
            assetId: panoramaId,
            name: panorama?.title ?? panoramaId,
            thumbnailUrl: panorama?.thumbnailUrl ?? assetUrl(panoramaId),
          }
        : null,
      skyColor: state.scene.environment.skyColor,
      panoramaYaw: state.scene.environment.panoramaYawDegrees,
      panoramaRadius: state.scene.environment.panoramaRadius,
      showLabels: state.scene.environment.characterLabelsVisible,
      snapToGrid: state.scene.environment.snapToGrid,
      showGround: state.scene.environment.groundVisible,
      showGrid: state.scene.environment.gridVisible,
      groundHeight: state.scene.transform.position.y,
      groundOpacity: state.scene.environment.groundVisible ? 1 : 0,
    };
  }
  if (entity.kind === "camera") {
    return {
      kind: "camera",
      id: entity.id,
      name: entity.name,
      position: { ...entity.transform.position },
      rotation: { ...entity.transform.rotation },
      fov: directorVerticalFovDegrees(entity),
      tab: cameraTab,
      captures: capturePresentation(state, assetUrl).filter(
        (capture) => capture.cameraId === entity.id,
      ),
    };
  }
  if (entity.kind === "character") {
    return {
      kind: "character",
      id: entity.id,
      name: entity.name,
      bodyType: entity.asset ? "已绑定真实素材" : "未绑定素材",
      position: { ...entity.transform.position },
      rotation: { ...entity.transform.rotation },
      scale: averageScale(entity),
      color: "#ffffff",
      posePresetId: null,
    };
  }
  return {
    kind: "object",
    id: entity.id,
    name: entity.name,
    modelLabel:
      entity.kind === "light"
        ? `真实 ${entity.lightType} 灯光`
        : entity.asset
          ? (assetLookup(entity.asset.assetId)?.title ?? entity.asset.assetId)
          : "未绑定模型素材",
    position: { ...entity.transform.position },
    rotation: { ...entity.transform.rotation },
    scale: averageScale(entity),
    color: entity.kind === "light" ? entity.color : "#ffffff",
    localAssetMissing: entity.kind === "object" && entity.asset === null,
  };
}

function targetLabel(
  state: DirectorState,
  track: DirectorTimelineTrack,
): string {
  if (track.target.kind === "scene") return state.scene.name;
  return findDirectorEntity(state, track.target)?.name ?? track.target.id;
}

export function directorTimelinePresentation(
  state: DirectorState,
  selection: {
    selectedTrackId: string | null;
    selectedKeyframeId: string | null;
    autoKey: boolean;
  },
): DirectorTimelinePresentation {
  return {
    open: state.panels.timelineOpen,
    height: 260,
    currentTimeSeconds: state.timeline.currentTimeSeconds,
    durationSeconds: state.timeline.durationSeconds,
    fps: state.timeline.framesPerSecond,
    playing: state.timeline.playing,
    loop: state.timeline.loop,
    autoKey: selection.autoKey,
    selectedTrackId: selection.selectedTrackId,
    selectedKeyframeId: selection.selectedKeyframeId,
    tracks: state.timeline.tracks.map((track) => ({
      id: track.id,
      label: `${targetLabel(state, track)} · ${track.property}`,
      kind:
        track.target.kind === "scene" || track.target.kind === "light"
          ? "scene"
          : track.target.kind,
      selected: track.id === selection.selectedTrackId,
      keyframes: track.keyframes.map((keyframe) => ({
        id: keyframe.id,
        timeSeconds: keyframe.timeSeconds,
        selected: keyframe.id === selection.selectedKeyframeId,
      })),
    })),
  };
}

const ASPECTS: Record<
  Exclude<DirectorAspectRatio, "free">,
  DirectorCameraAspectRatio
> = {
  "1:1": { width: 1, height: 1 },
  "4:3": { width: 4, height: 3 },
  "3:4": { width: 3, height: 4 },
  "16:9": { width: 16, height: 9 },
  "9:16": { width: 9, height: 16 },
  "21:9": { width: 21, height: 9 },
};

export function directorAspectRatio(state: DirectorState): DirectorAspectRatio {
  const camera = state.cameras.find(
    (candidate) => candidate.id === state.activeCameraId,
  );
  if (!camera) return "free";
  const match = Object.entries(ASPECTS).find(
    ([, ratio]) =>
      ratio.width === camera.aspectRatio.width &&
      ratio.height === camera.aspectRatio.height,
  );
  return (match?.[0] as DirectorAspectRatio | undefined) ?? "free";
}

export function directorCameraAspectRatio(
  ratio: DirectorAspectRatio,
): DirectorCameraAspectRatio | null {
  return ratio === "free" ? null : { ...ASPECTS[ratio] };
}

export function directorFocalLengthForVerticalFov(
  fovDegrees: number,
  aspectRatio: DirectorCameraAspectRatio,
): number {
  const aspect = aspectRatio.width / aspectRatio.height;
  const filmHeight = 36 / Math.max(aspect, 1);
  const radians = Math.max(1, Math.min(179, fovDegrees)) * (Math.PI / 180);
  return filmHeight / (2 * Math.tan(radians / 2));
}
