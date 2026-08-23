/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";

import {
  CREATIVE_IMAGE_DEFAULT_SPLIT,
  addCreativeImageSplitLine,
  buildCreativeImageSplitLines,
  creativeImageSplitPieces,
  creativeImageSplitTotal,
  moveCreativeImageSplitLine,
  removeCreativeImageSplitLine,
  resetCreativeImageSplitLines,
  setCreativeImageSplitCount,
} from "./splitModel";
import { translateCreativeImageTool } from "./imageToolI18n";

describe("creative image split model", () => {
  test("builds and clamps evenly spaced 1-12 grids", () => {
    expect(buildCreativeImageSplitLines(1)).toEqual([]);
    expect(buildCreativeImageSplitLines(4)).toEqual([0.25, 0.5, 0.75]);
    expect(buildCreativeImageSplitLines(99).length).toBe(11);
    expect(buildCreativeImageSplitLines(Number.NaN)).toEqual([]);
  });

  test("changes row and column counts independently", () => {
    const rows = setCreativeImageSplitCount(
      CREATIVE_IMAGE_DEFAULT_SPLIT,
      "horizontal",
      3,
    );
    const grid = setCreativeImageSplitCount(rows, "vertical", 4);
    expect(grid.horizontalLines).toEqual([1 / 3, 2 / 3]);
    expect(grid.verticalLines).toEqual([0.25, 0.5, 0.75]);
    expect(creativeImageSplitTotal(grid)).toBe(12);
  });

  test("adds a line in the largest gap and resets it evenly", () => {
    const added = addCreativeImageSplitLine(
      { horizontalLines: [0.2, 0.4], verticalLines: [] },
      "horizontal",
    );
    expect(added.horizontalLines).toEqual([0.2, 0.4, 0.7]);
    expect(resetCreativeImageSplitLines(added).horizontalLines).toEqual([
      0.25, 0.5, 0.75,
    ]);
  });

  test("moves selected lines without crossing neighbors and deletes exactly one", () => {
    const params = { horizontalLines: [0.25, 0.5, 0.75], verticalLines: [] };
    const left = moveCreativeImageSplitLine(params, "horizontal", 1, 0.1);
    expect(left.horizontalLines[1]).toBe(0.26);
    const right = moveCreativeImageSplitLine(params, "horizontal", 1, 0.99);
    expect(right.horizontalLines[1]).toBe(0.74);
    expect(
      removeCreativeImageSplitLine(right, "horizontal", 1).horizontalLines,
    ).toEqual([0.25, 0.75]);
  });

  test("partitions every decoded pixel once in row-major order", () => {
    const pieces = creativeImageSplitPieces(
      { horizontalLines: [0.4], verticalLines: [0.25, 0.75] },
      { width: 1_920, height: 1_080 },
    );
    expect(pieces).toEqual([
      { row: 0, column: 0, crop: { x: 0, y: 0, width: 480, height: 432 } },
      { row: 0, column: 1, crop: { x: 480, y: 0, width: 960, height: 432 } },
      { row: 0, column: 2, crop: { x: 1_440, y: 0, width: 480, height: 432 } },
      { row: 1, column: 0, crop: { x: 0, y: 432, width: 480, height: 648 } },
      { row: 1, column: 1, crop: { x: 480, y: 432, width: 960, height: 648 } },
      {
        row: 1,
        column: 2,
        crop: { x: 1_440, y: 432, width: 480, height: 648 },
      },
    ]);
    expect(
      pieces.reduce(
        (sum, piece) => sum + piece.crop.width * piece.crop.height,
        0,
      ),
    ).toBe(1_920 * 1_080);
  });

  test("rejects pixel-collapsed cuts instead of creating empty files", () => {
    let message = "";
    try {
      creativeImageSplitPieces(
        { horizontalLines: [], verticalLines: [0.01, 0.02] },
        { width: 10, height: 10 },
      );
    } catch (error) {
      message = error instanceof Error ? error.message : String(error);
    }
    expect(message).toBe(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.sliceDimensionTooSmall",
        {
          label: translateCreativeImageTool(
            "creativeStudio.canvas.imageTools.dimensions.imageWidth",
          ),
        },
      ),
    );
  });
});
