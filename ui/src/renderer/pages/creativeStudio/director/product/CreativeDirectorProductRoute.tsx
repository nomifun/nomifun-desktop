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
import { useNavigate, useParams } from "react-router-dom";

import { creativeAssetClient, type CreativeAsset } from "../../assets";
import {
  CREATIVE_STUDIO_PROJECTS_PATH,
  creativeStudioCanvasProjectPath,
} from "../../app/routes";
import { creativeProjectRepository } from "../../services";
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
import { transferDirectorCapturesWithReconciliation } from "./directorCanvasTransfer";
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
  loadDirectorProjectBaseline,
  persistDirectorProject,
  type DirectorProjectBaseline,
} from "./directorProjectPersistence";
import styles from "./CreativeDirectorProductRoute.module.css";

type LoadState =
  | { status: "loading"; error: null }
  | { status: "ready"; error: null }
  | { status: "error"; error: Error };

const INITIAL_SAVE: DirectorCasSaveSnapshot = {
  status: "idle",
  revision: null,
  hasPendingChanges: false,
  error: null,
};

const isPanorama = (asset: CreativeAsset): boolean =>
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
): Promise<DirectorProjectBaseline> {
  const detail = await creativeProjectRepository.load(projectId);
  return loadDirectorProjectBaseline(detail, creativeAssetClient);
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

const saveLabel = (save: DirectorCasSaveSnapshot): string => {
  switch (save.status) {
    case "dirty":
      return "等待保存";
    case "saving":
      return "正在保存";
    case "saved":
      return "已保存";
    case "conflict":
      return "保存冲突";
    case "error":
      return "保存失败";
    default:
      return "已载入";
  }
};

/** No-props route composition for the canonical Creative Studio Director. */
const CreativeDirectorProductRoute: React.FC = () => {
  const { canvasId: routeCanvasId } = useParams<{ canvasId: string }>();
  const canvasId = routeCanvasId?.trim() ?? "";
  // Legacy local adapter: the migrated Director internals still use projectId.
  const projectId = canvasId;
  const navigate = useNavigate();
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
        error: new TypeError("缺少 Creative Studio 画布 ID。"),
      });
      return;
    }
    try {
      const [baseline, imageAssets] = await Promise.all([
        loadCurrentDirectorBaseline(projectId),
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
  }, [adoptBaseline, projectId]);

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
      setNotice("截图仍在处理，请等待完成后再离开。");
      return false;
    }
    const result = await controller.flush();
    return result.status === "noop" || result.status === "saved";
  }, [controller]);

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
          ? "导演场景已保存。"
          : result.error.message,
      );
    } finally {
      setRecoveryBusy(false);
    }
  }, [controller, recoveryBusy]);

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
            "canonical Director v1 只持久化地面显示开关，不伪造透明度字段。",
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
    [applyCommands, cameraTab],
  );

  const addCamera = useCallback(() => {
    const current = stateRef.current;
    if (!current) return;
    const camera = createDirectorCamera({
      id: uuidv7(),
      name: `机位 ${current.cameras.length + 1}`,
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
  }, [applyCommands]);

  const choosePanorama = useCallback(
    (asset: CreativeAsset) => {
      if (!isPanorama(asset)) {
        setNotice("全景背景必须是具备真实尺寸信息的 2:1 图片。");
        return;
      }
      applyCommands(
        directorCommands.configureScene({ panorama: { assetId: asset.id } }),
      );
      setPanoramaPickerOpen(false);
      setNotice(`已连接真实全景素材“${asset.title}”。`);
    },
    [applyCommands],
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
            "图片已真实上传到素材库，但尺寸不是 2:1，未绑定为全景背景。",
          );
          return;
        }
        choosePanorama(asset);
      } catch (cause) {
        setNotice(cause instanceof Error ? cause.message : String(cause));
      } finally {
        setPanoramaUploading(false);
        if (panoramaInputRef.current) panoramaInputRef.current.value = "";
      }
    },
    [choosePanorama, panoramaUploading],
  );

  const captureCurrentCamera = useCallback(async () => {
    const current = stateRef.current;
    if (!current || captureInFlightRef.current) return;
    if (!current.activeCameraId) {
      setNotice("请先添加并选择一个真实机位。");
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
      setNotice("当前机位无法开始截图。");
      return;
    }
    const started = directorReducer(
      queued,
      directorCommands.startCapture(requestId),
    );
    commitState(started);
    captureInFlightRef.current = true;
    setNotice("正在从真实 Three.js 视口生成截图…");
    try {
      if (
        started.capture.operation.status !== "capturing" ||
        started.capture.operation.request.kind !== "image"
      ) {
        throw new Error("截图状态无效。");
      }
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
        title: `3D导演截图 ${new Date().toLocaleString()}`,
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
      setNotice("截图已上传为真实 NomiFun 素材，可发送到画布。");
    } catch (cause) {
      const latest = stateRef.current ?? started;
      commitState(
        directorReducer(
          latest,
          directorCommands.failCapture(requestId, "capture-failed"),
        ),
      );
      setNotice(cause instanceof Error ? cause.message : String(cause));
    } finally {
      captureInFlightRef.current = false;
    }
  }, [commitState]);

  const deleteCapture = useCallback(
    (captureId: string) => {
      applyCommands(directorCommands.deleteCaptureRecord(captureId));
      setNotice("已从场景记录移除截图；素材库原文件保持可恢复。");
    },
    [applyCommands],
  );

  const clearCaptures = useCallback(() => {
    const current = stateRef.current;
    if (!current) return;
    applyCommands(
      ...current.capture.records.map((capture) =>
        directorCommands.deleteCaptureRecord(capture.id),
      ),
    );
    setNotice("已清空场景截图记录；素材库原文件未删除。");
  }, [applyCommands]);

  const sendCapturesToCanvas = useCallback(
    async (captures: readonly DirectorCapture[]) => {
      if (captures.length === 0 || captureTransferInFlightRef.current) return;
      captureTransferInFlightRef.current = true;
      setCaptureTransferBusy(true);
      setNotice("正在校验真实截图素材并写入画布…");
      try {
        const flushed = await controller.flush();
        if (flushed.status === "conflict" || flushed.status === "error") {
          setNotice(`导演场景尚未保存，无法发送到画布：${flushed.error.message}`);
          return;
        }
        const baseline = controller.getBaseline();
        if (!baseline) throw new Error("导演画布尚未完成载入。");
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
          reloadBaseline: () => loadCurrentDirectorBaseline(projectId),
        });
        adoptBaseline(outcome.result.baseline);
        setCameraTab("captures");
        switch (outcome.status) {
          case "inserted":
            setNotice(
              `已将 ${outcome.result.insertedNodes.length} 张真实截图发送到画布。`,
            );
            break;
          case "already-present":
            setNotice("所选截图已在画布中，没有创建重复节点。");
            break;
          case "confirmed-after-response-loss":
            setNotice("发送响应曾中断，但已从权威画布确认截图已插入。");
            break;
          case "conflict":
            setNotice("画布已被其他操作更新，已载入远端版本；请重试发送。");
            break;
          case "failed":
            setNotice(`发送失败，已重新载入权威画布：${outcome.error.message}`);
            break;
        }
      } catch (cause) {
        setNotice(cause instanceof Error ? cause.message : String(cause));
      } finally {
        captureTransferInFlightRef.current = false;
        setCaptureTransferBusy(false);
      }
    },
    [adoptBaseline, controller, projectId],
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

  if (load.status === "loading") {
    return (
      <main className={styles.routeState} data-creative-director-product-route>
        <Spin dot size={8} />
        <h1>正在载入 3D 导演台</h1>
        <p>正在校验 canonical 画布、场景资产和真实素材引用。</p>
      </main>
    );
  }

  if (load.status === "error" || !state) {
    const message =
      load.status === "error" ? load.error.message : "导演场景没有可用状态。";
    return (
      <main className={styles.routeState} data-creative-director-product-route>
        <Attention size={34} strokeWidth={1.7} />
        <h1>无法打开 3D 导演台</h1>
        <p role="alert">{message}</p>
        <div className={styles.routeStateActions}>
          <Button icon={<Refresh />} onClick={() => void hydrate()}>
            重新载入
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
            返回画布
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
  );
  const timeline = directorTimelinePresentation(state, {
    selectedTrackId,
    selectedKeyframeId,
    autoKey,
  });
  const disabled = save.revision === null || recoveryBusy || captureTransferBusy;

  const saveActions = (
    <div className={styles.saveActions} data-save-status={save.status}>
      <span
        className={styles.saveStatus}
        role="status"
        title={save.error?.message}
      >
        {save.status === "saving" ? (
          <Loading className={styles.spin} size={15} />
        ) : save.status === "conflict" || save.status === "error" ? (
          <Attention size={15} />
        ) : (
          <CheckOne size={15} />
        )}
        {saveLabel(save)}
      </span>
      {save.status === "conflict" ? (
        <Button
          size="small"
          icon={<Refresh />}
          onClick={() => void handleReload()}
        >
          载入远端
        </Button>
      ) : null}
      {save.status === "error" ? (
        <Button
          size="small"
          icon={<Refresh />}
          onClick={() => void handleRetrySave()}
        >
          重试
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
            resolveAssetUrl={(assetId) => creativeAssetClient.url(assetId)}
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
                  <span>{runtimeError.message}</span>
                  <button
                    type="button"
                    aria-label="关闭渲染错误"
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
        sceneGroups={directorSceneGroups(state)}
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
            "角色需要真实 3D 模型；当前 NomiFun 素材后端尚未接收 GLB，不创建占位角色。",
          );
        }}
        onImportPanorama={() => panoramaInputRef.current?.click()}
        onAddCamera={addCamera}
        onCaptureViewport={(preset) => {
          if (preset === "current") void captureCurrentCamera();
          else
            setNotice(
              "多方位截图需要真实机位旋转编排，当前版本不会伪造四/十二方位结果。",
            );
        }}
        onModelLibraryOpenChange={setModelLibraryOpen}
        onAspectPickerOpenChange={setAspectPickerOpen}
        onAspectRatioChange={(ratio: DirectorAspectRatio) => {
          if (!state.activeCameraId) {
            setNotice("请先添加活动机位再设置画幅比例。");
            return;
          }
          const cameraRatio = directorCameraAspectRatio(ratio);
          if (!cameraRatio) {
            setNotice("自由画幅是视口预览模式，不覆盖机位的 canonical 比例。");
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
        title="选择真实 2:1 全景素材"
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
            <p>仅显示已验证宽高比为 2:1 的真实图片素材。</p>
            <Button
              icon={<Upload />}
              loading={panoramaUploading}
              onClick={() => panoramaInputRef.current?.click()}
            >
              上传全景图
            </Button>
          </div>
          {panoramas.length === 0 ? (
            <div className={styles.panoramaEmpty} role="status">
              <Picture size={28} strokeWidth={1.7} />
              <span>暂无符合条件的 2:1 图片</span>
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
                  <img
                    src={asset.thumbnailUrl ?? asset.originalUrl}
                    alt={`${asset.title} 全景缩略图`}
                  />
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
