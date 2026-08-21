/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import type { CreativeAsset } from "../../assets";
import {
  createEmptyCreativeProjectDocument,
  type CreativeProjectSummary,
} from "../../domain";
import type { CreativeProjectRepository } from "../../services";
import { CreativeProjectRepositoryError } from "../../services";
import {
  createDirectorCamera,
  createDirectorState,
  directorCommands,
  directorReducer,
} from "../domain";
import type { DirectorProjectBaseline } from "./directorProjectPersistence";
import {
  DirectorCanvasTransferError,
  directorCaptureAssetsPresent,
  planDirectorCapturesForCanvas,
  transferDirectorCapturesToCanvas,
  transferDirectorCapturesWithReconciliation,
} from "./directorCanvasTransfer";

const PROJECT_ID = "018f7a3c-1234-7abc-8abc-1234567890ab";
const CAPTURE_ID = "018f7a3c-1235-7abc-8abc-1234567890ab";
const ASSET_ID = "018f7a3c-1236-7abc-8abc-1234567890ab";
const DIRECTOR_NODE_ID = "018f7a3c-1237-7abc-8abc-1234567890ab";

const project = (revision = "4"): CreativeProjectSummary => ({
  projectId: PROJECT_ID,
  title: "导演项目",
  revision,
  nodeCount: 1,
  connectionCount: 0,
  createdAt: 1,
  updatedAt: 1,
});

const imageAsset = (overrides: Partial<CreativeAsset> = {}): CreativeAsset => ({
  id: ASSET_ID,
  kind: "image",
  title: "真实导演截图",
  collection: null,
  tags: ["director-capture"],
  mimeType: "image/png",
  width: 1_920,
  height: 1_080,
  bytes: 32_000,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/api/creative-studio/files/${ASSET_ID}`,
  thumbnailUrl: `/api/creative-studio/files/${ASSET_ID}?thumb=1`,
  createdAt: 1,
  updatedAt: 1,
  ...overrides,
});

function baseline(): DirectorProjectBaseline {
  const camera = createDirectorCamera({
    id: "018f7a3c-1238-7abc-8abc-1234567890ab",
    name: "主机位",
  });
  let state = directorReducer(
    createDirectorState({ projectId: PROJECT_ID, name: "导演项目" }),
    directorCommands.addEntity(camera),
  );
  state.capture.records.push({
    id: CAPTURE_ID,
    kind: "image",
    cameraId: camera.id,
    assetId: ASSET_ID,
    capturedAt: 10,
    width: 1_920,
    height: 1_080,
    format: "png",
  });
  const document = createEmptyCreativeProjectDocument(PROJECT_ID);
  document.nodes.push({
    id: DIRECTOR_NODE_ID,
    type: "director",
    position: { x: 100, y: 80 },
    size: { width: 360, height: 220 },
    groupId: null,
    zIndex: 2,
    locked: false,
    data: {
      sceneId: "018f7a3c-1239-7abc-8abc-1234567890ab",
      cameraId: camera.id,
      timelineMs: 0,
      durationMs: 10_000,
    },
  });
  return {
    project: project(),
    document,
    directorNodeId: DIRECTOR_NODE_ID,
    sceneAssetId: "018f7a3c-1239-7abc-8abc-1234567890ab",
    state,
  };
}

const repository = (
  save: CreativeProjectRepository["save"],
): CreativeProjectRepository => ({
  list: async () => [],
  create: async () => project(),
  load: async () => ({ project: project(), document: baseline().document }),
  save,
  rename: async () => project(),
  remove: async () => undefined,
});

describe("Director capture to canvas transfer", () => {
  test("places a real image node beside the canonical Director without persisting URLs", () => {
    const plan = planDirectorCapturesForCanvas(baseline(), [
      { captureId: CAPTURE_ID, asset: imageAsset() },
    ]);
    expect(plan.insertedNodes).toHaveLength(1);
    expect(plan.insertedNodes[0].position).toEqual({ x: 540, y: 80 });
    expect(plan.insertedNodes[0].zIndex).toBe(3);
    expect(plan.insertedNodes[0].data).toEqual({
      assetId: ASSET_ID,
      caption: "真实导演截图",
      alt: "真实导演截图",
      fit: "contain",
      naturalSize: { width: 1_920, height: 1_080 },
      composer: null,
    });
    expect(
      plan.document.nodes.find((node) => node.id === DIRECTOR_NODE_ID)?.type,
    ).toBe("director");
    expect(
      JSON.stringify(plan.document).includes("/api/creative-studio/files/"),
    ).toBe(false);
  });

  test("is idempotent per capture asset and does not write when it is already on canvas", async () => {
    const first = planDirectorCapturesForCanvas(baseline(), [
      { captureId: CAPTURE_ID, asset: imageAsset() },
    ]);
    const existing = baseline();
    existing.document = first.document;
    let saveCalls = 0;
    const result = await transferDirectorCapturesToCanvas({
      baseline: existing,
      captures: [{ captureId: CAPTURE_ID, asset: imageAsset() }],
      repository: repository(async () => {
        saveCalls += 1;
        return project("5");
      }),
    });
    expect(result.insertedNodes).toEqual([]);
    expect(result.existingAssetIds).toEqual([ASSET_ID]);
    expect(saveCalls).toBe(0);
    expect(directorCaptureAssetsPresent(result.document, [ASSET_ID])).toBe(
      true,
    );
  });

  test("saves the root document with exact revision CAS and advances the baseline", async () => {
    let expectedRevision = "";
    let savedNodeCount = 0;
    const result = await transferDirectorCapturesToCanvas({
      baseline: baseline(),
      captures: [{ captureId: CAPTURE_ID, asset: imageAsset() }],
      repository: repository(async (_projectId, revision, document) => {
        expectedRevision = revision;
        savedNodeCount = document.nodes.length;
        return { ...project("5"), nodeCount: document.nodes.length };
      }),
    });
    expect(expectedRevision).toBe("4");
    expect(savedNodeCount).toBe(2);
    expect(result.baseline.project.revision).toBe("5");
    expect(result.baseline.document.nodes).toHaveLength(2);
  });

  test("places later captures in the next free grid slot instead of overlapping", () => {
    const SECOND_CAPTURE_ID = "018f7a3c-1242-7abc-8abc-1234567890ab";
    const SECOND_ASSET_ID = "018f7a3c-1243-7abc-8abc-1234567890ab";
    const current = baseline();
    current.state.capture.records.push({
      id: SECOND_CAPTURE_ID,
      kind: "image",
      cameraId: current.state.cameras[0].id,
      assetId: SECOND_ASSET_ID,
      capturedAt: 11,
      width: 1_920,
      height: 1_080,
      format: "png",
    });
    const plan = planDirectorCapturesForCanvas(current, [
      { captureId: CAPTURE_ID, asset: imageAsset() },
      {
        captureId: SECOND_CAPTURE_ID,
        asset: imageAsset({ id: SECOND_ASSET_ID, title: "第二张截图" }),
      },
    ]);
    expect(plan.insertedNodes.map((node) => node.position)).toEqual([
      { x: 540, y: 80 },
      { x: 900, y: 80 },
    ]);
  });

  test("searches below a single oversized node that covers many grid rows", () => {
    const current = baseline();
    current.document.nodes.push({
      id: "018f7a3c-1244-7abc-8abc-1234567890ab",
      type: "group",
      position: { x: 500, y: 80 },
      size: { width: 1_000, height: 2_000 },
      groupId: null,
      zIndex: 3,
      locked: false,
      data: { title: "大型分组", color: null, collapsed: false },
    });
    const plan = planDirectorCapturesForCanvas(current, [
      { captureId: CAPTURE_ID, asset: imageAsset() },
    ]);
    expect(plan.insertedNodes[0].position).toEqual({ x: 540, y: 2_160 });
  });

  test("confirms a committed CAS after response loss without inserting a duplicate", async () => {
    const initial = baseline();
    const committed = planDirectorCapturesForCanvas(initial, [
      { captureId: CAPTURE_ID, asset: imageAsset() },
    ]);
    const authoritative: DirectorProjectBaseline = {
      ...initial,
      project: project("5"),
      document: committed.document,
    };
    const outcome = await transferDirectorCapturesWithReconciliation({
      baseline: initial,
      captures: [{ captureId: CAPTURE_ID, asset: imageAsset() }],
      repository: repository(async () => {
        throw new CreativeProjectRepositoryError({
          kind: "transport",
          message: "response lost",
        });
      }),
      reloadBaseline: async () => authoritative,
    });
    expect(outcome.status).toBe("confirmed-after-response-loss");
    expect(outcome.result.baseline.project.revision).toBe("5");
    expect(outcome.result.document.nodes).toHaveLength(2);
  });

  test("adopts the authoritative baseline and exposes a genuine revision conflict", async () => {
    const authoritative = baseline();
    authoritative.project = project("5");
    const outcome = await transferDirectorCapturesWithReconciliation({
      baseline: baseline(),
      captures: [{ captureId: CAPTURE_ID, asset: imageAsset() }],
      repository: repository(async () => {
        throw new CreativeProjectRepositoryError({
          kind: "revision-conflict",
          message: "stale revision",
          status: 409,
        });
      }),
      reloadBaseline: async () => authoritative,
    });
    expect(outcome.status).toBe("conflict");
    if (outcome.status !== "conflict")
      throw new Error("Expected a revision conflict");
    expect(outcome.result.baseline.project.revision).toBe("5");
    expect(outcome.error.message).toBe("stale revision");
  });

  test("rejects stale capture identity and non-image assets before CAS", () => {
    const stale = () =>
      planDirectorCapturesForCanvas(baseline(), [
        {
          captureId: "018f7a3c-1240-7abc-8abc-1234567890ab",
          asset: imageAsset(),
        },
      ]);
    const mismatch = () =>
      planDirectorCapturesForCanvas(baseline(), [
        {
          captureId: CAPTURE_ID,
          asset: imageAsset({ id: "018f7a3c-1241-7abc-8abc-1234567890ab" }),
        },
      ]);
    const video = () =>
      planDirectorCapturesForCanvas(baseline(), [
        { captureId: CAPTURE_ID, asset: imageAsset({ kind: "video" }) },
      ]);
    for (const [operation, code] of [
      [stale, "capture-not-found"],
      [mismatch, "capture-asset-mismatch"],
      [video, "capture-not-image"],
    ] as const) {
      let error: unknown;
      try {
        operation();
      } catch (cause) {
        error = cause;
      }
      expect(error instanceof DirectorCanvasTransferError).toBe(true);
      expect((error as DirectorCanvasTransferError).code).toBe(code);
    }
  });
});
