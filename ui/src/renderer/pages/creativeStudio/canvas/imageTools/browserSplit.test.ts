/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import type { CreativeAsset } from "../../assets";
import type { CreativeImageCropCodec } from "./browserCrop";
import { splitCreativeImageAsset } from "./browserSplit";

const ASSET: CreativeAsset = {
  id: "018f7a3c-1234-7abc-8abc-1234567890ab",
  kind: "image",
  title: "海报/主图",
  collection: null,
  tags: [],
  mimeType: "image/png",
  width: 1_200,
  height: 800,
  bytes: 100,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: "/source",
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
};

describe("browser creative image split adapter", () => {
  test("loads and decodes once, then creates exact row-major upload files", async () => {
    const crops: unknown[] = [];
    let closed = 0;
    const codec: CreativeImageCropCodec = {
      load: async () => new Blob(["source"]),
      decode: async () => ({
        width: 1_200,
        height: 800,
        source: {} as CanvasImageSource,
        close: () => {
          closed += 1;
        },
      }),
      encode: async (_image, crop, mimeType) => {
        crops.push(crop);
        return new Blob([`${crop.x},${crop.y}`], { type: mimeType });
      },
    };
    const result = await splitCreativeImageAsset({
      asset: ASSET,
      params: { horizontalLines: [0.5], verticalLines: [0.25, 0.75] },
      codec,
    });
    expect(crops).toEqual([
      { x: 0, y: 0, width: 300, height: 400 },
      { x: 300, y: 0, width: 600, height: 400 },
      { x: 900, y: 0, width: 300, height: 400 },
      { x: 0, y: 400, width: 300, height: 400 },
      { x: 300, y: 400, width: 600, height: 400 },
      { x: 900, y: 400, width: 300, height: 400 },
    ]);
    expect(result.map((piece) => piece.file.name)).toEqual([
      "海报-主图-1-1.png",
      "海报-主图-1-2.png",
      "海报-主图-1-3.png",
      "海报-主图-2-1.png",
      "海报-主图-2-2.png",
      "海报-主图-2-3.png",
    ]);
    expect(
      result.map(({ row, column, width, height }) => ({
        row,
        column,
        width,
        height,
      })),
    ).toEqual([
      { row: 0, column: 0, width: 300, height: 400 },
      { row: 0, column: 1, width: 600, height: 400 },
      { row: 0, column: 2, width: 300, height: 400 },
      { row: 1, column: 0, width: 300, height: 400 },
      { row: 1, column: 1, width: 600, height: 400 },
      { row: 1, column: 2, width: 300, height: 400 },
    ]);
    expect(closed).toBe(1);
  });

  test("closes the decoded source when one piece fails to encode", async () => {
    let closed = false;
    let call = 0;
    const codec: CreativeImageCropCodec = {
      load: async () => new Blob(),
      decode: async () => ({
        width: 100,
        height: 100,
        source: {} as CanvasImageSource,
        close: () => {
          closed = true;
        },
      }),
      encode: async () => {
        call += 1;
        if (call === 2) throw new Error("encode failed");
        return new Blob();
      },
    };
    let failure = "";
    try {
      await splitCreativeImageAsset({
        asset: ASSET,
        params: { horizontalLines: [0.5], verticalLines: [0.5] },
        codec,
      });
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
    }
    expect(failure).toBe("encode failed");
    expect(closed).toBe(true);
  });
});
