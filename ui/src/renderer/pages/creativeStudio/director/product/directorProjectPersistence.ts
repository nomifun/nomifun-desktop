/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { uuidv7 } from "@/common/utils/uuidv7";

import type { CreativeAsset, CreativeAssetLibraryPort } from "../../assets";
import type {
  CreativeCanvasNode,
  CreativeProjectDetail,
  CreativeProjectDocument,
  CreativeProjectSummary,
} from "../../domain";
import {
  isCreativeProjectRepositoryError,
  type CreativeProjectRepository,
} from "../../services";
import {
  createDirectorState,
  exportDirectorProjectV1,
  importDirectorProjectV1,
  type DirectorState,
} from "../domain";

const DIRECTOR_SCENE_PAGE_SIZE = 100;

export type DirectorProjectLoadErrorCode =
  | "multiple-directors"
  | "missing-scene-asset"
  | "invalid-scene-asset"
  | "project-mismatch"
  | "projection-mismatch";

export class DirectorProjectLoadError extends Error {
  readonly code: DirectorProjectLoadErrorCode;

  constructor(code: DirectorProjectLoadErrorCode, message: string) {
    super(message);
    this.name = "DirectorProjectLoadError";
    this.code = code;
  }
}

export interface DirectorProjectBaseline {
  project: CreativeProjectSummary;
  document: CreativeProjectDocument;
  directorNodeId: string | null;
  sceneAssetId: string | null;
  state: DirectorState;
}

export interface PersistDirectorProjectInput {
  baseline: DirectorProjectBaseline;
  state: DirectorState;
  repository: CreativeProjectRepository;
  assets: CreativeAssetLibraryPort;
}

function directorNodes(
  document: CreativeProjectDocument,
): Array<Extract<CreativeCanvasNode, { type: "director" }>> {
  return document.nodes.filter(
    (node): node is Extract<CreativeCanvasNode, { type: "director" }> =>
      node.type === "director",
  );
}

async function findTextAsset(
  assets: CreativeAssetLibraryPort,
  assetId: string,
): Promise<CreativeAsset | null> {
  let page = 1;
  for (;;) {
    const result = await assets.list({
      kind: "text",
      sort: "updated_desc",
      page,
      pageSize: DIRECTOR_SCENE_PAGE_SIZE,
    });
    const match = result.items.find((asset) => asset.id === assetId);
    if (match) return match;
    if (
      page * DIRECTOR_SCENE_PAGE_SIZE >= result.total ||
      result.items.length === 0
    ) {
      return null;
    }
    page += 1;
  }
}

function emptyDirectorState(detail: CreativeProjectDetail): DirectorState {
  return createDirectorState({
    projectId: detail.project.projectId,
    name: detail.project.title,
    sceneName: "场景",
  });
}

function assertNodeProjection(
  node: Extract<CreativeCanvasNode, { type: "director" }>,
  state: DirectorState,
): void {
  const durationMs = Math.round(state.timeline.durationSeconds * 1_000);
  const timelineMs = Math.round(state.timeline.currentTimeSeconds * 1_000);
  if (
    node.data.cameraId !== state.activeCameraId ||
    node.data.durationMs !== durationMs ||
    node.data.timelineMs !== timelineMs
  ) {
    throw new DirectorProjectLoadError(
      "projection-mismatch",
      "导演节点与其场景资产的机位或时间轴投影不一致。请重新载入或修复项目数据。",
    );
  }
}

/**
 * Load the one Director scene selected by the canonical canvas document.
 * `sceneId` is a stable pointer to a real NomiFun text asset containing the
 * versioned `nomifun.director.project` document. No URL or binary is persisted
 * in the canvas document.
 */
export async function loadDirectorProjectBaseline(
  detail: CreativeProjectDetail,
  assets: CreativeAssetLibraryPort,
): Promise<DirectorProjectBaseline> {
  const nodes = directorNodes(detail.document);
  if (nodes.length > 1) {
    throw new DirectorProjectLoadError(
      "multiple-directors",
      "当前项目包含多个导演节点，无法在没有明确节点路由的情况下选择场景。",
    );
  }

  const node = nodes[0] ?? null;
  if (!node || node.data.sceneId === null) {
    if (node?.data.cameraId) {
      throw new DirectorProjectLoadError(
        "projection-mismatch",
        "导演节点尚未绑定场景，却引用了活动机位。",
      );
    }
    const state = emptyDirectorState(detail);
    if (node) {
      state.timeline.durationSeconds = node.data.durationMs / 1_000;
      state.timeline.currentTimeSeconds = Math.min(
        node.data.timelineMs / 1_000,
        state.timeline.durationSeconds,
      );
    }
    return {
      project: detail.project,
      document: structuredClone(detail.document),
      directorNodeId: node?.id ?? null,
      sceneAssetId: null,
      state,
    };
  }

  const sceneAsset = await findTextAsset(assets, node.data.sceneId);
  if (!sceneAsset) {
    throw new DirectorProjectLoadError(
      "missing-scene-asset",
      "导演节点引用的场景资产不存在，未创建替代场景。",
    );
  }
  if (sceneAsset.kind !== "text" || sceneAsset.textContent === null) {
    throw new DirectorProjectLoadError(
      "invalid-scene-asset",
      "导演节点引用的场景资产不是可读取的文本场景文档。",
    );
  }

  const imported = importDirectorProjectV1(sceneAsset.textContent);
  if (!imported.ok) {
    throw new DirectorProjectLoadError(
      "invalid-scene-asset",
      `导演场景文档无效：${imported.error.path} ${imported.error.message}`,
    );
  }
  if (imported.state.projectId !== detail.project.projectId) {
    throw new DirectorProjectLoadError(
      "project-mismatch",
      "导演场景资产属于另一个 Creative Studio 项目。",
    );
  }
  assertNodeProjection(node, imported.state);

  return {
    project: detail.project,
    document: structuredClone(detail.document),
    directorNodeId: node.id,
    sceneAssetId: sceneAsset.id,
    state: imported.state,
  };
}

function directorNodeForState(
  baseline: DirectorProjectBaseline,
  state: DirectorState,
  sceneAssetId: string,
): Extract<CreativeCanvasNode, { type: "director" }> {
  const existing = baseline.directorNodeId
    ? baseline.document.nodes.find(
        (node): node is Extract<CreativeCanvasNode, { type: "director" }> =>
          node.id === baseline.directorNodeId && node.type === "director",
      )
    : null;
  const data = {
    sceneId: sceneAssetId,
    cameraId: state.activeCameraId,
    timelineMs: Math.round(state.timeline.currentTimeSeconds * 1_000),
    durationMs: Math.round(state.timeline.durationSeconds * 1_000),
  };
  if (existing) return { ...existing, data };

  const zIndex =
    baseline.document.nodes.reduce(
      (highest, node) => Math.max(highest, node.zIndex),
      -1,
    ) + 1;
  return {
    id: uuidv7(),
    type: "director",
    position: { x: 0, y: 0 },
    size: { width: 360, height: 220 },
    groupId: null,
    zIndex,
    locked: false,
    data,
  };
}

function documentWithDirectorNode(
  baseline: DirectorProjectBaseline,
  node: Extract<CreativeCanvasNode, { type: "director" }>,
): CreativeProjectDocument {
  const document = structuredClone(baseline.document);
  const index = document.nodes.findIndex(
    (candidate) => candidate.id === node.id,
  );
  if (index >= 0) document.nodes[index] = node;
  else document.nodes.push(node);
  return document;
}

async function cleanupRejectedSceneAsset(
  assets: CreativeAssetLibraryPort,
  assetId: string,
  cause: unknown,
): Promise<void> {
  if (
    !isCreativeProjectRepositoryError(cause) ||
    ![
      "contract",
      "not-found",
      "revision-conflict",
      "invalid-request",
      "permission-denied",
    ].includes(cause.kind)
  ) {
    // A transport/server response may have been lost after commit. Retaining
    // the new sidecar is safer than deleting bytes the committed pointer needs.
    return;
  }
  try {
    await assets.remove(assetId);
  } catch {
    // The original CAS error remains authoritative; orphan cleanup is best effort.
  }
}

/**
 * Persist one immutable scene sidecar, then atomically advance the canonical
 * project pointer with the root project's compare-and-swap revision.
 */
export async function persistDirectorProject(
  input: PersistDirectorProjectInput,
): Promise<DirectorProjectBaseline> {
  const exported = exportDirectorProjectV1(input.state);
  if (!exported.ok) {
    throw new TypeError(
      `导演场景无法保存：${exported.error.path} ${exported.error.message}`,
    );
  }

  const sceneAsset = await input.assets.createText({
    title: `${input.state.name} · 3D导演场景`,
    textContent: exported.json,
    inLibrary: false,
    tags: ["nomifun-director-v1"],
  });
  const node = directorNodeForState(input.baseline, input.state, sceneAsset.id);
  const document = documentWithDirectorNode(input.baseline, node);

  let project: CreativeProjectSummary;
  try {
    project = await input.repository.save(
      input.baseline.project.projectId,
      input.baseline.project.revision,
      document,
    );
  } catch (cause) {
    await cleanupRejectedSceneAsset(input.assets, sceneAsset.id, cause);
    throw cause;
  }

  const previousSceneAssetId = input.baseline.sceneAssetId;
  if (previousSceneAssetId && previousSceneAssetId !== sceneAsset.id) {
    try {
      await input.assets.remove(previousSceneAssetId);
    } catch {
      // The new pointer is already committed. Old immutable sidecar cleanup is
      // non-authoritative and must never turn a successful save into an error.
    }
  }

  return {
    project,
    document,
    directorNodeId: node.id,
    sceneAssetId: sceneAsset.id,
    state: structuredClone(input.state),
  };
}
