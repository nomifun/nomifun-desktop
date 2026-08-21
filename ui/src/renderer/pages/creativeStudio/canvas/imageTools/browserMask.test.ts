/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import type { CreativeAsset } from "../../assets";
import {
  buildCreativeImageMaskReference,
  type CreativeImageMaskCodec,
  type CreativeImageMaskSelection,
} from "./browserMask";

const ASSET: CreativeAsset = {
  id: "018f7a3c-1234-7abc-8abc-1234567890ab",
  kind: "image",
  title: "Director / Capture",
  collection: null,
  tags: [],
  mimeType: "image/jpeg",
  width: 1_920,
  height: 1_080,
  bytes: 100,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: "/api/creative-studio/files/asset",
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
};

const selection = (width = 1_920): CreativeImageMaskSelection => ({
  width,
  height: 1_080,
  source: {} as CanvasImageSource,
});

describe("browser creative image mask adapter", () => {
  test("loads, decodes, marks, closes, and returns a real hidden-reference file", async () => {
    const calls: string[] = [];
    let receivedSelection: CreativeImageMaskSelection | null = null;
    const codec: CreativeImageMaskCodec = {
      async load(url) {
        calls.push(`load:${url}`);
        return new Blob(["source"]);
      },
      async decode() {
        calls.push("decode");
        return {
          width: 1_920,
          height: 1_080,
          source: {} as CanvasImageSource,
          close: () => calls.push("close"),
        };
      },
      async encodeMarked(_image, nextSelection) {
        calls.push("encode");
        receivedSelection = nextSelection;
        return new Blob(["marked"], { type: "image/png" });
      },
    };
    const mask = selection();
    const result = await buildCreativeImageMaskReference({
      asset: ASSET,
      selection: mask,
      codec,
    });

    expect(calls).toEqual([
      `load:${ASSET.originalUrl}`,
      "decode",
      "encode",
      "close",
    ]);
    expect(receivedSelection).toBe(mask);
    expect(result.file instanceof File).toBe(true);
    expect(result.file.name).toBe("Director - Capture-mask-edit-reference.png");
    expect(result.width).toBe(1_920);
    expect(result.height).toBe(1_080);
    expect(result.mimeType).toBe("image/png");
  });

  test("rejects a stale mask size and still closes the decoded source", async () => {
    let closed = false;
    let encoded = false;
    const codec: CreativeImageMaskCodec = {
      load: async () => new Blob(["source"]),
      decode: async () => ({
        width: 1_920,
        height: 1_080,
        source: {} as CanvasImageSource,
        close: () => {
          closed = true;
        },
      }),
      encodeMarked: async () => {
        encoded = true;
        return new Blob();
      },
    };
    let failure: unknown;
    try {
      await buildCreativeImageMaskReference({
        asset: ASSET,
        selection: selection(640),
        codec,
      });
    } catch (error) {
      failure = error;
    }
    expect(failure instanceof Error ? failure.message : "").toBe(
      "局部编辑遮罩尺寸与原图不一致，请重新打开编辑器。",
    );
    expect(encoded).toBe(false);
    expect(closed).toBe(true);
  });

  test("rejects non-image assets before touching the codec", async () => {
    let loaded = false;
    const codec: CreativeImageMaskCodec = {
      load: async () => {
        loaded = true;
        return new Blob();
      },
      decode: async () => {
        throw new Error("unreachable");
      },
      encodeMarked: async () => {
        throw new Error("unreachable");
      },
    };
    let failure: unknown;
    try {
      await buildCreativeImageMaskReference({
        asset: { ...ASSET, kind: "video" },
        selection: selection(),
        codec,
      });
    } catch (error) {
      failure = error;
    }
    expect(failure instanceof Error ? failure.message : "").toBe(
      "只有真实图片素材可以进行局部编辑。",
    );
    expect(loaded).toBe(false);
  });
});
