/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import type { CreativeAsset, CreativeAssetPort } from "../../assets";
import type { SplitCreativeImageFile } from "./browserSplit";
import { translateCreativeImageTool } from "./imageToolI18n";
import { uploadCreativeImageSplit } from "./uploadSplit";

const SOURCE: CreativeAsset = {
  id: "018f7a3c-1234-7abc-8abc-1234567890ab",
  kind: "image",
  title: "主图",
  collection: null,
  tags: ["source"],
  mimeType: "image/png",
  width: 100,
  height: 100,
  bytes: 100,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: "/source",
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
};

const pieces: SplitCreativeImageFile[] = Array.from(
  { length: 4 },
  (_, index) => ({
    row: Math.floor(index / 2),
    column: index % 2,
    file: new File([String(index)], `piece-${index}.png`, {
      type: "image/png",
    }),
    width: 50,
    height: 50,
    mimeType: "image/png",
  }),
);

const asset = (index: number, tags: string[] = []): CreativeAsset => ({
  ...SOURCE,
  id: `018f7a3c-${String(index).padStart(4, "0")}-7abc-8abc-1234567890ab`,
  title: `piece-${index}`,
  tags,
  originalUrl: `/piece-${index}`,
});

const portWith = (
  overrides: Partial<CreativeAssetPort>,
): CreativeAssetPort => ({
  list: async () => ({ items: [], total: 0 }),
  upload: async () => asset(0),
  update: async () => asset(0),
  remove: async () => undefined,
  url: () => "/asset",
  ...overrides,
});

describe("creative image split upload", () => {
  test("uses bounded concurrency, stable result order, metadata, and monotonic progress", async () => {
    let active = 0;
    let maximumActive = 0;
    let uploadIndex = 0;
    const metadata: unknown[] = [];
    const progress: number[] = [];
    const port = portWith({
      async upload(_file, nextMetadata, _signal, onProgress) {
        const index = uploadIndex;
        uploadIndex += 1;
        active += 1;
        maximumActive = Math.max(maximumActive, active);
        metadata[index] = nextMetadata;
        onProgress?.(50);
        await Promise.resolve();
        active -= 1;
        return asset(index, nextMetadata?.tags ?? []);
      },
    });
    const result = await uploadCreativeImageSplit({
      port,
      source: SOURCE,
      pieces,
      operationId: "018f7a3c-aaaa-7abc-8abc-1234567890ab",
      concurrency: 2,
      onProgress: (percent) => progress.push(percent),
    });
    expect(maximumActive).toBe(2);
    expect(result.map((piece) => [piece.row, piece.column])).toEqual([
      [0, 0],
      [0, 1],
      [1, 0],
      [1, 1],
    ]);
    expect(metadata[0]).toEqual({
      title: "主图 1-1",
      tags: [
        "source",
        "canvas-split",
        "canvas-split-operation:018f7a3c-aaaa-7abc-8abc-1234567890ab:1-1",
      ],
      inLibrary: true,
    });
    expect(
      progress.every(
        (value, index) => index === 0 || value >= progress[index - 1],
      ),
    ).toBe(true);
    expect(progress.at(-1)).toBe(100);
  });

  test("recovers one exact piece after its upload response is lost", async () => {
    const operationId = "018f7a3c-bbbb-7abc-8abc-1234567890ab";
    const tag = `canvas-split-operation:${operationId}:1-1`;
    const port = portWith({
      upload: async () => {
        throw new Error("response lost");
      },
      list: async (query) => ({
        items: [asset(7, [query?.tag ?? ""])],
        total: 1,
      }),
    });
    const result = await uploadCreativeImageSplit({
      port,
      source: SOURCE,
      pieces: pieces.slice(0, 1),
      operationId,
    });
    expect(result[0].asset.tags).toEqual([tag]);
    expect(result[0].recoveredAfterResponseLoss).toBe(true);
  });

  test("rolls back every known uploaded piece after a partial batch failure", async () => {
    let uploadIndex = 0;
    const removed: string[] = [];
    const port = portWith({
      async upload() {
        const index = uploadIndex;
        uploadIndex += 1;
        if (index === 2) throw new Error("third failed");
        return asset(index);
      },
      list: async () => ({ items: [], total: 0 }),
      remove: async (assetId) => {
        removed.push(assetId);
      },
    });
    let failure = "";
    try {
      await uploadCreativeImageSplit({
        port,
        source: SOURCE,
        pieces,
        operationId: "018f7a3c-cccc-7abc-8abc-1234567890ab",
        concurrency: 1,
      });
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
    }
    expect(failure).toBe("third failed");
    expect(removed).toEqual([asset(0).id, asset(1).id]);
  });

  test("reports compensation failures instead of claiming a clean rollback", async () => {
    let uploadIndex = 0;
    const port = portWith({
      async upload() {
        const index = uploadIndex;
        uploadIndex += 1;
        if (index === 1) throw new Error("second failed");
        return asset(index);
      },
      list: async () => ({ items: [], total: 0 }),
      remove: async () => {
        throw new Error("cleanup failed");
      },
    });
    let failure = "";
    try {
      await uploadCreativeImageSplit({
        port,
        source: SOURCE,
        pieces: pieces.slice(0, 2),
        operationId: "018f7a3c-dddd-7abc-8abc-1234567890ab",
        concurrency: 1,
      });
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
    }
    expect(failure).toBe(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.splitCleanupFailedWithCause",
        { message: "second failed", count: 1 },
      ),
    );
  });
});
