/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import type { CreativeAsset, CreativeAssetPort } from "../../assets";
import {
  removeCreativeImageMaskReference,
  uploadCreativeImageMaskReference,
} from "./uploadMaskReference";

const SOURCE: CreativeAsset = {
  id: "018f7a3c-1234-7abc-8abc-1234567890ab",
  kind: "image",
  title: "原图",
  collection: "项目素材",
  tags: ["source"],
  mimeType: "image/png",
  width: 1_920,
  height: 1_080,
  bytes: 10,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: "/source",
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
};

const uploaded = (tags: string[]): CreativeAsset => ({
  ...SOURCE,
  id: "018f7a3c-5678-7abc-8abc-1234567890ab",
  title: "原图 · 局部编辑标记",
  tags,
  inLibrary: false,
  originalUrl: "/marked",
});

const portWith = (
  overrides: Partial<CreativeAssetPort>,
): CreativeAssetPort => ({
  list: async () => ({ items: [], total: 0 }),
  upload: async () => uploaded([]),
  update: async () => uploaded([]),
  remove: async () => undefined,
  url: () => "/asset",
  ...overrides,
});

describe("creative image mask reference upload", () => {
  test("uploads a hidden durable reference with the source metadata and operation tag", async () => {
    let metadata: Parameters<CreativeAssetPort["upload"]>[1];
    const port = portWith({
      async upload(_file, nextMetadata) {
        metadata = nextMetadata;
        return uploaded(nextMetadata?.tags ?? []);
      },
    });
    const result = await uploadCreativeImageMaskReference({
      port,
      source: SOURCE,
      file: new File(["marked"], "marked.png", { type: "image/png" }),
      operationId: "018f7a3c-aaaa-7abc-8abc-1234567890ab",
    });
    expect(result.recoveredAfterResponseLoss).toBe(false);
    expect(metadata).toEqual({
      title: "原图 · 局部编辑标记",
      collection: "项目素材",
      tags: [
        "source",
        "canvas-mask-edit-reference",
        "canvas-mask-edit-operation:018f7a3c-aaaa-7abc-8abc-1234567890ab",
      ],
      inLibrary: false,
    });
  });

  test("reconciles the exact hidden asset after an upload response is lost", async () => {
    const tag =
      "canvas-mask-edit-operation:018f7a3c-bbbb-7abc-8abc-1234567890ab";
    const port = portWith({
      upload: async () => {
        throw new Error("network response lost");
      },
      list: async (query) => {
        expect(query).toMatchObject({
          kind: "image",
          tag,
          inLibrary: false,
        });
        return { items: [uploaded([tag])], total: 1 };
      },
    });
    const result = await uploadCreativeImageMaskReference({
      port,
      source: SOURCE,
      file: new File(["marked"], "marked.png", { type: "image/png" }),
      operationId: "018f7a3c-bbbb-7abc-8abc-1234567890ab",
    });
    expect(result.asset.id).toBe("018f7a3c-5678-7abc-8abc-1234567890ab");
    expect(result.recoveredAfterResponseLoss).toBe(true);
  });

  test("does not reconcile an aborted upload and can explicitly compensate", async () => {
    let listed = false;
    let removed: string | null = null;
    const aborted = Object.assign(new Error("aborted"), {
      name: "AbortError",
    });
    const port = portWith({
      upload: async () => {
        throw aborted;
      },
      list: async () => {
        listed = true;
        return { items: [], total: 0 };
      },
      remove: async (assetId) => {
        removed = assetId;
      },
    });
    let failure: unknown;
    try {
      await uploadCreativeImageMaskReference({
        port,
        source: SOURCE,
        file: new File(["marked"], "marked.png", { type: "image/png" }),
        operationId: "018f7a3c-cccc-7abc-8abc-1234567890ab",
      });
    } catch (error) {
      failure = error;
    }
    expect(failure).toBe(aborted);
    expect(listed).toBe(false);

    await removeCreativeImageMaskReference(
      port,
      uploaded(["canvas-mask-edit-operation:verified"]),
    );
    expect(removed).toBe("018f7a3c-5678-7abc-8abc-1234567890ab");

    let cleanupFailure: unknown;
    try {
      await removeCreativeImageMaskReference(port, SOURCE);
    } catch (error) {
      cleanupFailure = error;
    }
    expect(cleanupFailure instanceof Error ? cleanupFailure.message : "").toBe(
      "拒绝清理未验证的局部编辑参考素材。",
    );
  });
});
