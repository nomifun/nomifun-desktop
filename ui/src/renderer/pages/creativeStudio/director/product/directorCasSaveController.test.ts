/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import { createEmptyCreativeProjectDocument } from "../../domain";
import { CreativeProjectRepositoryError } from "../../services";
import {
  createDirectorState,
  directorCommands,
  directorReducer,
} from "../domain";
import { DirectorCasSaveController } from "./directorCasSaveController";
import type { DirectorProjectBaseline } from "./directorProjectPersistence";

const PROJECT_ID = "018f7a3c-1234-7abc-8abc-1234567890ab";

const baseline = (): DirectorProjectBaseline => ({
  project: {
    projectId: PROJECT_ID,
    title: "导演项目",
    revision: "1",
    nodeCount: 0,
    connectionCount: 0,
    createdAt: 1,
    updatedAt: 1,
  },
  document: createEmptyCreativeProjectDocument(PROJECT_ID),
  directorNodeId: null,
  sceneAssetId: null,
  state: createDirectorState({ projectId: PROJECT_ID, name: "导演项目" }),
});

describe("DirectorCasSaveController", () => {
  test("persists changed canonical state and advances the revision", async () => {
    const initial = baseline();
    const controller = new DirectorCasSaveController(
      async (current, state) => ({
        ...current,
        project: { ...current.project, revision: "2" },
        state,
      }),
      { debounceMs: 60_000 },
    );
    controller.reset(initial);
    controller.queue(
      directorReducer(initial.state, directorCommands.renameScene("新场景")),
    );

    expect(controller.getSnapshot().status).toBe("dirty");
    expect(await controller.flush()).toEqual({
      status: "saved",
      revision: "2",
    });
    expect(controller.getSnapshot()).toMatchObject({
      status: "saved",
      revision: "2",
      hasPendingChanges: false,
    });
    controller.dispose();
  });

  test("blocks further writes after an explicit revision conflict", async () => {
    const initial = baseline();
    let calls = 0;
    const controller = new DirectorCasSaveController(
      async () => {
        calls += 1;
        throw new CreativeProjectRepositoryError({
          kind: "revision-conflict",
          message: "remote changed",
          status: 409,
        });
      },
      { debounceMs: 60_000 },
    );
    controller.reset(initial);
    controller.queue(
      directorReducer(initial.state, directorCommands.renameScene("冲突场景")),
    );

    const first = await controller.flush();
    const second = await controller.flush();
    expect(first.status).toBe("conflict");
    expect(second.status).toBe("conflict");
    expect(calls).toBe(1);
    controller.dispose();
  });

  test("does not persist runtime-only playing state", async () => {
    const initial = baseline();
    const controller = new DirectorCasSaveController(async () => {
      throw new Error("should not save");
    });
    controller.reset(initial);
    controller.queue(
      directorReducer(initial.state, directorCommands.setTimelinePlaying(true)),
    );
    expect(controller.getSnapshot().hasPendingChanges).toBe(false);
    expect(await controller.flush()).toEqual({ status: "noop", revision: "1" });
    controller.dispose();
  });
});
