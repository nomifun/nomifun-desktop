/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { uuidv7 } from "@/common/utils/uuidv7";
import {
  Attention,
  CheckOne,
  Close,
  Loading,
  Picture,
  Refresh,
  Upload,
} from "@icon-park/react";
import { Button, Modal, Spin } from "@arco-design/web-react";
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useParams } from "react-router-dom";

import { creativeAssetClient, CreativeAssetDeletedError, isCreativeAssetDeleted, useCreativeAssetAvailability, type CreativeAsset } from "../../assets";
import CreativeAssetMedia from "../../assets/components/CreativeAssetMedia";
import {
  CREATIVE_STUDIO_PROJECTS_PATH,
  creativeStudioCanvasProjectPath,
} from "../../app/routes";
import {
  creativeProjectRepository,
  isCreativeProjectRepositoryError,
} from "../../services";
import {
  DirectorWorkbenchShell,
  type DirectorAspectRatio,
  type DirectorCapture,
  type DirectorInspectorValue,
  type DirectorTransformMode,
} from "../components";
import {
  createDirectorCamera,
  directorCommands,
  directorReducer,
  findDirectorEntity,
  type DirectorCommand,
  type DirectorEntityRef,
  type DirectorKeyframe,
  type DirectorState,
  type DirectorTimelineTrack,
} from "../domain";
import {
  directorVerticalFovDegrees,
  DirectorRuntimeViewport,
  type DirectorRuntimeError,
  type DirectorRuntimeHandle,
} from "../runtime";
import { registerCreativeDirectorProductBeforeLeave } from "./beforeLeave";
import {
  DirectorCanvasTransferError,
  transferDirectorCapturesWithReconciliation,
} from "./directorCanvasTransfer";
import {
  DirectorCasSaveController,
  type DirectorCasSaveSnapshot,
} from "./directorCasSaveController";
import {
  directorAspectRatio,
  directorCameraAspectRatio,
  directorFocalLengthForVerticalFov,
  directorInspectorValue,
  directorSceneGroups,
  directorTimelinePresentation,
} from "./directorPresenter";
import {
  DirectorProjectLoadError,
  loadDirectorProjectBaseline,
  persistDirectorProject,
  type DirectorProjectBaseline,
} from "./directorProjectPersistence";
import styles from "./CreativeDirectorProductRoute.module.css";

type LoadState =
  | { status: "loading"; error: null }
  | { status: "ready"; error: null }
  | { status: "error"; error: Error };

class DirectorRouteLoadError extends Error {
  readonly code = "missing-canvas-id";

  constructor() {
    super("missing-canvas-id");
    this.name = "DirectorRouteLoadError";
  }
}

const isPanorama = (asset: CreativeAsset): boolean =>
  !isCreativeAssetDeleted(asset) &&
  asset.kind === "image" &&
  asset.width !== null &&
  asset.height !== null &&
  asset.width > 0 &&
  asset.height > 0 &&
  Math.abs(asset.width / asset.height - 2) <= 0.03;

async function loadAllImageAssets(): Promise<CreativeAsset[]> {
  const pageSize = 100;
  const items: CreativeAsset[] = [];
  for (let page = 1; ; page += 1) {
    const result = await creativeAssetClient.list({
      kind: "image",
      sort: "updated_desc",
      page,
      pageSize,
    });
    items.push(...result.items);
    if (items.length >= result.total || result.items.length === 0) return items;
  }
}

async function loadCurrentDirectorBaseline(
  projectId: string,
  defaultSceneName: string,
): Promise<DirectorProjectBaseline> {
  const detail = await creativeProjectRepository.load(projectId);
  return loadDirectorProjectBaseline(
    detail,
    creativeAssetClient,
    defaultSceneName,
  );
}

function entityReferenceById(
  state: DirectorState,
  id: string,
): DirectorEntityRef | null {
  for (const entity of [
    ...state.cameras,
    ...state.characters,
    ...state.objects,
    ...state.lights,
  ]) {
    if (entity.id === id) return { kind: entity.kind, id } as DirectorEntityRef;
  }
  return null;
}

function keyframeAt(
  state: DirectorState,
  track: DirectorTimelineTrack,
  timeSeconds: number,
): DirectorKeyframe | null {
  const target =
    track.target.kind === "scene"
      ? state.scene
      : findDirectorEntity(state, track.target);
  if (!target) return null;
  if (track.valueType === "vector3") {
    return {
      id: uuidv7(),
      valueType: "vector3",
      timeSeconds,
      value: { ...target.transform[track.property] },
      interpolation: "linear",
    };
  }
  if (track.valueType === "boolean") {
    if (!("visible" in target)) return null;
    return {
      id: uuidv7(),
      valueType: "boolean",
      timeSeconds,
      value: target.visible,
      interpolation: "step",
    };
  }
  if (track.property === "focalLengthMm" && "focalLengthMm" in target) {
    return {
      id: uuidv7(),
      valueType: "number",
      timeSeconds,
      value: target.focalLengthMm,
      interpolation: "linear",
    };
  }
  if (track.property === "intensity" && "intensity" in target) {
    return {
      id: uuidv7(),
      valueType: "number",
      timeSeconds,
      value: target.intensity,
      interpolation: "linear",
    };
  }
  return null;
}

const saveLabel = (
  save: DirectorCasSaveSnapshot,
  t: (key: string, options?: Record<string, unknown>) => string,
): string => {
  switch (save.status) {
    case "dirty":
      return t("creativeStudio.director.save.pending", {
        defaultValue: "等待保存",
      });
    case "saving":
      return t("creativeStudio.director.save.saving", {
        defaultValue: "正在保存",
      });
    case "saved":
      return t("creativeStudio.director.save.saved", {
        defaultValue: "已保存",
      });
    case "conflict":
      return t("creativeStudio.director.save.conflict", {
        defaultValue: "保存冲突",
      });
    case "error":
      return t("creativeStudio.director.save.error", {
        defaultValue: "保存失败",
      });
    default:
      return t("creativeStudio.director.save.loaded", {
        defaultValue: "已载入",
      });
  }
};

const runtimeErrorMessage = (
  error: DirectorRuntimeError,
  t: (key: string, options?: Record<string, unknown>) => string,
): string => {
  if (error.cause instanceof CreativeAssetDeletedError) {
    return t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' });
  }
  switch (error.code) {
    case "asset-url":
      return t("creativeStudio.director.errors.assetUrl", {
        defaultValue: "无法解析 3D 素材地址。",
      });
    case "asset-fetch":
      return t("creativeStudio.director.errors.assetFetch", {
        defaultValue: "无法下载 3D 素材。",
      });
    case "asset-decode":
      return t("creativeStudio.director.errors.assetDecode", {
        defaultValue: "无法解析 3D 素材。",
      });
    case "capture":
      return t("creativeStudio.director.errors.capture", {
        defaultValue: "无法截取当前视角。",
      });
    case "renderer":
      return t("creativeStudio.director.errors.renderer", {
        defaultValue: "无法初始化 3D 渲染器。",
      });
  }
};

const directorErrorMessage = (
  error: unknown,
  t: (key: string, options?: Record<string, unknown>) => string,
): string => {
  if (error instanceof CreativeAssetDeletedError) return error.message;
  if (error instanceof DirectorRouteLoadError) {
    return t("creativeStudio.director.errors.missingCanvasId", {
      defaultValue: "缺少 Creative Studio 画布 ID。",
    });
  }
  if (error instanceof DirectorProjectLoadError) {
    switch (error.code) {
      case "multiple-directors":
        return t("creativeStudio.director.errors.multipleDirectors", {
          defaultValue: "当前画布包含多个导演场景，无法确定要打开的场景。",
        });
      case "missing-scene-asset":
        return t("creativeStudio.director.errors.missingSceneAsset", {
          defaultValue: "导演场景素材不存在，无法打开场景。",
        });
      case "invalid-scene-asset":
        return t("creativeStudio.director.errors.invalidSceneAsset", {
          defaultValue: "导演场景素材无效或无法读取。",
        });
      case "project-mismatch":
        return t("creativeStudio.director.errors.projectMismatch", {
          defaultValue: "导演场景素材属于另一张画布。",
        });
      case "projection-mismatch":
        return t("creativeStudio.director.errors.projectionMismatch", {
          defaultValue: "导演场景与画布状态不一致，请重新载入。",
        });
    }
  }
  if (error instanceof DirectorCanvasTransferError) {
    switch (error.code) {
      case "empty-transfer":
        return t("creativeStudio.director.errors.emptyTransfer", {
          defaultValue: "没有可发送到画布的截图。",
        });
      case "duplicate-capture":
        return t("creativeStudio.director.errors.duplicateCapture", {
          defaultValue: "所选截图中存在重复项。",
        });
      case "capture-not-found":
        return t("creativeStudio.director.errors.captureNotFound", {
          defaultValue: "导演场景中找不到所选截图。",
        });
      case "capture-asset-mismatch":
        return t("creativeStudio.director.errors.captureAssetMismatch", {
          defaultValue: "截图素材已经变化，请重新选择。",
        });
      case "capture-not-image":
        return t("creativeStudio.director.errors.captureNotImage", {
          defaultValue: "所选截图不是可用的图片素材。",
        });
    }
  }
  if (isCreativeProjectRepositoryError(error)) {
    switch (error.kind) {
      case "not-found":
        return t("creativeStudio.director.errors.canvasNotFound", {
          defaultValue: "画布不存在或已被删除。",
        });
      case "permission-denied":
        return t("creativeStudio.director.errors.permissionDenied", {
          defaultValue: "没有权限访问这张画布。",
        });
      case "revision-conflict":
        return t("creativeStudio.director.errors.revisionConflict", {
          defaultValue: "画布已被其他操作更新，请重新载入。",
        });
      case "transport":
      case "server":
        return t("creativeStudio.director.errors.network", {
          defaultValue: "无法连接到服务，请检查网络后重试。",
        });
      default:
        break;
    }
  }
  return t("creativeStudio.director.errors.unknown", {
    defaultValue: "操作失败，请重试。",
  });
};

/** No-props route composition for the canonical Creative Studio Director. */
const CreativeDirectorProductRoute: React.FC = () => {
  const { canvasId: routeCanvasId } = useParams<{ canvasId: string }>();
  const canvasId = routeCanvasId?.trim() ?? "";
  // Legacy local adapter: the migrated Director internals still use projectId.
  const projectId = canvasId;
  const navigate = useNavigate();
  const { t, i18n } = useTranslation();
  const translateRef = useRef(t);
  translateRef.current = t;
  const runtimeRef = useRef<DirectorRuntimeHandle>(null);
  const panoramaInputRef = useRef<HTMLInputElement>(null);
  const stateRef = useRef<DirectorState | null>(null);
  const loadEpochRef = useRef(0);
  const captureInFlightRef = useRef(false);
  const captureTransferInFlightRef = useRef(false);

  const controller = useMemo(
    () =>
      new DirectorCasSaveController((baseline, state) =>
        persistDirectorProject({
          baseline,
           state,
           repository: creativeProjectRepository,
           assets: creativeAssetClient,
           sceneAssetTitle: translateRef.current(
             "creativeStudio.director.save.sceneAssetTitle",
             {
               defaultValue: "{{name}} · 3D导演场景",
               name: state.name,
             },
           ),
         }),
       ),
    [],
  );
  const save = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot,
  );

  const [load, setLoad] = useState<LoadState>({
    status: "loading",
    error: null,
  });
  const [state, setState] = useState<DirectorState | null>(null);
  const [assets, setAssets] = useState<CreativeAsset[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const [runtimeError, setRuntimeError] = useState<DirectorRuntimeError | null>(
    null,
  );
  const [transformMode, setTransformMode] =
    useState<DirectorTransformMode>("translate");
  const [sceneQuery, setSceneQuery] = useState("");
  const [modelLibraryOpen, setModelLibraryOpen] = useState(false);
  const [aspectPickerOpen, setAspectPickerOpen] = useState(false);
  const [panoramaPickerOpen, setPanoramaPickerOpen] = useState(false);
  const [panoramaUploading, setPanoramaUploading] = useState(false);
  const [panelsCollapsed, setPanelsCollapsed] = useState(false);
  const [cameraTab, setCameraTab] = useState<"properties" | "captures">(
    "properties",
  );
  const [selectedTrackId, setSelectedTrackId] = useState<string | null>(null);
  const [selectedKeyframeId, setSelectedKeyframeId] = useState<string | null>(
    null,
  );
  const [autoKey, setAutoKey] = useState(false);
  const [recoveryBusy, setRecoveryBusy] = useState(false);
  const [captureTransferBusy, setCaptureTransferBusy] = useState(false);

  const adoptBaseline = useCallback(
    (baseline: DirectorProjectBaseline) => {
      controller.reset(baseline);
      stateRef.current = baseline.state;
      setState(baseline.state);
      setPanelsCollapsed(
        !baseline.state.panels.leftSidebarOpen &&
          !baseline.state.panels.rightSidebarOpen,
      );
      setSelectedTrackId(baseline.state.timeline.tracks[0]?.id ?? null);
      setSelectedKeyframeId(null);
    },
    [controller],
  );

  const hydrate = useCallback(async () => {
    const epoch = ++loadEpochRef.current;
    setLoad({ status: "loading", error: null });
    setNotice(null);
    setRuntimeError(null);
    if (!projectId) {
      setLoad({
        status: "error",
        error: new DirectorRouteLoadError(),
      });
      return;
    }
    try {
      const [baseline, imageAssets] = await Promise.all([
        loadCurrentDirectorBaseline(
          projectId,
          t("creativeStudio.director.defaults.sceneName", {
            defaultValue: "场景",
          }),
        ),
        loadAllImageAssets(),
      ]);
      if (epoch !== loadEpochRef.current) return;
      adoptBaseline(baseline);
      setAssets(imageAssets);
      setLoad({ status: "ready", error: null });
    } catch (cause) {
      if (epoch !== loadEpochRef.current) return;
      setLoad({
        status: "error",
        error: cause instanceof Error ? cause : new Error(String(cause)),
      });
    }
  }, [adoptBaseline, projectId, t]);

  useEffect(() => {
    void hydrate();
    return () => {
      loadEpochRef.current += 1;
    };
  }, [hydrate]);

  useEffect(() => () => controller.dispose(), [controller]);

  const commitState = useCallback(
    (next: DirectorState) => {
      stateRef.current = next;
      setState(next);
      controller.queue(next);
    },
    [controller],
  );

  const applyCommands = useCallback(
    (...commands: DirectorCommand[]) => {
      const current = stateRef.current;
      if (!current) return;
      const next = commands.reduce(directorReducer, current);
      if (next !== current) commitState(next);
    },
    [commitState],
  );

  useEffect(() => {
    if (!state?.timeline.playing) return;
    let frame = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const delta = Math.min(0.25, Math.max(0, (now - last) / 1_000));
      last = now;
      applyCommands(directorCommands.tickTimeline(delta));
      if (stateRef.current?.timeline.playing)
        frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [applyCommands, state?.timeline.playing]);

  const flushBeforeLeave = useCallback(async (): Promise<boolean> => {
    if (captureInFlightRef.current || captureTransferInFlightRef.current) {
      setNotice(
        t("creativeStudio.director.notifications.captureBusy", {
          defaultValue: "截图仍在处理，请等待完成后再离开。",
        }),
      );
      return false;
    }
    const result = await controller.flush();
    return result.status === "noop" || result.status === "saved";
  }, [controller, t]);

  useEffect(
    () => registerCreativeDirectorProductBeforeLeave(flushBeforeLeave),
    [flushBeforeLeave],
  );

  const handleClose = useCallback(async () => {
    if (recoveryBusy) return;
    setRecoveryBusy(true);
    try {
      if (await flushBeforeLeave())
        navigate(creativeStudioCanvasProjectPath(projectId));
    } finally {
      setRecoveryBusy(false);
    }
  }, [flushBeforeLeave, navigate, projectId, recoveryBusy]);

  const handleRetrySave = useCallback(async () => {
    if (recoveryBusy) return;
    setRecoveryBusy(true);
    try {
      const result = await controller.flush();
      setNotice(
        result.status === "saved" || result.status === "noop"
          ? t("creativeStudio.director.notifications.sceneSaved", {
              defaultValue: "导演场景已保存。",
            })
          : t("creativeStudio.director.notifications.saveFailed", {
              defaultValue: "导演场景保存失败，请重试。",
            }),
      );
    } finally {
      setRecoveryBusy(false);
    }
  }, [controller, recoveryBusy, t]);

  const handleReload = useCallback(async () => {
    if (recoveryBusy) return;
    setRecoveryBusy(true);
    try {
      await hydrate();
    } finally {
      setRecoveryBusy(false);
    }
  }, [hydrate, recoveryBusy]);

  const handleSceneSelect = useCallback(
    (objectId: string) => {
      const current = stateRef.current;
      if (!current) return;
      const reference = entityReferenceById(current, objectId);
      if (reference) applyCommands(directorCommands.select(reference));
    },
    [applyCommands],
  );

  const handleInspectorChange = useCallback(
    (value: DirectorInspectorValue) => {
      const current = stateRef.current;
      if (!current) return;
      if (value.kind === "environment") {
        const scale = {
          x: value.sceneScale,
          y: value.sceneScale,
          z: value.sceneScale,
        };
        applyCommands(
          directorCommands.setSceneTransform({
            position: { ...value.position },
            rotation: { ...value.rotation },
            scale,
          }),
          directorCommands.configureScene({
            skyColor: value.skyColor,
            panoramaYawDegrees: value.panoramaYaw,
            panoramaRadius: value.panoramaRadius,
            characterLabelsVisible: value.showLabels,
            snapToGrid: value.snapToGrid,
            groundVisible: value.showGround,
            gridVisible: value.showGrid,
          }),
        );
        if (value.groundOpacity !== (value.showGround ? 1 : 0)) {
          setNotice(
            t("creativeStudio.director.notifications.groundOpacityUnsupported", {
              defaultValue:
                "当前场景格式只保存地面显示开关，暂不保存地面透明度。",
            }),
          );
        }
        return;
      }

      const selection = current.selection;
      if (!selection || selection.kind === "scene") return;
      const entity = findDirectorEntity(current, selection);
      if (!entity) return;
      if (value.kind === "camera" && entity.kind === "camera") {
        if (value.tab !== cameraTab) setCameraTab(value.tab);
        const commands: DirectorCommand[] = [];
        if (value.name !== entity.name) {
          commands.push(directorCommands.renameEntity(selection, value.name));
        }
        if (
          value.position.x !== entity.transform.position.x ||
          value.position.y !== entity.transform.position.y ||
          value.position.z !== entity.transform.position.z ||
          value.rotation.x !== entity.transform.rotation.x ||
          value.rotation.y !== entity.transform.rotation.y ||
          value.rotation.z !== entity.transform.rotation.z
        ) {
          commands.push(directorCommands.setEntityTransform(selection, {
            ...entity.transform,
            position: { ...value.position },
            rotation: { ...value.rotation },
          }));
        }
        if (value.fov !== directorVerticalFovDegrees(entity)) {
          commands.push(directorCommands.configureCamera(entity.id, {
            focalLengthMm: directorFocalLengthForVerticalFov(
              value.fov,
              entity.aspectRatio,
            ),
          }));
        }
        if (commands.length > 0) applyCommands(...commands);
        return;
      }
      if (value.kind === "character" && entity.kind === "character") {
        const scale = { x: value.scale, y: value.scale, z: value.scale };
        applyCommands(
          directorCommands.renameEntity(selection, value.name),
          directorCommands.setEntityTransform(selection, {
            position: { ...value.position },
            rotation: { ...value.rotation },
            scale,
          }),
        );
        return;
      }
      if (
        value.kind === "object" &&
        (entity.kind === "object" || entity.kind === "light")
      ) {
        const scale = { x: value.scale, y: value.scale, z: value.scale };
        const commands: DirectorCommand[] = [
          directorCommands.renameEntity(selection, value.name),
          directorCommands.setEntityTransform(selection, {
            position: { ...value.position },
            rotation: { ...value.rotation },
            scale,
          }),
        ];
        if (entity.kind === "light") {
          commands.push(
            directorCommands.configureLight(entity.id, { color: value.color }),
          );
        }
        applyCommands(...commands);
      }
    },
    [applyCommands, cameraTab, t],
  );

  const addCamera = useCallback(() => {
    const current = stateRef.current;
    if (!current) return;
    const camera = createDirectorCamera({
      id: uuidv7(),
      name: t("creativeStudio.director.defaults.cameraName", {
        defaultValue: "机位 {{index}}",
        index: current.cameras.length + 1,
      }),
      transform: {
        position: { x: 0, y: 2, z: 5 },
        rotation: { x: 0, y: 0, z: 0 },
        scale: { x: 1, y: 1, z: 1 },
      },
    });
    const track: DirectorTimelineTrack = {
      id: uuidv7(),
      target: { kind: "camera", id: camera.id },
      valueType: "vector3",
      property: "position",
      keyframes: [],
    };
    applyCommands(
      directorCommands.addEntity(camera),
      directorCommands.upsertTimelineTrack(track),
    );
    setSelectedTrackId(track.id);
    setSelectedKeyframeId(null);
  }, [applyCommands, t]);

  const choosePanorama = useCallback(
    async (candidate: CreativeAsset) => {
      let asset: CreativeAsset;
      try {
        asset = await creativeAssetClient.get(candidate.id);
        if (isCreativeAssetDeleted(asset)) throw new CreativeAssetDeletedError(asset.id);
      } catch (reason) {
        setNotice(directorErrorMessage(reason, t));
        return;
      }
      if (!isPanorama(asset)) {
        setNotice(
          t("creativeStudio.director.notifications.panoramaInvalid", {
            defaultValue: "全景背景必须是具备真实尺寸信息的 2:1 图片。",
          }),
        );
        return;
      }
      applyCommands(
        directorCommands.configureScene({ panorama: { assetId: asset.id } }),
      );
      setPanoramaPickerOpen(false);
      setNotice(
        t("creativeStudio.director.notifications.panoramaConnected", {
          defaultValue: "已连接真实全景素材“{{name}}”。",
          name: asset.title,
        }),
      );
    },
    [applyCommands, t],
  );

  const handlePanoramaFile = useCallback(
    async (file: File | undefined) => {
      if (!file || panoramaUploading) return;
      setPanoramaUploading(true);
      try {
        const asset = await creativeAssetClient.upload(file, {
          title: file.name,
          inLibrary: true,
          tags: ["panorama"],
        });
        setAssets((current) => [
          asset,
          ...current.filter((item) => item.id !== asset.id),
        ]);
        if (!isPanorama(asset)) {
          setNotice(
            t("creativeStudio.director.notifications.panoramaWrongRatio", {
              defaultValue:
                "图片已真实上传到素材库，但尺寸不是 2:1，未绑定为全景背景。",
            }),
          );
          return;
        }
        choosePanorama(asset);
      } catch {
        setNotice(
          t("creativeStudio.director.notifications.panoramaUploadFailed", {
            defaultValue: "全景图上传失败，请重试。",
          }),
        );
      } finally {
        setPanoramaUploading(false);
        if (panoramaInputRef.current) panoramaInputRef.current.value = "";
      }
    },
    [choosePanorama, panoramaUploading, t],
  );

  const captureCurrentCamera = useCallback(async () => {
    const current = stateRef.current;
    if (!current || captureInFlightRef.current) return;
    if (!current.activeCameraId) {
      setNotice(
        t("creativeStudio.director.notifications.captureCameraRequired", {
          defaultValue: "请先添加并选择一个真实机位。",
        }),
      );
      return;
    }
    const requestId = uuidv7();
    const queued = directorReducer(
      current,
      directorCommands.requestCapture(
        requestId,
        "image",
        current.activeCameraId,
      ),
    );
    if (queued.capture.operation.status !== "queued") {
      setNotice(
        t("creativeStudio.director.notifications.captureUnavailable", {
          defaultValue: "当前机位无法开始截图。",
        }),
      );
      return;
    }
    const started = directorReducer(
      queued,
      directorCommands.startCapture(requestId),
    );
    commitState(started);
    captureInFlightRef.current = true;
    setNotice(
      t("creativeStudio.director.notifications.captureProcessing", {
        defaultValue: "正在从真实 Three.js 视口生成截图…",
      }),
    );
    try {
      if (
        started.capture.operation.status !== "capturing" ||
        started.capture.operation.request.kind !== "image"
      ) {
        throw new Error("截图状态无效。");
      }
      const sceneAssetIds = [
        ...(current.scene.environment.panorama ? [current.scene.environment.panorama.assetId] : []),
        ...[...current.characters, ...current.objects].flatMap((entity) => entity.asset ? [entity.asset.assetId] : []),
      ];
      const sceneAssets = await Promise.all(sceneAssetIds.map((id) => creativeAssetClient.get(id)));
      const deleted = sceneAssets.find(isCreativeAssetDeleted);
      if (deleted) throw new CreativeAssetDeletedError(deleted.id);
      const result = await runtimeRef.current?.captureImage(
        started.capture.operation.request,
      );
      if (!result) throw new Error("3D 视口尚未就绪。");
      const extension = result.format === "jpeg" ? "jpg" : "png";
      const mime = result.format === "jpeg" ? "image/jpeg" : "image/png";
      const file = new File(
        [result.blob],
        `director-${requestId}.${extension}`,
        { type: mime },
      );
      const asset = await creativeAssetClient.upload(file, {
        title: t("creativeStudio.director.capture.assetTitle", {
          defaultValue: "3D导演截图 {{date}}",
          date: new Intl.DateTimeFormat(
            i18n.resolvedLanguage || i18n.language,
            {
              dateStyle: "short",
              timeStyle: "medium",
            },
          ).format(new Date()),
        }),
        inLibrary: true,
        tags: ["director-capture"],
      });
      setAssets((currentAssets) => [
        asset,
        ...currentAssets.filter((item) => item.id !== asset.id),
      ]);
      const latest = stateRef.current ?? started;
      const completed = directorReducer(
        latest,
        directorCommands.completeCapture(requestId, {
          captureId: uuidv7(),
          assetId: asset.id,
          capturedAt: Date.now(),
        }),
      );
      commitState(completed);
      setCameraTab("captures");
      setNotice(
        t("creativeStudio.director.notifications.captureUploaded", {
          defaultValue: "截图已上传为真实 NomiFun 素材，可发送到画布。",
        }),
      );
    } catch {
      const latest = stateRef.current ?? started;
      commitState(
        directorReducer(
          latest,
          directorCommands.failCapture(requestId, "capture-failed"),
        ),
      );
      setNotice(
        t("creativeStudio.director.notifications.captureFailed", {
          defaultValue: "截图失败，请确认 3D 视口已就绪后重试。",
        }),
      );
    } finally {
      captureInFlightRef.current = false;
    }
  }, [commitState, i18n.language, i18n.resolvedLanguage, t]);

  const deleteCapture = useCallback(
    (captureId: string) => {
      applyCommands(directorCommands.deleteCaptureRecord(captureId));
      setNotice(
        t("creativeStudio.director.notifications.captureRemoved", {
          defaultValue: "已从场景记录移除截图；素材库原文件保持可恢复。",
        }),
      );
    },
    [applyCommands, t],
  );

  const clearCaptures = useCallback(() => {
    const current = stateRef.current;
    if (!current) return;
    applyCommands(
      ...current.capture.records.map((capture) =>
        directorCommands.deleteCaptureRecord(capture.id),
      ),
    );
    setNotice(
      t("creativeStudio.director.notifications.capturesCleared", {
        defaultValue: "已清空场景截图记录；素材库原文件未删除。",
      }),
    );
  }, [applyCommands, t]);

  const sendCapturesToCanvas = useCallback(
    async (captures: readonly DirectorCapture[]) => {
      if (captures.length === 0 || captureTransferInFlightRef.current) return;
      captureTransferInFlightRef.current = true;
      setCaptureTransferBusy(true);
      setNotice(
        t("creativeStudio.director.notifications.transferProcessing", {
          defaultValue: "正在校验截图素材并写入画布…",
        }),
      );
      try {
        const flushed = await controller.flush();
        if (flushed.status === "conflict" || flushed.status === "error") {
          setNotice(
            t("creativeStudio.director.notifications.transferSaveRequired", {
              defaultValue: "导演场景尚未保存，无法发送到画布。",
            }),
          );
          return;
        }
        const baseline = controller.getBaseline();
        if (!baseline) {
          setNotice(
            t("creativeStudio.director.notifications.sceneNotLoaded", {
              defaultValue: "导演画布尚未完成载入。",
            }),
          );
          return;
        }
        const transfers = await Promise.all(
          captures.map(async (capture) => ({
            captureId: capture.id,
            asset: await creativeAssetClient.get(capture.assetId),
          })),
        );
        const outcome = await transferDirectorCapturesWithReconciliation({
          baseline,
           captures: transfers,
           repository: creativeProjectRepository,
           reloadBaseline: () =>
             loadCurrentDirectorBaseline(
               projectId,
               t("creativeStudio.director.defaults.sceneName", {
                 defaultValue: "场景",
               }),
             ),
         });
        adoptBaseline(outcome.result.baseline);
        setCameraTab("captures");
        switch (outcome.status) {
          case "inserted":
            setNotice(
              t("creativeStudio.director.notifications.transferInserted", {
                defaultValue: "已将 {{total}} 张截图发送到画布。",
                total: outcome.result.insertedNodes.length,
              }),
            );
            break;
          case "already-present":
            setNotice(
              t("creativeStudio.director.notifications.transferAlreadyPresent", {
                defaultValue: "所选截图已在画布中，没有创建重复节点。",
              }),
            );
            break;
          case "confirmed-after-response-loss":
            setNotice(
              t(
                "creativeStudio.director.notifications.transferConfirmed",
                {
                  defaultValue:
                    "发送响应曾中断，但已确认截图已插入画布。",
                },
              ),
            );
            break;
          case "conflict":
            setNotice(
              t("creativeStudio.director.notifications.transferConflict", {
                defaultValue: "画布已被其他操作更新，请重试发送。",
              }),
            );
            break;
          case "failed":
            setNotice(
              t("creativeStudio.director.notifications.transferFailed", {
                defaultValue: "发送失败，已重新载入画布，请重试。",
              }),
            );
            break;
        }
      } catch (cause) {
        setNotice(directorErrorMessage(cause, t));
      } finally {
        captureTransferInFlightRef.current = false;
        setCaptureTransferBusy(false);
      }
    },
    [adoptBaseline, controller, projectId, t],
  );

  const addKeyframe = useCallback(
    (trackId: string, timeSeconds: number) => {
      const current = stateRef.current;
      const track = current?.timeline.tracks.find(
        (candidate) => candidate.id === trackId,
      );
      if (!current || !track) return;
      const keyframe = keyframeAt(current, track, timeSeconds);
      if (!keyframe) return;
      applyCommands(directorCommands.upsertKeyframe(trackId, keyframe));
      setSelectedTrackId(trackId);
      setSelectedKeyframeId(keyframe.id);
    },
    [applyCommands],
  );

  const assetById = useMemo(
    () => new Map(assets.map((asset) => [asset.id, asset] as const)),
    [assets],
  );
  const panoramas = useMemo(() => assets.filter(isPanorama), [assets]);
  const mediaAvailability = useCreativeAssetAvailability([
    ...(state?.capture.records.map((capture) => capture.assetId) ?? []),
    ...(state?.scene.environment.panorama ? [state.scene.environment.panorama.assetId] : []),
    ...[...(state?.characters ?? []), ...(state?.objects ?? [])].flatMap((entity) => entity.asset ? [entity.asset.assetId] : []),
  ]);
  const assetResolutionRevision = JSON.stringify([...mediaAvailability.entries()].filter(([, value]) => value === 'deleted').map(([id]) => id).sort());

  if (load.status === "loading") {
    return (
      <main className={styles.routeState} data-creative-director-product-route>
        <Spin dot size={8} />
        <h1>
          {t("creativeStudio.director.loading.title", {
            defaultValue: "正在载入 3D 导演台",
          })}
        </h1>
        <p>
          {t("creativeStudio.director.loading.description", {
            defaultValue: "正在校验画布、场景资产和素材引用。",
          })}
        </p>
      </main>
    );
  }

  if (load.status === "error" || !state) {
    const message =
      load.status === "error"
        ? directorErrorMessage(load.error, t)
        : t("creativeStudio.director.errors.noState", {
            defaultValue: "导演场景没有可用状态。",
          });
    return (
      <main className={styles.routeState} data-creative-director-product-route>
        <Attention size={34} strokeWidth={1.7} />
        <h1>
          {t("creativeStudio.director.errors.title", {
            defaultValue: "无法打开 3D 导演台",
          })}
        </h1>
        <p role="alert">{message}</p>
        <div className={styles.routeStateActions}>
          <Button icon={<Refresh />} onClick={() => void hydrate()}>
            {t("creativeStudio.director.actions.reload", {
              defaultValue: "重新载入",
            })}
          </Button>
          <Button
            type="primary"
            onClick={() =>
              navigate(
                projectId
                  ? creativeStudioCanvasProjectPath(projectId)
                  : CREATIVE_STUDIO_PROJECTS_PATH,
              )
            }
          >
            {t("creativeStudio.director.actions.backToCanvas", {
              defaultValue: "返回画布",
            })}
          </Button>
        </div>
      </main>
    );
  }

  const captureBusy =
    captureInFlightRef.current ||
    state.capture.operation.status === "queued" ||
    state.capture.operation.status === "capturing";
  const aspectRatio = directorAspectRatio(state);
  const activeCamera = state.cameras.find(
    (camera) => camera.id === state.activeCameraId,
  );
  const inspector = directorInspectorValue(
    state,
    (assetId) => assetById.get(assetId),
    (assetId) => creativeAssetClient.url(assetId),
    cameraTab,
    t,
    mediaAvailability,
  );
  const timeline = directorTimelinePresentation(state, {
    selectedTrackId,
    selectedKeyframeId,
    autoKey,
  }, t);
  const disabled = save.revision === null || recoveryBusy || captureTransferBusy;

  const saveActions = (
    <div className={styles.saveActions} data-save-status={save.status}>
      <span
        className={styles.saveStatus}
        role="status"
        title={save.error ? directorErrorMessage(save.error, t) : undefined}
      >
        {save.status === "saving" ? (
          <Loading className={styles.spin} size={15} />
        ) : save.status === "conflict" || save.status === "error" ? (
          <Attention size={15} />
        ) : (
          <CheckOne size={15} />
        )}
        {saveLabel(save, t)}
      </span>
      {save.status === "conflict" ? (
        <Button
          size="small"
          icon={<Refresh />}
          onClick={() => void handleReload()}
        >
          {t("creativeStudio.director.actions.loadRemote", {
            defaultValue: "载入远端",
          })}
        </Button>
      ) : null}
      {save.status === "error" ? (
        <Button
          size="small"
          icon={<Refresh />}
          onClick={() => void handleRetrySave()}
        >
          {t("creativeStudio.director.actions.retry", {
            defaultValue: "重试",
          })}
        </Button>
      ) : null}
    </div>
  );

  return (
    <main
      className={styles.root}
      data-creative-director-product-route
      data-canvas-id={canvasId}
    >
      <input
        ref={panoramaInputRef}
        className={styles.hiddenInput}
        type="file"
        accept="image/*"
        aria-hidden="true"
        tabIndex={-1}
        onChange={(event) =>
          void handlePanoramaFile(event.currentTarget.files?.[0])
        }
      />
      <DirectorWorkbenchShell
        title={controller.getBaseline()?.project.title ?? state.name}
        viewMode={state.viewMode}
        transformMode={transformMode}
        viewportSlot={
          <DirectorRuntimeViewport
            ref={runtimeRef}
            state={state}
            assetResolutionRevision={assetResolutionRevision}
            resolveAssetUrl={async (assetId) => {
              const asset = await creativeAssetClient.get(assetId);
              if (isCreativeAssetDeleted(asset)) throw new CreativeAssetDeletedError(assetId);
              return asset.originalUrl;
            }}
            showAxes
            onError={setRuntimeError}
          />
        }
        viewportOverlaySlot={
          notice || runtimeError ? (
            <div className={styles.viewportNotices}>
              {runtimeError ? (
                <div className={styles.runtimeError} role="alert">
                  <Attention size={16} />
                   <span>{runtimeErrorMessage(runtimeError, t)}</span>
                   <button
                     type="button"
                     aria-label={t(
                       "creativeStudio.director.actions.closeRuntimeError",
                       {
                         defaultValue: "关闭渲染错误",
                       },
                     )}
                    onClick={() => setRuntimeError(null)}
                  >
                    <Close size={14} />
                  </button>
                </div>
              ) : null}
              {notice ? (
                <div className={styles.notice} role="status">
                  {notice}
                </div>
              ) : null}
            </div>
          ) : undefined
        }
        headerActionsSlot={saveActions}
        sceneQuery={sceneQuery}
         sceneGroups={directorSceneGroups(state, t)}
        inspector={inspector}
        bodyTypeOptions={[]}
        posePresetOptions={[]}
        modelLibraryOpen={modelLibraryOpen}
        modelLibraryItems={[]}
        aspectPickerOpen={aspectPickerOpen}
        aspectRatio={aspectRatio}
        showRuleOfThirds={activeCamera?.guides.thirds ?? false}
        panelsCollapsed={panelsCollapsed}
        timeline={timeline}
        disabled={disabled}
        captureBusy={captureBusy}
        onClose={() => void handleClose()}
        onViewModeChange={(mode) =>
          applyCommands(directorCommands.setViewMode(mode))
        }
        onTransformModeChange={setTransformMode}
        onSceneQueryChange={setSceneQuery}
        onSceneObjectSelect={handleSceneSelect}
        onSceneObjectVisibilityChange={(objectId, visible) => {
          const reference = entityReferenceById(state, objectId);
          if (reference)
            applyCommands(
              directorCommands.setEntityVisible(reference, visible),
            );
        }}
        onSceneObjectLockChange={(objectId, locked) => {
          const reference = entityReferenceById(state, objectId);
          if (reference)
            applyCommands(directorCommands.setEntityLocked(reference, locked));
        }}
        onInspectorChange={handleInspectorChange}
        onChoosePanorama={() => setPanoramaPickerOpen(true)}
        onRemovePanorama={() =>
          applyCommands(directorCommands.configureScene({ panorama: null }))
        }
        onCameraCapture={() => void captureCurrentCamera()}
        onCaptureView={(capture: DirectorCapture) =>
          window.open(capture.imageUrl, "_blank", "noopener,noreferrer")
        }
        onCaptureDelete={deleteCapture}
        onCaptureSendToCanvas={(capture) => void sendCapturesToCanvas([capture])}
        onCaptureClearAll={clearCaptures}
        onCaptureSendAll={() => {
          if (inspector.kind === "camera") {
            void sendCapturesToCanvas(inspector.captures);
          }
        }}
         onAddCharacter={() => {
           setModelLibraryOpen(true);
           setNotice(
             t("creativeStudio.director.notifications.characterUnavailable", {
               defaultValue:
                 "角色需要真实 3D 模型；当前素材服务尚未提供 GLB，因此不会创建占位角色。",
             }),
           );
        }}
        onImportPanorama={() => panoramaInputRef.current?.click()}
        onAddCamera={addCamera}
        onCaptureViewport={(preset) => {
          if (preset === "current") void captureCurrentCamera();
           else
             setNotice(
               t("creativeStudio.director.notifications.multiCaptureUnavailable", {
                 defaultValue:
                   "多方位截图需要真实机位旋转编排，当前版本不会生成四方位或十二方位占位结果。",
               }),
             );
        }}
        onModelLibraryOpenChange={setModelLibraryOpen}
        onAspectPickerOpenChange={setAspectPickerOpen}
        onAspectRatioChange={(ratio: DirectorAspectRatio) => {
           if (!state.activeCameraId) {
             setNotice(
               t("creativeStudio.director.notifications.aspectCameraRequired", {
                 defaultValue: "请先添加活动机位再设置画幅比例。",
               }),
             );
             return;
          }
          const cameraRatio = directorCameraAspectRatio(ratio);
           if (!cameraRatio) {
             setNotice(
               t("creativeStudio.director.notifications.aspectFreePreview", {
                 defaultValue:
                   "自由画幅是视口预览模式，不会覆盖机位的画幅比例。",
               }),
             );
            return;
          }
          applyCommands(
            directorCommands.setCameraAspectRatio(
              state.activeCameraId,
              cameraRatio,
            ),
          );
        }}
        onRuleOfThirdsChange={(enabled) => {
          if (state.activeCameraId) {
            applyCommands(
              directorCommands.setCameraGuides(state.activeCameraId, {
                thirds: enabled,
              }),
            );
          }
        }}
        onPanelsCollapsedChange={(collapsed) => {
          setPanelsCollapsed(collapsed);
          applyCommands(
            directorCommands.setPanel("leftSidebar", !collapsed),
            directorCommands.setPanel("rightSidebar", !collapsed),
          );
        }}
        onTimelineOpenChange={(open) =>
          applyCommands(directorCommands.setPanel("timeline", open))
        }
        onTimelinePlayingChange={(playing) =>
          applyCommands(directorCommands.setTimelinePlaying(playing))
        }
        onTimelineLoopChange={(loop) =>
          applyCommands(directorCommands.setTimelineLoop(loop))
        }
        onTimelineAutoKeyChange={setAutoKey}
        onTimelineTimeChange={(time) =>
          applyCommands(directorCommands.seekTimeline(time))
        }
        onTimelineDurationChange={(duration) =>
          applyCommands(directorCommands.setTimelineDuration(duration))
        }
        onTimelineTrackSelect={(trackId) => {
          setSelectedTrackId(trackId);
          setSelectedKeyframeId(null);
        }}
        onKeyframeSelect={(trackId, keyframeId) => {
          setSelectedTrackId(trackId);
          setSelectedKeyframeId(keyframeId);
        }}
        onKeyframeAdd={addKeyframe}
        onKeyframeDelete={(trackId, keyframeId) => {
          applyCommands(directorCommands.deleteKeyframe(trackId, keyframeId));
          setSelectedKeyframeId(null);
        }}
      />

      <Modal
        visible={panoramaPickerOpen}
        title={t("creativeStudio.director.panoramaPicker.title", {
          defaultValue: "选择真实 2:1 全景素材",
        })}
        footer={null}
        autoFocus={false}
        unmountOnExit
        getPopupContainer={() =>
          document.getElementById("creative-studio-portal-root") ??
          document.body
        }
        onCancel={() => setPanoramaPickerOpen(false)}
      >
        <div className={styles.panoramaPicker}>
          <div className={styles.panoramaPickerHeader}>
            <p>
              {t("creativeStudio.director.panoramaPicker.description", {
                defaultValue: "仅显示已验证宽高比为 2:1 的图片素材。",
              })}
            </p>
            <Button
              icon={<Upload />}
              loading={panoramaUploading}
              onClick={() => panoramaInputRef.current?.click()}
            >
              {t("creativeStudio.director.panoramaPicker.upload", {
                defaultValue: "上传全景图",
              })}
            </Button>
          </div>
          {panoramas.length === 0 ? (
            <div className={styles.panoramaEmpty} role="status">
              <Picture size={28} strokeWidth={1.7} />
              <span>
                {t("creativeStudio.director.panoramaPicker.empty", {
                  defaultValue: "暂无符合条件的 2:1 图片",
                })}
              </span>
            </div>
          ) : (
            <div className={styles.panoramaGrid} role="list">
              {panoramas.map((asset) => (
                <button
                  key={asset.id}
                  type="button"
                  className={styles.panoramaCard}
                  onClick={() => choosePanorama(asset)}
                >
                  <div className={styles.panoramaPreview}>
                    <CreativeAssetMedia
                      asset={asset}
                      compact
                      unavailableLabel={t("creativeStudio.assets.library.mediaUnavailable", {
                        defaultValue: "素材暂时无法预览",
                      })}
                    />
                  </div>
                  <strong title={asset.title}>{asset.title}</strong>
                  <small>
                    {asset.width} × {asset.height}
                  </small>
                </button>
              ))}
            </div>
          )}
        </div>
      </Modal>
    </main>
  );
};

export default CreativeDirectorProductRoute;
