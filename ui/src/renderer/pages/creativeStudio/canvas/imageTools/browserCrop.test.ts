/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import type { CreativeAsset } from "../../assets";
import {
  cropCreativeImageAsset,
  type CreativeImageCropCodec,
} from "./browserCrop";
import { translateCreativeImageTool } from "./imageToolI18n";

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

describe("browser creative image crop adapter", () => {
  test("loads, decodes, crops, closes, and returns a real upload file", async () => {
    const calls: string[] = [];
    let encodedCrop: unknown;
    const codec: CreativeImageCropCodec = {
      async load(url) {
        calls.push(`load:${url}`);
        return new Blob(["source"], { type: "image/jpeg" });
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
      async encode(_image, crop, mimeType) {
        calls.push(`encode:${mimeType}`);
        encodedCrop = crop;
        return new Blob(["cropped"], { type: mimeType });
      },
    };
    const result = await cropCreativeImageAsset({
      asset: ASSET,
      crop: { x: 0.25, y: 0.25, width: 0.5, height: 0.5 },
      codec,
    });
    expect(calls).toEqual([
      `load:${ASSET.originalUrl}`,
      "decode",
      "encode:image/jpeg",
      "close",
    ]);
    expect(encodedCrop).toEqual({ x: 480, y: 270, width: 960, height: 540 });
    expect(result.file instanceof File).toBe(true);
    expect(result.file.name).toBe(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.fileNames.crop",
        { stem: "Director - Capture", extension: "jpg" },
      ),
    );
    expect(result.width).toBe(960);
    expect(result.height).toBe(540);
  });

  test("closes decoded image resources when encoding fails", async () => {
    let closed = false;
    const codec: CreativeImageCropCodec = {
      load: async () => new Blob(["source"]),
      decode: async () => ({
        width: 100,
        height: 100,
        source: {} as CanvasImageSource,
        close: () => {
          closed = true;
        },
      }),
      encode: async () => {
        throw new Error("encode failed");
      },
    };
    let failure: unknown;
    try {
      await cropCreativeImageAsset({
        asset: ASSET,
        crop: { x: 0, y: 0, width: 1, height: 1 },
        codec,
      });
    } catch (error) {
      failure = error;
    }
    expect(failure instanceof Error ? failure.message : "").toBe(
      "encode failed",
    );
    expect(closed).toBe(true);
  });

  test("rejects non-image assets before touching the codec", async () => {
    let loaded = false;
    const codec: CreativeImageCropCodec = {
      load: async () => {
        loaded = true;
        return new Blob();
      },
      decode: async () => {
        throw new Error("unreachable");
      },
      encode: async () => {
        throw new Error("unreachable");
      },
    };
    let failure: unknown;
    try {
      await cropCreativeImageAsset({
        asset: { ...ASSET, kind: "video" },
        crop: { x: 0, y: 0, width: 1, height: 1 },
        codec,
      });
    } catch (error) {
      failure = error;
    }
    expect(failure instanceof Error ? failure.message : "").toBe(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.imageRequiredForCrop",
      ),
    );
    expect(loaded).toBe(false);
  });
});
