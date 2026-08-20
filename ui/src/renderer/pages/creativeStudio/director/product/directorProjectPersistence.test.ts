/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import type {
  CreateCreativeTextAsset,
  CreativeAsset,
  CreativeAssetLibraryPort,
  CreativeAssetPage,
  CreativeAssetPatch,
  CreativeAssetQuery,
} from "../../assets";
import {
  createEmptyCreativeProjectDocument,
  type CreativeProjectDetail,
  type CreativeProjectSummary,
} from "../../domain";
import type { CreativeProjectRepository } from "../../services";
import {
  createDirectorCamera,
  createDirectorState,
  directorCommands,
  directorReducer,
  exportDirectorProjectV1,
} from "../domain";
import {
  DirectorProjectLoadError,
  loadDirectorProjectBaseline,
  persistDirectorProject,
} from "./directorProjectPersistence";

const PROJECT_ID = "018f7a3c-1234-7abc-8abc-1234567890ab";
const SCENE_ASSET_ID = "018f7a3c-1235-7abc-8abc-1234567890ab";
const NEXT_SCENE_ASSET_ID = "018f7a3c-1236-7abc-8abc-1234567890ab";

const summary = (revision = "1"): CreativeProjectSummary => ({
  projectId: PROJECT_ID,
  title: "导演项目",
  revision,
  nodeCount: 0,
  connectionCount: 0,
  createdAt: 1,
  updatedAt: 1,
});

const detail = (): CreativeProjectDetail => ({
  project: summary(),
  document: createEmptyCreativeProjectDocument(PROJECT_ID),
});

const asset = (id: string, textContent: string): CreativeAsset => ({
  id,
  kind: "text",
  title: "导演场景",
  collection: null,
  tags: ["nomifun-director-v1"],
  mimeType: "text/plain",
  width: null,
  height: null,
  bytes: textContent.length,
  inLibrary: false,
  textContent,
  origin: null,
  originalUrl: `/assets/${id}`,
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
});

class FakeAssets implements CreativeAssetLibraryPort {
  readonly items = new Map<string, CreativeAsset>();
  readonly removed: string[] = [];
  nextId = NEXT_SCENE_ASSET_ID;

  async list(_query?: CreativeAssetQuery): Promise<CreativeAssetPage> {
    const items = [...this.items.values()];
    return { items, total: items.length };
  }

  async createText(input: CreateCreativeTextAsset): Promise<CreativeAsset> {
    const created = asset(this.nextId, input.textContent);
    this.items.set(created.id, created);
    return created;
  }

  async upload(): Promise<CreativeAsset> {
    throw new Error("not used");
  }

  async update(
    _assetId: string,
    _patch: CreativeAssetPatch,
  ): Promise<CreativeAsset> {
    throw new Error("not used");
  }

  async remove(assetId: string): Promise<void> {
    this.removed.push(assetId);
    this.items.delete(assetId);
  }

  async renameCollection(): Promise<number> {
    return 0;
  }

  url(assetId: string): string {
    return `/assets/${assetId}`;
  }
}

function repository(
  onSave: CreativeProjectRepository["save"],
): CreativeProjectRepository {
  return {
    list: async () => [],
    create: async () => summary(),
    load: async () => detail(),
    save: onSave,
    rename: async () => summary(),
    remove: async () => undefined,
  };
}

describe("Director project persistence", () => {
  test("opens an empty real Director state without inventing a sidecar or node", async () => {
    const assets = new FakeAssets();
    const baseline = await loadDirectorProjectBaseline(detail(), assets);

    expect(baseline.directorNodeId).toBeNull();
    expect(baseline.sceneAssetId).toBeNull();
    expect(baseline.state.projectId).toBe(PROJECT_ID);
    expect(baseline.state.cameras).toEqual([]);
    expect(assets.items.size).toBe(0);
  });

  test("stores v1 scene JSON in a real text asset and advances the root pointer by CAS", async () => {
    const assets = new FakeAssets();
    const baseline = await loadDirectorProjectBaseline(detail(), assets);
    const camera = createDirectorCamera({ id: "camera-1", name: "主机位" });
    const state = directorReducer(
      baseline.state,
      directorCommands.addEntity(camera),
    );
    let savedRevision = "";
    let savedSceneId: string | null = null;
    const repo = repository(async (_projectId, expectedRevision, document) => {
      savedRevision = expectedRevision;
      const node = document.nodes.find(
        (candidate) => candidate.type === "director",
      );
      savedSceneId = node?.type === "director" ? node.data.sceneId : null;
      return { ...summary("2"), nodeCount: document.nodes.length };
    });

    const persisted = await persistDirectorProject({
      baseline,
      state,
      repository: repo,
      assets,
    });

    expect(savedRevision).toBe("1");
    expect(savedSceneId).toBe(NEXT_SCENE_ASSET_ID);
    expect(persisted.project.revision).toBe("2");
    expect(persisted.directorNodeId).not.toBeNull();
    const scene = assets.items.get(NEXT_SCENE_ASSET_ID);
    expect(scene?.inLibrary).toBe(false);
    expect(
      scene?.textContent?.includes('"kind": "nomifun.director.project"'),
    ).toBe(true);
  });

  test("reloads only when the node projection and sidecar agree", async () => {
    const assets = new FakeAssets();
    const state = createDirectorState({
      projectId: PROJECT_ID,
      name: "导演项目",
    });
    const camera = createDirectorCamera({ id: "camera-1", name: "主机位" });
    const withCamera = directorReducer(
      state,
      directorCommands.addEntity(camera),
    );
    const exported = exportDirectorProjectV1(withCamera);
    if (!exported.ok) throw new Error(exported.error.message);
    assets.items.set(SCENE_ASSET_ID, asset(SCENE_ASSET_ID, exported.json));
    const project = detail();
    project.document.nodes.push({
      id: "director-1",
      type: "director",
      position: { x: 0, y: 0 },
      size: { width: 360, height: 220 },
      groupId: null,
      zIndex: 0,
      locked: false,
      data: {
        sceneId: SCENE_ASSET_ID,
        cameraId: withCamera.activeCameraId,
        timelineMs: 0,
        durationMs: 10_000,
      },
    });

    const baseline = await loadDirectorProjectBaseline(project, assets);
    expect(baseline.state.activeCameraId).toBe(camera.id);

    const node = project.document.nodes[0];
    if (node.type !== "director") throw new Error("expected director");
    node.data.timelineMs = 1_000;
    let projectionError: unknown;
    try {
      await loadDirectorProjectBaseline(project, assets);
    } catch (error) {
      projectionError = error;
    }
    expect(projectionError instanceof DirectorProjectLoadError).toBe(true);
    expect((projectionError as DirectorProjectLoadError).code).toBe(
      "projection-mismatch",
    );
  });

  test("refuses ambiguous projects with multiple director nodes", async () => {
    const assets = new FakeAssets();
    const project = detail();
    const node = {
      id: "director-1",
      type: "director" as const,
      position: { x: 0, y: 0 },
      size: { width: 360, height: 220 },
      groupId: null,
      zIndex: 0,
      locked: false,
      data: { sceneId: null, cameraId: null, timelineMs: 0, durationMs: 0 },
    };
    project.document.nodes.push(node, { ...node, id: "director-2", zIndex: 1 });

    let ambiguityError: unknown;
    try {
      await loadDirectorProjectBaseline(project, assets);
    } catch (error) {
      ambiguityError = error;
    }
    expect(ambiguityError instanceof DirectorProjectLoadError).toBe(true);
    expect((ambiguityError as DirectorProjectLoadError).code).toBe(
      "multiple-directors",
    );
  });
});
